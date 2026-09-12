//! Isolated actual-binary controller installation, remediation, account and restart evidence.
use std::{collections::BTreeMap, fs, path::Path, process::Command, time::Duration};

use milkdrift_authority::ActorRef;
use milkdrift_blueprint::{BlueprintRevision, BlueprintRevisionDocument, Mutation, MutationBatch};
use milkdrift_control::{
    ActorAuthorityContext, ClaimedStopCondition, ControlCommand, ControlCommandDocument, ControlId,
    OptimisticGuard, ProposalApplicationPolicy, ProposalId, ProposalProvenance, WorkflowProposal,
    WorkflowProposalDocument,
};
use milkdrift_daemon::{ActorGrantConfig, ControllerActivation, DaemonConfig, SecretSourceConfig};
use milkdrift_evidence::{
    EvidenceResult,
    application::{
        CliRunner, EvidenceConfig, ensure, path_text, required_text, required_u64,
        reserve_endpoint, run_command, wait_for_readiness, wait_for_run, write_config,
        write_private, write_process_profile,
    },
};
use milkdrift_persistence::{Reason, TimestampMillis};
use milkdrift_workspace::RunId;
use serde_json::{Value, json};

#[path = "controller/refusals.rs"]
mod refusals;
#[path = "controller/workflow.rs"]
mod workflow;

const ACTOR: &str = "controller:qualification";
const HUMAN: &str = "human:qualification";
const ROOT: &str = "run-controller-qualification";

pub(super) fn run(arguments: &super::Arguments) -> EvidenceResult {
    let temporary = tempfile::tempdir()?;
    let directory = if let Some(output) = &arguments.controller_output {
        fs::create_dir(output)?;
        output.canonicalize()?
    } else {
        temporary.path().to_owned()
    };
    let executable = std::env::current_exe()?;
    let mut profiles = Vec::new();
    for (name, mode) in [
        ("work", "work"),
        ("repair", "repair"),
        ("verify", "verify"),
        ("race", "race"),
        ("crash", "crash"),
    ] {
        let path = write_process_profile(
            &directory,
            &executable,
            &format!("controller-{name}"),
            &format!("controller-{name}"),
            &format!("--fixture-controller-{mode}"),
            if name == "crash" {
                "non_idempotent_write"
            } else {
                "read_only"
            },
            Some("stdout"),
        )?;
        if name == "verify" {
            let mut profile: Value = serde_json::from_slice(&fs::read(&path)?)?;
            profile["profile"]["inputs"] = json!([{"input":"work","relative_path":"work.txt"}]);
            fs::write(&path, serde_json::to_vec(&profile)?)?;
        }
        if name == "race" || name == "crash" {
            let mut profile: Value = serde_json::from_slice(&fs::read(&path)?)?;
            profile["profile"]["max_concurrent"] = json!(3);
            profile["profile"]["limits"]["wall_timeout_ms"] = json!(30_000);
            profile["profile"]["stdout"]["stream_progress"] = json!(true);
            profile["profile"]["stdout"]["max_progress_events"] = json!(4);
            fs::write(&path, serde_json::to_vec(&profile)?)?;
        }
        profiles.push(path);
    }
    let token = write_private(
        &directory.join("controller.token"),
        b"isolated-controller-token",
    )?;
    let human_token = write_private(&directory.join("human.token"), b"isolated-approver-token")?;
    let bind = reserve_endpoint()?;
    let config_path = write_config(
        &directory,
        bind,
        &token,
        ACTOR,
        EvidenceConfig {
            process_profiles: profiles,
            model_profiles: Vec::new(),
            secret_sources: BTreeMap::new(),
            lease_duration_ms: 60_000,
            authority: ActorGrantConfig::dangerous_administrator(),
        },
    )?;
    let mut config: DaemonConfig = toml::from_str(&fs::read_to_string(&config_path)?)?;
    let models = refusals::Models::configure(arguments, &directory, &mut config)?;
    config.runtime.effect_threads = 4;
    config.application_receipts.hot_receipt_bound = 8;
    config.application_receipts.archive_batch_size = 4;
    let mut human = config.actors[0].clone();
    human.actor = HUMAN.to_owned();
    human.grant_id = "grant:controller-human".to_owned();
    human.credential_ref = "credential:controller-human".to_owned();
    config.secret_sources.insert(
        human.credential_ref.clone(),
        SecretSourceConfig::File {
            path: human_token.clone(),
        },
    );
    config.actors.push(human);
    save_config(&config_path, &config)?;
    let runner = CliRunner {
        executable: arguments.cli.clone(),
        endpoint: format!("http://{bind}/"),
        token_file: token,
        forbidden_storage_path: directory.join("data"),
    };
    let approver = CliRunner {
        executable: arguments.cli.clone(),
        endpoint: runner.endpoint.clone(),
        token_file: human_token,
        forbidden_storage_path: directory.join("data"),
    };
    let body = workflow::body().map_err(|error| format!("controller body: {error:?}"))?;
    let root = workflow::root(&body).map_err(|error| format!("controller wrapper: {error:?}"))?;

    // Disabled production composition is exercised before any qualification installation.
    let mut daemon = start_daemon(&arguments.daemon, &config_path)?;
    wait_for_readiness(&runner, &mut daemon)?;
    import(&runner, &directory, "body", &body)?;
    refusals::prelude(&runner, &directory, &body, "disabled")?;
    import(
        &runner,
        &directory,
        "base-wrapper",
        &workflow::wrapper(&body, "controller-evidence", 4)?,
    )?;
    import(&runner, &directory, "root", &root)?;
    let started = runner.run(
        &[
            "--command-id",
            "controller-start",
            "run",
            "start",
            "run-controller-disabled",
            root.semantic().workflow().as_str(),
            root.id().as_str(),
        ],
        None,
    )?;
    fs::write(directory.join("disabled-start.txt"), &started.stdout)?;
    let disabled = runner.success(&["run", "show", "run-controller-disabled"])?;
    ensure(
        disabled["value"]["controller_accounting"]["state"] == "inactive",
        "disabled controller acquired an account",
    )?;
    ensure(
        children(&runner, body.semantic().workflow().as_str())?.is_empty(),
        "disabled controller created a child",
    )?;
    daemon.terminate()?;
    config.runtime.controller_activation = ControllerActivation::Enabled;
    save_config(&config_path, &config)?;
    let refused = run_command(
        Command::new(&arguments.daemon)
            .arg("--config")
            .arg(&config_path)
            .arg("--check-config"),
        None,
        Duration::from_secs(10),
    )?;
    ensure(
        !refused.status.success() && refused.stderr.contains("real external controller loop"),
        "unqualified production activation was accepted",
    )?;
    fs::write(
        directory.join("production-activation-refusal.txt"),
        refused.stderr,
    )?;
    config.runtime.controller_activation = ControllerActivation::Qualification;
    save_config(&config_path, &config)?;
    daemon = start_daemon(&arguments.daemon, &config_path)?;
    wait_for_readiness(&runner, &mut daemon)?;
    refusals::prelude(&runner, &directory, &body, "qualification")?;
    // The disabled command retains its refusal. Qualification starts a distinct aggregate.
    runner.success(&[
        "--command-id",
        "controller-start-qualified",
        "run",
        "start",
        ROOT,
        root.semantic().workflow().as_str(),
        root.id().as_str(),
    ])?;
    let child = wait_for_child(&runner, body.semantic().workflow().as_str())?;
    let rejected = wait_node(&runner, &child, "repair-release")?;
    let original_account = rejected["value"]["controller_accounting"].clone();
    ensure(
        original_account["state"] == "active"
            && original_account["committed"]["process_admissions"] == 2,
        "independent work and verification did not share the canonical account",
    )?;
    let rejection = inspect_node(&runner, &child, &rejected, "first-acceptance")?;
    ensure(
        rejection["value"]["result_acceptance"]["accepted"] == false,
        "failed verification was accepted",
    )?;
    fs::write(
        directory.join("rejected.json"),
        serde_json::to_vec_pretty(&rejection)?,
    )?;
    let source = inspect_node(&runner, &child, &rejected, "verify")?;
    let authority = &source["value"]["execution_authority"];
    let claim = milkdrift_runtime::CommandAuthorityClaim::new(
        milkdrift_authority::GrantId::new(required_text(authority, &["grant_id"])?)?,
        required_u64(authority, &["grant_revision"])?,
        serde_json::from_value(authority["grant_digest"].clone())?,
        required_u64(authority, &["revocation_generation"])?,
    )?;
    let repair = proposal(
        "repair",
        ACTOR,
        &child,
        &body,
        required_u64(&rejected, &["value", "sequence"])?,
        vec![Mutation::ReplaceNode {
            node: workflow::process("repair", "controller-repair", true, false)?,
        }],
        artifact_references(&source)?
            .into_iter()
            .chain(artifact_references(&rejection)?)
            .collect(),
    )?;
    let control = ControlCommandDocument::new(
        ControlId::new("controller-remediation-proposal")?,
        ActorAuthorityContext::new(ActorRef::new(ACTOR)?, claim),
        TimestampMillis::new(0),
        OptimisticGuard {
            expected_revision: Some(body.id().clone()),
            expected_run_sequence: repair.proposal().observed_run_sequence(),
            expected_proposal_digest: Some(repair.proposal().digest().clone()),
        },
        Reason::new("Remediate the exact failed verification; preserve entered work")?,
        Vec::new(),
        ControlCommand::SubmitProposal {
            proposal: repair.clone(),
        },
    )?;
    let preparation = proposal(
        "install-decision",
        HUMAN,
        ROOT,
        &root,
        required_u64(
            &runner.success(&["run", "show", ROOT])?,
            &["value", "sequence"],
        )?,
        vec![Mutation::ReplaceNode {
            node: workflow::proposer(&control)?,
        }],
        Vec::new(),
    )?;
    let prepared_revision = submit(&approver, &directory, "install-decision", &preparation)?;
    approve_apply(
        &approver,
        ROOT,
        "install-decision",
        &preparation,
        &prepared_revision,
    )?;
    signal(
        &approver,
        ROOT,
        "authorize-proposer",
        "release-proposer",
        required_u64(
            &runner.success(&["run", "show", ROOT])?,
            &["value", "sequence"],
        )?,
    )?;
    let proposed = wait_node(&runner, ROOT, "proposer-finished")?;
    let proposer = inspect_node(&runner, ROOT, &proposed, "proposer")?;
    let result = download_output(&runner, &directory, "proposed", &proposer, "control_result")?;
    let proposed_revision = required_text(&result, &["value", "proposed_revision"])?;
    fs::write(
        directory.join("proposal-repair.json"),
        repair.to_canonical_json()?,
    )?;
    let before_approval = runner.success(&["run", "show", &child])?;
    ensure(
        before_approval["value"]["revision_id"] == body.id().as_str()
            && node(&before_approval, "repair").is_none(),
        "proposal ran or replaced work before approval",
    )?;

    // A settled crash retains proposal/artifact/account identity and rejects a disabled restart.
    daemon.terminate()?;
    config.runtime.controller_activation = ControllerActivation::Disabled;
    save_config(&config_path, &config)?;
    let disabled_reopen = run_command(
        Command::new(&arguments.daemon)
            .arg("--config")
            .arg(&config_path),
        None,
        Duration::from_secs(10),
    )?;
    ensure(
        !disabled_reopen.status.success(),
        "disabled daemon recovered active accounted work",
    )?;
    fs::write(
        directory.join("disabled-recovery-refusal.txt"),
        disabled_reopen.stderr,
    )?;
    config.runtime.controller_activation = ControllerActivation::Qualification;
    save_config(&config_path, &config)?;
    daemon = start_daemon(&arguments.daemon, &config_path)?;
    wait_for_readiness(&runner, &mut daemon)?;
    let reopened = runner.success(&["run", "show", &child])?;
    ensure(
        reopened["value"]["controller_accounting"]
            == before_approval["value"]["controller_accounting"],
        "settled restart changed the cumulative account",
    )?;
    // Revoking the approver during a durable approval wait cannot release remediation.
    daemon.terminate()?;
    config.actors[1].enabled = false;
    save_config(&config_path, &config)?;
    daemon = start_daemon(&arguments.daemon, &config_path)?;
    wait_for_readiness(&runner, &mut daemon)?;
    let revoked = approver.run(
        &[
            "--yes",
            "--command-id",
            "revoked-approval",
            "--expected-sequence",
            &required_u64(&reopened, &["value", "sequence"])?.to_string(),
            "--expected-revision",
            &proposed_revision,
            "proposal",
            "approve",
            &child,
            "repair",
            repair.proposal().digest().as_str(),
            &proposed_revision,
            "revoked-decision",
        ],
        None,
    )?;
    ensure(
        !revoked.status.success(),
        "revoked approver released remediation",
    )?;
    fs::write(directory.join("revoked-approval.json"), &revoked.stdout)?;
    ensure(
        node(&runner.success(&["run", "show", &child])?, "repair").is_none(),
        "revoked approval entered repair",
    )?;
    daemon.terminate()?;
    config.actors[1].enabled = true;
    save_config(&config_path, &config)?;
    daemon = start_daemon(&arguments.daemon, &config_path)?;
    wait_for_readiness(&runner, &mut daemon)?;
    approve_apply(&approver, &child, "repair", &repair, &proposed_revision)?;
    let release_sequence = required_u64(
        &runner.success(&["run", "show", &child])?,
        &["value", "sequence"],
    )?;
    let released = signal(
        &approver,
        &child,
        "repair-release",
        "release-repair",
        release_sequence,
    )?;
    let replayed = signal(
        &approver,
        &child,
        "repair-release",
        "release-repair",
        release_sequence,
    )?;
    ensure(
        replayed["value"]["replayed"] == true
            && released["value"]["command_id"] == replayed["value"]["command_id"],
        "lost signal reply did not recover by exact replay",
    )?;
    let child_done = wait_for_run(&runner, &child, Duration::from_secs(20), |run| {
        run["value"]["terminal"] == "succeeded"
    })?;
    let accepted = inspect_node(&runner, &child, &child_done, "second-acceptance")?;
    ensure(
        accepted["value"]["result_acceptance"]["accepted"] == true,
        "remediation did not pass independent acceptance",
    )?;
    let stopped = wait_for_run(&runner, ROOT, Duration::from_secs(20), |run| {
        node(run, "controller-repeat").is_some_and(|node| node["state"] == "terminal(_failed)")
    })?;
    let execution = required_text(
        node(&stopped, "controller-repeat").ok_or("controller occurrence absent")?,
        &["execution_id"],
    )?;
    let status = runner.success(&["controller", "status", ROOT, &execution])?;
    ensure(
        status["value"]["value"]["reached_bound"] == "process_invocations",
        "second cycle did not stop at cumulative process ceiling",
    )?;
    ensure(
        children(&runner, body.semantic().workflow().as_str())?.len() == 1,
        "bound admitted an extra child cycle",
    )?;
    let accounting = &stopped["value"]["controller_accounting"];
    ensure(
        accounting["committed"]["process_admissions"] == 4
            && accounting["remaining"]["process_admissions"] == 0
            && accounting["account"]["reservations"]
                .as_object()
                .is_some_and(|values| values.is_empty()),
        "settled cumulative account is wrong",
    )?;
    fs::write(
        directory.join("accepted.json"),
        serde_json::to_vec_pretty(&accepted)?,
    )?;
    fs::write(
        directory.join("controller-status.json"),
        serde_json::to_vec_pretty(&status)?,
    )?;
    signal(
        &approver,
        ROOT,
        "proposer-finished",
        "finish-decision",
        required_u64(
            &runner.success(&["run", "show", ROOT])?,
            &["value", "sequence"],
        )?,
    )?;
    wait_for_run(&runner, ROOT, Duration::from_secs(20), |run| {
        run["value"]["terminal"] == "failed"
    })?;
    daemon.terminate()?;
    daemon = start_daemon(&arguments.daemon, &config_path)?;
    wait_for_readiness(&runner, &mut daemon)?;
    let final_read = runner.success(&["run", "show", ROOT])?;
    ensure(
        final_read["value"]["controller_accounting"] == *accounting,
        "terminal reopen reset or charged the account",
    )?;
    let health = runner.success(&["daemon", "health"])?;
    ensure(
        health["value"]["application_receipts"]["cold_count"]
            .as_u64()
            .is_some_and(|count| count > 0),
        "receipt archival did not occur",
    )?;
    let cold_replay = signal(
        &approver,
        &child,
        "repair-release",
        "release-repair",
        release_sequence,
    )?;
    ensure(
        cold_replay["value"]["replayed"] == true,
        "archived command lost exact replay",
    )?;
    let retained_proposer = runner.success(&[
        "attempt",
        "inspect",
        ROOT,
        &required_text(&proposer, &["value", "attempt_id"])?,
    ])?;
    ensure(
        retained_proposer["value"]["outputs"] == proposer["value"]["outputs"],
        "compaction lost the exact proposal result artifact",
    )?;
    ensure(
        download_output(
            &runner,
            &directory,
            "compacted-proposal",
            &retained_proposer,
            "control_result",
        )? == result,
        "compacted proposal bytes changed",
    )?;
    let retained_rejection = runner.success(&[
        "attempt",
        "inspect",
        &child,
        &required_text(&rejection, &["value", "attempt_id"])?,
    ])?;
    ensure(
        retained_rejection["value"]["outputs"] == rejection["value"]["outputs"]
            && retained_rejection["value"]["result_acceptance"]
                == rejection["value"]["result_acceptance"],
        "compaction changed failed acceptance evidence",
    )?;
    ensure(
        runner.success(&["run", "show", ROOT])?["value"]["controller_accounting"] == *accounting,
        "cold replay changed the cumulative account",
    )?;
    let report = json!({"production_activation":"blocked", "reason":"bounded real external controller loop remains unqualified",
        "installed_qualification_loop":"passed", "initial_account":original_account, "final_account":accounting,
        "accepted":accepted, "status":status, "receipt_health":health, "external_model_settings":"unknown"});
    models.exercise(arguments, &runner, &directory)?;
    refusals::race(&runner, &directory)?;
    daemon.terminate()?;
    config.runtime.lease_duration_ms = 30_000;
    save_config(&config_path, &config)?;
    daemon = start_daemon(&arguments.daemon, &config_path)?;
    wait_for_readiness(&runner, &mut daemon)?;
    let crash = refusals::entered_process(&runner, &directory)?;
    daemon.terminate()?;
    // Recovery respects the last durable lease. Wait past it, also allowing the ten-second
    // fixture to exit on platforms where killing the daemon does not contain descendants.
    std::thread::sleep(Duration::from_secs(31));
    daemon = start_daemon(&arguments.daemon, &config_path)?;
    wait_for_readiness(&runner, &mut daemon)?;
    let crashed_run = required_text(&crash, &["value", "run_id"])?;
    let recovered = runner.success(&["run", "show", &crashed_run])?;
    fs::write(
        directory.join("crash-observation.json"),
        serde_json::to_vec_pretty(&json!({"entered":crash,"recovered":recovered}))?,
    )?;
    ensure(
        recovered["value"]["uncertainty_count"] == 1
            && recovered["value"]["controller_accounting"]
                == crash["value"]["controller_accounting"],
        "recovery released or duplicated the entered process reservation",
    )?;
    daemon.terminate()?;
    daemon = start_daemon(&arguments.daemon, &config_path)?;
    wait_for_readiness(&runner, &mut daemon)?;
    let reopened = runner.success(&["run", "show", &crashed_run])?;
    ensure(
        reopened["value"]["controller_accounting"] == recovered["value"]["controller_accounting"]
            && reopened["value"]["uncertainty_count"] == 1,
        "second reopen retried uncertain controlled work",
    )?;
    fs::write(
        directory.join("entered-process-crash.json"),
        serde_json::to_vec_pretty(
            &json!({"entered":crash,"recovered":recovered,"reopened":reopened}),
        )?,
    )?;
    fs::write(
        directory.join("report.json"),
        serde_json::to_vec_pretty(&report)?,
    )?;
    daemon.terminate()?;
    println!(
        "controller qualification evidence passed: {}",
        directory.join("report.json").display()
    );
    Ok(())
}

fn save_config(path: &Path, config: &DaemonConfig) -> EvidenceResult {
    fs::write(path, toml::to_string_pretty(config)?)?;
    Ok(())
}

fn start_daemon(
    executable: &Path,
    config: &Path,
) -> EvidenceResult<milkdrift_evidence::application::OwnedChild> {
    use std::sync::atomic::{AtomicUsize, Ordering};
    static START: AtomicUsize = AtomicUsize::new(0);
    let output = config
        .parent()
        .ok_or("configuration directory absent")?
        .join(format!(
            "daemon-{}.log",
            START.fetch_add(1, Ordering::Relaxed)
        ));
    let log = fs::File::create(output)?;
    milkdrift_evidence::application::OwnedChild::spawn(
        Command::new(executable)
            .arg("--config")
            .arg(config)
            .env("RUST_LOG", "warn")
            .stdout(log.try_clone()?)
            .stderr(log),
    )
}

fn import(
    runner: &CliRunner,
    directory: &Path,
    name: &str,
    revision: &BlueprintRevision,
) -> EvidenceResult {
    let path = directory.join(format!("{name}.json"));
    fs::write(
        &path,
        BlueprintRevisionDocument::new(revision).to_canonical_json()?,
    )?;
    runner.success(&["blueprint", "import", path_text(&path)?])?;
    Ok(())
}

fn node<'a>(read: &'a Value, name: &str) -> Option<&'a Value> {
    read["value"]["nodes"]
        .as_array()?
        .iter()
        .find(|node| node["node_id"] == name)
}

fn wait_node(runner: &CliRunner, run: &str, name: &str) -> EvidenceResult<Value> {
    wait_for_run(runner, run, Duration::from_secs(20), |read| {
        node(read, name).is_some()
    })
}

fn inspect_node(runner: &CliRunner, run: &str, read: &Value, name: &str) -> EvidenceResult<Value> {
    let attempt = required_text(
        node(read, name).ok_or("expected node absent")?,
        &["latest_attempt_id"],
    )?;
    runner.success(&["attempt", "inspect", run, &attempt])
}

fn children(runner: &CliRunner, workflow: &str) -> EvidenceResult<Vec<String>> {
    let runs = runner.success(&["run", "list", "--limit", "100"])?;
    runs["value"]["items"]
        .as_array()
        .ok_or("run list absent")?
        .iter()
        .filter(|run| run["workflow_id"] == workflow)
        .map(|run| required_text(run, &["run_id"]))
        .collect()
}

fn wait_for_child(runner: &CliRunner, workflow: &str) -> EvidenceResult<String> {
    let deadline = std::time::Instant::now() + Duration::from_secs(20);
    loop {
        if let Some(child) = children(runner, workflow)?.first() {
            return Ok(child.clone());
        }
        ensure(
            std::time::Instant::now() < deadline,
            "controller child was not created",
        )?;
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn proposal(
    name: &str,
    actor: &str,
    run: &str,
    base: &BlueprintRevision,
    observed_sequence: u64,
    mutations: Vec<Mutation>,
    artifacts: Vec<milkdrift_capability::ArtifactReference>,
) -> EvidenceResult<WorkflowProposalDocument> {
    Ok(WorkflowProposalDocument::new(WorkflowProposal::new(
        ProposalId::new(name)?,
        ActorRef::new(actor)?,
        ProposalProvenance::Direct,
        base.semantic().workflow().clone(),
        Some(RunId::new(run)?),
        base.id().clone(),
        base.content_digest().clone(),
        Some(milkdrift_persistence::RunSequence::new(observed_sequence)),
        MutationBatch::new(mutations)?,
        "Bounded prospective controller remediation from independently rejected work",
        None,
        Vec::new(),
        Vec::new(),
        Vec::new(),
        artifacts,
        ProposalApplicationPolicy::RequireApproval,
        None,
        ClaimedStopCondition::Continue,
    )?))
}

fn artifact_references(
    attempt: &Value,
) -> EvidenceResult<Vec<milkdrift_capability::ArtifactReference>> {
    attempt["value"]["outputs"]
        .as_array()
        .ok_or("evidence outputs absent")?
        .iter()
        .map(|output| {
            let artifact = &output["artifact"];
            Ok(milkdrift_capability::ArtifactReference::new(
                required_text(artifact, &["artifact_id"])?,
                required_text(artifact, &["digest"])?,
                artifact["content_type"].as_str().map(str::to_owned),
                Some(required_u64(artifact, &["size"])?),
            )?)
        })
        .collect()
}

fn submit(
    runner: &CliRunner,
    directory: &Path,
    name: &str,
    proposal: &WorkflowProposalDocument,
) -> EvidenceResult<String> {
    let path = directory.join(format!("proposal-{name}.json"));
    fs::write(&path, proposal.to_canonical_json()?)?;
    let submitted = runner.success(&[
        "--command-id",
        &format!("submit-{name}"),
        "--expected-sequence",
        &proposal
            .proposal()
            .observed_run_sequence()
            .ok_or("live proposal sequence absent")?
            .get()
            .to_string(),
        "--expected-revision",
        proposal.proposal().base_revision().as_str(),
        "proposal",
        "submit",
        path_text(&path)?,
    ])?;
    required_text(&submitted, &["value", "value", "proposed_revision"])
}

fn approve_apply(
    runner: &CliRunner,
    run: &str,
    name: &str,
    proposal: &WorkflowProposalDocument,
    revision: &str,
) -> EvidenceResult {
    for operation in ["approve", "apply"] {
        let read = runner.success(&["run", "show", run])?;
        let sequence = required_u64(&read, &["value", "sequence"])?.to_string();
        let mut args = vec!["--yes", "--command-id"];
        let command = format!("{name}-{operation}");
        args.extend([
            &command,
            "--expected-sequence",
            &sequence,
            "--expected-revision",
            revision,
            "proposal",
            operation,
            run,
            name,
            proposal.proposal().digest().as_str(),
            revision,
        ]);
        let decision = format!("decision-{name}");
        if operation == "approve" {
            args.push(&decision);
        }
        runner.success(&args)?;
    }
    Ok(())
}

fn signal(
    runner: &CliRunner,
    run: &str,
    signal: &str,
    identity: &str,
    sequence: u64,
) -> EvidenceResult<Value> {
    runner.success(&[
        "--command-id",
        identity,
        "--expected-sequence",
        &sequence.to_string(),
        "run",
        "signal",
        run,
        "--signal-id",
        identity,
        "--signal-type",
        &format!("controller.{signal}"),
        "--payload",
        "{}",
    ])
}

fn download_output(
    runner: &CliRunner,
    directory: &Path,
    name: &str,
    attempt: &Value,
    output_name: &str,
) -> EvidenceResult<Value> {
    let output = attempt["value"]["outputs"]
        .as_array()
        .ok_or("attempt outputs absent")?
        .iter()
        .find(|output| output["name"] == output_name)
        .ok_or("named output absent")?;
    let artifact = required_text(output, &["artifact", "artifact_id"])?;
    let path = directory.join(format!("{name}-output.json"));
    runner.success(&["artifact", "get", &artifact, "--output", path_text(&path)?])?;
    Ok(serde_json::from_slice(&fs::read(path)?)?)
}
