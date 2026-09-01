//! Shared daemon process, configuration, blueprint, and authority fixtures.

//! Loopback-only integration coverage for the durable daemon control plane.

pub(super) use std::{
    collections::BTreeMap,
    fs,
    net::{IpAddr, Ipv4Addr, SocketAddr},
    sync::Arc,
    time::Duration,
};

pub(super) use futures_util::StreamExt;
pub(super) use milkdrift_authority::{
    ActorRef, ArtifactAuthorityScope, AuthorityBudget, BoundaryTimeMillis,
    CapabilityAuthorityScope, CapabilityAuthorityScopeBuilder, DaemonAuthorityScope,
    LayoutAuthorityScope, NetworkScope, PeerAuthorityScope, ResourceScope, WorkflowRunScope,
    WorkspaceAuthorityScope,
};
pub(super) use milkdrift_blueprint::{
    AuthorRef, BlueprintMetadata, BlueprintRevision, BlueprintRevisionDocument, Edge, EdgeId,
    EdgeKind, Mutation, MutationBatch, Node, NodeId, NodeKind, PortId, TerminalOutcome, WorkflowId,
};
pub(super) use milkdrift_capability::{
    CapabilityId, CapabilityRequirement, OperationId, SideEffectClass,
};
pub(super) use milkdrift_control::{
    ClaimedStopCondition, ProposalApplicationPolicy, ProposalId, ProposalProvenance,
    WorkflowProposal, WorkflowProposalDocument,
};
pub(super) use milkdrift_control_client::{
    BearerCredential, ClientConfig, ClientError, ControlClient,
};
pub(super) use milkdrift_control_protocol::{
    Command, CommandRequest, ErrorCode, LayoutDocument, LayoutPoint, Observation, PageRequest,
    ProtocolVersion, decode_json,
};
pub(super) use milkdrift_daemon::{
    ActorBindingConfig, ActorGrantConfig, AdapterConfig, ApplicationReceiptConfig,
    AuthorityPresetConfig, DaemonConfig, DaemonHost, DaemonPlan, PeerHostConfig, RuntimeHostConfig,
    SecretSourceConfig, ShutdownConfig, serve,
};
pub(super) use milkdrift_persistence::{
    ApplicationPageQuery, ArtifactPublicationId, ArtifactStore, BeginArtifactPublication, PageSize,
    SecurityAuditStore,
};
pub(super) use milkdrift_prompt_sequence::PromptSequenceDocument;
pub(super) use milkdrift_prompt_sequence::{
    PromptSource, RemediationProposalSpec, build_remediation_proposal,
};
pub(super) use milkdrift_redb_store::RedbStore;
pub(super) use milkdrift_workspace::{
    ArtifactId, ArtifactMetadata, ArtifactProvenance, ArtifactReference, ArtifactRetention,
    ArtifactSensitivity, CausalId, CausalReference, ContentDigest, MediaType, RunId,
    WorkspaceBudget, WorkspaceUsage,
};
pub(super) use tempfile::TempDir;
pub(super) use tokio::{sync::oneshot, task::JoinHandle};
pub(super) use url::Url;

pub(super) type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

pub(super) const CONTROLLER_TOKEN: &str = "controller-integration-token";
pub(super) const OBSERVER_TOKEN: &str = "observer-integration-token";

pub(super) struct RunningDaemon {
    pub(super) endpoint: Url,
    pub(super) client: ControlClient,
    pub(super) stop: oneshot::Sender<()>,
    pub(super) task: JoinHandle<Result<(), milkdrift_daemon::HostError>>,
}

impl RunningDaemon {
    pub(super) async fn stop(self) -> TestResult {
        let Self { stop, task, .. } = self;
        let _ = stop.send(());
        tokio::time::timeout(Duration::from_secs(10), task).await???;
        Ok(())
    }
}

pub(super) async fn start(config: DaemonPlan, token: &str) -> TestResult<RunningDaemon> {
    let host = DaemonHost::start(config)?;
    let listener = tokio::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await?;
    let address = listener.local_addr()?;
    let endpoint = Url::parse(&format!("http://{address}/"))?;
    let (stop, stopped) = oneshot::channel();
    let task = tokio::spawn(serve(listener, host, async move {
        let _ = stopped.await;
    }));
    let client = client(&endpoint, token)?;
    let health = client.readiness().await?;
    assert!(health.ready);
    Ok(RunningDaemon {
        endpoint,
        client,
        stop,
        task,
    })
}

pub(super) fn client(endpoint: &Url, token: &str) -> Result<ControlClient, ClientError> {
    let mut config = ClientConfig::new(endpoint.clone());
    config.safe_query_retries = 0;
    config.retry_delay = Duration::from_millis(10);
    ControlClient::new(config, BearerCredential::new(token)?)
}

pub(super) fn configuration(directory: &TempDir, request_queue: u32) -> TestResult<DaemonPlan> {
    configuration_with_process_profiles(directory, request_queue, Vec::new())
}

pub(super) fn configuration_with_process_profiles(
    directory: &TempDir,
    request_queue: u32,
    process_profiles: Vec<std::path::PathBuf>,
) -> TestResult<DaemonPlan> {
    configuration_document_with_process_profiles(directory, request_queue, process_profiles)?
        .validate(directory.path())
        .map_err(Into::into)
}

pub(super) fn configuration_document_with_process_profiles(
    directory: &TempDir,
    request_queue: u32,
    process_profiles: Vec<std::path::PathBuf>,
) -> TestResult<DaemonConfig> {
    write_secret(&directory.path().join("controller.token"), CONTROLLER_TOKEN)?;
    write_secret(&directory.path().join("observer.token"), OBSERVER_TOKEN)?;
    let runtime = RuntimeHostConfig {
        request_queue,
        maintenance_interval_ms: 10,
        ..RuntimeHostConfig::default()
    };
    Ok(DaemonConfig {
        schema_version: milkdrift_daemon::DAEMON_CONFIG_SCHEMA_VERSION,
        data_root: directory.path().join("data"),
        bind: SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0),
        secret_sources: BTreeMap::from([
            (
                "credential:controller".to_owned(),
                SecretSourceConfig::File {
                    path: directory.path().join("controller.token"),
                },
            ),
            (
                "credential:observer".to_owned(),
                SecretSourceConfig::File {
                    path: directory.path().join("observer.token"),
                },
            ),
        ]),
        actors: vec![
            ActorBindingConfig {
                credential_ref: "credential:controller".to_owned(),
                actor: "human:integration-controller".to_owned(),
                grant_id: "grant:integration-controller".to_owned(),
                grant_revision: 1,
                revocation_generation: 0,
                preset: AuthorityPresetConfig::Controller,
                authority: ActorGrantConfig::dangerous_administrator(),
                enabled: true,
            },
            ActorBindingConfig {
                credential_ref: "credential:observer".to_owned(),
                actor: "human:integration-observer".to_owned(),
                grant_id: "grant:integration-observer".to_owned(),
                grant_revision: 1,
                revocation_generation: 0,
                preset: AuthorityPresetConfig::Observer,
                authority: observer_authority()?,
                enabled: true,
            },
        ],
        runtime,
        adapters: AdapterConfig {
            process_profiles,
            ..AdapterConfig::default()
        },
        peers: PeerHostConfig::default(),
        shutdown: ShutdownConfig::default(),
        application_receipts: ApplicationReceiptConfig {
            hot_receipt_bound: 1_000,
            archive_batch_size: 64,
        },
        security_audit_record_bound: 1_000,
    })
}

pub(super) fn configured_process_profile(directory: &TempDir) -> TestResult<std::path::PathBuf> {
    let executable = std::path::Path::new("/bin/echo");
    let bytes = fs::read(executable)?;
    let mut profile: serde_json::Value = serde_json::from_slice(include_bytes!(
        "../../../../adapters/local-process/tests/fixtures/process-profile-v2.json"
    ))?;
    profile["profile"]["implementation"]["content_digest"] =
        serde_json::json!(format!("b3_{}", blake3::hash(&bytes)));
    profile["profile"]["implementation"]["size_bytes"] = serde_json::json!(bytes.len());
    let path = directory.path().join("process-profile-v2.json");
    fs::write(&path, serde_json::to_vec(&profile)?)?;
    Ok(path)
}

pub(super) struct DogfoodProfiles {
    pub(super) coding: std::path::PathBuf,
    pub(super) good_verification: std::path::PathBuf,
    pub(super) weak_verification: std::path::PathBuf,
    pub(super) reviewer: std::path::PathBuf,
}

#[allow(clippy::too_many_arguments)] // Test fixture mirrors every independently variable process-profile field.
pub(super) fn write_dogfood_process_profile(
    directory: &TempDir,
    repository: &std::path::Path,
    name: &str,
    capability: &str,
    executable: &std::path::Path,
    arguments: serde_json::Value,
    substitutions: serde_json::Value,
    working_directory: serde_json::Value,
    inputs: serde_json::Value,
    stdin: serde_json::Value,
    stdout_name: &str,
    stderr_name: &str,
    outputs: serde_json::Value,
    side_effect: &str,
) -> TestResult<std::path::PathBuf> {
    let bytes = fs::read(executable)?;
    let executable_root = executable.parent().ok_or("executable has no parent")?;
    let value = serde_json::json!({
        "schema_version": 2,
        "profile": {
            "profile_id": name,
            "revision": 1,
            "capability": capability,
            "descriptor_revision": 1,
            "provider_profile": null,
            "operation": "process.execute",
            "side_effect": side_effect,
            "idempotency": "unsupported",
            "cancellation": "best_effort",
            "trust_class": "trusted_host_process",
            "executable": executable,
            "implementation": {
                "content_digest": format!("b3_{}", blake3::hash(&bytes)),
                "size_bytes": bytes.len(),
                "package_revision": "deterministic-dogfood-fixture-v1",
                "documentation_reference": "urn:milkdrift:deterministic-dogfood-fixture"
            },
            "arguments": arguments,
            "substitutions": substitutions,
            "working_directory": working_directory,
            "filesystem_roots": [
                {"path": executable_root, "access": "execute"},
                {"path": directory.path(), "access": "read_write"}
            ],
            "inputs": inputs,
            "environment": {
                "allowed_non_secret": [],
                "secrets": {},
                "max_value_bytes": 4096
            },
            "stdin": stdin,
            "stdout": {
                "max_capture_bytes": 1048576,
                "stream_progress": true,
                "max_progress_events": 8,
                "overflow_action": "continue_truncated",
                "artifact_name": stdout_name
            },
            "stderr": {
                "max_capture_bytes": 1048576,
                "stream_progress": true,
                "max_progress_events": 8,
                "overflow_action": "continue_truncated",
                "artifact_name": stderr_name
            },
            "outputs": outputs,
            "limits": {
                "max_argv_entries": 16,
                "max_argv_bytes": 16384,
                "max_children_observed": 8,
                "max_files": 16,
                "max_file_bytes": 2097152,
                "max_total_materialized_bytes": 4194304,
                "max_path_bytes": 4096,
                "max_directory_depth": 32,
                "artifact_chunk_bytes": 65536,
                "max_output_files": 8,
                "max_total_output_bytes": 4194304,
                "wall_timeout_ms": 5000,
                "graceful_termination_ms": 100,
                "forced_termination_ms": 100,
                "heartbeat_interval_ms": 1000
            },
            "restart": "retain_uncertain",
            "platform": milkdrift_local_process::PlatformSupport::current(),
            "max_concurrent": 1,
            "extensions": {
                "org.milkdrift/test-fixture": {
                    "deterministic": true,
                    "repository": repository
                }
            }
        }
    });
    let path = directory.path().join(format!("{name}.json"));
    fs::write(&path, serde_json::to_vec(&value)?)?;
    Ok(path)
}

pub(super) fn dogfood_process_profiles(
    directory: &TempDir,
    repository: &std::path::Path,
) -> TestResult<DogfoodProfiles> {
    let host_directory = serde_json::json!({
        "type": "authorized_host_path",
        "path": repository
    });
    let coding = write_dogfood_process_profile(
        directory,
        repository,
        "dogfood-coding",
        "dogfood-coding-agent",
        std::path::Path::new("/usr/bin/tee"),
        serde_json::json!(["-a", "progress.md"]),
        serde_json::json!({}),
        host_directory.clone(),
        serde_json::json!([{"input": "prompt", "relative_path": "prompt.json"}]),
        serde_json::json!({"type": "input", "input": "prompt", "max_bytes": 65536}),
        "diff",
        "logs",
        serde_json::json!([]),
        "non_idempotent_write",
    )?;
    let good_verification = write_dogfood_process_profile(
        directory,
        repository,
        "dogfood-verification-good",
        "dogfood-verifier-good",
        std::path::Path::new("/bin/cp"),
        serde_json::json!(["progress.md", "{{execution_root}}/verification-pass.json"]),
        serde_json::json!({
            "execution_root": {"type": "execution_root"}
        }),
        host_directory.clone(),
        serde_json::json!([]),
        serde_json::json!({"type": "disabled"}),
        "verification_result",
        "verification_logs",
        serde_json::json!([{
            "name": "verification_pass",
            "relative_path": "verification-pass.json",
            "media_type": "application/json",
            "required": true
        }]),
        "read_only",
    )?;
    let weak_verification = write_dogfood_process_profile(
        directory,
        repository,
        "dogfood-verification-weak",
        "dogfood-verifier-weak",
        std::path::Path::new("/bin/cp"),
        serde_json::json!([
            "progress.md",
            "{{execution_root}}/weak-verification-result.json"
        ]),
        serde_json::json!({
            "execution_root": {"type": "execution_root"}
        }),
        host_directory.clone(),
        serde_json::json!([]),
        serde_json::json!({"type": "disabled"}),
        "verification_result",
        "verification_logs",
        serde_json::json!([]),
        "read_only",
    )?;
    let reviewer = write_dogfood_process_profile(
        directory,
        repository,
        "dogfood-reviewer",
        "dogfood-reviewer",
        std::path::Path::new("/bin/cp"),
        serde_json::json!(["progress.md", "{{execution_root}}/review.json"]),
        serde_json::json!({
            "execution_root": {"type": "execution_root"}
        }),
        host_directory,
        serde_json::json!([]),
        serde_json::json!({"type": "disabled"}),
        "review",
        "remediation_proposal",
        serde_json::json!([]),
        "read_only",
    )?;
    Ok(DogfoodProfiles {
        coding,
        good_verification,
        weak_verification,
        reviewer,
    })
}

pub(super) fn observer_authority() -> TestResult<ActorGrantConfig> {
    Ok(ActorGrantConfig {
        resources: ResourceScope {
            workflow_run: WorkflowRunScope::Workflow {
                workflow: WorkflowId::new("golden")?,
            },
            capability: CapabilityAuthorityScopeBuilder::new(SideEffectClass::ReadOnly)
                .only_capabilities(std::collections::BTreeSet::from([CapabilityId::new(
                    "milkdrift-workflow-control",
                )?]))?
                .only_operations(std::collections::BTreeSet::from([OperationId::new(
                    "workflow.inspect",
                )?]))?
                .build(),
            filesystem: Vec::new(),
            network: NetworkScope::empty(),
            secrets: std::collections::BTreeSet::new(),
            artifacts: ArtifactAuthorityScope::none(),
            layouts: LayoutAuthorityScope::none(),
            peers: PeerAuthorityScope::none(),
            daemon: DaemonAuthorityScope {
                readiness: true,
                detailed_health: false,
                own_authority: true,
                configuration: false,
                audit: false,
            },
            workspace: WorkspaceAuthorityScope::none(),
        },
        budget: AuthorityBudget {
            cost_minor: Some(1_000),
            duration_ms: Some(300_000),
            invocations: Some(1_000),
            artifact_bytes: Some(67_108_864),
            units: Some(1_000_000),
            concurrency: Some(4),
        },
        valid_from: BoundaryTimeMillis::new(0),
        valid_until: BoundaryTimeMillis::new(4_102_444_800_000),
        dangerous_allow_broad_authority: false,
    })
}

pub(super) fn write_secret(path: &std::path::Path, value: &str) -> TestResult {
    fs::write(path, value)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}

pub(super) fn publish_restricted_test_artifact(
    root: &std::path::Path,
) -> TestResult<(String, Vec<u8>)> {
    let artifact_id = "artifact-daemon-protected-read";
    let bytes = b"protected daemon artifact".to_vec();
    let metadata = ArtifactMetadata::new(
        ArtifactReference::new(
            ArtifactId::new(artifact_id)?,
            ContentDigest::for_bytes(&bytes),
            MediaType::new("application/octet-stream")?,
            u64::try_from(bytes.len())?,
        ),
        ArtifactSensitivity::Restricted,
        ArtifactRetention::WhileReferenced,
        ArtifactProvenance::new(
            CausalReference::External {
                source: CausalId::new("daemon-authorization-integration")?,
            },
            Vec::new(),
        )?,
    )?;
    let publication = BeginArtifactPublication::new(
        ArtifactPublicationId::new("publication-daemon-protected-read")?,
        RunId::new("run-daemon-protected-read")?,
        metadata,
        WorkspaceBudget::new(0, 0, 0, 1, 1_024, 1_024)?,
        WorkspaceUsage::EMPTY,
    )?;
    let store = RedbStore::open(root)?;
    store.begin_publication(&publication)?;
    store.write_chunk(publication.publication(), 0, &bytes)?;
    store.commit_publication(publication.publication())?;
    Ok((artifact_id.to_owned(), bytes))
}

pub(super) fn request(
    command_id: &str,
    expected_sequence: Option<u64>,
    command: Command,
) -> CommandRequest {
    CommandRequest {
        protocol: ProtocolVersion::CURRENT,
        command_id: command_id.to_owned(),
        expected_sequence,
        expected_revision: None,
        reason: "deterministic daemon integration test".to_owned(),
        evidence: Vec::new(),
        command,
    }
}

pub(super) fn blueprint() -> TestResult<serde_json::Value> {
    decode_json(include_bytes!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../crates/blueprint/tests/fixtures/revision-v2.json"
    )))
    .map_err(Into::into)
}

pub(super) fn prompt_sequence() -> TestResult<serde_json::Value> {
    let document = PromptSequenceDocument::from_bytes(include_bytes!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../examples/headless-dogfood-sequence.md"
    )))?;
    serde_json::to_value(document).map_err(Into::into)
}

pub(super) fn dogfood_profile_reference(capability: &str, side_effect: &str) -> serde_json::Value {
    serde_json::json!({
        "capability": capability,
        "operation": "process.execute",
        "provider_profile": null,
        "execution_trust": "trusted_host_process",
        "maximum_side_effect": side_effect
    })
}

pub(super) fn dogfood_stage(identity: &str, prompt: &str, verifier: &str) -> serde_json::Value {
    serde_json::json!({
        "id": identity,
        "title": format!("Dogfood stage {identity}"),
        "prompt": {"type": "inline_markdown", "content": prompt},
        "session": "fresh",
        "coding": dogfood_profile_reference("dogfood-coding-agent", "unknown"),
        "verification": {
            "profile": dogfood_profile_reference(verifier, "read_only"),
            "checks": ["fixture.repository_progress"],
            "success_artifact": "verification_pass",
            "result_artifact": "verification_result",
            "log_artifact": "verification_logs"
        },
        "failure": "pause_for_review",
        "reviewer": dogfood_profile_reference("dogfood-reviewer", "read_only"),
        "approval": "shared_control_path",
        "context_policy_ref": "context:dogfood-v1",
        "outputs": [
            {"name": "diff", "media_type": "application/octet-stream", "required": true},
            {"name": "logs", "media_type": "application/octet-stream", "required": false}
        ]
    })
}

pub(super) fn executable_dogfood_sequence() -> TestResult<PromptSequenceDocument> {
    let value = serde_json::json!({
        "schema_version": 2,
        "sequence": {
            "id": "daemon-headless-dogfood",
            "title": "Daemon headless dogfood",
            "workflow_id": "daemon-headless-dogfood",
            "repository": {
                "id": "repository:daemon-dogfood",
                "root_ref": "workspace:daemon-dogfood",
                "starting_revision": "fixture-start",
                "allowed_paths": ["progress.md"],
                "allowed_operations": ["read", "write", "execute"],
                "dirty_tree": "allow_recorded",
                "isolation": "shared_sequential",
                "cleanup": "retain_accepted",
                "artifacts": {
                    "require_starting_state": true,
                    "require_diff": true,
                    "require_verification_evidence": true
                },
                "credential_refs": [],
                "remote_access_refs": []
            },
            "stages": [
                dogfood_stage(
                    "one",
                    "First fresh process writes accepted repository progress.\n",
                    "dogfood-verifier-good"
                ),
                dogfood_stage(
                    "two",
                    "Second fresh process intentionally receives weak verification.\n",
                    "dogfood-verifier-weak"
                )
            ],
            "budget": {
                "max_review_loops": 3
            },
            "extensions": {}
        }
    });
    PromptSequenceDocument::from_json(&serde_json::to_vec(&value)?).map_err(Into::into)
}

pub(super) async fn wait_for_run<F>(
    client: &ControlClient,
    run: &str,
    predicate: F,
) -> TestResult<milkdrift_control_protocol::RunRead>
where
    F: Fn(&milkdrift_control_protocol::RunRead) -> bool,
{
    let mut last = None;
    for _ in 0..500 {
        let state = client.run(run).await?;
        if predicate(&state) {
            return Ok(state);
        }
        last = Some(state);
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    Err(format!(
        "run did not reach the expected bounded state; last={}",
        serde_json::to_string(&last)?
    )
    .into())
}

pub(super) async fn attempt_id_for_node(
    client: &ControlClient,
    run: &str,
    node: &str,
) -> TestResult<String> {
    let page = client
        .timeline(
            run,
            &PageRequest {
                cursor: None,
                limit: 1_000,
            },
        )
        .await?;
    page.items
        .iter()
        .rev()
        .find(|entry| entry.node_id.as_deref() == Some(node) && entry.attempt_id.is_some())
        .and_then(|entry| entry.attempt_id.clone())
        .ok_or_else(|| format!("timeline has no attempt for node {node}").into())
}

pub(super) fn proposal_document(run: &RunId, sequence: u64) -> TestResult<serde_json::Value> {
    let bytes = serde_json::to_vec(&blueprint()?)?;
    let (_document, base) = BlueprintRevisionDocument::from_json(&bytes)?;
    let proposal = WorkflowProposal::new(
        ProposalId::new("proposal-daemon-index")?,
        ActorRef::new("human:integration-controller")?,
        ProposalProvenance::Direct,
        base.semantic().workflow().clone(),
        Some(run.clone()),
        base.id().clone(),
        base.content_digest().clone(),
        Some(milkdrift_persistence::RunSequence::new(sequence)),
        MutationBatch::new(vec![Mutation::SetMetadata {
            metadata: BlueprintMetadata::new(
                "golden",
                "proposal discovery survives restart",
                Default::default(),
                Default::default(),
            )?,
        }])?,
        "exercise first-class proposal discovery",
        None,
        Vec::new(),
        Vec::new(),
        Vec::new(),
        Vec::new(),
        ProposalApplicationPolicy::ProposeOnly,
        None,
        ClaimedStopCondition::Continue,
    )?;
    let document = WorkflowProposalDocument::new(proposal);
    decode_json(&document.to_canonical_json()?).map_err(Into::into)
}

pub(super) fn process_blueprint() -> TestResult<serde_json::Value> {
    let task = Node::new(
        NodeId::new("process")?,
        NodeKind::task_direct_inputs(CapabilityRequirement::new(OperationId::new(
            "process.execute",
        )?))?,
    )?
    .with_control_output(PortId::new("next")?)?;
    let terminal = Node::new(
        NodeId::new("done")?,
        NodeKind::Terminal {
            outcome: TerminalOutcome::Success,
        },
    )?
    .with_control_input(PortId::new("in")?)?;
    let revision = BlueprintRevision::genesis(
        WorkflowId::new("daemon-process")?,
        MutationBatch::new(vec![
            Mutation::AddNode { node: task },
            Mutation::AddNode { node: terminal },
            Mutation::AddEdge {
                edge: Edge::new(
                    EdgeId::new("process-done")?,
                    EdgeKind::Control,
                    NodeId::new("process")?,
                    PortId::new("next")?,
                    NodeId::new("done")?,
                    PortId::new("in")?,
                ),
            },
        ])?,
        AuthorRef::new("human:daemon-integration")?,
        "configured process adapter integration",
    )?;
    let bytes = BlueprintRevisionDocument::new(&revision).to_canonical_json()?;
    decode_json(&bytes).map_err(Into::into)
}

pub(super) async fn import_blueprint(
    client: &ControlClient,
    command_id: &str,
) -> TestResult<String> {
    let accepted = client
        .submit(&request(
            command_id,
            None,
            Command::ImportBlueprint {
                document: blueprint()?,
            },
        ))
        .await?;
    accepted
        .value
        .get("revision_id")
        .and_then(serde_json::Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| "import response did not contain a revision identity".into())
}
