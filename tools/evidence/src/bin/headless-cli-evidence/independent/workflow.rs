//! Two real daemon processes: the origin owns workflow truth; the serving host owns execution.
use super::{
    Arguments, ArtifactId, ArtifactStore, CausalReference, CliRunner, EvidenceResult, PageSize,
    Path, RunQueryStore, RunSummaryFilter, RunSummaryPageQuery, TOKEN, Value, download, ensure, fs,
    json, path_text, reserve_endpoint, setup, start_daemon, upload, wait_for_readiness,
    write_private,
};
use milkdrift_blueprint::{
    AuthorRef, BindingSource, BlueprintRevision, BlueprintRevisionDocument, DataPort, Mutation,
    MutationBatch, Node, NodeId, NodeKind, PortId, SchemaRef, TerminalOutcome, WorkflowId,
};
use milkdrift_capability::{
    CapabilityId, CapabilityRequirement, OperationId, PeerId, SchemaId, SideEffectClass,
};
use milkdrift_evidence::application::required_text;
use milkdrift_peer_protocol::PeerRequestId;
use milkdrift_persistence::{PeerExecutionSnapshot, PeerExecutionStore};
use std::sync::atomic::Ordering;

pub(super) fn run(
    arguments: &Arguments,
    serving_directory: &Path,
    serving_config: &Value,
    serving_runner: &CliRunner,
    process_bytes: &[u8],
    task: &Value,
    endpoint: &setup::MockModel,
) -> EvidenceResult {
    let origin = tempfile::tempdir()?;
    let directory = origin.path();
    let origin_endpoint = reserve_endpoint()?;
    let target = "independent-evidence-host";
    let source = "workflow-evidence-host";
    let mut config = serving_config.clone();
    config["secret_sources"]["credential:peer"] = json!({"type":"file","path":"peer.token"});
    write_private(&serving_directory.join("peer.token"), b"binary-peer-token")?;
    write_private(&directory.join("peer.token"), b"binary-peer-token")?;
    let relationship = json!({
        "peer_id":source,"endpoint":format!("http://{origin_endpoint}/"),"credential_ref":"credential:peer",
        "insecure_loopback_development":true,"actions":["read_catalog","invoke","cancel","artifact_upload","artifact_download"],
        "capability_allow":["independent-process","operator-model"],"capability_deny":[],"operation_allow":["process.execute","model.generate"],
        "maximum_side_effect":"unknown","execution_filesystem":config["actors"][0]["authority"]["resources"]["filesystem"],
        "execution_network_profiles":["local-model-loopback"],"execution_network_destinations":config["actors"][0]["authority"]["resources"]["network"]["destinations"],
        "maximum_artifact_bytes":16777216,"artifact_sensitivities":["public","internal","restricted"],"maximum_duration_ms":300000,
        "maximum_input_units":65536,"maximum_output_units":4096,"maximum_observations":4096,"maximum_requests_per_minute":10000,
        "catalog_ttl_ms":300000,"trust_zone":"binary-peer","delegation_ref":"delegation:binary-peer","expires_at_unix_ms":4102444800000_u64
    });
    config["peers"] = json!({"mode":"enabled","relationships":[relationship]});
    fs::remove_file(serving_directory.join("daemon.toml"))?;
    let serving_path = setup::checked_config(serving_directory, &arguments.daemon, &config)?;
    let mut origin_config = config.clone();
    origin_config["host_id"] = json!(source);
    origin_config["role"] = json!("workflow_enabled");
    origin_config["runtime"]["controller_activation"] = json!("enabled");
    origin_config["bind"] = json!(origin_endpoint.to_string());
    origin_config["adapters"] = json!({"process_profiles":[],"model_profiles":[]});
    origin_config["peers"]["relationships"][0]["peer_id"] = json!(target);
    origin_config["peers"]["relationships"][0]["endpoint"] = json!(serving_runner.endpoint);
    origin_config["actors"][0]["authority"]["resources"]["peers"] =
        json!({"identities":[target],"allow_any":false});
    origin_config["actors"][0]["authority"]["resources"]["network"] =
        json!({"profiles":[format!("peer:{target}")],"destinations":[config["bind"]]});
    let token = write_private(&directory.join("operator.token"), TOKEN.as_bytes())?;
    let origin_path = setup::checked_config(directory, &arguments.daemon, &origin_config)?;
    let runner = CliRunner {
        executable: arguments.cli.clone(),
        endpoint: format!("http://{origin_endpoint}/"),
        token_file: token,
        forbidden_storage_path: directory.join("data"),
    };
    let mut serving = start_daemon(&arguments.daemon, &serving_path)?;
    wait_for_readiness(serving_runner, &mut serving)?;
    let mut daemon = start_daemon(&arguments.daemon, &origin_path)?;
    wait_for_readiness(&runner, &mut daemon)?;
    measure_idle("execution_only", serving.id())?;
    measure_idle("workflow_enabled", daemon.id())?;
    runner.success(&["peer", "connect", target])?;
    let process = upload(
        &runner,
        directory,
        "remote-process-input",
        "text/plain",
        process_bytes,
    )?;
    let model = upload(
        &runner,
        directory,
        "remote-model-input",
        "application/json",
        &serde_json::to_vec(task)?,
    )?;
    let mut origins = Vec::new();
    let mut uncertain_root = None;
    for (name, capability, operation, input_name, input, output_name) in [
        (
            "process",
            "independent-process",
            "process.execute",
            "source",
            process,
            "stdout",
        ),
        (
            "model",
            "operator-model",
            "model.generate",
            "milkdrift.model_task",
            model.clone(),
            "final_text",
        ),
        (
            "uncertain-model",
            "operator-model",
            "model.generate",
            "milkdrift.model_task",
            model,
            "final_text",
        ),
    ] {
        if name == "uncertain-model" {
            endpoint.disconnect_next.store(true, Ordering::SeqCst);
        }
        let capabilities = runner.success(&["capability", "list"])?;
        let registered = capabilities["value"]
            .as_array()
            .and_then(|items| {
                items.iter().find(|item| {
                    item["capability_id"]
                        .as_str()
                        .is_some_and(|id| id.starts_with("peer:") && id.ends_with(capability))
                })
            })
            .ok_or_else(|| format!("remote capability missing: {capabilities}"))?;
        let remote = required_text(registered, &["capability_id"])?;
        let workflow = format!("remote-{name}");
        let revision = blueprint(
            &workflow,
            &remote,
            operation,
            input_name,
            &input,
            output_name,
        )?;
        let path = directory.join(format!("{workflow}.json"));
        fs::write(
            &path,
            BlueprintRevisionDocument::new(&revision).to_canonical_json()?,
        )?;
        runner.success(&[
            "--command-id",
            &format!("import-{name}"),
            "blueprint",
            "import",
            path_text(&path)?,
        ])?;
        let controlled = controlled(&revision)?;
        let root = controlled.semantic().workflow().as_str();
        let root_path = directory.join(format!("{root}.json"));
        fs::write(
            &root_path,
            BlueprintRevisionDocument::new(&controlled).to_canonical_json()?,
        )?;
        runner.success(&[
            "--command-id",
            &format!("import-controller-{name}"),
            "blueprint",
            "import",
            path_text(&root_path)?,
        ])?;
        runner.success(&[
            "--command-id",
            &format!("start-{name}"),
            "run",
            "start",
            root,
            root,
            controlled.id().as_str(),
        ])?;
        let waited = runner.run(
            &[
                "--timeout-secs",
                "8",
                "run",
                "wait",
                root,
                "--terminal",
                "succeeded",
            ],
            None,
        )?;
        let runs = runner.success(&["run", "list", "--limit", "100"])?;
        let children = runs["value"]["items"]
            .as_array()
            .ok_or("run list absent")?
            .iter()
            .filter(|run| run["workflow_id"] == workflow)
            .collect::<Vec<_>>();
        ensure(
            children.len() == 1,
            &format!("controller did not create one child: {runs}"),
        )?;
        let workflow = required_text(children[0], &["run_id"])?;
        // An uncertain attempt is not terminal settlement: its complete reservation stays
        // outstanding. Observe the child boundary, not an invented controller completion.
        let completed = milkdrift_evidence::application::wait_for_run(
            &runner,
            if name == "uncertain-model" {
                &workflow
            } else {
                root
            },
            std::time::Duration::from_secs(45),
            |read| {
                !read["value"]["terminal"].is_null()
                    || read["value"]["uncertainty_count"]
                        .as_u64()
                        .is_some_and(|count| count > 0)
            },
        );
        let controlled_read = runner.success(&["run", "show", root])?;
        let account = &controlled_read["value"]["controller_accounting"];
        let run = runner.success(&["run", "show", &workflow])?;
        let node = run["value"]["nodes"]
            .as_array()
            .and_then(|nodes| nodes.iter().find(|node| node["node_id"] == "operation"))
            .ok_or("remote task node missing")?;
        let attempt = required_text(node, &["latest_attempt_id"])?;
        let inspected = runner.success(&["attempt", "inspect", &workflow, &attempt])?;
        let invocation = required_text(&inspected, &["value", "invocation_id"])?;
        let completed = match completed {
            Ok(completed) => completed,
            Err(error) => {
                daemon.terminate()?;
                serving.terminate()?;
                let store = milkdrift_redb_store::RedbStore::open(serving_directory.join("data"))?;
                let caller = milkdrift_peer_protocol::ServingCaller::peer(
                    &PeerId::new(target)?,
                    &PeerId::new(source)?,
                );
                let retained = store.peer_execution_by_request(
                    &caller,
                    &PeerRequestId::new(format!("request:{invocation}"))?,
                )?;
                return Err(format!("{error}; child={run}; attempt={inspected}; model requests={}; serving={retained:?}", endpoint.invocations.load(Ordering::SeqCst)).into());
            }
        };
        if name == "uncertain-model" {
            ensure(
                inspected["value"]["uncertain"] == true
                    && account["account"]["outstanding"]["input_units"] == 65536
                    && account["account"]["outstanding"]["output_units"] == 4096
                    && account["account"]["outstanding"]["artifact_bytes"] == 16777216
                    && account["account"]["settled"]["model_admissions"] == 1
                    && account["account"]["reservations"]
                        .as_object()
                        .is_some_and(|items| items.len() == 1),
                &format!(
                    "lost response released or concealed its obligation: {inspected}; account={account}"
                ),
            )?;
            uncertain_root = Some((root.to_owned(), account.clone()));
            origins.push((workflow, invocation, None));
            continue;
        }
        if completed["value"]["terminal"] != "succeeded" {
            return Err(format!(
                "remote {name} failed: {}; attempt={inspected}; account={account}",
                waited.stdout
            )
            .into());
        }
        let output = inspected["value"]["outputs"]
            .as_array()
            .and_then(|outputs| outputs.iter().find(|output| output["name"] == output_name))
            .ok_or("remote output was not imported")?;
        let artifact = required_text(output, &["artifact", "artifact_id"])?;
        let bytes = download(&runner, directory, &artifact, &format!("{name}-result"))?;
        ensure(
            account["state"] == "active"
                && account["committed"][if name == "process" {
                    "process_admissions"
                } else {
                    "model_admissions"
                }] == 1
                && account["committed"]["artifact_bytes"]
                    .as_u64()
                    .is_some_and(|charged| charged >= bytes.len() as u64)
                && account["account"]["reservations"]
                    .as_object()
                    .is_some_and(|reservations| reservations.is_empty()),
            &format!("remote result did not settle the origin's reservation: {account}"),
        )?;
        ensure(
            if name == "process" {
                bytes == process_bytes
            } else {
                String::from_utf8(bytes)?.contains("independent model result")
            },
            "remote operation returned different bytes",
        )?;
        origins.push((workflow, invocation, Some(artifact)));
    }
    daemon.terminate()?;
    daemon = start_daemon(&arguments.daemon, &origin_path)?;
    wait_for_readiness(&runner, &mut daemon)?;
    let (uncertain_root, retained_account) = uncertain_root.ok_or("loss scenario missing")?;
    ensure(
        runner.success(&["run", "show", &uncertain_root])?["value"]["controller_accounting"]
            == retained_account,
        "restart changed or released unknown remote usage",
    )?;
    daemon.terminate()?;
    serving.terminate()?;
    let serving_store = milkdrift_redb_store::RedbStore::open(serving_directory.join("data"))?;
    let origin_store = milkdrift_redb_store::RedbStore::open(directory.join("data"))?;
    ensure(
        serving_store
            .run_summaries(&RunSummaryPageQuery {
                filter: RunSummaryFilter::default(),
                cursor: None,
                limit: PageSize::new(1)?,
            })?
            .runs
            .is_empty(),
        "remote serving created synthetic runs",
    )?;
    for (run, invocation, artifact) in origins {
        let caller = milkdrift_peer_protocol::ServingCaller::peer(
            &PeerId::new(target)?,
            &PeerId::new(source)?,
        );
        let record = serving_store
            .peer_execution_by_request(
                &caller,
                &PeerRequestId::new(format!("request:{invocation}"))?,
            )?
            .ok_or("remote acceptance missing")?;
        let PeerExecutionSnapshot::Hot(record) = record else {
            return Err("remote acceptance unexpectedly archived".into());
        };
        ensure(
            record
                .request
                .authorization
                .origin()
                .workflow()
                .is_some_and(|origin| origin.run == run),
            "remote acceptance lost its real workflow origin",
        )?;
        ensure(
            record
                .request
                .authorization
                .delegation()
                .is_some_and(|delegation| delegation.controller_reservation.is_some()),
            "controlled remote request lost its originating reservation",
        )?;
        let Some(artifact) = artifact else {
            continue;
        };
        let metadata = origin_store
            .metadata(&ArtifactId::new(artifact)?)?
            .ok_or("origin output missing")?;
        ensure(
            matches!(metadata.provenance().producer(), CausalReference::PeerClaim { peer, reference } if peer.as_str() == target && matches!(reference.as_ref(), CausalReference::HostInvocation { .. })),
            "imported output lost the authenticated serving producer",
        )?;
    }
    println!(
        "independent remote hosting: actual origin/serving daemon and CLI, same process/model operations, staged inputs/manifests, useful imported outputs, real workflow origins, controller settlement and unknown usage across lost reply/restart verified"
    );
    Ok(())
}

fn controlled(body: &BlueprintRevision) -> EvidenceResult<BlueprintRevision> {
    use milkdrift_blueprint::{Condition, PinnedSubworkflow, WorkflowInterface};
    use milkdrift_control::{
        ControllerBlueprintSpec, ControllerLimits, build_controller_blueprint,
    };
    Ok(build_controller_blueprint(ControllerBlueprintSpec {
        cost_currency: None,
        workflow: WorkflowId::new(format!("controlled-{}", body.semantic().workflow()))?,
        body: PinnedSubworkflow::new(
            body.semantic().workflow().clone(),
            body.id().clone(),
            WorkflowInterface::new([], [])?,
        ),
        continue_condition: Condition::Constant { value: false },
        limits: ControllerLimits::new(
            4, 4, 4, 4, 900_000, 0, 1_000_000, 32_768, 64_000_000, 2, 2, 2, 2, 2, 2, None,
        )?,
        author: AuthorRef::new(super::super::ACTOR)?,
    })?)
}

fn measure_idle(role: &str, process: u32) -> EvidenceResult {
    #[cfg(windows)]
    {
        let script = format!(
            "$measuredProcess = Get-Process -Id {process}; [pscustomobject]@{{ os = [Environment]::OSVersion.VersionString; architecture = [Runtime.InteropServices.RuntimeInformation]::OSArchitecture.ToString(); threads = $measuredProcess.Threads.Count; working_set_bytes = $measuredProcess.WorkingSet64; private_bytes = $measuredProcess.PrivateMemorySize64 }} | ConvertTo-Json -Compress"
        );
        let result = milkdrift_evidence::application::run_command(
            std::process::Command::new("powershell.exe").args([
                "-NoProfile",
                "-NonInteractive",
                "-WindowStyle",
                "Hidden",
                "-Command",
                &script,
            ]),
            None,
            std::time::Duration::from_secs(5),
        )?;
        ensure(
            result.status.success(),
            "idle Windows process observation failed",
        )?;
        let measured: Value = serde_json::from_str(&result.stdout)?;
        println!(
            "idle role evidence (debug binary, same-machine loopback, before requests): role={role}, process={process}, measurement={measured}"
        );
    }
    #[cfg(not(windows))]
    println!("idle role observation not measured on this platform: role={role}, process={process}");
    Ok(())
}

fn blueprint(
    workflow: &str,
    capability: &str,
    operation: &str,
    input_name: &str,
    input: &milkdrift_capability::ArtifactReference,
    output: &str,
) -> EvidenceResult<BlueprintRevision> {
    let schema = SchemaRef::new(SchemaId::new("milkdrift.artifact-reference")?, 1)?;
    let reference = milkdrift_workspace::ArtifactReference::new(
        ArtifactId::new(input.identity())?,
        milkdrift_workspace::ContentDigest::from_hex(input.digest())?,
        milkdrift_workspace::MediaType::new(input.media_type().ok_or("media type missing")?)?,
        input.size_bytes().ok_or("size missing")?,
    );
    let mut node = Node::new(
        NodeId::new("operation")?,
        NodeKind::task_direct_inputs(
            CapabilityRequirement::new(OperationId::new(operation)?)
                .exact(CapabilityId::new(capability)?)
                .maximum_side_effect(SideEffectClass::Unknown),
        )?,
    )?
    .with_data_input(
        PortId::new(input_name)?,
        DataPort::input(
            schema.clone(),
            true,
            Some(BindingSource::Artifact {
                reference: serde_json::to_string(&reference)?,
                contract: schema.clone(),
            }),
        )?,
    )?
    .with_data_output(PortId::new(output)?, DataPort::output(schema.clone()))?
    .with_control_output(PortId::new("next")?)?;
    if operation == "model.generate" {
        for name in ["model_response", "provider_metadata"] {
            node = node.with_data_output(PortId::new(name)?, DataPort::output(schema.clone()))?;
        }
    }
    BlueprintRevision::genesis(
        WorkflowId::new(workflow)?,
        MutationBatch::new(vec![
            Mutation::AddNode { node },
            Mutation::AddNode {
                node: super::super::terminal_node("done", TerminalOutcome::Success, true)?,
            },
            super::super::control_edge("operation-done", "operation", "next", "done", "in")?,
        ])?,
        AuthorRef::new(super::super::ACTOR)?,
        "Actual binary independent remote execution evidence",
    )
    .map_err(Into::into)
}
