use super::*;

pub(super) struct BlockingExecutor {
    pub(super) resolver: DeterministicExecutor,
    pub(super) blocking_operation: Mutex<OperationId>,
    pub(super) entered: (Mutex<bool>, Condvar),
    pub(super) released: (Mutex<bool>, Condvar),
    pub(super) cancellation_requests: AtomicUsize,
    pub(super) cancellation_sequences: Mutex<BTreeMap<InvocationId, u64>>,
}

impl BlockingExecutor {
    pub(super) fn new(descriptor: CapabilityDescriptor) -> TestResult<Self> {
        Ok(Self {
            resolver: DeterministicExecutor::new(descriptor),
            blocking_operation: Mutex::new(OperationId::new("model.generate")?),
            entered: (Mutex::new(false), Condvar::new()),
            released: (Mutex::new(false), Condvar::new()),
            cancellation_requests: AtomicUsize::new(0),
            cancellation_sequences: Mutex::new(BTreeMap::new()),
        })
    }

    pub(super) fn block_operation(&self, operation: OperationId) -> TestResult {
        *self
            .blocking_operation
            .lock()
            .map_err(|_| "blocking operation lock poisoned")? = operation;
        Ok(())
    }

    pub(super) fn has_entered(&self) -> TestResult<bool> {
        let (lock, _) = &self.entered;
        Ok(*lock.lock().map_err(|_| "entered lock poisoned")?)
    }

    pub(super) fn wait_until_entered(&self) -> TestResult {
        let (lock, ready) = &self.entered;
        let entered = lock.lock().map_err(|_| "entered lock poisoned")?;
        let (entered, timeout) = ready
            .wait_timeout_while(entered, Duration::from_secs(5), |entered| !*entered)
            .map_err(|_| "entered wait poisoned")?;
        if timeout.timed_out() || !*entered {
            return Err("executor dispatch was not observed before timeout".into());
        }
        Ok(())
    }

    pub(super) fn release(&self) -> TestResult {
        let (lock, released) = &self.released;
        *lock.lock().map_err(|_| "release lock poisoned")? = true;
        released.notify_all();
        Ok(())
    }

    pub(super) fn cancellation_request_sequence(
        &self,
        invocation: &InvocationId,
    ) -> TestResult<Option<u64>> {
        Ok(self
            .cancellation_sequences
            .lock()
            .map_err(|_| "cancellation sequence lock poisoned")?
            .get(invocation)
            .copied())
    }
}

impl TaskExecutor for BlockingExecutor {
    delegate_resolve!(resolver);

    fn prepare_exact_entry<'a>(
        &'a self,
        dispatch: &ExecutionDispatch,
    ) -> Result<PreparedExecution<'a>, ExecutorError> {
        Ok(prepared_test_entry(dispatch, move |dispatch, reporter| {
            let blocked = dispatch.request().operation()
                == &*self.blocking_operation.lock().map_err(|_| {
                    ExecutorError::Boundary("blocking operation lock poisoned".to_owned())
                })?;
            if blocked {
                {
                    let (lock, entered) = &self.entered;
                    *lock.lock().map_err(|_| {
                        ExecutorError::Boundary("entered lock poisoned".to_owned())
                    })? = true;
                    entered.notify_all();
                }
                let (lock, released) = &self.released;
                let mut permit = lock
                    .lock()
                    .map_err(|_| ExecutorError::Boundary("release lock poisoned".to_owned()))?;
                while !*permit {
                    permit = released
                        .wait(permit)
                        .map_err(|_| ExecutorError::Boundary("release wait poisoned".to_owned()))?;
                }
            }
            let cancellation_requested = self
                .cancellation_sequences
                .lock()
                .map_err(|_| {
                    ExecutorError::Boundary("cancellation sequence lock poisoned".to_owned())
                })?
                .contains_key(dispatch.request().invocation());
            let terminal = InvocationTerminal::new(
                if cancellation_requested {
                    TerminalStatus::Cancelled
                } else {
                    TerminalStatus::Success
                },
                Vec::new(),
                None,
                None,
                SideEffectClass::None,
            )?;
            let event = InvocationEvent::new(
                dispatch.request().invocation().clone(),
                1,
                InvocationEventKind::Terminal { terminal },
            )?;
            let _disposition = reporter.invocation(event)?;
            Ok(())
        }))
    }

    fn cancel(
        &self,
        request: &CancellationRequest,
    ) -> Result<CancellationAcknowledgement, ExecutorError> {
        self.cancellation_sequences
            .lock()
            .map_err(|_| ExecutorError::Boundary("cancellation sequence lock poisoned".to_owned()))?
            .insert(request.invocation().clone(), request.request_sequence());
        self.cancellation_requests.fetch_add(1, Ordering::SeqCst);
        Ok(CancellationAcknowledgement::new(
            request.invocation().clone(),
            request.request_sequence(),
            true,
            false,
            Some("blocking executor observed cancellation intent".to_owned()),
        )?)
    }
}

pub(super) struct PanickingExecutor {
    pub(super) resolver: DeterministicExecutor,
}

impl TaskExecutor for PanickingExecutor {
    delegate_resolve!(resolver);

    fn prepare_exact_entry<'a>(
        &'a self,
        dispatch: &ExecutionDispatch,
    ) -> Result<PreparedExecution<'a>, ExecutorError> {
        Ok(prepared_test_entry(
            dispatch,
            move |_dispatch, _reporter| {
                std::panic::resume_unwind(Box::new(
                    "intentional crash after durable invocation start",
                ))
            },
        ))
    }

    delegate_cancel!(resolver);
}

pub(super) struct InvalidReportsCountingExecutor {
    pub(super) resolver: DeterministicExecutor,
    pub(super) dispatches: AtomicUsize,
}

impl InvalidReportsCountingExecutor {
    pub(super) fn new(descriptor: CapabilityDescriptor) -> Self {
        Self {
            resolver: DeterministicExecutor::new(descriptor),
            dispatches: AtomicUsize::new(0),
        }
    }

    pub(super) fn dispatches(&self) -> usize {
        self.dispatches.load(Ordering::SeqCst)
    }
}

impl TaskExecutor for InvalidReportsCountingExecutor {
    delegate_resolve!(resolver);

    fn prepare_exact_entry<'a>(
        &'a self,
        dispatch: &ExecutionDispatch,
    ) -> Result<PreparedExecution<'a>, ExecutorError> {
        Ok(prepared_test_entry(dispatch, move |dispatch, reporter| {
            self.dispatches.fetch_add(1, Ordering::SeqCst);
            let skipped_first = InvocationEvent::new(
                dispatch.request().invocation().clone(),
                2,
                InvocationEventKind::Progress {
                    message: "invalid sequence fixture".to_owned(),
                    completed_units: None,
                    total_units: None,
                },
            )?;
            let _disposition = reporter.invocation(skipped_first)?;
            Ok(())
        }))
    }

    delegate_cancel!(resolver);
}

pub(super) struct TerminalThenFailingExecutor {
    pub(super) resolver: DeterministicExecutor,
    pub(super) dispatches: AtomicUsize,
}

impl TerminalThenFailingExecutor {
    pub(super) fn new(descriptor: CapabilityDescriptor) -> Self {
        Self {
            resolver: DeterministicExecutor::new(descriptor),
            dispatches: AtomicUsize::new(0),
        }
    }

    pub(super) fn set_script(
        &self,
        operation: OperationId,
        events: Vec<InvocationEventKind>,
    ) -> Result<(), ExecutorError> {
        self.resolver.set_script(operation, events)
    }

    pub(super) fn dispatches(&self) -> usize {
        self.dispatches.load(Ordering::SeqCst)
    }
}

impl TaskExecutor for TerminalThenFailingExecutor {
    delegate_resolve!(resolver);

    fn prepare_exact_entry<'a>(
        &'a self,
        dispatch: &ExecutionDispatch,
    ) -> Result<PreparedExecution<'a>, ExecutorError> {
        let prepared = self.resolver.prepare_exact_entry(dispatch)?;
        let envelope = prepared.admission_envelope().clone();
        Ok(PreparedExecution::new(
            dispatch,
            envelope,
            move |dispatch, reporter| {
                self.dispatches.fetch_add(1, Ordering::SeqCst);
                prepared.enter(dispatch, reporter)?;
                Err(ExecutorError::BoundaryAfterEntry(
                    "worker failed after the terminal observation was committed".to_owned(),
                ))
            },
        ))
    }

    delegate_cancel!(resolver);
}

pub(super) struct BoundaryFailingExecutor {
    pub(super) resolver: DeterministicExecutor,
    pub(super) failures_remaining: AtomicUsize,
    pub(super) dispatches: Mutex<Vec<RecordedDispatch>>,
}

impl BoundaryFailingExecutor {
    pub(super) fn new(descriptor: CapabilityDescriptor, failures: usize) -> Self {
        Self {
            resolver: DeterministicExecutor::new(descriptor),
            failures_remaining: AtomicUsize::new(failures),
            dispatches: Mutex::new(Vec::new()),
        }
    }

    pub(super) fn dispatches(&self) -> TestResult<Vec<RecordedDispatch>> {
        Ok(self
            .dispatches
            .lock()
            .map_err(|_| "dispatch log lock poisoned")?
            .clone())
    }

    pub(super) fn set_script(
        &self,
        operation: OperationId,
        events: Vec<InvocationEventKind>,
    ) -> Result<(), ExecutorError> {
        self.resolver.set_script(operation, events)
    }
}

impl TaskExecutor for BoundaryFailingExecutor {
    delegate_resolve!(resolver);

    fn prepare_exact_entry<'a>(
        &'a self,
        dispatch: &ExecutionDispatch,
    ) -> Result<PreparedExecution<'a>, ExecutorError> {
        let prepared = self.resolver.prepare_exact_entry(dispatch)?;
        let envelope = prepared.admission_envelope().clone();
        Ok(PreparedExecution::new(
            dispatch,
            envelope,
            move |dispatch, reporter| {
                self.dispatches
                    .lock()
                    .map_err(|_| ExecutorError::Boundary("dispatch log lock poisoned".to_owned()))?
                    .push(RecordedDispatch {
                        resolution: dispatch.resolution().clone(),
                        request: dispatch.request().clone(),
                    });
                if self
                    .failures_remaining
                    .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |remaining| {
                        remaining.checked_sub(1)
                    })
                    .is_ok()
                {
                    return Err(ExecutorError::Boundary(
                        "executor disconnected after accepting dispatch".to_owned(),
                    ));
                }
                prepared.enter(dispatch, reporter)
            },
        ))
    }

    delegate_cancel!(resolver);
}

#[derive(Clone)]
pub(super) struct RecordedDispatch {
    pub(super) resolution: ResolvedCapabilitySnapshot,
    pub(super) request: InvocationRequest,
}

impl RecordedDispatch {
    pub(super) fn resolution(&self) -> &ResolvedCapabilitySnapshot {
        &self.resolution
    }

    pub(super) fn request(&self) -> &InvocationRequest {
        &self.request
    }
}

pub(super) struct BoundaryThenBlockingExecutor {
    pub(super) resolver: DeterministicExecutor,
    pub(super) calls: AtomicUsize,
    pub(super) entered: (Mutex<bool>, Condvar),
    pub(super) released: (Mutex<bool>, Condvar),
    pub(super) cancellation_requests: AtomicUsize,
}

impl BoundaryThenBlockingExecutor {
    pub(super) fn new(descriptor: CapabilityDescriptor) -> Self {
        Self {
            resolver: DeterministicExecutor::new(descriptor),
            calls: AtomicUsize::new(0),
            entered: (Mutex::new(false), Condvar::new()),
            released: (Mutex::new(false), Condvar::new()),
            cancellation_requests: AtomicUsize::new(0),
        }
    }

    pub(super) fn wait_until_entered(&self) -> TestResult {
        let (lock, ready) = &self.entered;
        let entered = lock.lock().map_err(|_| "entered lock poisoned")?;
        let (entered, timeout) = ready
            .wait_timeout_while(entered, Duration::from_secs(5), |entered| !*entered)
            .map_err(|_| "entered wait poisoned")?;
        if timeout.timed_out() || !*entered {
            return Err("retry dispatch was not observed before timeout".into());
        }
        Ok(())
    }

    pub(super) fn release(&self) -> TestResult {
        let (lock, released) = &self.released;
        *lock.lock().map_err(|_| "release lock poisoned")? = true;
        released.notify_all();
        Ok(())
    }
}

impl TaskExecutor for BoundaryThenBlockingExecutor {
    delegate_resolve!(resolver);

    fn prepare_exact_entry<'a>(
        &'a self,
        dispatch: &ExecutionDispatch,
    ) -> Result<PreparedExecution<'a>, ExecutorError> {
        Ok(prepared_test_entry(dispatch, move |dispatch, reporter| {
            if self.calls.fetch_add(1, Ordering::SeqCst) == 0 {
                return Err(ExecutorError::Boundary(
                    "executor disconnected after accepting first dispatch".to_owned(),
                ));
            }
            {
                let (lock, entered) = &self.entered;
                *lock
                    .lock()
                    .map_err(|_| ExecutorError::Boundary("entered lock poisoned".to_owned()))? =
                    true;
                entered.notify_all();
            }
            let (lock, released) = &self.released;
            let mut permit = lock
                .lock()
                .map_err(|_| ExecutorError::Boundary("release lock poisoned".to_owned()))?;
            while !*permit {
                permit = released
                    .wait(permit)
                    .map_err(|_| ExecutorError::Boundary("release wait poisoned".to_owned()))?;
            }
            let terminal = InvocationTerminal::new(
                TerminalStatus::Cancelled,
                Vec::new(),
                None,
                None,
                dispatch.resolution().operation_contract().side_effect(),
            )?;
            let event = InvocationEvent::new(
                dispatch.request().invocation().clone(),
                1,
                InvocationEventKind::Terminal { terminal },
            )?;
            let _disposition = reporter.invocation(event)?;
            Ok(())
        }))
    }

    fn cancel(
        &self,
        request: &CancellationRequest,
    ) -> Result<CancellationAcknowledgement, ExecutorError> {
        self.cancellation_requests.fetch_add(1, Ordering::SeqCst);
        Ok(CancellationAcknowledgement::new(
            request.invocation().clone(),
            request.request_sequence(),
            true,
            false,
            Some("retry executor observed cancellation intent".to_owned()),
        )?)
    }
}

pub(super) struct AdmissionRaceExecutor {
    pub(super) resolver: DeterministicExecutor,
    pub(super) resolver_barrier: Barrier,
    pub(super) barrier_enabled: AtomicBool,
    pub(super) resolver_entries: (Mutex<usize>, Condvar),
    pub(super) execute_entries: (Mutex<usize>, Condvar),
    pub(super) released: (Mutex<bool>, Condvar),
}

impl AdmissionRaceExecutor {
    pub(super) fn new(descriptor: CapabilityDescriptor) -> Self {
        Self {
            resolver: DeterministicExecutor::new(descriptor),
            resolver_barrier: Barrier::new(2),
            barrier_enabled: AtomicBool::new(true),
            resolver_entries: (Mutex::new(0), Condvar::new()),
            execute_entries: (Mutex::new(0), Condvar::new()),
            released: (Mutex::new(false), Condvar::new()),
        }
    }

    pub(super) fn wait_for_resolvers(&self, expected: usize) -> TestResult {
        wait_for_count(&self.resolver_entries, expected, "resolver admission")
    }

    pub(super) fn wait_for_execute(&self, expected: usize) -> TestResult {
        wait_for_count(&self.execute_entries, expected, "executor dispatch")
    }

    pub(super) fn release(&self) -> TestResult {
        let (lock, released) = &self.released;
        *lock.lock().map_err(|_| "admission release lock poisoned")? = true;
        released.notify_all();
        Ok(())
    }
}

impl TaskExecutor for AdmissionRaceExecutor {
    fn resolve(
        &self,
        requirement: &CapabilityRequirement,
        observed_at_unix_ms: u64,
    ) -> Result<ResolvedCapability, ExecutorError> {
        let resolved = self.resolver.resolve(requirement, observed_at_unix_ms)?;
        if self.barrier_enabled.load(Ordering::SeqCst) {
            {
                let (lock, entered) = &self.resolver_entries;
                let mut count = lock.lock().map_err(|_| {
                    ExecutorError::Boundary("resolver count lock poisoned".to_owned())
                })?;
                *count = count.saturating_add(1);
                entered.notify_all();
            }
            if self.resolver_barrier.wait().is_leader() {
                self.barrier_enabled.store(false, Ordering::SeqCst);
            }
        }
        Ok(resolved)
    }

    fn prepare_exact_entry<'a>(
        &'a self,
        dispatch: &ExecutionDispatch,
    ) -> Result<PreparedExecution<'a>, ExecutorError> {
        Ok(prepared_test_entry(dispatch, move |dispatch, reporter| {
            {
                let (lock, entered) = &self.execute_entries;
                let mut count = lock.lock().map_err(|_| {
                    ExecutorError::Boundary("execute count lock poisoned".to_owned())
                })?;
                *count = count.saturating_add(1);
                entered.notify_all();
            }
            let (lock, released) = &self.released;
            let mut permit = lock.lock().map_err(|_| {
                ExecutorError::Boundary("admission release lock poisoned".to_owned())
            })?;
            while !*permit {
                permit = released.wait(permit).map_err(|_| {
                    ExecutorError::Boundary("admission release wait poisoned".to_owned())
                })?;
            }
            let terminal = InvocationTerminal::new(
                TerminalStatus::Success,
                Vec::new(),
                None,
                None,
                SideEffectClass::None,
            )?;
            let event = InvocationEvent::new(
                dispatch.request().invocation().clone(),
                1,
                InvocationEventKind::Terminal { terminal },
            )?;
            let _disposition = reporter.invocation(event)?;
            Ok(())
        }))
    }

    delegate_cancel!(resolver);
}

pub(super) fn wait_for_count(
    state: &(Mutex<usize>, Condvar),
    expected: usize,
    label: &str,
) -> TestResult {
    let (lock, ready) = state;
    let count = lock.lock().map_err(|_| format!("{label} lock poisoned"))?;
    let (count, timeout) = ready
        .wait_timeout_while(count, Duration::from_secs(5), |count| *count < expected)
        .map_err(|_| format!("{label} wait poisoned"))?;
    if timeout.timed_out() || *count < expected {
        return Err(format!("{label} did not reach {expected} before timeout").into());
    }
    Ok(())
}

pub(super) fn block_first_runnable_operation(
    store: &RedbStore,
    runtime: &RuntimeService,
    run: &RunId,
    executor: &BlockingExecutor,
) -> TestResult {
    let entry = store
        .runnable_page(TimestampMillis::new(NOW), None, PageSize::new(1)?)?
        .entries
        .into_iter()
        .next()
        .ok_or("run has no runnable executor operation")?;
    let projection = runtime.projection(run)?;
    let execution = projection
        .node_executions()
        .get(&entry.execution)
        .ok_or("runnable execution is absent from its projection")?;
    let revision_id = execution.revision();
    let revision = store
        .revision(revision_id)?
        .ok_or("runnable execution governing revision is absent")?;
    let node = revision
        .semantic()
        .nodes()
        .get(execution.node())
        .ok_or("runnable execution node is absent from its revision")?;
    let NodeKind::Task { config } = node.kind() else {
        return Err("first runnable execution is not a task".into());
    };
    executor.block_operation(config.requirement().operation().clone())
}
