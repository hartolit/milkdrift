//! Loopback-only integration coverage for the durable daemon control plane.

use std::{
    collections::BTreeMap,
    fs,
    net::{IpAddr, Ipv4Addr, SocketAddr},
    sync::Arc,
    time::Duration,
};

use futures_util::StreamExt as _;
use milkdrift_authority::{
    ActorRef, ArtifactAuthorityScope, AuthorityBudget, BoundaryTimeMillis,
    CapabilityAuthorityScope, CapabilityAuthorityScopeBuilder, DaemonAuthorityScope,
    LayoutAuthorityScope, NetworkScope, PeerAuthorityScope, ResourceScope, WorkflowRunScope,
    WorkspaceAuthorityScope,
};
use milkdrift_blueprint::{
    AuthorRef, BlueprintMetadata, BlueprintRevision, BlueprintRevisionDocument, Edge, EdgeId,
    EdgeKind, Mutation, MutationBatch, Node, NodeId, NodeKind, PortId, TerminalOutcome, WorkflowId,
};
use milkdrift_capability::{CapabilityId, CapabilityRequirement, OperationId, SideEffectClass};
use milkdrift_control::{
    ClaimedStopCondition, ProposalApplicationPolicy, ProposalId, ProposalProvenance,
    WorkflowProposal, WorkflowProposalDocument,
};
use milkdrift_control_client::{BearerCredential, ClientConfig, ClientError, ControlClient};
use milkdrift_control_protocol::{
    Command, CommandRequest, ErrorCode, LayoutDocument, LayoutPoint, Observation, PageRequest,
    ProtocolVersion, decode_json,
};
use milkdrift_daemon::{
    ActorBindingConfig, ActorGrantConfig, AdapterConfig, ApplicationReceiptConfig,
    AuthorityPresetConfig, DaemonConfig, DaemonHost, PeerHostConfig, RuntimeHostConfig,
    SecretSourceConfig, ShutdownConfig, ValidatedDaemonConfig, serve,
};
use milkdrift_persistence::{
    ApplicationPageQuery, ArtifactPublicationId, ArtifactStore, BeginArtifactPublication, PageSize,
    SecurityAuditStore,
};
use milkdrift_prompt_sequence::PromptSequenceDocument;
use milkdrift_prompt_sequence::{
    PromptSource, RemediationProposalSpec, build_remediation_proposal,
};
use milkdrift_redb_store::RedbStore;
use milkdrift_workspace::{
    ArtifactId, ArtifactMetadata, ArtifactProvenance, ArtifactReference, ArtifactRetention,
    ArtifactSensitivity, CausalId, CausalReference, ContentDigest, MediaType, RunId,
    WorkspaceBudget, WorkspaceUsage,
};
use tempfile::TempDir;
use tokio::{sync::oneshot, task::JoinHandle};
use url::Url;

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

const CONTROLLER_TOKEN: &str = "controller-integration-token";
const OBSERVER_TOKEN: &str = "observer-integration-token";

struct RunningDaemon {
    endpoint: Url,
    client: ControlClient,
    stop: oneshot::Sender<()>,
    task: JoinHandle<Result<(), milkdrift_daemon::HostError>>,
}

impl RunningDaemon {
    async fn stop(self) -> TestResult {
        let Self { stop, task, .. } = self;
        let _ = stop.send(());
        tokio::time::timeout(Duration::from_secs(10), task).await???;
        Ok(())
    }
}

async fn start(config: ValidatedDaemonConfig, token: &str) -> TestResult<RunningDaemon> {
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

fn client(endpoint: &Url, token: &str) -> Result<ControlClient, ClientError> {
    let mut config = ClientConfig::new(endpoint.clone());
    config.safe_query_retries = 0;
    config.retry_delay = Duration::from_millis(10);
    ControlClient::new(config, BearerCredential::new(token)?)
}

fn configuration(directory: &TempDir, request_queue: u32) -> TestResult<ValidatedDaemonConfig> {
    configuration_with_process_profiles(directory, request_queue, Vec::new())
}

fn configuration_with_process_profiles(
    directory: &TempDir,
    request_queue: u32,
    process_profiles: Vec<std::path::PathBuf>,
) -> TestResult<ValidatedDaemonConfig> {
    write_secret(&directory.path().join("controller.token"), CONTROLLER_TOKEN)?;
    write_secret(&directory.path().join("observer.token"), OBSERVER_TOKEN)?;
    let runtime = RuntimeHostConfig {
        request_queue,
        maintenance_interval_ms: 10,
        ..RuntimeHostConfig::default()
    };
    DaemonConfig {
        schema_version: 7,
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
    }
    .validate(directory.path())
    .map_err(Into::into)
}

fn configured_process_profile(directory: &TempDir) -> TestResult<std::path::PathBuf> {
    let executable = std::path::Path::new("/bin/echo");
    let bytes = fs::read(executable)?;
    let mut profile: serde_json::Value = serde_json::from_slice(include_bytes!(
        "../../../adapters/local-process/tests/fixtures/process-profile-v2.json"
    ))?;
    profile["profile"]["implementation"]["content_digest"] =
        serde_json::json!(format!("b3_{}", blake3::hash(&bytes)));
    profile["profile"]["implementation"]["size_bytes"] = serde_json::json!(bytes.len());
    let path = directory.path().join("process-profile-v2.json");
    fs::write(&path, serde_json::to_vec(&profile)?)?;
    Ok(path)
}

struct DogfoodProfiles {
    coding: std::path::PathBuf,
    good_verification: std::path::PathBuf,
    weak_verification: std::path::PathBuf,
    reviewer: std::path::PathBuf,
}

#[allow(clippy::too_many_arguments)]
fn write_dogfood_process_profile(
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

fn dogfood_process_profiles(
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

fn observer_authority() -> TestResult<ActorGrantConfig> {
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

fn write_secret(path: &std::path::Path, value: &str) -> TestResult {
    fs::write(path, value)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}

fn publish_restricted_test_artifact(root: &std::path::Path) -> TestResult<(String, Vec<u8>)> {
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

fn request(command_id: &str, expected_sequence: Option<u64>, command: Command) -> CommandRequest {
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

fn blueprint() -> TestResult<serde_json::Value> {
    decode_json(include_bytes!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../crates/blueprint/tests/fixtures/revision-v2.json"
    )))
    .map_err(Into::into)
}

fn prompt_sequence() -> TestResult<serde_json::Value> {
    let document = PromptSequenceDocument::from_bytes(include_bytes!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../examples/headless-dogfood-sequence.md"
    )))?;
    serde_json::to_value(document).map_err(Into::into)
}

fn dogfood_profile_reference(capability: &str, side_effect: &str) -> serde_json::Value {
    serde_json::json!({
        "capability": capability,
        "operation": "process.execute",
        "provider_profile": null,
        "execution_trust": "trusted_host_process",
        "maximum_side_effect": side_effect
    })
}

fn dogfood_stage(identity: &str, prompt: &str, verifier: &str) -> serde_json::Value {
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

fn executable_dogfood_sequence() -> TestResult<PromptSequenceDocument> {
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

async fn wait_for_run<F>(
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

async fn attempt_id_for_node(client: &ControlClient, run: &str, node: &str) -> TestResult<String> {
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

fn proposal_document(run: &RunId, sequence: u64) -> TestResult<serde_json::Value> {
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

fn process_blueprint() -> TestResult<serde_json::Value> {
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

async fn import_blueprint(client: &ControlClient, command_id: &str) -> TestResult<String> {
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

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn prompt_sequence_validate_import_inspect_and_restart_are_one_control_path() -> TestResult {
    let directory = tempfile::tempdir()?;
    let config = configuration(&directory, 32)?;
    let daemon = start(config.clone(), CONTROLLER_TOKEN).await?;
    let document = prompt_sequence()?;

    let validated = daemon
        .client
        .submit(&request(
            "prompt-sequence-validate",
            None,
            Command::ValidatePromptSequence {
                document: document.clone(),
            },
        ))
        .await?;
    assert_eq!(validated.result_type, "prompt_sequence_valid");
    let revision = validated.value["revision_id"]
        .as_str()
        .ok_or("validation omitted revision")?
        .to_owned();
    assert!(daemon.client.revision(&revision).await.is_err());

    let imported = daemon
        .client
        .submit(&request(
            "prompt-sequence-import",
            None,
            Command::ImportPromptSequence {
                document: document.clone(),
            },
        ))
        .await?;
    assert_eq!(imported.result_type, "prompt_sequence_imported");
    assert_eq!(imported.value["revision_id"], revision);
    assert_eq!(imported.value["stages"].as_array().map(Vec::len), Some(1));
    assert!(
        imported.value["import_digest"]
            .as_str()
            .is_some_and(|value| value.starts_with("b3_"))
    );
    let read = daemon.client.revision(&revision).await?;
    assert_eq!(read.summary.workflow_id, "milkdrift-core-convergence");
    assert_eq!(read.node_count, 7);
    assert!(read.document.is_some());

    let duplicate = daemon
        .client
        .submit(&request(
            "prompt-sequence-import-duplicate",
            None,
            Command::ImportPromptSequence { document },
        ))
        .await?;
    assert!(duplicate.replayed);
    daemon.stop().await?;

    let restarted = start(config, CONTROLLER_TOKEN).await?;
    let reopened = restarted.client.revision(&revision).await?;
    assert_eq!(reopened.node_count, 7);
    assert_eq!(
        reopened.summary.semantic_digest,
        read.summary.semantic_digest
    );
    restarted.stop().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn headless_dogfood_failure_remediation_and_restart_are_durable() -> TestResult {
    let directory = tempfile::tempdir()?;
    let repository = directory.path().join("repository");
    fs::create_dir(&repository)?;
    let profiles = dogfood_process_profiles(&directory, &repository)?;
    let profile_paths = vec![
        profiles.coding,
        profiles.good_verification,
        profiles.weak_verification,
        profiles.reviewer,
    ];
    let config = configuration_with_process_profiles(&directory, 64, profile_paths)?;
    let sequence = executable_dogfood_sequence()?;
    let sequence_value = serde_json::to_value(&sequence)?;
    let daemon = start(config.clone(), CONTROLLER_TOKEN).await?;

    let imported = daemon
        .client
        .submit(&request(
            "dogfood-import",
            None,
            Command::ImportPromptSequence {
                document: sequence_value,
            },
        ))
        .await?;
    let revision_id = imported.value["revision_id"]
        .as_str()
        .ok_or("dogfood import omitted revision")?
        .to_owned();
    daemon
        .client
        .submit(&request(
            "dogfood-start",
            None,
            Command::StartRun {
                run_id: "run-headless-dogfood".to_owned(),
                workflow_id: "daemon-headless-dogfood".to_owned(),
                revision_id: revision_id.clone(),
            },
        ))
        .await?;

    let waiting = wait_for_run(&daemon.client, "run-headless-dogfood", |state| {
        state
            .nodes
            .iter()
            .any(|node| node.node_id == "stage-two-approval")
    })
    .await?;
    assert_eq!(waiting.lifecycle, "running");
    assert!(waiting.terminal.is_none());
    assert!(!waiting.nodes.iter().any(|node| {
        node.node_id == "sequence-succeeded" || node.node_id.starts_with("stage-three-")
    }));
    daemon.stop().await?;

    let daemon = start(config.clone(), CONTROLLER_TOKEN).await?;
    let waiting = daemon.client.run("run-headless-dogfood").await?;
    assert_eq!(waiting.lifecycle, "running");
    assert!(
        waiting
            .nodes
            .iter()
            .any(|node| node.node_id == "stage-two-approval")
    );
    assert!(!waiting.nodes.iter().any(|node| {
        node.node_id == "sequence-succeeded" || node.node_id.starts_with("stage-three-")
    }));
    let reviewer =
        attempt_id_for_node(&daemon.client, "run-headless-dogfood", "stage-two-review").await?;
    let reviewer_attempt = daemon
        .client
        .attempt("run-headless-dogfood", &reviewer)
        .await?;
    assert_eq!(reviewer_attempt.context_access, "authorized");
    let reviewer_context = reviewer_attempt
        .context
        .ok_or("reviewer context is absent")?;
    assert_eq!(reviewer_context.policy["session"], "fresh");
    assert!(
        serde_json::to_string(&reviewer_context.policy)?.contains("prior_prompt"),
        "review policy must explicitly exclude chronological prior prompts"
    );

    daemon
        .client
        .submit(&request(
            "dogfood-pause-after-failure",
            Some(waiting.sequence),
            Command::PauseRun {
                run_id: "run-headless-dogfood".to_owned(),
            },
        ))
        .await?;
    let paused = daemon.client.run("run-headless-dogfood").await?;
    assert_eq!(paused.lifecycle, "paused");

    let revision_read = daemon.client.revision(&revision_id).await?;
    let base_bytes = serde_json::to_vec(
        revision_read
            .document
            .as_ref()
            .ok_or("base revision document is absent")?,
    )?;
    let (_base_document, base) = BlueprintRevisionDocument::from_json(&base_bytes)?;
    let good_verification = sequence.sequence().stages[0].verification.clone();
    let proposal = build_remediation_proposal(
        &sequence,
        &base,
        RemediationProposalSpec {
            run: RunId::new("run-headless-dogfood")?,
            observed_sequence: milkdrift_persistence::RunSequence::new(paused.sequence),
            proposal: ProposalId::new("proposal-headless-remediation-1")?,
            proposer: ActorRef::new("human:integration-controller")?,
            stage_id: "two".to_owned(),
            generation: 1,
            prompt: PromptSource::InlineMarkdown {
                content: "Remediation fresh process repairs the weak implementation.\n".to_owned(),
            },
            verification_override: Some(good_verification),
        },
    )?;
    let proposal_digest = proposal.proposal().digest().as_str().to_owned();
    let proposal_value = decode_json(&proposal.to_canonical_json()?)?;
    let mut submit_request = request(
        "dogfood-submit-remediation",
        Some(paused.sequence),
        Command::SubmitProposal {
            document: proposal_value,
        },
    );
    submit_request.expected_revision = Some(revision_id.clone());
    let submitted = daemon.client.submit(&submit_request).await?;
    let proposed_revision = submitted.value["proposed_revision"]
        .as_str()
        .ok_or("proposal response omitted proposed revision")?
        .to_owned();
    assert!(!submitted.value["applied"].as_bool().unwrap_or(false));
    daemon.stop().await?;

    let restarted = start(config.clone(), CONTROLLER_TOKEN).await?;
    let proposal_status = restarted
        .client
        .proposal(
            "run-headless-dogfood",
            "proposal-headless-remediation-1",
            &proposed_revision,
        )
        .await?;
    assert!(!proposal_status.approved);
    let decision_boundary = restarted.client.run("run-headless-dogfood").await?.sequence;
    let mut approve_request = request(
        "dogfood-approve-remediation",
        Some(decision_boundary),
        Command::DecideProposal {
            run_id: "run-headless-dogfood".to_owned(),
            proposal_id: "proposal-headless-remediation-1".to_owned(),
            proposal_digest: proposal_digest.clone(),
            proposed_revision: proposed_revision.clone(),
            decision_id: "decision-headless-remediation-1".to_owned(),
            decision: milkdrift_control_protocol::ProposalDecision::Approve,
        },
    );
    approve_request.expected_revision = Some(proposed_revision.clone());
    restarted.client.submit(&approve_request).await?;
    let apply_boundary = restarted.client.run("run-headless-dogfood").await?.sequence;
    let mut apply_request = request(
        "dogfood-apply-remediation",
        Some(apply_boundary),
        Command::ApplyProposal {
            run_id: "run-headless-dogfood".to_owned(),
            proposal_id: "proposal-headless-remediation-1".to_owned(),
            proposal_digest,
            proposed_revision: proposed_revision.clone(),
        },
    );
    apply_request.expected_revision = Some(proposed_revision.clone());
    restarted.client.submit(&apply_request).await?;
    let adopted = restarted.client.run("run-headless-dogfood").await?;
    assert_eq!(
        adopted.revision_id.as_deref(),
        Some(proposed_revision.as_str())
    );
    assert_eq!(adopted.lifecycle, "paused");
    restarted.stop().await?;

    let resumed = start(config, CONTROLLER_TOKEN).await?;
    let signal_boundary = resumed.client.run("run-headless-dogfood").await?.sequence;
    resumed
        .client
        .submit(&request(
            "dogfood-release-approved-remediation",
            Some(signal_boundary),
            Command::SignalRun {
                run_id: "run-headless-dogfood".to_owned(),
                signal_id: "signal-headless-remediation-1".to_owned(),
                signal_type: "sequence.approved".to_owned(),
                correlation: None,
                broadcast: false,
                payload: serde_json::json!({
                    "proposal": "proposal-headless-remediation-1",
                    "revision": proposed_revision
                }),
            },
        ))
        .await?;
    let resume_boundary = resumed.client.run("run-headless-dogfood").await?.sequence;
    resumed
        .client
        .submit(&request(
            "dogfood-resume-remediation",
            Some(resume_boundary),
            Command::ResumeRun {
                run_id: "run-headless-dogfood".to_owned(),
            },
        ))
        .await?;
    let completed = wait_for_run(&resumed.client, "run-headless-dogfood", |state| {
        state.terminal.as_deref() == Some("succeeded")
    })
    .await?;
    assert_eq!(completed.lifecycle, "terminal");
    for node in &completed.nodes {
        if node.node_id.contains("coding")
            || node.node_id.contains("verification")
            || node.node_id.contains("review")
        {
            assert_eq!(
                node.attempt_count, 1,
                "{} executed more than once",
                node.node_id
            );
        }
    }
    for coding_node in [
        "stage-one-coding",
        "stage-two-coding",
        "stage-two-remediation-1-coding",
    ] {
        let attempt =
            attempt_id_for_node(&resumed.client, "run-headless-dogfood", coding_node).await?;
        let inspected = resumed
            .client
            .attempt("run-headless-dogfood", &attempt)
            .await?;
        assert_eq!(inspected.context_access, "authorized");
        assert_eq!(
            inspected.context.ok_or("coding context absent")?.policy["session"],
            "fresh"
        );
    }
    let progress = fs::read_to_string(repository.join("progress.md"))?;
    assert!(progress.contains("First fresh process"));
    assert!(progress.contains("Second fresh process"));
    assert!(progress.contains("Remediation fresh process"));
    resumed.stop().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn daemon_auth_startup_readiness_and_authority() -> TestResult {
    let directory = tempfile::tempdir()?;
    let daemon = start(configuration(&directory, 16)?, CONTROLLER_TOKEN).await?;
    let raw = reqwest::Client::new();
    let unauthenticated = raw.get(daemon.endpoint.join("v1/health")?).send().await?;
    assert_eq!(unauthenticated.status(), reqwest::StatusCode::UNAUTHORIZED);
    let invalid = raw
        .get(daemon.endpoint.join("v1/health")?)
        .bearer_auth("invalid-token")
        .send()
        .await?;
    assert_eq!(invalid.status(), reqwest::StatusCode::UNAUTHORIZED);

    let authority = daemon.client.authority().await?;
    assert_eq!(authority.actor, "human:integration-controller");
    let observer = client(&daemon.endpoint, OBSERVER_TOKEN)?;
    let denied = observer
        .submit(&request(
            "observer-import-denied",
            None,
            Command::ImportBlueprint {
                document: blueprint()?,
            },
        ))
        .await;
    assert!(matches!(
        denied,
        Err(ClientError::Api(error)) if error.code == ErrorCode::Unauthorized
    ));
    daemon.stop().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn daemon_accepts_commands_across_hot_receipt_turnovers_and_replays_cold_after_restart()
-> TestResult {
    let directory = tempfile::tempdir()?;
    let mut config = configuration(&directory, 16)?;
    config.document.application_receipts.hot_receipt_bound = 1;
    config.document.application_receipts.archive_batch_size = 1;
    let config = config.document.validate(&config.configuration_directory)?;
    let restart_config = config.clone();
    let daemon = start(config, CONTROLLER_TOKEN).await?;
    let import_request = request(
        "capacity-import",
        None,
        Command::ImportBlueprint {
            document: blueprint()?,
        },
    );
    let imported = daemon.client.submit(&import_request).await?;
    let revision = imported
        .value
        .get("revision_id")
        .and_then(serde_json::Value::as_str)
        .ok_or("capacity import omitted revision")?;
    let start_request = request(
        "capacity-start-remains-replayable",
        None,
        Command::StartRun {
            run_id: "run-capacity-accepted".to_owned(),
            workflow_id: "golden".to_owned(),
            revision_id: revision.to_owned(),
        },
    );
    let accepted = daemon.client.submit(&start_request).await?;
    for ordinal in 0..8 {
        daemon
            .client
            .submit(&request(
                &format!("capacity-validate-{ordinal}"),
                None,
                Command::ValidateBlueprint {
                    document: blueprint()?,
                },
            ))
            .await?;
    }
    let health = daemon.client.health().await?;
    assert!(health.application_receipts.cold_count >= 9);
    assert!(health.application_receipts.hot_count <= 1);
    let before_restart = daemon.client.run("run-capacity-accepted").await?;
    daemon.stop().await?;

    let restarted = start(restart_config, CONTROLLER_TOKEN).await?;
    let replay = restarted.client.submit(&start_request).await?;
    assert!(replay.replayed);
    assert_eq!(replay.resulting_sequence, accepted.resulting_sequence);
    assert_eq!(
        restarted
            .client
            .run("run-capacity-accepted")
            .await?
            .sequence,
        before_restart.sequence
    );
    assert!(restarted.client.submit(&import_request).await?.replayed);
    restarted.stop().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn scoped_read_matrix_and_continuations_fail_closed() -> TestResult {
    let directory = tempfile::tempdir()?;
    let profile = configured_process_profile(&directory)?;
    let config = configuration_with_process_profiles(&directory, 16, vec![profile])?;
    let daemon = start(config.clone(), CONTROLLER_TOKEN).await?;
    let golden_revision = import_blueprint(&daemon.client, "matrix-import-golden").await?;
    let process_import = daemon
        .client
        .submit(&request(
            "matrix-import-process",
            None,
            Command::ImportBlueprint {
                document: process_blueprint()?,
            },
        ))
        .await?;
    let process_revision = process_import
        .value
        .get("revision_id")
        .and_then(serde_json::Value::as_str)
        .ok_or("process import omitted revision")?
        .to_owned();
    daemon
        .client
        .submit(&request(
            "matrix-start-golden",
            None,
            Command::StartRun {
                run_id: "run-matrix".to_owned(),
                workflow_id: "golden".to_owned(),
                revision_id: golden_revision.clone(),
            },
        ))
        .await?;
    daemon
        .client
        .submit(&request(
            "matrix-start-process",
            None,
            Command::StartRun {
                run_id: "run-matrix-process".to_owned(),
                workflow_id: "daemon-process".to_owned(),
                revision_id: process_revision.clone(),
            },
        ))
        .await?;
    let layout = LayoutDocument {
        schema_version: 1,
        workflow_id: "golden".to_owned(),
        revision_id: golden_revision.clone(),
        generation: 1,
        author: "human:client-placeholder".to_owned(),
        digest: String::new(),
        nodes: BTreeMap::from([(
            "first".to_owned(),
            LayoutPoint {
                x: 1.0,
                y: 2.0,
                width: None,
                height: None,
            },
        )]),
        collapsed_groups: std::collections::BTreeSet::new(),
        annotations: BTreeMap::new(),
        viewport: None,
    }
    .seal()?;
    daemon
        .client
        .submit(&request(
            "matrix-put-layout",
            None,
            Command::PutLayout { layout },
        ))
        .await?;

    let observer = client(&daemon.endpoint, OBSERVER_TOKEN)?;
    assert!(observer.readiness().await?.ready);
    assert!(matches!(
        observer.health().await,
        Err(ClientError::Api(error)) if error.code == ErrorCode::Unauthorized
    ));
    let visible = observer.capabilities().await?;
    assert_eq!(visible.len(), 1);
    assert_eq!(visible[0].capability_id, "milkdrift-workflow-control");
    assert!(visible[0].provider_profile.is_none());
    assert_eq!(
        observer
            .revisions(
                Some("golden"),
                &PageRequest {
                    cursor: None,
                    limit: 10,
                },
            )
            .await?
            .items
            .len(),
        1
    );
    assert!(matches!(
        observer.revision(&process_revision).await,
        Err(ClientError::Api(error)) if error.code == ErrorCode::NotFound
    ));
    assert!(matches!(
        observer.revisions(
            Some("daemon-process"),
            &PageRequest { cursor: None, limit: 10 },
        ).await,
        Err(ClientError::Api(error)) if error.code == ErrorCode::Unauthorized
    ));
    assert_eq!(
        observer.run("run-matrix").await?.workflow_id.as_deref(),
        Some("golden")
    );
    let hidden_run = observer.run("run-matrix-process").await;
    assert!(
        matches!(
            hidden_run,
            Err(ClientError::Api(ref error)) if error.code == ErrorCode::NotFound
        ),
        "out-of-scope exact run was distinguishable from absence: {hidden_run:?}"
    );
    assert!(
        observer
            .timeline(
                "run-matrix",
                &PageRequest {
                    cursor: None,
                    limit: 10,
                },
            )
            .await?
            .items
            .iter()
            .all(|event| event.run_id == "run-matrix")
    );
    assert!(
        observer
            .proposals(
                "run-matrix",
                &PageRequest {
                    cursor: None,
                    limit: 10,
                },
            )
            .await?
            .items
            .is_empty()
    );
    assert!(matches!(
        observer.runs(
            None,
            Some("daemon-process"),
            &PageRequest { cursor: None, limit: 10 },
        ).await,
        Err(ClientError::Api(error)) if error.code == ErrorCode::Unauthorized
    ));
    assert!(matches!(
        observer.layout("golden", &golden_revision).await,
        Err(ClientError::Api(error)) if error.code == ErrorCode::Unauthorized
    ));

    let first_page = daemon
        .client
        .revisions(
            None,
            &PageRequest {
                cursor: None,
                limit: 1,
            },
        )
        .await?;
    let cursor = first_page
        .next_cursor
        .ok_or("expected a second revision page")?;
    assert!(matches!(
        observer.revisions(
            None,
            &PageRequest {
                cursor: Some(cursor.clone()),
                limit: 1,
            },
        ).await,
        Err(ClientError::Api(error)) if error.code == ErrorCode::InvalidInput
    ));

    let mut health = daemon.client.subscribe("v1/stream/health", None);
    let health_cursor = tokio::time::timeout(Duration::from_secs(3), health.next())
        .await?
        .ok_or("health stream closed before an observation")??
        .cursor;
    drop(health);
    daemon.stop().await?;

    let mut narrowed = config;
    narrowed.document.actors[0].grant_revision = 2;
    narrowed.document.actors[0].authority.resources.capability =
        CapabilityAuthorityScope::deny_all();
    let restarted = start(narrowed, CONTROLLER_TOKEN).await?;
    assert!(matches!(
        restarted.client.revisions(
            None,
            &PageRequest {
                cursor: Some(cursor),
                limit: 1,
            },
        ).await,
        Err(ClientError::Api(error)) if error.code == ErrorCode::InvalidInput
    ));
    let mut reconnect = restarted
        .client
        .subscribe("v1/stream/health", Some(health_cursor));
    assert!(matches!(
        tokio::time::timeout(Duration::from_secs(3), reconnect.next())
            .await?
            .ok_or("narrowed stream ended without a typed error")?,
        Err(ClientError::Api(error)) if error.code == ErrorCode::InvalidInput
    ));
    restarted.stop().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn daemon_command_idempotency_restart_and_stale_conflict() -> TestResult {
    let directory = tempfile::tempdir()?;
    let config = configuration(&directory, 16)?;
    let daemon = start(config.clone(), CONTROLLER_TOKEN).await?;
    let revision = import_blueprint(&daemon.client, "import-retry").await?;
    let replay = daemon
        .client
        .submit(&request(
            "import-retry",
            None,
            Command::ImportBlueprint {
                document: blueprint()?,
            },
        ))
        .await?;
    assert!(replay.replayed);
    assert!(matches!(
        daemon
            .client
            .submit(&request(
                "import-retry",
                None,
                Command::ValidateBlueprint {
                    document: blueprint()?,
                },
            ))
            .await,
        Err(ClientError::Api(error)) if error.code == ErrorCode::Conflict
    ));

    let started = daemon
        .client
        .submit(&request(
            "start-run",
            None,
            Command::StartRun {
                run_id: "run-integration".to_owned(),
                workflow_id: "golden".to_owned(),
                revision_id: revision,
            },
        ))
        .await?;
    assert!(started.resulting_sequence.is_some());
    let stale = daemon
        .client
        .submit(&request(
            "pause-stale",
            Some(0),
            Command::PauseRun {
                run_id: "run-integration".to_owned(),
            },
        ))
        .await;
    let stale_error = match stale {
        Err(ClientError::Api(error)) if error.code == ErrorCode::Conflict => error,
        other => return Err(format!("expected durable stale conflict, got {other:?}").into()),
    };
    let repeated_stale = daemon
        .client
        .submit(&request(
            "pause-stale",
            Some(0),
            Command::PauseRun {
                run_id: "run-integration".to_owned(),
            },
        ))
        .await;
    assert!(matches!(
        repeated_stale,
        Err(ClientError::Api(ref error))
            if error.code == stale_error.code
                && error.message == stale_error.message
                && error.details == stale_error.details
    ));
    daemon.stop().await?;

    let restarted = start(config, CONTROLLER_TOKEN).await?;
    let restart_replay = restarted
        .client
        .submit(&request(
            "import-retry",
            None,
            Command::ImportBlueprint {
                document: blueprint()?,
            },
        ))
        .await?;
    assert!(restart_replay.replayed);
    let restarted_stale = restarted
        .client
        .submit(&request(
            "pause-stale",
            Some(0),
            Command::PauseRun {
                run_id: "run-integration".to_owned(),
            },
        ))
        .await;
    assert!(matches!(
        restarted_stale,
        Err(ClientError::Api(ref error))
            if error.code == stale_error.code
                && error.message == stale_error.message
                && error.details == stale_error.details
    ));
    assert!(restarted.client.run("run-integration").await?.sequence > 0);
    restarted.stop().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn layout_is_optimistic_restart_durable_and_semantically_inert() -> TestResult {
    let directory = tempfile::tempdir()?;
    let config = configuration(&directory, 16)?;
    let daemon = start(config.clone(), CONTROLLER_TOKEN).await?;
    let revision_id = import_blueprint(&daemon.client, "layout-import").await?;
    let semantic_digest = daemon
        .client
        .revision(&revision_id)
        .await?
        .summary
        .semantic_digest;
    let layout = LayoutDocument {
        schema_version: 1,
        workflow_id: "golden".to_owned(),
        revision_id: revision_id.clone(),
        generation: 1,
        author: "ignored:client-author".to_owned(),
        digest: String::new(),
        nodes: BTreeMap::from([(
            "first".to_owned(),
            LayoutPoint {
                x: 10.0,
                y: 20.0,
                width: Some(120.0),
                height: None,
            },
        )]),
        collapsed_groups: Default::default(),
        annotations: Default::default(),
        viewport: None,
    }
    .seal()?;
    daemon
        .client
        .submit(&request(
            "layout-put-one",
            None,
            Command::PutLayout {
                layout: layout.clone(),
            },
        ))
        .await?;
    let mut stale = layout.clone();
    stale.nodes.get_mut("first").ok_or("missing layout node")?.x = 30.0;
    stale.digest = stale.computed_digest()?;
    let first_conflict = daemon
        .client
        .submit(&request(
            "layout-stale",
            None,
            Command::PutLayout {
                layout: stale.clone(),
            },
        ))
        .await;
    assert!(matches!(
        first_conflict,
        Err(ClientError::Api(error)) if error.code == ErrorCode::Conflict
    ));
    assert!(matches!(
        daemon
            .client
            .submit(&request(
                "layout-stale",
                None,
                Command::PutLayout { layout: stale },
            ))
            .await,
        Err(ClientError::Api(error)) if error.code == ErrorCode::Conflict
    ));
    assert_eq!(
        daemon.client.layout("golden", &revision_id).await?.nodes,
        layout.nodes
    );
    assert_eq!(
        daemon
            .client
            .revision(&revision_id)
            .await?
            .summary
            .semantic_digest,
        semantic_digest
    );
    assert!(
        !config
            .document
            .data_root
            .join("control-state-v1.json")
            .exists()
    );
    daemon.stop().await?;

    let restarted = start(config, CONTROLLER_TOKEN).await?;
    assert_eq!(
        restarted.client.layout("golden", &revision_id).await?.nodes,
        layout.nodes
    );
    assert_eq!(
        restarted
            .client
            .revision(&revision_id)
            .await?
            .summary
            .semantic_digest,
        semantic_digest
    );
    restarted.stop().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn proposal_listing_uses_durable_projection_and_survives_restart() -> TestResult {
    let directory = tempfile::tempdir()?;
    let config = configuration(&directory, 16)?;
    let daemon = start(config.clone(), CONTROLLER_TOKEN).await?;
    let revision_id = import_blueprint(&daemon.client, "proposal-index-import").await?;
    daemon
        .client
        .submit(&request(
            "proposal-index-start",
            None,
            Command::StartRun {
                run_id: "run-proposal-index".to_owned(),
                workflow_id: "golden".to_owned(),
                revision_id: revision_id.clone(),
            },
        ))
        .await?;
    let started_sequence = daemon.client.run("run-proposal-index").await?.sequence;
    daemon
        .client
        .submit(&request(
            "proposal-index-pause",
            Some(started_sequence),
            Command::PauseRun {
                run_id: "run-proposal-index".to_owned(),
            },
        ))
        .await?;
    let run = RunId::new("run-proposal-index")?;
    let sequence = daemon.client.run(run.as_str()).await?.sequence;
    let mut submit = request(
        "proposal-index-submit",
        Some(sequence),
        Command::SubmitProposal {
            document: proposal_document(&run, sequence)?,
        },
    );
    submit.expected_revision = Some(revision_id);
    daemon.client.submit(&submit).await?;
    let listed = daemon
        .client
        .proposals(
            run.as_str(),
            &PageRequest {
                cursor: None,
                limit: 10,
            },
        )
        .await?;
    assert_eq!(listed.items.len(), 1);
    assert_eq!(listed.items[0].proposal_id, "proposal-daemon-index");
    daemon.stop().await?;

    let restarted = start(config, CONTROLLER_TOKEN).await?;
    let reopened = restarted
        .client
        .proposals(
            run.as_str(),
            &PageRequest {
                cursor: None,
                limit: 10,
            },
        )
        .await?;
    assert_eq!(reopened.items, listed.items);
    restarted.stop().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn daemon_bounded_overload_returns_stable_error() -> TestResult {
    let directory = tempfile::tempdir()?;
    let daemon = start(configuration(&directory, 1)?, CONTROLLER_TOKEN).await?;
    let raw = reqwest::Client::new();
    let target = daemon.endpoint.join("v1/revisions?limit=100")?;
    let mut overload = None;

    for _round in 0..4 {
        let barrier = Arc::new(tokio::sync::Barrier::new(129));
        let mut requests = Vec::new();
        for _ in 0..128 {
            let barrier = barrier.clone();
            let raw = raw.clone();
            let target = target.clone();
            requests.push(tokio::spawn(async move {
                barrier.wait().await;
                raw.get(target).bearer_auth(CONTROLLER_TOKEN).send().await
            }));
        }
        barrier.wait().await;
        for request in requests {
            let response = request.await??;
            if response.status() == reqwest::StatusCode::TOO_MANY_REQUESTS {
                overload = Some(response.bytes().await?);
            }
        }
        if overload.is_some() {
            break;
        }
    }

    let bytes = overload.ok_or("concurrent requests did not exercise the queue bound")?;
    let error: milkdrift_control_protocol::ErrorEnvelope = decode_json(&bytes)?;
    assert_eq!(error.code, ErrorCode::Overload);
    assert!(error.retryable);
    daemon.stop().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn daemon_stream_reconnect_auth_rotation_and_shutdown() -> TestResult {
    let directory = tempfile::tempdir()?;
    let daemon = start(configuration(&directory, 16)?, CONTROLLER_TOKEN).await?;
    let revision = import_blueprint(&daemon.client, "stream-import").await?;
    daemon
        .client
        .submit(&request(
            "stream-start",
            None,
            Command::StartRun {
                run_id: "run-stream".to_owned(),
                workflow_id: "golden".to_owned(),
                revision_id: revision,
            },
        ))
        .await?;

    let mut stream = daemon.client.subscribe("v1/runs/run-stream/stream", None);
    let first = tokio::time::timeout(Duration::from_secs(3), stream.next())
        .await?
        .ok_or("run stream closed before its first observation")??;
    assert_eq!(first.feed, "run:run-stream");
    let first_position = first.cursor.position_for("run:run-stream")?;
    drop(stream);

    let mut resumed = daemon
        .client
        .subscribe("v1/runs/run-stream/stream", Some(first.cursor));
    let second = tokio::time::timeout(Duration::from_secs(3), resumed.next())
        .await?
        .ok_or("resumed run stream closed")??;
    assert!(second.cursor.position_for("run:run-stream")? > first_position);
    drop(resumed);

    let mut capabilities = daemon.client.subscribe("v1/stream/capabilities", None);
    let capability = tokio::time::timeout(Duration::from_secs(3), capabilities.next())
        .await?
        .ok_or("capability stream closed before its first observation")??;
    assert_eq!(capability.feed, "capability-health");
    assert!(matches!(capability.observation, Observation::Capability(_)));
    drop(capabilities);

    let mut health = daemon.client.subscribe("v1/stream/health", None);
    let initial_health = tokio::time::timeout(Duration::from_secs(3), health.next())
        .await?
        .ok_or("health stream closed before its first observation")??;
    assert!(matches!(
        initial_health.observation,
        Observation::DaemonHealth(_)
    ));
    write_secret(
        &directory.path().join("controller.token"),
        "rotated-controller-token",
    )?;
    let closing = tokio::time::timeout(Duration::from_secs(3), health.next())
        .await?
        .ok_or("health stream closed without a rotation observation")??;
    assert!(matches!(
        closing.observation,
        Observation::StreamClosing { .. }
    ));
    assert!(
        matches!(daemon.client.health().await, Err(ClientError::Api(error)) if error.code == ErrorCode::Unauthenticated)
    );
    assert!(
        client(&daemon.endpoint, "rotated-controller-token")?
            .readiness()
            .await?
            .ready
    );
    daemon.stop().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn daemon_graceful_shutdown_and_restart() -> TestResult {
    let directory = tempfile::tempdir()?;
    let config = configuration(&directory, 16)?;
    let daemon = start(config.clone(), CONTROLLER_TOKEN).await?;
    daemon.stop().await?;
    let restarted = start(config, CONTROLLER_TOKEN).await?;
    assert!(restarted.client.readiness().await?.ready);
    restarted.stop().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn daemon_configured_process_adapter_executes_to_terminal() -> TestResult {
    let directory = tempfile::tempdir()?;
    let profile = configured_process_profile(&directory)?;
    let config = configuration_with_process_profiles(&directory, 16, vec![profile])?;
    let (artifact_id, artifact_bytes) =
        publish_restricted_test_artifact(&config.document.data_root)?;
    let daemon = start(config.clone(), CONTROLLER_TOKEN).await?;
    assert!(
        daemon
            .client
            .capabilities()
            .await?
            .iter()
            .any(|capability| capability.capability_id == "golden-local-process")
    );
    let imported = daemon
        .client
        .submit(&request(
            "process-import",
            None,
            Command::ImportBlueprint {
                document: process_blueprint()?,
            },
        ))
        .await?;
    let revision = imported
        .value
        .get("revision_id")
        .and_then(serde_json::Value::as_str)
        .ok_or("process import response omitted revision identity")?;
    daemon
        .client
        .submit(&request(
            "process-start",
            None,
            Command::StartRun {
                run_id: "run-process".to_owned(),
                workflow_id: "daemon-process".to_owned(),
                revision_id: revision.to_owned(),
            },
        ))
        .await?;

    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        let run = daemon.client.run("run-process").await?;
        if run.lifecycle == "terminal" {
            assert_eq!(run.terminal.as_deref(), Some("succeeded"));
            break;
        }
        if tokio::time::Instant::now() >= deadline {
            let health = daemon.client.health().await?;
            let capabilities = daemon.client.capabilities().await?;
            let timeline = daemon
                .client
                .timeline(
                    "run-process",
                    &PageRequest {
                        cursor: None,
                        limit: 100,
                    },
                )
                .await?;
            return Err(format!(
                "configured process invocation did not reach terminal state: run={run:?}, health={health:?}, capabilities={capabilities:?}, timeline={:?}",
                timeline.items
            )
            .into());
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let attempt_id = daemon
        .client
        .timeline(
            "run-process",
            &PageRequest {
                cursor: None,
                limit: 100,
            },
        )
        .await?
        .items
        .into_iter()
        .find(|entry| entry.node_id.as_deref() == Some("process") && entry.attempt_id.is_some())
        .and_then(|entry| entry.attempt_id)
        .ok_or("process run omitted its exact attempt")?;
    let attempt = daemon.client.attempt("run-process", &attempt_id).await?;
    assert_eq!(attempt.context_access, "authorized");
    assert_eq!(
        attempt.capability_id.as_deref(),
        Some("golden-local-process")
    );
    assert_eq!(attempt.descriptor_revision, Some(2));
    let provenance = attempt
        .capability_provenance
        .as_ref()
        .ok_or("process attempt omitted exact capability provenance")?;
    assert_eq!(provenance.execution_trust, "trusted_host_process");
    assert_eq!(provenance.snapshot_digest.len(), 64);
    assert!(
        provenance
            .snapshot_digest
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    );
    assert!(
        provenance
            .implementation_identity
            .as_deref()
            .is_some_and(|digest| digest.starts_with("b3_"))
    );
    assert!(
        provenance
            .implementation_content_digest
            .as_deref()
            .is_some_and(|digest| digest.starts_with("b3_"))
    );
    assert!(
        provenance
            .implementation_size_bytes
            .is_some_and(|size| size > 0)
    );
    assert!(
        provenance
            .process_profile_digest
            .as_deref()
            .is_some_and(|digest| digest.starts_with("b3_"))
    );
    assert!(
        provenance
            .execution_policy_digest
            .as_deref()
            .is_some_and(|digest| digest.starts_with("b3_"))
    );
    let manifest_metadata = attempt
        .context_manifest
        .as_ref()
        .ok_or("process attempt omitted context manifest metadata")?;
    assert_eq!(
        manifest_metadata.content_type,
        "application/vnd.milkdrift.context-manifest.v2+json"
    );
    let inspected_context = attempt
        .context
        .clone()
        .ok_or("authorized process attempt omitted context details")?;
    assert_eq!(inspected_context.schema_version, 2);
    assert!(inspected_context.digest.starts_with("b3_"));
    assert!(inspected_context.entries.is_empty());
    assert!(!inspected_context.truncated);
    let metadata = daemon.client.artifact_metadata(&artifact_id).await?;
    assert_eq!(metadata.artifact_id, artifact_id);
    let range = daemon
        .client
        .artifact_range(&artifact_id, 0, metadata.size.saturating_sub(1))
        .await?;
    assert_eq!(range.bytes, artifact_bytes);
    let observer = client(&daemon.endpoint, OBSERVER_TOKEN)?;
    assert!(matches!(
        observer.artifact_metadata(&artifact_id).await,
        Err(ClientError::Api(error)) if error.code == ErrorCode::Unauthorized
    ));
    assert!(matches!(
        observer.artifact_range(&artifact_id, 0, 0).await,
        Err(ClientError::Api(error)) if error.code == ErrorCode::Unauthorized
    ));
    assert!(!directory.path().join("data/control-state-v1.json").exists());
    daemon.stop().await?;
    let store = RedbStore::open(directory.path().join("data"))?;
    let audit = store.security_audit(&ApplicationPageQuery {
        after: None,
        limit: PageSize::new(1_000)?,
    })?;
    assert!(audit.items.iter().any(|record| {
        record.entry.operation == "read_artifact_content"
            && record.entry.grant_revision == 1
            && record.entry.decision_digest.starts_with("b3_")
    }));
    drop(store);
    let restarted = start(config, CONTROLLER_TOKEN).await?;
    let reopened = restarted.client.attempt("run-process", &attempt_id).await?;
    assert_eq!(reopened.context_access, "authorized");
    assert_eq!(reopened.context, Some(inspected_context));
    assert_eq!(reopened.context_manifest, attempt.context_manifest);
    restarted.stop().await
}

#[test]
fn daemon_startup_refuses_legacy_sidecar_and_peer_prototype_authority() -> TestResult {
    let directory = tempfile::tempdir()?;
    let config = configuration(&directory, 16)?;
    fs::create_dir_all(&config.document.data_root)?;
    fs::write(
        config.document.data_root.join("control-state-v1.json"),
        br#"{"schema_version":1,"layouts":{},"commands":{"broken":true}}"#,
    )?;
    let result = DaemonHost::start(config);
    assert!(result.is_err());
    for prototype in ["peer-executions-v1", "peer-artifacts-v1"] {
        let directory = tempfile::tempdir()?;
        let config = configuration(&directory, 16)?;
        fs::create_dir_all(config.document.data_root.join(prototype))?;
        assert!(DaemonHost::start(config).is_err());
    }
    Ok(())
}

#[test]
fn page_requests_remain_explicitly_bounded() -> TestResult {
    PageRequest {
        cursor: None,
        limit: 100,
    }
    .validate()?;
    Ok(())
}
