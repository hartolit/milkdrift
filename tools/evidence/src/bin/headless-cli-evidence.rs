//! Shell-free actual-binary proof for the storage-free Milkdrift CLI product surface.

use std::{
    fs,
    path::{Path, PathBuf},
    thread,
    time::Duration,
};

use clap::Parser;
use milkdrift_authority::ActorRef;
use milkdrift_blueprint::{
    AuthorRef, BlueprintMetadata, BlueprintRevision, BlueprintRevisionDocument, DataPort, Edge,
    EdgeId, EdgeKind, Mutation, MutationBatch, Node, NodeId, NodeKind, PortId, SchemaRef,
    TerminalOutcome, WorkflowId,
};
use milkdrift_capability::{CapabilityId, CapabilityRequirement, OperationId, SchemaId};
use milkdrift_control::{
    ClaimedStopCondition, ProposalApplicationPolicy, ProposalId, ProposalProvenance,
    WorkflowProposal, WorkflowProposalDocument,
};
use milkdrift_persistence::RunSequence;
use milkdrift_workspace::RunId;
use serde_json::Value;

#[path = "headless_cli_evidence/harness.rs"]
mod harness;

use harness::{
    CliRunner, EvidenceConfig, assert_error, reserve_endpoint, start_daemon, stop_daemon,
    wait_for_failed_exit, wait_for_readiness, wait_for_run, write_config, write_process_profile,
};

type EvidenceResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

const TOKEN: &str = "headless-cli-evidence-token";
const WRONG_TOKEN: &str = "headless-cli-evidence-wrong-token";
const ACTOR: &str = "human:headless-cli-evidence";

#[derive(Parser)]
#[command(name = "headless-cli-evidence")]
struct Arguments {
    /// Built `milkdrift-daemon` executable.
    #[arg(long)]
    daemon: PathBuf,
    /// Built `milkdrift` executable.
    #[arg(long)]
    cli: PathBuf,
}

fn main() {
    if let Some(mode) = std::env::args_os()
        .nth(1)
        .and_then(|value| value.into_string().ok())
    {
        match mode.as_str() {
            "--fixture-artifact" => {
                println!("headless-cli-artifact");
                return;
            }
            "--fixture-wait" => {
                thread::sleep(Duration::from_secs(5));
                return;
            }
            _ => {}
        }
    }
    if let Err(error) = run(Arguments::parse()) {
        eprintln!("headless CLI evidence failed: {error}");
        std::process::exit(1);
    }
}

fn run(arguments: Arguments) -> EvidenceResult {
    require_executable(&arguments.daemon)?;
    require_executable(&arguments.cli)?;
    let directory = tempfile::tempdir()?;
    let endpoint = reserve_endpoint()?;
    let executable = std::env::current_exe()?;
    let token_file = write_private(&directory.path().join("operator.token"), TOKEN.as_bytes())?;
    let wrong_token_file = write_private(
        &directory.path().join("wrong.token"),
        WRONG_TOKEN.as_bytes(),
    )?;
    let artifact_profile = write_process_profile(
        directory.path(),
        &executable,
        "headless-artifact-profile",
        "headless-artifact-capability",
        "--fixture-artifact",
        "none",
        Some("stdout"),
    )?;
    let wait_profile = write_process_profile(
        directory.path(),
        &executable,
        "headless-wait-profile",
        "headless-wait-capability",
        "--fixture-wait",
        "unknown",
        None,
    )?;
    let config_path = write_config(
        directory.path(),
        endpoint,
        &token_file,
        EvidenceConfig {
            process_profiles: vec![artifact_profile, wait_profile],
            model_profiles: Vec::new(),
            secret_sources: Default::default(),
            lease_duration_ms: 100,
            authority: milkdrift_daemon::ActorGrantConfig::dangerous_administrator(),
        },
    )?;
    let runner = CliRunner {
        executable: arguments.cli,
        endpoint: format!("http://{endpoint}/"),
        token_file,
        forbidden_storage_path: directory.path().join("data"),
    };
    let mut daemon = start_daemon(&arguments.daemon, &config_path)?;
    wait_for_readiness(&runner, &mut daemon)?;

    runner.success(&["daemon", "readiness"])?;
    runner.success(&["daemon", "authority"])?;
    let unauthorized = runner.run_with_token(&wrong_token_file, &["daemon", "authority"], None)?;
    assert_error(&unauthorized, 3, "authorization", Some("unauthenticated"))?;

    let initial_revisions = runner.success(&["blueprint", "list", "--limit", "10"])?;
    let invalid_path = directory.path().join("invalid-blueprint.json");
    fs::write(&invalid_path, b"{}")?;
    let invalid = runner.run(
        &[
            "--command-id",
            "blueprint-invalid-1",
            "blueprint",
            "validate",
            path_text(&invalid_path)?,
        ],
        None,
    )?;
    assert_error(&invalid, 7, "daemon_api", Some("invalid_input"))?;
    let duplicate_path = directory.path().join("duplicate-blueprint.json");
    fs::write(
        &duplicate_path,
        br#"{"schema_version":2,"schema_version":2}"#,
    )?;
    let duplicate = runner.run(
        &["blueprint", "validate", path_text(&duplicate_path)?],
        None,
    )?;
    assert_error(&duplicate, 2, "invalid_input", None)?;
    let after_invalid = runner.success(&["blueprint", "list", "--limit", "10"])?;
    ensure(
        initial_revisions["value"]["items"] == after_invalid["value"]["items"],
        "invalid blueprint validation changed revision storage",
    )?;

    let primary = process_blueprint("headless-cli-primary", "headless-artifact-capability", true)?;
    let primary_bytes = BlueprintRevisionDocument::new(&primary).to_canonical_json()?;
    let primary_path = directory.path().join("primary-blueprint.json");
    fs::write(&primary_path, &primary_bytes)?;
    let validated = runner.success_with_input(
        &[
            "--command-id",
            "blueprint-validate-1",
            "blueprint",
            "validate",
            "-",
        ],
        &primary_bytes,
    )?;
    ensure(
        validated["value"]["result_type"] == "blueprint_valid",
        "valid blueprint did not validate through the daemon",
    )?;
    let absent = runner.run(&["blueprint", "show", primary.id().as_str()], None)?;
    assert_error(&absent, 6, "not_found", Some("not_found"))?;

    let imported = runner.success(&[
        "--command-id",
        "blueprint-import-1",
        "blueprint",
        "import",
        path_text(&primary_path)?,
    ])?;
    let revision = required_text(&imported, &["value", "value", "revision_id"])?;
    ensure(
        revision == primary.id().as_str(),
        "import returned another revision",
    )?;
    let replay = runner.success(&[
        "--command-id",
        "blueprint-import-1",
        "blueprint",
        "import",
        path_text(&primary_path)?,
    ])?;
    ensure(
        replay["value"]["replayed"] == true,
        "exact import command did not replay",
    )?;
    let changed = terminal_blueprint("headless-cli-changed", TerminalOutcome::Success)?;
    let changed_path = directory.path().join("changed-blueprint.json");
    fs::write(
        &changed_path,
        BlueprintRevisionDocument::new(&changed).to_canonical_json()?,
    )?;
    let conflict = runner.run(
        &[
            "--command-id",
            "blueprint-import-1",
            "blueprint",
            "import",
            path_text(&changed_path)?,
        ],
        None,
    )?;
    assert_error(&conflict, 4, "conflict", Some("conflict"))?;

    let exact = runner.run(&["blueprint", "show", &revision, "--document"], None)?;
    ensure(exact.status.success(), "canonical blueprint stdout failed")?;
    ensure(
        exact.stdout.as_bytes() == primary_bytes,
        "canonical stdout bytes changed",
    )?;
    let exported_path = directory.path().join("exported-blueprint.json");
    runner.success(&[
        "blueprint",
        "show",
        &revision,
        "--output",
        path_text(&exported_path)?,
    ])?;
    ensure(
        fs::read(&exported_path)? == primary_bytes,
        "canonical file bytes changed",
    )?;
    let existing = runner.run(
        &[
            "blueprint",
            "show",
            &revision,
            "--output",
            path_text(&exported_path)?,
        ],
        None,
    )?;
    assert_error(&existing, 2, "invalid_input", None)?;

    let started = runner.success(&[
        "--command-id",
        "run-primary-start-1",
        "run",
        "start",
        "run-headless-primary",
        "headless-cli-primary",
        &revision,
    ])?;
    let start_replay = runner.success(&[
        "--command-id",
        "run-primary-start-1",
        "run",
        "start",
        "run-headless-primary",
        "headless-cli-primary",
        &revision,
    ])?;
    ensure(
        started["value"]["command_id"] == start_replay["value"]["command_id"]
            && start_replay["value"]["replayed"] == true,
        "exact run-start command did not replay",
    )?;
    let start_conflict = runner.run(
        &[
            "--command-id",
            "run-primary-start-1",
            "run",
            "start",
            "run-headless-other",
            "headless-cli-primary",
            &revision,
        ],
        None,
    )?;
    assert_error(&start_conflict, 4, "conflict", Some("conflict"))?;

    let waiting = wait_for_run(&runner, "run-headless-primary", |run| {
        run["value"]["nodes"]
            .as_array()
            .is_some_and(|nodes| nodes.iter().any(|node| node["node_id"] == "approval"))
    })?;
    let pause_sequence = required_u64(&waiting, &["value", "sequence"])?;
    runner.success(&[
        "--command-id",
        "run-primary-pause-1",
        "--expected-sequence",
        &pause_sequence.to_string(),
        "run",
        "pause",
        "run-headless-primary",
    ])?;
    let paused = runner.success(&["run", "show", "run-headless-primary"])?;
    ensure(
        paused["value"]["lifecycle"] == "paused",
        "run did not pause",
    )?;
    let paused_sequence = required_u64(&paused, &["value", "sequence"])?;

    let process_node = paused["value"]["nodes"]
        .as_array()
        .and_then(|nodes| nodes.iter().find(|node| node["node_id"] == "process"))
        .ok_or("process node is absent")?;
    let execution = required_text(process_node, &["execution_id"])?;
    let attempt = required_text(process_node, &["latest_attempt_id"])?;
    runner.success(&["node", "run-headless-primary", &execution])?;
    let attempt_read = runner.success(&["attempt", "inspect", "run-headless-primary", &attempt])?;
    runner.success(&["run", "timeline", "run-headless-primary", "--limit", "100"])?;

    let proposal = proposal_document(&primary, paused_sequence)?;
    let proposal_digest = proposal.proposal().digest().as_str().to_owned();
    let proposal_path = directory.path().join("proposal.json");
    fs::write(&proposal_path, proposal.to_canonical_json()?)?;
    let submitted = runner.success(&[
        "--command-id",
        "proposal-submit-1",
        "--expected-sequence",
        &paused_sequence.to_string(),
        "--expected-revision",
        primary.id().as_str(),
        "proposal",
        "submit",
        path_text(&proposal_path)?,
    ])?;
    let proposed_revision = required_text(&submitted, &["value", "value", "proposed_revision"])?;
    runner.success(&[
        "proposal",
        "show",
        "run-headless-primary",
        "proposal-headless-cli-1",
        &proposed_revision,
    ])?;
    let decision_state = runner.success(&["run", "show", "run-headless-primary"])?;
    let decision_sequence = required_u64(&decision_state, &["value", "sequence"])?;
    runner.success(&[
        "--yes",
        "--command-id",
        "proposal-approve-1",
        "--expected-sequence",
        &decision_sequence.to_string(),
        "--expected-revision",
        &proposed_revision,
        "proposal",
        "approve",
        "run-headless-primary",
        "proposal-headless-cli-1",
        &proposal_digest,
        &proposed_revision,
        "proposal-decision-1",
    ])?;
    let apply_state = runner.success(&["run", "show", "run-headless-primary"])?;
    let apply_sequence = required_u64(&apply_state, &["value", "sequence"])?;
    runner.success(&[
        "--yes",
        "--command-id",
        "proposal-apply-1",
        "--expected-sequence",
        &apply_sequence.to_string(),
        "--expected-revision",
        &proposed_revision,
        "proposal",
        "apply",
        "run-headless-primary",
        "proposal-headless-cli-1",
        &proposal_digest,
        &proposed_revision,
    ])?;
    let signal_state = runner.success(&["run", "show", "run-headless-primary"])?;
    let signal_sequence = required_u64(&signal_state, &["value", "sequence"])?;
    runner.success(&[
        "--command-id",
        "run-primary-signal-1",
        "--expected-sequence",
        &signal_sequence.to_string(),
        "run",
        "signal",
        "run-headless-primary",
        "--signal-id",
        "signal-primary-1",
        "--signal-type",
        "evidence.continue",
        "--payload",
        r#"{"approved":true}"#,
    ])?;
    let resume_state = runner.success(&["run", "show", "run-headless-primary"])?;
    let resume_sequence = required_u64(&resume_state, &["value", "sequence"])?;
    runner.success(&[
        "--command-id",
        "run-primary-resume-1",
        "--expected-sequence",
        &resume_sequence.to_string(),
        "run",
        "resume",
        "run-headless-primary",
    ])?;
    wait_for_run(&runner, "run-headless-primary", |run| {
        run["value"]["terminal"] == "succeeded"
    })?;

    let artifact = attempt_read["value"]["outputs"]
        .as_array()
        .and_then(|outputs| outputs.iter().find(|output| output["name"] == "stdout"))
        .and_then(|output| output.get("artifact"))
        .ok_or("attempt did not expose its stdout artifact")?;
    let artifact_id = required_text(artifact, &["artifact_id"])?;
    let artifact_digest = required_text(artifact, &["digest"])?;
    let artifact_size = required_u64(artifact, &["size"])?;
    let metadata = runner.success(&["artifact", "metadata", &artifact_id])?;
    ensure(
        metadata["value"]["digest"] == artifact_digest
            && metadata["value"]["size"] == artifact_size,
        "artifact metadata changed",
    )?;
    let artifact_path = directory.path().join("downloaded-artifact.bin");
    runner.success(&[
        "artifact",
        "get",
        &artifact_id,
        "--output",
        path_text(&artifact_path)?,
    ])?;
    let downloaded = fs::read(&artifact_path)?;
    ensure(
        u64::try_from(downloaded.len())? == artifact_size
            && blake3::hash(&downloaded).to_hex().as_str() == artifact_digest,
        "artifact download did not preserve size and digest",
    )?;

    stop_daemon(&mut daemon)?;
    daemon = start_daemon(&arguments.daemon, &config_path)?;
    wait_for_readiness(&runner, &mut daemon)?;
    runner.success(&["blueprint", "show", &revision])?;
    runner.success(&["run", "show", "run-headless-primary"])?;
    runner.success(&["artifact", "metadata", &artifact_id])?;

    let uncertain_blueprint =
        process_blueprint("headless-cli-uncertain", "headless-wait-capability", false)?;
    let uncertain_path = directory.path().join("uncertain-blueprint.json");
    fs::write(
        &uncertain_path,
        BlueprintRevisionDocument::new(&uncertain_blueprint).to_canonical_json()?,
    )?;
    runner.success(&[
        "--command-id",
        "uncertain-import-1",
        "blueprint",
        "import",
        path_text(&uncertain_path)?,
    ])?;
    runner.success(&[
        "--command-id",
        "uncertain-start-1",
        "run",
        "start",
        "run-headless-uncertain",
        "headless-cli-uncertain",
        uncertain_blueprint.id().as_str(),
    ])?;
    let entered = wait_for_run(&runner, "run-headless-uncertain", |run| {
        run["value"]["nodes"].as_array().is_some_and(|nodes| {
            nodes.iter().any(|node| {
                node["node_id"] == "process" && node["latest_attempt"]["invocation_id"].is_string()
            })
        })
    })?;
    let uncertain_attempt = entered["value"]["nodes"]
        .as_array()
        .and_then(|nodes| nodes.iter().find(|node| node["node_id"] == "process"))
        .and_then(|node| node["latest_attempt_id"].as_str())
        .ok_or("uncertain run omitted its attempt identity")?
        .to_owned();
    stop_daemon(&mut daemon)?;
    thread::sleep(Duration::from_millis(200));
    daemon = start_daemon(&arguments.daemon, &config_path)?;
    wait_for_readiness(&runner, &mut daemon)?;
    let uncertain = wait_for_run(&runner, "run-headless-uncertain", |run| {
        run["value"]["uncertainty_count"]
            .as_u64()
            .is_some_and(|count| count > 0)
    })?;
    let resolution_sequence = required_u64(&uncertain, &["value", "sequence"])?;
    runner.success(&[
        "--command-id",
        "uncertain-retain-1",
        "--expected-sequence",
        &resolution_sequence.to_string(),
        "--evidence",
        "recovery_observation=evidence-retain-1",
        "attempt",
        "resolve",
        "run-headless-uncertain",
        &uncertain_attempt,
        "uncertain-decision-1",
        "--action",
        "retain",
    ])?;
    let retained = runner.success(&[
        "attempt",
        "inspect",
        "run-headless-uncertain",
        &uncertain_attempt,
    ])?;
    ensure(
        retained["value"]["uncertain"] == true,
        "retained work was hidden",
    )?;

    let failed_blueprint = terminal_blueprint("headless-cli-failed", TerminalOutcome::Failure)?;
    let failed_path = directory.path().join("failed-blueprint.json");
    fs::write(
        &failed_path,
        BlueprintRevisionDocument::new(&failed_blueprint).to_canonical_json()?,
    )?;
    runner.success(&[
        "--command-id",
        "failed-import-1",
        "blueprint",
        "import",
        path_text(&failed_path)?,
    ])?;
    runner.success(&[
        "--command-id",
        "failed-start-1",
        "run",
        "start",
        "run-headless-failed",
        "headless-cli-failed",
        failed_blueprint.id().as_str(),
    ])?;
    wait_for_failed_exit(&runner, "run-headless-failed")?;

    stop_daemon(&mut daemon)?;
    let unavailable = runner.run(&["daemon", "health"], None)?;
    assert_error(&unavailable, 5, "unavailable", None)?;
    println!(
        "headless CLI evidence passed: actual daemon/CLI, restart, replay/conflict, proposal, artifact, and uncertainty paths"
    );
    Ok(())
}

fn process_blueprint(
    workflow: &str,
    capability: &str,
    with_signal: bool,
) -> EvidenceResult<BlueprintRevision> {
    let requirement = CapabilityRequirement::new(OperationId::new("process.execute")?)
        .exact(CapabilityId::new(capability)?);
    let process = Node::new(
        NodeId::new("process")?,
        NodeKind::task_direct_inputs(requirement)?,
    )?
    .with_control_output(PortId::new("next")?)?
    .with_data_output(
        PortId::new("stdout")?,
        DataPort::output(SchemaRef::new(SchemaId::new("evidence.stdout")?, 1)?),
    )?;
    let mut mutations = vec![Mutation::AddNode { node: process }];
    if with_signal {
        let signal = Node::new(
            NodeId::new("approval")?,
            NodeKind::SignalWait {
                signal: OperationId::new("evidence.continue")?,
            },
        )?
        .with_control_input(PortId::new("in")?)?
        .with_control_output(PortId::new("next")?)?;
        let done = terminal_node("done", TerminalOutcome::Success, true)?;
        mutations.extend([
            Mutation::AddNode { node: signal },
            Mutation::AddNode { node: done },
            control_edge("process-approval", "process", "next", "approval", "in")?,
            control_edge("approval-done", "approval", "next", "done", "in")?,
        ]);
    } else {
        let done = terminal_node("done", TerminalOutcome::Success, true)?;
        mutations.extend([
            Mutation::AddNode { node: done },
            control_edge("process-done", "process", "next", "done", "in")?,
        ]);
    }
    BlueprintRevision::genesis(
        WorkflowId::new(workflow)?,
        MutationBatch::new(mutations)?,
        AuthorRef::new(ACTOR)?,
        "actual-binary headless CLI evidence",
    )
    .map_err(Into::into)
}

fn terminal_blueprint(
    workflow: &str,
    outcome: TerminalOutcome,
) -> EvidenceResult<BlueprintRevision> {
    BlueprintRevision::genesis(
        WorkflowId::new(workflow)?,
        MutationBatch::new(vec![Mutation::AddNode {
            node: terminal_node("done", outcome, false)?,
        }])?,
        AuthorRef::new(ACTOR)?,
        "actual-binary terminal evidence",
    )
    .map_err(Into::into)
}

fn terminal_node(identity: &str, outcome: TerminalOutcome, input: bool) -> EvidenceResult<Node> {
    let node = Node::new(NodeId::new(identity)?, NodeKind::Terminal { outcome })?;
    if input {
        Ok(node.with_control_input(PortId::new("in")?)?)
    } else {
        Ok(node)
    }
}

fn control_edge(
    identity: &str,
    source: &str,
    source_port: &str,
    target: &str,
    target_port: &str,
) -> EvidenceResult<Mutation> {
    Ok(Mutation::AddEdge {
        edge: Edge::new(
            EdgeId::new(identity)?,
            EdgeKind::Control,
            NodeId::new(source)?,
            PortId::new(source_port)?,
            NodeId::new(target)?,
            PortId::new(target_port)?,
        ),
    })
}

fn proposal_document(
    base: &BlueprintRevision,
    observed_sequence: u64,
) -> EvidenceResult<WorkflowProposalDocument> {
    let proposal = WorkflowProposal::new(
        ProposalId::new("proposal-headless-cli-1")?,
        ActorRef::new(ACTOR)?,
        ProposalProvenance::Direct,
        base.semantic().workflow().clone(),
        Some(RunId::new("run-headless-primary")?),
        base.id().clone(),
        base.content_digest().clone(),
        Some(RunSequence::new(observed_sequence)),
        MutationBatch::new(vec![Mutation::SetMetadata {
            metadata: BlueprintMetadata::new(
                "Headless CLI primary (approved)",
                "Prospective metadata revision exercised by the actual CLI binary",
                Default::default(),
                Default::default(),
            )?,
        }])?,
        "exercise proposal submit, approval, and application through the CLI",
        None,
        Vec::new(),
        Vec::new(),
        Vec::new(),
        Vec::new(),
        ProposalApplicationPolicy::ProposeOnly,
        None,
        ClaimedStopCondition::Continue,
    )?;
    Ok(WorkflowProposalDocument::new(proposal))
}

fn write_private(path: &Path, bytes: &[u8]) -> EvidenceResult<PathBuf> {
    fs::write(path, bytes)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
    }
    Ok(path.to_owned())
}

fn require_executable(path: &Path) -> EvidenceResult {
    ensure(path.is_file(), "required binary path is not a file")
}

fn required_text(value: &Value, path: &[&str]) -> EvidenceResult<String> {
    let mut current = value;
    for segment in path {
        current = current.get(*segment).ok_or("JSON field is absent")?;
    }
    current
        .as_str()
        .map(str::to_owned)
        .ok_or_else(|| "JSON field is not text".into())
}

fn required_u64(value: &Value, path: &[&str]) -> EvidenceResult<u64> {
    let mut current = value;
    for segment in path {
        current = current.get(*segment).ok_or("JSON field is absent")?;
    }
    current
        .as_u64()
        .ok_or_else(|| "JSON field is not an unsigned integer".into())
}

fn path_text(path: &Path) -> EvidenceResult<&str> {
    path.to_str()
        .ok_or_else(|| "fixture path is not UTF-8".into())
}

fn ensure(condition: bool, message: &str) -> EvidenceResult {
    if condition {
        Ok(())
    } else {
        Err(message.to_owned().into())
    }
}
