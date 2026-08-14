//! Bounded single-thread host wrapper around the synchronous runtime registry.

mod dispatch;

use std::collections::{BTreeMap, VecDeque};
use std::error::Error;
use std::fmt::{self, Display, Formatter};
use std::time::Duration;

use domain_contracts::{ModelHandle, ModelId, ModelLifecycleState, ModelLoader, MonotonicMillis};
use host_runtime::{
    BoundedReceiver, BoundedSender, HostThread, MonotonicClock, OutputInitializationError,
    OutputPullError, ReceiveTimeoutError, ThreadPanicked, ThreadSpawnError, TokenOutputBatch,
    TokenOutputConsumer, TokenOutputProducer, TryReceiveError, TrySendError, bounded, spawn_named,
    token_output_accumulator,
};

use crate::generation::GenerationScheduler;
use crate::{
    CommandTicket, GenerationOutputState, HostedRuntimeConfiguration, InferenceRuntime,
    RuntimeCommand, RuntimeError, RuntimeEvent, RuntimeLimits, RuntimeReceiveError,
    RuntimeSubmitError, ShutdownReceipt,
};

const MAXIMUM_COMMANDS_PER_TURN: usize = 8;

/// Client-side bounded command and event endpoints.
pub struct HostedRuntime<S> {
    commands: BoundedSender<RuntimeCommand<S>>,
    events: BoundedReceiver<RuntimeEvent>,
    token_output: TokenOutputConsumer<GenerationOutputState>,
}

impl<S> HostedRuntime<S> {
    /// Attempts to submit a command without blocking.
    ///
    /// # Errors
    ///
    /// Returns the command if the bounded queue is full or the worker has disconnected.
    #[expect(
        clippy::result_large_err,
        reason = "bounded submission errors intentionally return ownership of the unsent command"
    )]
    pub fn try_submit(&self, command: RuntimeCommand<S>) -> Result<(), RuntimeSubmitError<S>> {
        self.commands
            .try_send(command)
            .map_err(|error| match error {
                TrySendError::Full(command) => RuntimeSubmitError::Full(command),
                TrySendError::Disconnected(command) => RuntimeSubmitError::Disconnected(command),
            })
    }

    /// Attempts to receive one event without blocking.
    ///
    /// # Errors
    ///
    /// Returns [`RuntimeReceiveError::Timeout`] if the queue is empty, or
    /// [`RuntimeReceiveError::Disconnected`] if the worker has stopped.
    pub fn try_receive(&self) -> Result<RuntimeEvent, RuntimeReceiveError> {
        self.events.try_receive().map_err(|error| match error {
            TryReceiveError::Empty => RuntimeReceiveError::Timeout,
            TryReceiveError::Disconnected => RuntimeReceiveError::Disconnected,
        })
    }

    /// Waits up to `timeout` for one runtime event.
    ///
    /// # Errors
    ///
    /// Returns [`RuntimeReceiveError::Timeout`] if no event arrives before `timeout`,
    /// or [`RuntimeReceiveError::Disconnected`] if the worker has stopped.
    pub fn receive_timeout(&self, timeout: Duration) -> Result<RuntimeEvent, RuntimeReceiveError> {
        self.events
            .receive_timeout(timeout)
            .map_err(|error| match error {
                ReceiveTimeoutError::Timeout => RuntimeReceiveError::Timeout,
                ReceiveTimeoutError::Disconnected => RuntimeReceiveError::Disconnected,
            })
    }

    /// Pulls all currently published token ranges and generation state records.
    ///
    /// The callback borrows the retained accumulator storage. Copy any values needed
    /// after it returns; the next worker write may reuse the same allocation.
    ///
    /// # Errors
    ///
    /// Returns [`OutputPullError::Poisoned`] after a consumer panic poisons the
    /// short-lived accumulator mutex.
    pub fn pull_token_output<R, F>(&self, consume: F) -> Result<R, OutputPullError>
    where
        F: for<'batch> FnOnce(TokenOutputBatch<'batch, GenerationOutputState>) -> R,
    {
        self.token_output.pull(consume)
    }

    /// Returns the number of currently queued commands.
    #[must_use]
    pub fn queued_commands(&self) -> usize {
        self.commands.len()
    }

    /// Returns the number of currently queued events.
    #[must_use]
    pub fn queued_events(&self) -> usize {
        self.events.len()
    }
}

/// Correlation state retained while one model unload is asynchronous.
#[derive(Clone, Copy)]
struct PendingUnload {
    handle: ModelHandle,
    ticket: CommandTicket,
    failure_reported: bool,
    cancelled_requests: u32,
}

#[derive(Clone, Copy)]
enum WorkerStop {
    DropRuntime,
    RetainUntilProcessExit,
}

#[derive(Clone, Copy)]
enum WorkerExit {
    Disconnected,
    OutputPoisoned,
    Terminal(WorkerStop),
}

struct CommandOutcome {
    event: RuntimeEvent,
    stop: Option<WorkerStop>,
}

/// Exclusively owned state for one hosted-runtime worker loop.
struct WorkerState<'a, L>
where
    L: ModelLoader,
{
    runtime: InferenceRuntime<L>,
    scheduler: GenerationScheduler,
    commands: &'a BoundedReceiver<RuntimeCommand<L::Source>>,
    events: &'a BoundedSender<RuntimeEvent>,
    token_output: &'a TokenOutputProducer<GenerationOutputState>,
    pending_event: Option<RuntimeEvent>,
    queued_events: VecDeque<RuntimeEvent>,
    pending_unloads: BTreeMap<ModelId, PendingUnload>,
    maintenance_events: BTreeMap<ModelId, RuntimeEvent>,
    terminal_event: Option<RuntimeEvent>,
    stop_after_publication: Option<WorkerStop>,
    event_backlog_capacity: usize,
    clock: MonotonicClock,
    poll_interval: Duration,
}

/// Join handle for the exclusively owning runtime worker.
pub struct RuntimeThread {
    thread: HostThread<()>,
}

impl RuntimeThread {
    /// Reports whether the runtime worker has completed without blocking.
    #[must_use]
    pub fn is_finished(&self) -> bool {
        self.thread.is_finished()
    }

    /// Waits for worker termination.
    ///
    /// # Errors
    ///
    /// Returns [`ThreadPanicked`] if the runtime worker panicked.
    pub fn join(self) -> Result<(), ThreadPanicked> {
        self.thread.join()
    }
}

/// Failure to initialize generation output or spawn the runtime worker.
#[derive(Debug)]
pub enum HostedRuntimeStartError {
    /// Cold allocation of the bounded token accumulator failed.
    TokenOutput(OutputInitializationError),
    /// Host thread creation failed.
    Thread(ThreadSpawnError),
}

impl Display for HostedRuntimeStartError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::TokenOutput(error) => {
                write!(formatter, "token output initialization failed: {error:?}")
            }
            Self::Thread(error) => Display::fmt(error, formatter),
        }
    }
}

impl Error for HostedRuntimeStartError {}

impl From<ThreadSpawnError> for HostedRuntimeStartError {
    fn from(value: ThreadSpawnError) -> Self {
        Self::Thread(value)
    }
}

/// Starts one bounded runtime worker around a concrete loader and model type.
///
/// # Errors
///
/// Returns [`ThreadSpawnError`] if the host cannot spawn the runtime worker thread.
pub fn start_hosted_runtime<L>(
    loader: L,
    limits: RuntimeLimits,
    configuration: HostedRuntimeConfiguration,
) -> Result<(HostedRuntime<L::Source>, RuntimeThread), HostedRuntimeStartError>
where
    L: ModelLoader + Send + 'static,
    L::Source: Send + 'static,
{
    let (command_sender, command_receiver) = bounded(configuration.command_capacity);
    let (event_sender, event_receiver) = bounded(configuration.event_capacity);
    let (token_output_producer, token_output_consumer) = token_output_accumulator(
        configuration.token_output_capacity,
        configuration.token_output_record_capacity,
    )
    .map_err(HostedRuntimeStartError::TokenOutput)?;
    let thread = spawn_named("llm-inference-runtime", move || {
        run_worker(
            InferenceRuntime::new(loader, limits),
            &command_receiver,
            &event_sender,
            &token_output_producer,
            configuration.event_capacity.get(),
            configuration.poll_interval(),
        );
    })?;

    Ok((
        HostedRuntime {
            commands: command_sender,
            events: event_receiver,
            token_output: token_output_consumer,
        },
        RuntimeThread { thread },
    ))
}

fn run_worker<L>(
    runtime: InferenceRuntime<L>,
    commands: &BoundedReceiver<RuntimeCommand<L::Source>>,
    events: &BoundedSender<RuntimeEvent>,
    token_output: &TokenOutputProducer<GenerationOutputState>,
    event_backlog_capacity: usize,
    poll_interval: Duration,
) where
    L: ModelLoader,
{
    WorkerState::new(
        runtime,
        commands,
        events,
        token_output,
        event_backlog_capacity,
        poll_interval,
    )
    .run();
}

impl<'a, L> WorkerState<'a, L>
where
    L: ModelLoader,
{
    fn new(
        runtime: InferenceRuntime<L>,
        commands: &'a BoundedReceiver<RuntimeCommand<L::Source>>,
        events: &'a BoundedSender<RuntimeEvent>,
        token_output: &'a TokenOutputProducer<GenerationOutputState>,
        event_backlog_capacity: usize,
        poll_interval: Duration,
    ) -> Self {
        Self {
            runtime,
            scheduler: GenerationScheduler::new(),
            commands,
            events,
            token_output,
            pending_event: None,
            queued_events: VecDeque::with_capacity(event_backlog_capacity),
            pending_unloads: BTreeMap::new(),
            maintenance_events: BTreeMap::new(),
            terminal_event: None,
            stop_after_publication: None,
            event_backlog_capacity,
            clock: MonotonicClock::new(),
            poll_interval,
        }
    }

    fn run(mut self) {
        let exit = loop {
            if let Some(exit) = self.run_one_turn() {
                break exit;
            }
        };
        self.finish(exit);
    }

    fn run_one_turn(&mut self) -> Option<WorkerExit> {
        if self.stop_after_publication.is_none() {
            self.run_maintenance();
        }
        self.select_next_event();
        let publication_blocked = match self.publish_one_event() {
            Ok(blocked) => blocked,
            Err(exit) => return Some(exit),
        };
        if self.stop_after_publication.is_some() {
            if publication_blocked {
                std::thread::sleep(self.poll_interval);
            }
            return None;
        }

        let handled_command = match self.process_bounded_commands() {
            Ok(handled) => handled,
            Err(exit) => return Some(exit),
        };
        let generation_progressed = match self.advance_generation() {
            Ok(progressed) => progressed,
            Err(exit) => return Some(exit),
        };
        if !handled_command
            && !generation_progressed
            && self.queued_events.len() < self.event_backlog_capacity
        {
            return self.wait_for_one_command();
        }
        if publication_blocked
            || (!handled_command && self.queued_events.len() >= self.event_backlog_capacity)
        {
            std::thread::sleep(self.poll_interval);
        }
        None
    }

    fn run_maintenance(&mut self) {
        if let Err(error) = self.runtime.poll_cleanup() {
            self.runtime.record_maintenance_error(error);
        }
        if let Some((model_id, event)) = maintenance_event(
            &mut self.runtime,
            self.clock.now(),
            &mut self.pending_unloads,
        ) {
            if matches!(
                event,
                RuntimeEvent::ModelUnload {
                    result: Ok(crate::UnloadReceipt { cancelled_requests, .. }),
                    ..
                } if cancelled_requests > 0
            ) {
                self.scheduler.request_model_cancellation(
                    model_id,
                    domain_contracts::CancellationReason::DrainTimeout,
                );
            }
            self.maintenance_events.insert(model_id, event);
        }
        collect_one_naturally_completed_unload(
            &self.runtime,
            &mut self.pending_unloads,
            &mut self.maintenance_events,
        );
    }

    fn select_next_event(&mut self) {
        if self.pending_event.is_none() {
            self.pending_event = self
                .queued_events
                .pop_front()
                .or_else(|| self.maintenance_events.pop_first().map(|(_, event)| event))
                .or_else(|| self.terminal_event.take());
        }
    }

    fn publish_one_event(&mut self) -> Result<bool, WorkerExit> {
        if let Some(stop) = self.stop_after_publication
            && self.pending_event.is_none()
            && self.queued_events.is_empty()
            && self.maintenance_events.is_empty()
            && self.terminal_event.is_none()
        {
            return Err(WorkerExit::Terminal(stop));
        }

        let Some(event) = self.pending_event.take() else {
            return Ok(false);
        };
        match self.events.try_send(event) {
            Ok(()) => Ok(false),
            Err(TrySendError::Full(event)) => {
                self.pending_event = Some(event);
                Ok(true)
            }
            Err(TrySendError::Disconnected(_)) => Err(WorkerExit::Disconnected),
        }
    }

    fn process_bounded_commands(&mut self) -> Result<bool, WorkerExit> {
        let mut handled = false;
        for _ in 0..MAXIMUM_COMMANDS_PER_TURN {
            if self.queued_events.len() >= self.event_backlog_capacity {
                break;
            }
            match self.commands.try_receive() {
                Ok(command) => {
                    handled = true;
                    self.apply_command(command);
                    if self.stop_after_publication.is_some() {
                        break;
                    }
                }
                Err(TryReceiveError::Empty) => break,
                Err(TryReceiveError::Disconnected) => {
                    return Err(WorkerExit::Disconnected);
                }
            }
        }
        Ok(handled)
    }

    fn apply_command(&mut self, command: RuntimeCommand<L::Source>) {
        let unload_identity = unload_command_identity(&command);
        let CommandOutcome { event, stop } = self.dispatch(command);
        remember_pending_unload(
            unload_identity,
            &event,
            &self.runtime,
            &mut self.pending_unloads,
        );
        if let Some(stop) = stop {
            self.enter_terminal_stop(event, stop);
        } else {
            self.enqueue_event(event);
        }
    }

    fn enqueue_event(&mut self, event: RuntimeEvent) {
        if self.pending_event.is_none() {
            self.pending_event = Some(event);
        } else {
            debug_assert!(self.queued_events.len() < self.event_backlog_capacity);
            self.queued_events.push_back(event);
        }
    }

    fn enter_terminal_stop(&mut self, event: RuntimeEvent, stop: WorkerStop) {
        debug_assert!(self.terminal_event.is_none());
        self.terminal_event = Some(event);
        self.pending_unloads.clear();
        self.stop_after_publication = Some(stop);
    }

    fn advance_generation(&mut self) -> Result<bool, WorkerExit> {
        let advance = self.scheduler.advance(&mut self.runtime, self.token_output);
        if advance.output_poisoned {
            Err(WorkerExit::OutputPoisoned)
        } else {
            Ok(advance.progressed)
        }
    }

    fn wait_for_one_command(&mut self) -> Option<WorkerExit> {
        match self.commands.receive_timeout(self.poll_interval) {
            Ok(command) => {
                self.apply_command(command);
                None
            }
            Err(ReceiveTimeoutError::Timeout) => None,
            Err(ReceiveTimeoutError::Disconnected) => Some(WorkerExit::Disconnected),
        }
    }

    fn finish(mut self, exit: WorkerExit) {
        match exit {
            WorkerExit::Disconnected | WorkerExit::OutputPoisoned => {
                let result = shutdown_runtime(&mut self.runtime, &mut self.scheduler);
                if result.is_err() && self.runtime.owns_backend_resources() {
                    retain_until_process_exit(self.runtime);
                }
            }
            WorkerExit::Terminal(stop) => finish_worker(self.runtime, stop),
        }
    }
}

fn finish_worker<L>(runtime: InferenceRuntime<L>, stop: WorkerStop)
where
    L: ModelLoader,
{
    if matches!(stop, WorkerStop::RetainUntilProcessExit) {
        retain_until_process_exit(runtime);
    }
}

fn retain_until_process_exit<L>(runtime: InferenceRuntime<L>)
where
    L: ModelLoader,
{
    // Explicit cleanup is exhausted while backend ownership remains. Avoid an
    // unverified implicit backend drop; process termination is the reclamation
    // boundary for this deliberately abandoned allocation.
    std::mem::forget(runtime);
}

fn shutdown_runtime<L>(
    runtime: &mut InferenceRuntime<L>,
    scheduler: &mut GenerationScheduler,
) -> Result<ShutdownReceipt, RuntimeError>
where
    L: ModelLoader,
{
    let mut result = runtime.shutdown();
    if let Err(error) = scheduler.discard_all(runtime) {
        if result.is_ok() {
            result = Err(error);
        } else {
            runtime.record_maintenance_error(error);
        }
    }
    result
}

fn maintenance_event<L>(
    runtime: &mut InferenceRuntime<L>,
    now: MonotonicMillis,
    pending_unloads: &mut BTreeMap<ModelId, PendingUnload>,
) -> Option<(ModelId, RuntimeEvent)>
where
    L: ModelLoader,
{
    let (handle, result) = runtime.poll_unload_transition(now)?;
    let pending = pending_unloads.get(&handle.id).copied()?;
    match result {
        Ok(receipt) => {
            pending_unloads.remove(&handle.id);
            Some((
                handle.id,
                RuntimeEvent::ModelUnload {
                    ticket: pending.ticket,
                    result: Ok(receipt),
                },
            ))
        }
        Err(error) if !pending.failure_reported => {
            if let Some(pending) = pending_unloads.get_mut(&handle.id) {
                pending.failure_reported = true;
            }
            Some((
                handle.id,
                RuntimeEvent::ModelUnload {
                    ticket: pending.ticket,
                    result: Err(error),
                },
            ))
        }
        Err(_) => None,
    }
}

fn collect_one_naturally_completed_unload<L>(
    runtime: &InferenceRuntime<L>,
    pending_unloads: &mut BTreeMap<ModelId, PendingUnload>,
    events: &mut BTreeMap<ModelId, RuntimeEvent>,
) where
    L: ModelLoader,
{
    let completed = pending_unloads.iter().find_map(|(model_id, pending)| {
        runtime
            .model_lifecycle_state(*model_id)
            .is_none()
            .then_some((*model_id, *pending))
            .filter(|_| !runtime.is_model_cleanup_pending(*model_id))
    });
    let Some((model_id, pending)) = completed else {
        return;
    };
    pending_unloads.remove(&model_id);
    events.insert(
        model_id,
        RuntimeEvent::ModelUnload {
            ticket: pending.ticket,
            result: Ok(crate::UnloadReceipt {
                handle: pending.handle,
                status: crate::UnloadStatus::Unloaded,
                cancelled_requests: pending.cancelled_requests,
            }),
        },
    );
}

const fn unload_command_identity<S>(
    command: &RuntimeCommand<S>,
) -> Option<(ModelHandle, CommandTicket)> {
    match command {
        RuntimeCommand::UnloadModel { ticket, handle, .. } => Some((*handle, *ticket)),
        _ => None,
    }
}

fn remember_pending_unload<L>(
    identity: Option<(ModelHandle, CommandTicket)>,
    event: &RuntimeEvent,
    runtime: &InferenceRuntime<L>,
    pending_unloads: &mut BTreeMap<ModelId, PendingUnload>,
) where
    L: ModelLoader,
{
    let Some((handle, ticket)) = identity else {
        return;
    };
    let model_id = handle.id;
    let pending = runtime.is_model_cleanup_pending(model_id)
        || runtime
            .model_lifecycle_state(model_id)
            .is_some_and(|state| {
                matches!(
                    state,
                    ModelLifecycleState::Draining { .. }
                        | ModelLifecycleState::Cancelling { .. }
                        | ModelLifecycleState::Unloading
                )
            });
    if pending {
        if pending_unloads.contains_key(&model_id) {
            return;
        }
        let failure_reported = matches!(event, RuntimeEvent::ModelUnload { result: Err(_), .. });
        let cancelled_requests = match event {
            RuntimeEvent::ModelUnload {
                result: Ok(receipt),
                ..
            } => receipt.cancelled_requests,
            RuntimeEvent::ModelUnload { result: Err(_), .. } => runtime
                .model_cancelled_requests_during_unload(model_id)
                .unwrap_or(0),
            _ => 0,
        };
        pending_unloads.insert(
            model_id,
            PendingUnload {
                handle,
                ticket,
                failure_reported,
                cancelled_requests,
            },
        );
    } else {
        pending_unloads.remove(&model_id);
    }
}
