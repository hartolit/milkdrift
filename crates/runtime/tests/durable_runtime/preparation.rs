//! Exact preparation distinguishes proven refusal, unknown ownership, capacity and settlement.
use super::{InvocationAllowance, TaskStoreClock, TestResult, context, request};
use milkdrift_authority::AuthorityBudget;
use milkdrift_blueprint::TaskInputValue;
use milkdrift_capability::{
    CapabilityDescriptorDocument, CapabilityRequirement, OperationId, SideEffectClass,
};
use milkdrift_control::TaskControlService;
use milkdrift_redb_store::{RedbStore, RedbStoreConfig};
use milkdrift_runtime::{
    DeterministicExecutor, ManualClock, SequentialIdGenerator, TaskExecutor, TaskRuntime, TaskState,
};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

#[derive(Clone, Copy)]
enum PreparationBehavior {
    ProvenRefusal,
    OwnershipUnknown,
    CapacityOnce,
    Success,
    ObservationCapacity,
}

struct PreparationExecutor {
    inner: DeterministicExecutor,
    behavior: PreparationBehavior,
    preparations: AtomicUsize,
    settlements: AtomicUsize,
}

impl milkdrift_runtime::TaskExecutor for PreparationExecutor {
    fn resolve(
        &self,
        requirement: &CapabilityRequirement,
        now: u64,
    ) -> Result<milkdrift_runtime::ResolvedCapability, milkdrift_runtime::ExecutorError> {
        self.inner.resolve(requirement, now)
    }
    fn prepare_exact_entry<'a>(
        &'a self,
        dispatch: &milkdrift_runtime::ExecutionDispatch,
    ) -> Result<milkdrift_runtime::PreparedExecution<'a>, milkdrift_runtime::ExecutorError> {
        match self.behavior {
            PreparationBehavior::ProvenRefusal => {
                Err(milkdrift_runtime::ExecutorError::BoundaryBeforeEntry(
                    "fixture proved no entry".into(),
                ))
            }
            PreparationBehavior::OwnershipUnknown => {
                Err(milkdrift_runtime::ExecutorError::Overloaded(
                    "another worker may own this invocation".into(),
                ))
            }
            PreparationBehavior::CapacityOnce => {
                if self.preparations.fetch_add(1, Ordering::SeqCst) == 0 {
                    Err(milkdrift_runtime::ExecutorError::CapacityUnavailable(
                        "generation permit is busy".into(),
                    ))
                } else {
                    self.inner.prepare_exact_entry(dispatch)
                }
            }
            PreparationBehavior::Success => self.inner.prepare_exact_entry(dispatch),
            PreparationBehavior::ObservationCapacity => {
                Ok(milkdrift_runtime::PreparedExecution::new(
                    dispatch,
                    milkdrift_capability::InvocationAdmissionEnvelope::not_applicable(),
                    |dispatch, reporter| {
                        use milkdrift_capability::{
                            InvocationEvent, InvocationEventKind, InvocationTerminal,
                            TerminalStatus,
                        };
                        let progress = || InvocationEventKind::Progress {
                            message: "accepted progress".into(),
                            completed_units: None,
                            total_units: None,
                        };
                        for sequence in 1..=255 {
                            reporter.invocation(InvocationEvent::new(
                                dispatch.request().invocation().clone(),
                                sequence,
                                progress(),
                            )?)?;
                        }
                        assert!(
                            reporter
                                .invocation(InvocationEvent::new(
                                    dispatch.request().invocation().clone(),
                                    256,
                                    progress()
                                )?)
                                .is_err()
                        );
                        reporter.invocation(InvocationEvent::new(
                            dispatch.request().invocation().clone(),
                            256,
                            InvocationEventKind::Terminal {
                                terminal: InvocationTerminal::new(
                                    TerminalStatus::Success,
                                    Vec::new(),
                                    None,
                                    None,
                                    SideEffectClass::None,
                                )?,
                            },
                        )?)?;
                        Ok(())
                    },
                ))
            }
        }
    }
    fn cancel(
        &self,
        request: &milkdrift_capability::CancellationRequest,
    ) -> Result<milkdrift_capability::CancellationAcknowledgement, milkdrift_runtime::ExecutorError>
    {
        self.inner.cancel(request)
    }
    fn settle(
        &self,
        _dispatch: &milkdrift_runtime::ExecutionDispatch,
    ) -> Result<(), milkdrift_runtime::ExecutorError> {
        self.settlements.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

#[test]
fn preparation_unknown_ownership_never_fabricates_non_entry_or_releases_claims() -> TestResult {
    for (behavior, expected_state, expected_settles) in [
        (PreparationBehavior::ProvenRefusal, TaskState::Failed, 1),
        (
            PreparationBehavior::OwnershipUnknown,
            TaskState::Uncertain,
            0,
        ),
        (PreparationBehavior::CapacityOnce, TaskState::Queued, 0),
        (PreparationBehavior::Success, TaskState::Completed, 1),
        (
            PreparationBehavior::ObservationCapacity,
            TaskState::Completed,
            1,
        ),
    ] {
        let root = tempfile::tempdir()?;
        let clock = Arc::new(ManualClock::new(1_000));
        let mut fixture: serde_json::Value = serde_json::from_slice(include_bytes!(
            "../../../capability/tests/fixtures/descriptor-v1.json"
        ))?;
        fixture
            .get_mut("descriptor")
            .and_then(serde_json::Value::as_object_mut)
            .ok_or("fixture descriptor absent")?
            .insert("category".into(), serde_json::json!({"type":"process"}));
        let descriptor = CapabilityDescriptorDocument::from_json(&serde_json::to_vec(&fixture)?)?;
        let executor = Arc::new(PreparationExecutor {
            inner: DeterministicExecutor::new(descriptor.body().clone()),
            behavior,
            preparations: AtomicUsize::new(0),
            settlements: AtomicUsize::new(0),
        });
        let store = Arc::new(RedbStore::open_with_config(
            RedbStoreConfig::new(root.path()).with_clock(Arc::new(TaskStoreClock(clock.clone()))),
        )?);
        let authority = Arc::new(InvocationAllowance(Some(8)));
        let runtime = Arc::new(TaskRuntime::new(
            store.clone(),
            executor.clone(),
            authority.clone(),
            clock.clone(),
            Arc::new(SequentialIdGenerator::new("prepare-test", 1)?),
        ));
        let control = TaskControlService::new(runtime.clone(), authority, clock, store.clone());
        let mut create = request()?;
        create.inputs.insert(
            "manifest".into(),
            TaskInputValue::Text {
                text: "captured input".into(),
            },
        );
        let receipt = control.create(&context()?, "submit", create)?;
        let dispatch = runtime.tick(4)?.pop().ok_or("dispatch absent")?;
        // Verify this is an actual executor boundary fixture, not a state-construction fixture.
        executor.resolve(
            &CapabilityRequirement::new(OperationId::new("model.generate")?),
            1_000,
        )?;
        runtime.execute(dispatch.clone())?;
        let record = runtime
            .workflow(&receipt.workflow)?
            .ok_or("workflow absent")?;
        assert!(
            record
                .tasks
                .values()
                .all(|task| task.state == expected_state)
        );
        if matches!(behavior, PreparationBehavior::CapacityOnce) {
            let task = record
                .tasks
                .values()
                .next()
                .ok_or("capacity-deferred task absent")?;
            let frozen = task
                .frozen
                .as_ref()
                .ok_or("accepted start lost during capacity refusal")?;
            assert_eq!(frozen.dispatch, dispatch);
            assert!(!frozen.entered);
            assert!(frozen.reports.is_empty());
            assert_eq!(executor.inner.entry_count(), 0);
            assert_eq!(executor.settlements.load(Ordering::SeqCst), 0);
            assert_eq!(record.committed_allowance, AuthorityBudget::default());
            let retry = runtime
                .tick(1)?
                .pop()
                .ok_or("deferred exact start was not scheduled")?;
            assert_eq!(retry, dispatch);
            runtime.execute(retry)?;
            assert_eq!(executor.inner.entry_count(), 1);
            assert_eq!(executor.preparations.load(Ordering::SeqCst), 2);
            assert_eq!(executor.settlements.load(Ordering::SeqCst), 1);
            assert!(runtime.execute(dispatch).is_err());
            assert_eq!(executor.inner.entry_count(), 1);
            runtime.recover()?;
            assert!(runtime.tick(1)?.is_empty());
            assert_eq!(executor.settlements.load(Ordering::SeqCst), 2);
            continue;
        }
        if matches!(behavior, PreparationBehavior::ObservationCapacity) {
            assert_eq!(
                record
                    .tasks
                    .values()
                    .next()
                    .and_then(|task| task.frozen.as_ref())
                    .ok_or("frozen reports absent")?
                    .reports
                    .len(),
                256
            );
        }
        assert_eq!(
            executor.settlements.load(Ordering::SeqCst),
            expected_settles
        );
        runtime.recover()?;
        assert!(runtime.tick(1)?.is_empty());
        assert_eq!(
            executor.settlements.load(Ordering::SeqCst),
            expected_settles * 2
        );
    }
    Ok(())
}
