//! An escaped pipe holder cannot keep the daemon's owned workers/store alive.

use super::support::*;
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
        let parent_exit = directory.path().join("parent-exit");
        let fixture_cleanup = cleanup::ProbeCleanup(pids.clone());
        let profile_path = configured_process_profile(&directory)?;
        let mut profile: serde_json::Value = serde_json::from_slice(&fs::read(&profile_path)?)?;
        // Deliberately exceed the former 800 ms wall budget before the holder starts.
        // Shutdown timing starts only after both processes have published readiness.
        *profile
            .pointer_mut("/profile/arguments")
            .ok_or("fixture field /profile/arguments absent")? =
            json!(["escaped-pipes", pids, release, "wait", parent_exit, "1000"]);
        *profile
            .pointer_mut("/profile/side_effect")
            .ok_or("fixture field /profile/side_effect absent")? = json!("unknown");
        *profile
            .pointer_mut("/profile/limits/wall_timeout_ms")
            .ok_or("fixture field /profile/limits/wall_timeout_ms absent")? = json!(30000);
        fs::write(&profile_path, serde_json::to_vec(&profile)?)?;
        let mut config =
            configuration_document_with_process_profiles(&directory, 16, vec![profile_path])?;
        config.shutdown = ShutdownConfig {
            // Include durable settlement on an unoptimized, contended runner. The
            // escaped holder still cannot keep shutdown or its I/O workers alive.
            deadline_ms: 10000,
            effect_policy: mode,
        };
        let plan = config.validate(directory.path())?;
        let daemon = start(plan.clone(), CONTROLLER_TOKEN).await?;
        let request = create_request("cleanup-start", true);
        let accepted = daemon.client.create_workflow(&request).await?;
        let task_id = accepted
            .aliases
            .get("inspect")
            .ok_or("cleanup alias absent")?;
        let ready_deadline = tokio::time::Instant::now() + Duration::from_secs(10);
        while cleanup::read_pids(&pids).len() < 2 {
            assert!(
                tokio::time::Instant::now() < ready_deadline,
                "fixture did not enter: {mode:?}; observed PIDs: {:?}",
                cleanup::read_pids(&pids)
            );
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        let started = std::time::Instant::now();
        let (stopped, released) = tokio::join!(
            biased;
            daemon.stop(),
            async {
                if mode != ShutdownEffectPolicy::Cancel {
                    fs::write(&parent_exit, b"exit")?;
                }
                std::io::Result::Ok(())
            },
        );
        released?;
        assert!(
            started.elapsed() < Duration::from_secs(16),
            "shutdown exceeded its bound: {mode:?}"
        );
        if mode == ShutdownEffectPolicy::Cancel {
            assert!(
                matches!(
                    stopped
                        .as_ref()
                        .err()
                        .and_then(|error| error.downcast_ref::<milkdrift_daemon::HostError>()),
                    Some(milkdrift_daemon::HostError::Shutdown(_))
                ),
                "forced removal must report retained uncertainty, not a harness timeout: {stopped:?}"
            );
        } else {
            stopped?;
        }
        let entered = cleanup::read_pids(&pids);
        assert!(!cleanup::process_alive(
            *entered.first().ok_or("parent pid absent")?
        )?);
        assert!(
            cleanup::process_alive(*entered.get(1).ok_or("child pid absent")?)?,
            "local closure must not imply external termination"
        );
        // Reopening the same store also proves the old owner and effect workers
        // released it. The unresolved non-idempotent attempt must never re-enter.
        let restarted = start(plan, CONTROLLER_TOKEN).await?;
        let task = restarted
            .client
            .task(&accepted.workflow_id, task_id)
            .await?;
        assert_eq!(task.state, TaskStateDto::Uncertain);
        assert!(
            task.result
                .as_ref()
                .is_none_or(|result| result.artifacts.is_empty())
        );
        let detail = task
            .reason
            .as_deref()
            .ok_or("missing cleanup explanation")?;
        assert!(detail.contains("I/O workers joined"), "{detail}");
        assert!(detail.contains("stdout EOF=false"), "{detail}");
        assert!(detail.contains("stderr EOF=false"), "{detail}");
        let workflow = restarted.client.workflow(&accepted.workflow_id).await?;
        let retry = restarted
            .client
            .change_workflow(
                &accepted.workflow_id,
                &ChangeWorkflowRequest {
                    request_id: "cleanup-unsafe-retry".into(),
                    expected_generation: workflow.generation,
                    changes: vec![TaskChangeDto::Revise {
                        task: task_id.clone(),
                        expected_version: task.version,
                        definition: task.definition.clone(),
                    }],
                },
            )
            .await;
        assert!(
            retry.is_err(),
            "entered uncertain work must refuse in-place retry"
        );
        assert_eq!(restarted.client.create_workflow(&request).await?, accepted);
        assert_eq!(
            restarted
                .client
                .task(&accepted.workflow_id, task_id)
                .await?,
            task
        );
        restarted.stop().await?;
        assert_eq!(cleanup::read_pids(&pids), entered);
        fs::write(release, b"release")?;
        drop(fixture_cleanup);
    }
    Ok(())
}
