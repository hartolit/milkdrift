//! An escaped pipe holder cannot keep the daemon's owned workers/store alive.

use super::support::*;
use milkdrift_control_protocol::ResolveAction;
use milkdrift_daemon::ShutdownEffectPolicy;
use serde_json::json;

#[path = "../../../../adapters/local-process/tests/support/process_cleanup.rs"]
mod cleanup;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn inherited_pipes_settle_drain_cancel_and_retain_without_duplicate_entry() -> TestResult {
    for mode in [
        ShutdownEffectPolicy::Drain,
        ShutdownEffectPolicy::Cancel,
        ShutdownEffectPolicy::Retain,
    ] {
        let directory = tempfile::tempdir()?;
        let pids = directory.path().join("pids");
        let release = directory.path().join("release");
        let fixture_cleanup = cleanup::ProbeCleanup(pids.clone());
        let profile_path = configured_process_profile(&directory)?;
        let mut profile: serde_json::Value = serde_json::from_slice(&fs::read(&profile_path)?)?;
        profile["profile"]["arguments"] = json!(["escaped-pipes", pids, release, "wait"]);
        profile["profile"]["side_effect"] = json!("unknown");
        profile["profile"]["limits"]["wall_timeout_ms"] = json!(800);
        fs::write(&profile_path, serde_json::to_vec(&profile)?)?;
        let mut config =
            configuration_document_with_process_profiles(&directory, 16, vec![profile_path])?;
        config.shutdown = ShutdownConfig {
            deadline_ms: 3000,
            effect_policy: mode,
        };
        let plan = config.validate(directory.path())?;
        let daemon = start(plan.clone(), CONTROLLER_TOKEN).await?;
        let imported = daemon
            .client
            .submit(&request(
                "cleanup-import",
                None,
                Command::ImportBlueprint {
                    document: process_blueprint()?,
                },
            ))
            .await?;
        let revision = imported.value["revision_id"]
            .as_str()
            .ok_or("missing revision")?
            .to_owned();
        daemon
            .client
            .submit(&request(
                "cleanup-start",
                None,
                Command::StartRun {
                    run_id: "run-cleanup".to_owned(),
                    workflow_id: "daemon-process".to_owned(),
                    revision_id: revision,
                },
            ))
            .await?;
        let ready_deadline = tokio::time::Instant::now() + Duration::from_secs(3);
        while cleanup::read_pids(&pids).len() < 2 {
            assert!(
                tokio::time::Instant::now() < ready_deadline,
                "fixture did not enter: {mode:?}"
            );
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        let started = std::time::Instant::now();
        let stopped = daemon.stop().await;
        assert!(
            started.elapsed() < Duration::from_secs(3),
            "shutdown exceeded its bound: {mode:?}"
        );
        if mode == ShutdownEffectPolicy::Cancel {
            assert!(
                stopped.is_err(),
                "forced removal retains the unresolved invocation"
            );
        } else {
            stopped?;
        }
        let entered = cleanup::read_pids(&pids);
        assert!(!cleanup::process_alive(entered[0])?);
        assert!(
            cleanup::process_alive(entered[1])?,
            "local closure must not imply external termination"
        );
        // Reopening the same store also proves the old owner and effect workers
        // released it. The unresolved non-idempotent attempt must never re-enter.
        let restarted = start(plan, CONTROLLER_TOKEN).await?;
        let timeline = restarted
            .client
            .timeline(
                "run-cleanup",
                &PageRequest {
                    cursor: None,
                    limit: 100,
                },
            )
            .await?;
        let attempt_id = timeline
            .items
            .iter()
            .find_map(|entry| entry.attempt_id.clone())
            .ok_or("missing attempt")?;
        let attempt = restarted.client.attempt("run-cleanup", &attempt_id).await?;
        assert!(attempt.uncertain);
        assert!(attempt.outputs.is_empty());
        let detail = attempt
            .terminal_detail
            .as_deref()
            .ok_or("missing cleanup explanation")?;
        assert!(detail.contains("I/O workers joined"), "{detail}");
        assert!(detail.contains("stdout EOF=false"), "{detail}");
        assert!(detail.contains("stderr EOF=false"), "{detail}");
        let retry = restarted
            .client
            .submit(&request(
                "cleanup-retry",
                None,
                Command::ResolveWork {
                    run_id: "run-cleanup".to_owned(),
                    attempt_id: attempt_id.clone(),
                    decision_id: "unsafe-retry".to_owned(),
                    action: ResolveAction::Retry,
                    remediation_node: None,
                },
            ))
            .await;
        assert!(
            retry.is_err(),
            "unknown non-idempotent work must refuse retry"
        );
        assert_eq!(
            restarted.client.attempt("run-cleanup", &attempt_id).await?,
            attempt
        );
        restarted.stop().await?;
        assert_eq!(cleanup::read_pids(&pids), entered);
        fs::write(release, b"release")?;
        drop(fixture_cleanup);
    }
    Ok(())
}
