//! Raising a limit leaves unknown execution and its elapsed-time watermark intact.

use super::{Fixture, TestResult, context};
use milkdrift_blueprint::{TaskBatch, TaskChange};
use milkdrift_capability::TerminalStatus;
use milkdrift_persistence::{WorkflowStorageLimits, WorkflowStore};
use milkdrift_runtime::{ManualClock, TaskState, WorkflowState};
use std::sync::Arc;

#[test]
fn prospective_duration_increase_preserves_unknown_clock_and_spent_allowance() -> TestResult {
    let root = tempfile::tempdir()?;
    let clock = Arc::new(ManualClock::new(1_000));
    let id;
    let original;
    {
        let fixture = Fixture::open(root.path(), clock.clone(), 10_000)?;
        id = fixture.create(1)?;
        let active = fixture.start(fixture.runtime.tick(1)?.pop().ok_or("dispatch absent")?)?;
        clock.advance(10_000)?;
        assert!(!active.heartbeat()?);
        active.lose_outcome()?;
        let record = fixture.record(&id)?;
        original = record.tasks.keys().next().ok_or("task absent")?.clone();
        assert_eq!(record.active_execution_ms, 10_000);
        assert_eq!(
            record.tasks.get(&original).ok_or("task absent")?.state,
            TaskState::Uncertain
        );
    }
    let fixture = Fixture::open(root.path(), clock.clone(), 1_000)?;
    fixture.runtime.recover()?;
    let record = fixture.record(&id)?;
    assert!(
        fixture
            .control
            .edit(
                &context()?,
                "resume_spent",
                &id,
                TaskBatch {
                    expected_generation: record.generation,
                    changes: vec![TaskChange::Continue],
                }
            )
            .is_err()
    );
    let mut limits = record.limits.clone();
    limits.active_execution_ms = 9_999;
    assert!(
        fixture
            .control
            .edit(
                &context()?,
                "below_elapsed",
                &id,
                TaskBatch {
                    expected_generation: record.generation,
                    changes: vec![TaskChange::SetLimits {
                        limits: limits.clone()
                    }],
                }
            )
            .is_err()
    );
    limits.active_execution_ms = 22_000;
    fixture.control.edit(
        &context()?,
        "raise_duration",
        &id,
        TaskBatch {
            expected_generation: record.generation,
            changes: vec![TaskChange::SetLimits { limits }, TaskChange::Continue],
        },
    )?;
    let raised = fixture.record(&id)?;
    assert_eq!(raised.active_execution_ms, record.active_execution_ms);
    assert_eq!(raised.committed_allowance, record.committed_allowance);
    assert_eq!(raised.allowance, record.allowance);
    assert_eq!(raised.tasks.get(&original), record.tasks.get(&original));
    fixture.add(&id, "future")?;
    let active = fixture.start(
        fixture
            .runtime
            .tick(2)?
            .pop()
            .ok_or("new dispatch absent")?,
    )?;
    clock.advance(1_000)?;
    active.finish(TerminalStatus::Success)?;
    let record = fixture.record(&id)?;
    assert_eq!(
        record.active_execution_ms, 12_000,
        "both the uncertain and new clocks accrue"
    );
    assert_eq!(record.committed_allowance.invocations, Some(2));
    assert_eq!(
        record.tasks.get(&original).ok_or("task absent")?.state,
        TaskState::Uncertain
    );
    assert_eq!(record.state, WorkflowState::Blocked);
    Ok(())
}

#[test]
fn full_ordinary_receipt_capacity_preserves_active_stop_and_terminal_settlement() -> TestResult {
    let root = tempfile::tempdir()?;
    let fixture = Fixture::open_with_storage_limits(
        root.path(),
        Arc::new(ManualClock::new(1_000)),
        10_000,
        WorkflowStorageLimits {
            history_bytes: 64 * 1024 * 1024,
            receipt_bytes: 1024 * 1024,
            receipt_entries: 2,
        },
    )?;
    let id = fixture.create(1)?;
    let active = fixture.start(fixture.runtime.tick(1)?.pop().ok_or("dispatch absent")?)?;
    let record = fixture.record(&id)?;
    let task = record.tasks.values().next().ok_or("task absent")?;
    assert_eq!(task.state, TaskState::Running);
    assert!(
        fixture
            .control
            .edit(
                &context()?,
                "ordinary_when_full",
                &id,
                TaskBatch {
                    expected_generation: record.generation,
                    changes: vec![TaskChange::Wait {
                        question: "Should this work continue?".into()
                    }],
                }
            )
            .is_err()
    );
    assert_eq!(fixture.record(&id)?, record);
    let key = format!("stop:{id}:{}", task.id);
    let stop = fixture.control.edit(
        &context()?,
        &key,
        &id,
        TaskBatch {
            expected_generation: record.generation,
            changes: vec![TaskChange::Stop {
                task: task.id.clone(),
            }],
        },
    )?;
    let stopped = fixture.record(&id)?;
    assert!(
        stopped
            .tasks
            .get(&task.id)
            .ok_or("task absent")?
            .stop_requested
    );
    assert_eq!(
        stopped.tasks.get(&task.id).ok_or("task absent")?.state,
        TaskState::Running
    );
    assert_eq!(fixture.runtime.cancellation_actions(1)?.len(), 1);
    active.finish(TerminalStatus::Cancelled)?;
    let settled = fixture.record(&id)?;
    assert_eq!(
        settled.tasks.get(&task.id).ok_or("task absent")?.state,
        TaskState::Stopped
    );
    assert_eq!(
        fixture
            .store
            .workflow_receipt(context()?.actor().as_str(), &key)?,
        Some(stop)
    );
    assert!(
        fixture
            .store
            .workflow_receipt(context()?.actor().as_str(), "create")?
            .is_some()
    );
    assert!(
        fixture
            .store
            .workflow_receipt(context()?.actor().as_str(), "ordinary_when_full")?
            .is_none()
    );
    let usage = fixture.store.workflow_storage_usage()?;
    assert_eq!(usage.reserved_receipts, 0);
    assert_eq!(usage.reserved_bytes, 0);
    Ok(())
}
