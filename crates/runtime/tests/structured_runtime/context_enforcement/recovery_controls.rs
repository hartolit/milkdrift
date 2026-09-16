//! Repair the active frontier through ordinary authorized reconciliation, then reopen it.
use super::*;

#[test]
fn recovery_controls_replace_unsafe_context_prospectively_and_resume_the_same_run() -> TestResult {
    for case in [LegacyCase::RequiredLoss, LegacyCase::DisclosureLoss] {
        let (harness, run, request) = legacy_schedule(case)?;
        let saved = ContextManifestDocument::new(manifest(&harness.store, &request)?)
            .to_canonical_json()?;
        let history = harness.runtime.history(&run)?;
        let old = harness
            .store
            .revision(
                harness
                    .runtime
                    .projection(&run)?
                    .revision()
                    .ok_or("revision")?,
            )?
            .ok_or("stored revision")?;
        let mut revised_policy = serde_json::to_value(policy("fresh", false, true)?)?;
        revised_policy["budget"]["max_bytes"] = json!(100_000);
        let new = old.revise(
            old.id(),
            MutationBatch::new(vec![Mutation::ReplaceNode {
                node: work(serde_json::from_value(revised_policy)?)?,
            }])?,
            AuthorRef::new("human:structured-runtime-test")?,
            "replace unsafe retained selection prospectively",
        )?;
        harness.put_revision(&new)?;
        let unrelated = RunId::new("unrelated-recovery-run")?;
        harness.create(&unrelated, &old)?;
        let unrelated_history = harness.runtime.history(&unrelated)?;
        let directory = harness.close();
        let (store, _, executor, runtime) =
            open_closed_runtime_at(directory.path(), "repair", NOW, 64)?;
        assert!(runtime.initialize_startup().is_err());
        runtime.enable_recovery_controls()?;
        assert!(runtime.enable_recovery_controls().is_err());
        assert!(runtime.initialize_startup().is_err());
        assert!(runtime.recover_startup_closed().is_err());
        assert!(runtime.resume_admission().is_err());
        assert!(runtime.recover().is_err());
        assert!(runtime.scheduler_tick().is_err());
        assert!(runtime.claim_effects(PageSize::new(8)?).is_err());
        assert!(runtime.claim_execution_effects(PageSize::new(8)?).is_err());
        assert!(
            runtime
                .claim_cancellation_effects(PageSize::new(8)?)
                .is_err()
        );
        assert_eq!(executor.entry_count(), 0);
        assert_eq!(runtime.history(&run)?, history);
        let start = runtime.command(
            unrelated.clone(),
            ActorRef::new("human:structured-runtime-test")?,
            store.head(&unrelated)?,
            Reason::new("start while only recovery commands are available")?,
            Vec::new(),
            RunCommand::StartRun,
        )?;
        let rejected = runtime.handle_authorized_command(&start, &test_authority_claim()?)?;
        assert_eq!(
            rejected.result().disposition(),
            CommandDisposition::Rejected
        );
        assert_eq!(
            rejected.result().result().value()["reason"],
            "invalid run transition: command is unavailable in recovery mode; restart normally after reconciliation"
        );
        let replay = runtime.handle_authorized_command(&start, &test_authority_claim()?)?;
        assert!(replay.replayed());
        assert_eq!(replay.result(), rejected.result());
        assert_eq!(
            submit_command(&runtime, &store, &run, RunCommand::ResumeRun)?,
            CommandDisposition::Rejected
        );
        assert_eq!(
            submit_command(
                &runtime,
                &store,
                &run,
                RunCommand::RequestRevisionAdoption {
                    reconciliation: ReconciliationId::new("repair-context")?,
                    revision: new.id().clone(),
                    policy: ReconciliationPolicy::CancelAndRestartSafeWork,
                }
            )?,
            CommandDisposition::Accepted
        );
        let projection = runtime.projection(&run)?;
        let plan = projection
            .reconciliation()
            .plans()
            .values()
            .next()
            .ok_or("plan")?
            .plan()
            .clone();
        assert!(
            projection
                .reconciliation()
                .plans()
                .values()
                .next()
                .ok_or("plan")?
                .items()
                .iter()
                .any(|item| item.action
                    == milkdrift_persistence::ReconciliationAction::CancelAndRestart)
        );
        let apply = runtime.command(
            run.clone(),
            ActorRef::new("human:structured-runtime-test")?,
            store.head(&run)?,
            Reason::new("apply reviewed recovery plan")?,
            Vec::new(),
            RunCommand::ApplyReconciliation { plan },
        )?;
        let accepted = runtime.handle_authorized_command(&apply, &test_authority_claim()?)?;
        assert_eq!(
            accepted.result().disposition(),
            CommandDisposition::Accepted
        );
        let replay = runtime.handle_authorized_command(&apply, &test_authority_claim()?)?;
        assert!(replay.replayed());
        assert_eq!(replay.result(), accepted.result());
        let repaired = runtime.history(&run)?;
        assert_eq!(&repaired[..history.len()], history);
        assert!(repaired.iter().any(|event| matches!(
            event.kind(),
            RunEventKind::ReconciliationCancellationRequested { .. }
        )));
        assert_eq!(runtime.projection(&run)?.revision(), Some(new.id()));
        assert_eq!(runtime.history(&unrelated)?, unrelated_history);
        assert_eq!(
            saved,
            ContextManifestDocument::new(manifest(&store, &request)?).to_canonical_json()?
        );
        drop(runtime);
        drop(store);
        let (store, _, executor, runtime) =
            open_closed_runtime_at(directory.path(), "repaired", NOW, 64)?;
        runtime.initialize_startup()?;
        for _ in 0..16 {
            runtime_tick(&runtime)?;
            if runtime.projection(&run)?.is_completed() {
                break;
            }
        }
        assert!(
            runtime.projection(&run)?.is_completed(),
            "{:#?}",
            runtime.history(&run)?
        );
        assert_eq!(executor.entry_count(), 1);
        assert_eq!(runtime.history(&unrelated)?, unrelated_history);
        assert_eq!(
            saved,
            ContextManifestDocument::new(manifest(&store, &request)?).to_canonical_json()?
        );
        let requests: Vec<_> = runtime
            .history(&run)?
            .into_iter()
            .filter_map(|event| match event.kind() {
                RunEventKind::NodeScheduled { request, .. } => Some(request.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(requests.len(), 2);
        assert_ne!(requests[0].invocation(), requests[1].invocation());
        let fresh = manifest(&store, &requests[1])?;
        assert_eq!(fresh.policy_version(), 2);
        assert_eq!(fresh.revision(), new.id());
    }
    Ok(())
}
