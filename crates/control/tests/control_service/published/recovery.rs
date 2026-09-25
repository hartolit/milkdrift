//! Lost commit replies must recover the saved commands, including a partially created child.
use super::*;
use milkdrift_persistence::RunJournal;
use milkdrift_redb_store::{FaultInjector, FaultPoint};
use std::sync::Mutex;

#[derive(Default)]
struct CommitFault(Mutex<Option<(FaultPoint, usize)>>);
impl CommitFault {
    fn arm(&self, point: FaultPoint, ordinal: usize) -> TestResult {
        *self.0.lock().map_err(|_| "fault lock")? = Some((point, ordinal));
        Ok(())
    }
}
impl FaultInjector for CommitFault {
    fn check(&self, point: FaultPoint) -> Result<(), milkdrift_persistence::PersistenceError> {
        let mut slot = self
            .0
            .lock()
            .map_err(|_| milkdrift_redb_store::injected_failure(point))?;
        if let Some((target, remaining)) = slot.as_mut()
            && *target == point
        {
            *remaining -= 1;
            if *remaining == 0 {
                *slot = None;
                return Err(milkdrift_redb_store::injected_failure(point));
            }
        }
        Ok(())
    }
}

#[test]
fn failed_or_lost_promotion_commit_preserves_old_generation_and_exact_replay() -> TestResult {
    for point in [
        FaultPoint::BeforePublishedMethodCommit,
        FaultPoint::AfterPublishedMethodCommit,
    ] {
        let directory = TempDir::new()?;
        let request = milkdrift_persistence::IntegrityDigest::hash(b"reviewed-method-promotion");
        let (original, next);
        {
            let faults = Arc::new(CommitFault::default());
            let fixture = fixture_with_faults(
                directory.path(),
                "promotion-fault",
                false,
                Some(faults.clone()),
            )?;
            original = fixture.method.clone();
            let mut method = original.clone();
            let mut descriptor = serde_json::to_value(&method.descriptor)?;
            descriptor["descriptor_revision"] = serde_json::json!(2);
            method.descriptor = serde_json::from_value(descriptor)?;
            next = method;
            faults.arm(point, 1)?;
            assert!(
                fixture
                    .published
                    .publish(next.clone(), Some(1), &fixture.decision, &request)
                    .is_err()
            );
            assert_eq!(
                fixture
                    .store
                    .published_method(original.descriptor.identity(), 1)?
                    .ok_or("original absent")?
                    .method,
                original
            );
            assert_eq!(
                fixture
                    .store
                    .published_method(original.descriptor.identity(), 2)?
                    .is_some(),
                point == FaultPoint::AfterPublishedMethodCommit
            );
        }
        let fixture = fixture(directory.path(), "promotion-reopened")?;
        let accepted =
            fixture
                .published
                .publish(next.clone(), Some(1), &fixture.decision, &request)?;
        let replay =
            fixture
                .published
                .publish(next.clone(), Some(1), &fixture.decision, &request)?;
        assert_eq!(accepted, replay);
        assert_eq!(accepted.method, next);
        assert_eq!(
            fixture
                .store
                .published_method(original.descriptor.identity(), 1)?
                .ok_or("original absent after reopen")?
                .method,
            original
        );
        assert!(
            fixture
                .store
                .published_method(original.descriptor.identity(), 3)?
                .is_none()
        );
    }
    Ok(())
}

#[test]
fn each_create_bind_start_commit_boundary_recovers_one_child_after_reopen() -> TestResult {
    for managed in [false, true] {
        for point in [
            FaultPoint::BeforeCommandCommit,
            FaultPoint::AfterCommandCommit,
        ] {
            for ordinal in 1..=3 {
                let directory = TempDir::new()?;
                let run;
                let plan;
                {
                    let fault = Arc::new(CommitFault::default());
                    let fixture = fixture_with_faults(
                        directory.path(),
                        "lost-reply",
                        managed,
                        Some(fault.clone()),
                    )?;
                    run = start_outer(&fixture, "run:faulted-parent")?;
                    runtime_tick(&fixture.runtime)?;
                    plan = fixture
                        .store
                        .published_local_page(None, PageSize::new(8)?)?
                        .0
                        .pop()
                        .ok_or("pending association absent")?;
                    fault.arm(point, ordinal)?;
                    assert!(
                        fixture
                            .runtime
                            .arrange_published_run(&plan, fixture.store.as_ref())
                            .is_err(),
                        "fault {point:?}/{ordinal} must be reached"
                    );
                    assert_eq!(fixture.process.entries(), 0);
                }
                let fixture = fixture_with_resources(directory.path(), "reopened", managed)?;
                for _ in 0..64 {
                    fixture.clock.advance(1)?;
                    runtime_tick(&fixture.runtime)?;
                    if fixture.runtime.projection(&run)?.lifecycle().is_completed() {
                        break;
                    }
                }
                assert_eq!(
                    fixture.runtime.projection(&run)?.lifecycle(),
                    RunLifecycle::Terminal(RunOutcome::Succeeded)
                );
                let history = fixture.runtime.history(&plan.child_run)?;
                assert_eq!(
                    history
                        .iter()
                        .filter(|event| matches!(event.kind(), RunEventKind::RunCreated { .. }))
                        .count(),
                    1
                );
                assert_eq!(
                    history
                        .iter()
                        .filter(|event| matches!(
                            event.kind(),
                            RunEventKind::PublishedRunBound { .. }
                        ))
                        .count(),
                    1
                );
                assert_eq!(
                    fixture.store.published_invocation(&plan.source)?.as_ref(),
                    Some(&plan)
                );
                assert_eq!(fixture.process.entries(), 2);
                fixture
                    .runtime
                    .arrange_published_run(&plan, fixture.store.as_ref())?;
                assert_eq!(fixture.process.entries(), 2);
            }
        }
    }
    Ok(())
}

#[test]
fn cancellation_after_lost_create_reply_settles_without_starting_the_child() -> TestResult {
    for (managed, cancel_child) in [(false, false), (true, false), (false, true), (true, true)] {
        let directory = TempDir::new()?;
        let run;
        let plan;
        {
            let fault = Arc::new(CommitFault::default());
            let fixture = fixture_with_faults(
                directory.path(),
                "partial-cancel",
                managed,
                Some(fault.clone()),
            )?;
            run = start_outer(&fixture, "run:partial-cancel")?;
            runtime_tick(&fixture.runtime)?;
            plan = fixture
                .store
                .published_local_page(None, PageSize::new(8)?)?
                .0
                .pop()
                .ok_or("public association absent")?;
            fault.arm(FaultPoint::AfterCommandCommit, 1)?;
            assert!(
                fixture
                    .runtime
                    .arrange_published_run(&plan, fixture.store.as_ref())
                    .is_err()
            );
            assert!(
                fixture
                    .runtime
                    .projection(&plan.child_run)?
                    .published_source()
                    .is_none()
            );
            let command = milkdrift_runtime::RunCommandDocument::new(
                milkdrift_persistence::CommandId::new("command:cancel-partial-create")?,
                if cancel_child {
                    plan.child_run.clone()
                } else {
                    run.clone()
                },
                fixture.context.actor().clone(),
                fixture
                    .runtime
                    .projection(if cancel_child { &plan.child_run } else { &run })?
                    .sequence(),
                TimestampMillis::new(NOW),
                Reason::new("cancel accepted call after lost create reply")?,
                vec![],
                milkdrift_runtime::RunCommand::RequestCancellation,
            )?;
            fixture
                .runtime
                .handle_authorized_command(&command, fixture.context.authority())?;
        }
        let fixture = fixture_with_resources(directory.path(), "partial-cancel-reopen", managed)?;
        for _ in 0..32 {
            fixture.clock.advance(1)?;
            runtime_tick(&fixture.runtime)?;
            if fixture.runtime.projection(&run)?.lifecycle().is_completed() {
                break;
            }
        }
        if cancel_child {
            assert!(fixture.runtime.projection(&run)?.lifecycle().is_completed());
        } else {
            assert_eq!(
                fixture.runtime.projection(&run)?.lifecycle(),
                RunLifecycle::Terminal(RunOutcome::Cancelled)
            );
        }
        assert!(
            fixture.runtime.history(&run)?.iter().any(|event| matches!(
                event.kind(),
                RunEventKind::NodeTerminal {
                    outcome: milkdrift_persistence::NodeOutcome::Cancelled,
                    ..
                }
            )),
            "the public operation must report cancellation, never service success"
        );
        assert_eq!(fixture.process.entries(), 0);
        let child = fixture.runtime.projection(&plan.child_run)?;
        assert_eq!(child.published_source(), Some(&plan.source));
        assert!(child.execution_authority().is_none());
        assert!(child.accepted_agreement().is_none());
        if managed {
            use milkdrift_persistence::managed::ManagedResourceStore;
            assert!(
                fixture
                    .store
                    .managed_installation(&milkdrift_capability::managed::ManagedName::new(
                        "publication-test"
                    )?)?
                    .ok_or("managed inventory absent")?
                    .uses
                    .is_empty()
            );
        }
        assert!(
            !fixture
                .runtime
                .history(&plan.child_run)?
                .iter()
                .any(|event| matches!(event.kind(), RunEventKind::RunStarted))
        );
        assert!(!fixture.store.published_local_pending(&plan.source)?);
        assert_complete_integrity(fixture.store.as_ref())?;
    }
    Ok(())
}

#[test]
fn cancellation_after_child_start_has_one_linked_control_action_and_no_work() -> TestResult {
    let directory = TempDir::new()?;
    let fixture = fixture(directory.path(), "started-cancel")?;
    let run = start_outer(&fixture, "run:started-cancel")?;
    runtime_tick(&fixture.runtime)?;
    let plan = fixture
        .store
        .published_local_page(None, PageSize::new(8)?)?
        .0
        .pop()
        .ok_or("pending association absent")?;
    fixture
        .runtime
        .arrange_published_run(&plan, fixture.store.as_ref())?;
    fixture
        .runtime
        .cancel_published_run(&plan, fixture.store.as_ref())?;
    let receipt = fixture
        .store
        .command_result(&plan.child_run, &plan.cancel_command)?
        .ok_or("internal cancellation receipt absent")?;
    assert_eq!(
        receipt.disposition(),
        milkdrift_persistence::CommandDisposition::Accepted
    );
    fixture
        .runtime
        .cancel_published_run(&plan, fixture.store.as_ref())?;
    assert_eq!(
        fixture
            .store
            .command_result(&plan.child_run, &plan.cancel_command)?,
        Some(receipt)
    );
    for _ in 0..24 {
        fixture.clock.advance(1)?;
        runtime_tick(&fixture.runtime)?;
    }
    assert_eq!(
        fixture.runtime.projection(&plan.child_run)?.lifecycle(),
        RunLifecycle::Terminal(RunOutcome::Cancelled)
    );
    assert_eq!(fixture.process.entries(), 0);
    assert!(fixture.runtime.projection(&run)?.lifecycle().is_completed());
    Ok(())
}

#[test]
fn lost_public_terminal_commit_does_not_repeat_a_settled_internal_method() -> TestResult {
    for point in [
        FaultPoint::BeforeCommandCommit,
        FaultPoint::AfterCommandCommit,
    ] {
        let directory = TempDir::new()?;
        let run;
        let plan;
        {
            let fault = Arc::new(CommitFault::default());
            let fixture =
                fixture_with_faults(directory.path(), "result-loss", false, Some(fault.clone()))?;
            run = start_outer(&fixture, "run:lost-public-result")?;
            runtime_tick(&fixture.runtime)?;
            plan = fixture
                .store
                .published_local_page(None, PageSize::new(8)?)?
                .0
                .pop()
                .ok_or("pending association absent")?;
            for _ in 0..64 {
                fixture.clock.advance(1)?;
                runtime_tick(&fixture.runtime)?;
                if fixture
                    .runtime
                    .projection(&plan.child_run)?
                    .lifecycle()
                    .is_completed()
                {
                    break;
                }
            }
            assert!(
                fixture
                    .runtime
                    .projection(&plan.child_run)?
                    .lifecycle()
                    .is_completed()
            );
            assert!(!fixture.runtime.projection(&run)?.lifecycle().is_completed());
            assert_eq!(fixture.process.entries(), 2);
            fault.arm(point, 1)?;
            assert!(runtime_tick(&fixture.runtime).is_err());
            assert_eq!(fixture.process.entries(), 2);
        }
        let reopened = fixture(directory.path(), "result-reopen")?;
        for _ in 0..32 {
            runtime_tick(&reopened.runtime)?;
            if reopened
                .runtime
                .projection(&run)?
                .lifecycle()
                .is_completed()
            {
                break;
            }
        }
        assert_eq!(
            reopened.runtime.projection(&run)?.lifecycle(),
            RunLifecycle::Terminal(RunOutcome::Succeeded)
        );
        assert_eq!(
            reopened.process.entries(),
            0,
            "publication recovery must not repeat internal effects"
        );
        assert_eq!(
            reopened.store.published_invocation(&plan.source)?.as_ref(),
            Some(&plan)
        );
        assert_eq!(
            reopened
                .runtime
                .history(&plan.child_run)?
                .iter()
                .filter(|event| matches!(event.kind(), RunEventKind::RunCreated { .. }))
                .count(),
            1
        );
    }
    Ok(())
}
