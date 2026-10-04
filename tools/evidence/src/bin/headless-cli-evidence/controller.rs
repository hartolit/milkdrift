//! Isolated actual-binary controller installation, remediation, account and restart evidence.
use std::io::Write as _;
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
#[path = "controller/review.rs"]
mod review;
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
            profile
                .pointer_mut("/profile")
                .and_then(serde_json::Value::as_object_mut)
                .ok_or("missing object /profile")?
                .insert(
                    "inputs".into(),
                    json!([{"input":"work","relative_path":"work.txt"}]),
                );
            profile
                .pointer_mut("/profile/stdout")
                .and_then(serde_json::Value::as_object_mut)
                .ok_or("missing object /profile/stdout")?
                .insert("artifact_name".into(), Value::Null);
            profile.pointer_mut("/profile").and_then(serde_json::Value::as_object_mut).ok_or("missing object /profile")?.insert("outputs".into(), json!([{"name":"stdout","relative_path":"verification.json","media_type":"application/json","required":true}]));
            fs::write(&path, serde_json::to_vec(&profile)?)?;
        }
        if name == "race" || name == "crash" {
            let mut profile: Value = serde_json::from_slice(&fs::read(&path)?)?;
            profile
                .pointer_mut("/profile")
                .and_then(serde_json::Value::as_object_mut)
                .ok_or("missing object /profile")?
                .insert("max_concurrent".into(), json!(3));
            profile
                .pointer_mut("/profile/limits")
                .and_then(serde_json::Value::as_object_mut)
                .ok_or("missing object /profile/limits")?
                .insert("wall_timeout_ms".into(), json!(30_000));
            let stdout = profile
                .pointer_mut("/profile/stdout")
                .and_then(serde_json::Value::as_object_mut)
                .ok_or("missing object /profile/stdout")?;
            stdout.insert("stream_progress".into(), json!(true));
            stdout.insert("max_progress_events".into(), json!(4));
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
    let review = review::Review::configure(arguments, &directory, &mut config)?;
    let real_agent = review::configure_agent(arguments, &directory, &mut config)?;
    config.runtime.effect_threads = 4;
    config.application_receipts.hot_receipt_bound = 8;
    config.application_receipts.archive_batch_size = 4;
    let mut human = config
        .actors
        .first()
        .ok_or("controller actor absent")?
        .clone();
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
    let body = workflow::body(&review, real_agent)
        .map_err(|error| format!("controller body: {error:?}"))?;
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
        &workflow::wrapper(&body, "controller-evidence", 8, 2)?,
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
        disabled
            .pointer("/value/controller_accounting/state")
            .and_then(Value::as_str)
            == Some("inactive"),
        "disabled controller acquired an account",
    )?;
    ensure(
        children(&runner, body.semantic().workflow().as_str())?.is_empty(),
        "disabled controller created a child",
    )?;
    daemon.terminate()?;
    config.runtime.controller_activation = ControllerActivation::Enabled;
    save_config(&config_path, &config)?;
    let checked = run_command(
        Command::new(&arguments.daemon)
            .arg("--config")
            .arg(&config_path)
            .arg("--check-config"),
        None,
        Duration::from_secs(10),
    )?;
    ensure(
        checked.status.success(),
        "explicit controller activation was refused",
    )?;
    fs::write(
        directory.join("production-activation-check.txt"),
        checked.stdout,
    )?;
    daemon = start_daemon(&arguments.daemon, &config_path)?;
    wait_for_readiness(&runner, &mut daemon)?;
    refusals::prelude(&runner, &directory, &body, "qualification")?;
    // The disabled command retains its refusal. Explicit activation starts a distinct aggregate.
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
    let rejected = wait_for_run(&runner, &child, Duration::from_secs(540), |read| {
        node(read, "repair-release").is_some()
            || read
                .pointer("/value/terminal")
                .is_some_and(|value| !value.is_null())
    })?;
    fs::write(
        directory.join("initial-review-boundary.json"),
        serde_json::to_vec_pretty(&rejected)?,
    )?;
    let original_account = rejected
        .pointer("/value/controller_accounting")
        .ok_or("missing /value/controller_accounting")?
        .clone();
    ensure(
        original_account.pointer("/state").and_then(Value::as_str) == Some("active")
            && original_account
                .pointer("/committed/process_admissions")
                .and_then(Value::as_u64)
                == Some(2),
        "independent work and verification did not share the canonical account",
    )?;
    let rejection = inspect_node(&runner, &child, &rejected, "first-acceptance")?;
    ensure(
        rejection
            .pointer("/value/result_acceptance/accepted")
            .and_then(Value::as_bool)
            == Some(false),
        "failed verification was accepted",
    )?;
    fs::write(
        directory.join("rejected.json"),
        serde_json::to_vec_pretty(&rejection)?,
    )?;
    let source = inspect_node(&runner, &child, &rejected, "verify")?;
    let reviewed = review::Review::inspect(&runner, &directory, &child, &rejected, "review")?;
    let authority = source
        .pointer("/value/execution_authority")
        .ok_or("missing /value/execution_authority")?;
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
            node: workflow::work("repair", "controller-repair", true, real_agent, true)?,
        }],
        artifact_references(&source)?
            .into_iter()
            .chain(artifact_references(&rejection)?)
            .chain(artifact_references(&reviewed)?)
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
        before_approval
            .pointer("/value/revision_id")
            .ok_or("missing /value/revision_id")?
            == body.id().as_str()
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
    config.runtime.controller_activation = ControllerActivation::Enabled;
    save_config(&config_path, &config)?;
    daemon = start_daemon(&arguments.daemon, &config_path)?;
    wait_for_readiness(&runner, &mut daemon)?;
    let reopened = runner.success(&["run", "show", &child])?;
    ensure(
        reopened
            .pointer("/value/controller_accounting")
            .ok_or("missing /value/controller_accounting")?
            == before_approval
                .pointer("/value/controller_accounting")
                .ok_or("missing /value/controller_accounting")?,
        "settled restart changed the cumulative account",
    )?;
    // Revoking the approver during a durable approval wait cannot release remediation.
    daemon.terminate()?;
    config
        .actors
        .get_mut(1)
        .ok_or("human actor absent")?
        .enabled = false;
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
    let human = config.actors.get_mut(1).ok_or("human actor absent")?;
    human.enabled = true;
    human.grant_revision += 1;
    human.revocation_generation += 1;
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
        replayed.pointer("/value/replayed").and_then(Value::as_bool) == Some(true)
            && released
                .pointer("/value/command_id")
                .ok_or("missing /value/command_id")?
                == replayed
                    .pointer("/value/command_id")
                    .ok_or("missing /value/command_id")?,
        "lost signal reply did not recover by exact replay",
    )?;
    let child_done = wait_for_run(&runner, &child, Duration::from_secs(540), |run| {
        run.pointer("/value/terminal")
            .is_some_and(|value| !value.is_null())
            || node(run, "failed-again").is_some()
            || node(run, "model-budget-release").is_some()
    })?;
    fs::write(
        directory.join("final-review-boundary.json"),
        serde_json::to_vec_pretty(&child_done)?,
    )?;
    ensure(
        node(&child_done, "model-budget-release").is_some(),
        "repair did not pass independent verification, acceptance and final review",
    )?;
    let _final_review =
        review::Review::inspect(&runner, &directory, &child, &child_done, "final-review")?;
    review.verify_count()?;
    let accepted = inspect_node(&runner, &child, &child_done, "second-acceptance")?;
    ensure(
        accepted
            .pointer("/value/result_acceptance/accepted")
            .and_then(Value::as_bool)
            == Some(true),
        "remediation did not pass independent acceptance",
    )?;
    let completed_account = child_done
        .pointer("/value/controller_accounting")
        .ok_or("missing /value/controller_accounting")?
        .clone();
    daemon.terminate()?;
    daemon = start_daemon(&arguments.daemon, &config_path)?;
    wait_for_readiness(&runner, &mut daemon)?;
    let completed_reopen = runner.success(&["run", "show", &child])?;
    ensure(
        completed_reopen
            .pointer("/value/controller_accounting")
            .ok_or("missing /value/controller_accounting")?
            == &completed_account,
        "restart after completed reviews changed accounting",
    )?;
    signal(
        &approver,
        &child,
        "model-budget-release",
        "exhaust-model-budget",
        required_u64(&completed_reopen, &["value", "sequence"])?,
    )?;
    let refused_run = wait_for_run(&runner, &child, Duration::from_secs(30), |run| {
        run.pointer("/value/terminal")
            .is_some_and(|value| !value.is_null())
    })?;
    let refused = inspect_node(&runner, &child, &refused_run, "excess-review")?;
    ensure(
        refused.pointer("/value/uncertain").and_then(Value::as_bool) == Some(false)
            && refused
                .pointer("/value/terminal_detail")
                .ok_or("missing /value/terminal_detail")?
                .as_str()
                .is_some_and(|detail| detail.contains("model_admissions"))
            && refused_run
                .pointer("/value/controller_accounting/committed/model_admissions")
                .and_then(Value::as_u64)
                == Some(2),
        "excess model request was not refused at account admission",
    )?;
    fs::write(
        directory.join("excess-model-refusal.json"),
        serde_json::to_vec_pretty(&refused)?,
    )?;
    review.verify_count()?;
    let stopped = wait_for_run(&runner, ROOT, Duration::from_secs(20), |run| {
        node(run, "controller-repeat").is_some_and(|node| node["state"] == "terminal(_failed)")
    })?;
    let execution = required_text(
        node(&stopped, "controller-repeat").ok_or("controller occurrence absent")?,
        &["execution_id"],
    )?;
    let status = runner.success(&["controller", "status", ROOT, &execution])?;
    ensure(
        status
            .pointer("/value/value/cycle_eligible")
            .and_then(Value::as_bool)
            == Some(false),
        "controller remained eligible after a refused model request",
    )?;
    ensure(
        children(&runner, body.semantic().workflow().as_str())?.len() == 1,
        "bound admitted an extra child cycle",
    )?;
    let accounting = stopped
        .pointer("/value/controller_accounting")
        .ok_or("missing /value/controller_accounting")?;
    ensure(
        accounting
            .pointer("/committed/process_admissions")
            .and_then(Value::as_u64)
            == Some(4)
            && accounting
                .pointer("/committed/model_admissions")
                .and_then(Value::as_u64)
                == Some(2)
            && accounting
                .pointer("/remaining/model_admissions")
                .and_then(Value::as_u64)
                == Some(0)
            && accounting
                .pointer("/committed/cost_micros")
                .and_then(Value::as_u64)
                == Some(0)
            && accounting
                .pointer("/account/reservations")
                .and_then(Value::as_object)
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
        run.pointer("/value/terminal").and_then(Value::as_str) == Some("failed")
    })?;
    daemon.terminate()?;
    daemon = start_daemon(&arguments.daemon, &config_path)?;
    wait_for_readiness(&runner, &mut daemon)?;
    let final_read = runner.success(&["run", "show", ROOT])?;
    ensure(
        final_read
            .pointer("/value/controller_accounting")
            .ok_or("missing /value/controller_accounting")?
            == accounting,
        "terminal reopen reset or charged the account",
    )?;
    let health = runner.success(&["daemon", "health"])?;
    ensure(
        health
            .pointer("/value/application_receipts/cold_count")
            .and_then(Value::as_u64)
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
        cold_replay
            .pointer("/value/replayed")
            .and_then(Value::as_bool)
            == Some(true),
        "archived command lost exact replay",
    )?;
    let retained_proposer = runner.success(&[
        "attempt",
        "inspect",
        ROOT,
        &required_text(&proposer, &["value", "attempt_id"])?,
    ])?;
    ensure(
        retained_proposer
            .pointer("/value/outputs")
            .ok_or("missing /value/outputs")?
            == proposer
                .pointer("/value/outputs")
                .ok_or("missing /value/outputs")?,
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
        retained_rejection
            .pointer("/value/outputs")
            .ok_or("missing /value/outputs")?
            == rejection
                .pointer("/value/outputs")
                .ok_or("missing /value/outputs")?
            && retained_rejection
                .pointer("/value/result_acceptance")
                .ok_or("missing /value/result_acceptance")?
                == rejection
                    .pointer("/value/result_acceptance")
                    .ok_or("missing /value/result_acceptance")?,
        "compaction changed failed acceptance evidence",
    )?;
    ensure(
        runner
            .success(&["run", "show", ROOT])?
            .pointer("/value/controller_accounting")
            .ok_or("missing /value/controller_accounting")?
            == accounting,
        "cold replay changed the cumulative account",
    )?;
    let report = json!({"production_activation":"explicit_enabled", "reason":"isolated evidence uses explicit activation; ordinary startup remains disabled by default",
        "real_coding_agent":real_agent,"real_model_review":arguments.controller_review_profile.is_some(),
        "model_profile":review.profile, "identities":review::identities(arguments, &directory)?,
        "installed_qualification_loop":"passed", "initial_account":original_account, "final_account":accounting,
        "accepted":accepted, "status":status, "receipt_health":health, "external_model_settings":review.settings,
        "refused_model_request":refused});
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
        recovered
            .pointer("/value/uncertainty_count")
            .and_then(Value::as_u64)
            == Some(1)
            && recovered
                .pointer("/value/controller_accounting")
                .ok_or("missing /value/controller_accounting")?
                == crash
                    .pointer("/value/controller_accounting")
                    .ok_or("missing /value/controller_accounting")?,
        "recovery released or duplicated the entered process reservation",
    )?;
    daemon.terminate()?;
    daemon = start_daemon(&arguments.daemon, &config_path)?;
    wait_for_readiness(&runner, &mut daemon)?;
    let reopened = runner.success(&["run", "show", &crashed_run])?;
    ensure(
        reopened
            .pointer("/value/controller_accounting")
            .ok_or("missing /value/controller_accounting")?
            == recovered
                .pointer("/value/controller_accounting")
                .ok_or("missing /value/controller_accounting")?
            && reopened
                .pointer("/value/uncertainty_count")
                .and_then(Value::as_u64)
                == Some(1),
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
    writeln!(
        std::io::stdout().lock(),
        "controller qualification evidence passed: {}",
        directory.join("report.json").display()
    )?;
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
    read.pointer("/value/nodes")?
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
    child_ids(&runs, workflow)
}

fn child_ids(runs: &Value, workflow: &str) -> EvidenceResult<Vec<String>> {
    runs.pointer("/value/items")
        .ok_or("missing /value/items")?
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
        let runs = runner.success_until(&["run", "list", "--limit", "100"], deadline)?;
        if let Some(child) = child_ids(&runs, workflow)?.first() {
            return Ok(child.clone());
        }
        ensure(
            std::time::Instant::now() < deadline,
            "controller child was not created",
        )?;
        std::thread::sleep(
            deadline
                .saturating_duration_since(std::time::Instant::now())
                .min(Duration::from_millis(20)),
        );
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
    attempt
        .pointer("/value/outputs")
        .ok_or("missing /value/outputs")?
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
    let output = attempt
        .pointer("/value/outputs")
        .ok_or("missing /value/outputs")?
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
