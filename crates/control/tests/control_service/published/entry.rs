//! The caller can change while an internal adapter prepares, before any external entry.
use super::*;

#[test]
fn expired_or_cancelled_public_call_cannot_enter_after_internal_preparation() -> TestResult {
    for (cancel, managed) in [(false, false), (true, false), (false, true), (true, true)] {
        let directory = TempDir::new()?;
        let fixture = fixture_with_resources(directory.path(), "final-public-entry", managed)?;
        let mut method = fixture.method.clone();
        let mut descriptor = serde_json::to_value(&method.descriptor)?;
        descriptor["descriptor_revision"] = serde_json::json!(2);
        method.descriptor = serde_json::from_value(descriptor)?;
        method.maximum_duration_ms = 1_000;
        fixture.published.publish(
            method,
            Some(1),
            &fixture.decision,
            &milkdrift_persistence::IntegrityDigest::hash(b"short-public-call"),
        )?;
        let run = start_outer(&fixture, "run:final-public-entry")?;
        runtime_tick(&fixture.runtime)?;
        let plan = fixture
            .store
            .published_local_page(None, PageSize::new(8)?)?
            .0
            .pop()
            .ok_or("public association absent")?;
        assert_eq!(plan.generation, 2);
        let clock = fixture.clock.clone();
        let runtime = fixture.runtime.clone();
        let caller = fixture.context.clone();
        let parent = run.clone();
        *fixture
            .process
            .2
            .lock()
            .map_err(|_| "preparation hook lock")? = Some(Box::new(move || {
            let change = || -> TestResult {
                if cancel {
                    let command = milkdrift_runtime::RunCommandDocument::new(
                        milkdrift_persistence::CommandId::new("command:cancel-during-preparation")?,
                        parent.clone(),
                        caller.actor().clone(),
                        runtime.projection(&parent)?.sequence(),
                        TimestampMillis::new(NOW),
                        Reason::new("cancel while child prepares")?,
                        vec![],
                        milkdrift_runtime::RunCommand::RequestCancellation,
                    )?;
                    let receipt =
                        runtime.handle_authorized_command(&command, caller.authority())?;
                    assert_eq!(
                        receipt.result().disposition(),
                        milkdrift_persistence::CommandDisposition::Accepted
                    );
                } else {
                    // Cross the method deadline while its 30-second worker lease remains valid.
                    clock.advance(1_001)?;
                }
                Ok(())
            };
            change().map_err(|error| AdapterError::unavailable(error.to_string()))
        }));
        for _ in 0..32 {
            fixture.clock.advance(1)?;
            runtime_tick(&fixture.runtime)?;
            if fixture.runtime.projection(&run)?.lifecycle().is_completed() {
                break;
            }
        }
        assert!(
            fixture
                .process
                .2
                .lock()
                .map_err(|_| "preparation hook lock")?
                .is_none(),
            "the test must reach internal preparation"
        );
        assert_eq!(
            fixture.process.entries(),
            0,
            "changed public call entered internal work"
        );
        assert!(
            fixture
                .runtime
                .history(&plan.child_run)?
                .iter()
                .any(|event| matches!(
                    event.kind(), RunEventKind::NodeTerminal { detail: Some(detail), .. }
                    if detail.as_str().contains("enclosing published invocation")
                ))
        );
        assert!(fixture.runtime.projection(&run)?.lifecycle().is_completed());
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
                    .is_empty(),
                "pre-entry refusal must return editing and release both lifetime holds"
            );
        }
        assert_complete_integrity(fixture.store.as_ref())?;
    }
    Ok(())
}
