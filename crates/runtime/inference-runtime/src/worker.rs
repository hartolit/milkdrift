//! Bounded single-thread host wrapper around the synchronous runtime registry.

use std::collections::{BTreeMap, VecDeque};
use std::error::Error;
use std::fmt::{self, Display, Formatter};
use std::time::Duration;

use domain_contracts::{ModelHandle, ModelId, ModelLifecycleState, ModelLoader, MonotonicMillis};
use host_runtime::{
    BoundedReceiver, BoundedSender, HostThread, MonotonicClock, OutputPullError,
    ReceiveTimeoutError, ThreadPanicked, ThreadSpawnError, TokenOutputBatch, TokenOutputConsumer,
    TokenOutputInitializationError, TokenOutputProducer, TryReceiveError, TrySendError, bounded,
    spawn_named, token_output_accumulator,
};

use crate::generation::GenerationScheduler;
use crate::{
    CommandTicket, GenerationOutputState, HostedRuntimeConfiguration, InferenceRuntime,
    RuntimeCommand, RuntimeEvent, RuntimeLimits, RuntimeReceiveError, RuntimeSubmitError,
};

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
    PreserveRuntime,
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
    TokenOutput(TokenOutputInitializationError),
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

#[expect(
    clippy::too_many_lines,
    reason = "the worker loop keeps control, generation, maintenance, and publication ordering \
              explicit"
)]
fn run_worker<L>(
    mut runtime: InferenceRuntime<L>,
    commands: &BoundedReceiver<RuntimeCommand<L::Source>>,
    events: &BoundedSender<RuntimeEvent>,
    token_output: &TokenOutputProducer<GenerationOutputState>,
    event_backlog_capacity: usize,
    poll_interval: Duration,
) where
    L: ModelLoader,
{
    let clock = MonotonicClock::new();
    let mut scheduler = GenerationScheduler::new();
    let mut pending_event = None;
    let mut queued_events = VecDeque::with_capacity(event_backlog_capacity);
    let mut stop_after_event = None;
    let mut pending_unloads = BTreeMap::<ModelId, PendingUnload>::new();
    let mut maintenance_events = BTreeMap::<ModelId, RuntimeEvent>::new();

    loop {
        if let Err(error) = runtime.poll_cleanup() {
            runtime.record_maintenance_error(error);
        }
        if let Some((model_id, event)) =
            maintenance_event(&mut runtime, clock.now(), &mut pending_unloads)
        {
            if matches!(
                event,
                RuntimeEvent::ModelUnload {
                    result: Ok(crate::UnloadReceipt { cancelled_requests, .. }),
                    ..
                } if cancelled_requests > 0
            ) {
                scheduler.request_model_cancellation(
                    model_id,
                    domain_contracts::CancellationReason::DrainTimeout,
                );
            }
            maintenance_events.insert(model_id, event);
        }
        collect_naturally_completed_unloads(
            &runtime,
            &mut pending_unloads,
            &mut maintenance_events,
        );
        if pending_event.is_none() {
            pending_event = queued_events
                .pop_front()
                .or_else(|| maintenance_events.pop_first().map(|(_, event)| event));
        }

        if let Some(stop) = stop_after_event
            && pending_event.is_none()
            && queued_events.is_empty()
        {
            finish_worker(runtime, stop);
            return;
        }

        if let Some(event) = pending_event.take() {
            match events.try_send(event) {
                Ok(()) => {
                    if let Some(stop) = stop_after_event {
                        finish_worker(runtime, stop);
                        return;
                    }
                }
                Err(TrySendError::Full(event)) => pending_event = Some(event),
                Err(TrySendError::Disconnected(_)) => {
                    shutdown_after_disconnect(runtime);
                    return;
                }
            }
        }

        if stop_after_event.is_some() {
            std::thread::sleep(poll_interval);
            continue;
        }

        let mut handled_command = false;
        for _ in 0..8 {
            if queued_events.len() >= event_backlog_capacity {
                break;
            }
            match commands.try_receive() {
                Ok(command) => {
                    handled_command = true;
                    let unload_identity = unload_command_identity(&command);
                    let (event, stop) = dispatch(
                        &mut runtime,
                        &mut scheduler,
                        token_output,
                        command,
                        clock.now(),
                    );
                    remember_pending_unload(
                        unload_identity,
                        &event,
                        &runtime,
                        &mut pending_unloads,
                    );
                    if let Some(stop) = stop {
                        pending_event = Some(event);
                        queued_events.clear();
                        maintenance_events.clear();
                        pending_unloads.clear();
                        stop_after_event = Some(stop);
                        break;
                    }
                    if pending_event.is_none() {
                        pending_event = Some(event);
                    } else {
                        queued_events.push_back(event);
                    }
                }
                Err(TryReceiveError::Empty) => break,
                Err(TryReceiveError::Disconnected) => {
                    shutdown_after_disconnect(runtime);
                    return;
                }
            }
        }

        let advance = scheduler.advance(&mut runtime, token_output);
        if !handled_command && !advance.progressed && queued_events.len() < event_backlog_capacity {
            match commands.receive_timeout(poll_interval) {
                Ok(command) => {
                    let unload_identity = unload_command_identity(&command);
                    let (event, stop) = dispatch(
                        &mut runtime,
                        &mut scheduler,
                        token_output,
                        command,
                        clock.now(),
                    );
                    remember_pending_unload(
                        unload_identity,
                        &event,
                        &runtime,
                        &mut pending_unloads,
                    );
                    if let Some(stop) = stop {
                        pending_event = Some(event);
                        queued_events.clear();
                        maintenance_events.clear();
                        pending_unloads.clear();
                        stop_after_event = Some(stop);
                    } else if pending_event.is_none() {
                        pending_event = Some(event);
                    } else {
                        queued_events.push_back(event);
                    }
                }
                Err(ReceiveTimeoutError::Timeout) => {}
                Err(ReceiveTimeoutError::Disconnected) => {
                    shutdown_after_disconnect(runtime);
                    return;
                }
            }
        }
    }
}

fn finish_worker<L>(runtime: InferenceRuntime<L>, stop: WorkerStop)
where
    L: ModelLoader,
{
    if matches!(stop, WorkerStop::PreserveRuntime) {
        std::mem::forget(runtime);
    }
}

fn shutdown_after_disconnect<L>(mut runtime: InferenceRuntime<L>)
where
    L: ModelLoader,
{
    if runtime.shutdown().is_err() && runtime.owns_backend_resources() {
        // The worker endpoint is gone, so no caller can drive or inspect further
        // cleanup. Preserve native ownership rather than invoking an unverified
        // implicit drop after explicit cleanup has failed.
        std::mem::forget(runtime);
    }
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

fn collect_naturally_completed_unloads<L>(
    runtime: &InferenceRuntime<L>,
    pending_unloads: &mut BTreeMap<ModelId, PendingUnload>,
    events: &mut BTreeMap<ModelId, RuntimeEvent>,
) where
    L: ModelLoader,
{
    let completed = pending_unloads
        .iter()
        .filter_map(|(model_id, pending)| {
            runtime
                .model_lifecycle_state(*model_id)
                .is_none()
                .then_some((*model_id, *pending))
                .filter(|_| !runtime.is_model_cleanup_pending(*model_id))
        })
        .collect::<Vec<_>>();

    for (model_id, pending) in completed {
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

#[expect(
    clippy::too_many_lines,
    reason = "command dispatch is an exhaustive ownership boundary over all runtime commands"
)]
fn dispatch<L>(
    runtime: &mut InferenceRuntime<L>,
    scheduler: &mut GenerationScheduler,
    token_output: &TokenOutputProducer<GenerationOutputState>,
    command: RuntimeCommand<L::Source>,
    now: MonotonicMillis,
) -> (RuntimeEvent, Option<WorkerStop>)
where
    L: ModelLoader,
{
    match command {
        RuntimeCommand::LoadModel {
            ticket,
            model_id,
            source,
            device,
            device_kind,
        } => (
            RuntimeEvent::ModelLoaded {
                ticket,
                result: runtime.load_model(model_id, &source, device, device_kind),
            },
            None,
        ),
        RuntimeCommand::StartRequest {
            ticket,
            handle,
            request_id,
            sequence_id,
            configuration,
        } => (
            RuntimeEvent::RequestStarted {
                ticket,
                result: if scheduler.contains(request_id) {
                    Err(crate::RuntimeError::RequestAlreadyActive(request_id))
                } else {
                    runtime.start_request(handle, request_id, sequence_id, configuration)
                },
            },
            None,
        ),
        RuntimeCommand::Generate {
            ticket,
            handle,
            request,
        } => (
            RuntimeEvent::GenerationAdmitted {
                ticket,
                result: scheduler.admit(runtime, token_output, handle, request),
            },
            None,
        ),
        RuntimeCommand::Prefill {
            ticket,
            request_id,
            tokens,
            emit_logits,
            logits,
        } => dispatch_prefill(runtime, ticket, request_id, &tokens, emit_logits, logits),
        RuntimeCommand::Decode {
            ticket,
            request_id,
            token,
            logits,
        } => dispatch_decode(runtime, ticket, request_id, token, logits),
        RuntimeCommand::CompleteRequest {
            ticket,
            request_id,
            reason,
        } => (
            RuntimeEvent::RequestFinished {
                ticket,
                request_id,
                result: runtime.complete_request(request_id, reason),
            },
            None,
        ),
        RuntimeCommand::CancelRequest {
            ticket,
            request_id,
            reason,
        } if scheduler.contains(request_id) => (
            RuntimeEvent::GenerationCancellationRequested {
                ticket,
                request_id,
                result: scheduler.request_cancellation(request_id, reason),
            },
            None,
        ),
        RuntimeCommand::CancelRequest {
            ticket,
            request_id,
            reason,
        } => (
            RuntimeEvent::RequestFinished {
                ticket,
                request_id,
                result: runtime.cancel_request(request_id, reason),
            },
            None,
        ),
        RuntimeCommand::UnloadModel {
            ticket,
            handle,
            policy,
        } => {
            if matches!(policy, domain_contracts::UnloadPolicy::CancelActive) {
                scheduler.request_model_cancellation(
                    handle.id,
                    domain_contracts::CancellationReason::ModelUnload,
                );
            }
            (
                RuntimeEvent::ModelUnload {
                    ticket,
                    result: runtime.unload_model(handle, policy, now),
                },
                None,
            )
        }
        RuntimeCommand::Snapshot { ticket } => (
            RuntimeEvent::Snapshot {
                ticket,
                runtime: runtime.snapshot(),
                models: runtime.model_snapshots(),
            },
            None,
        ),
        RuntimeCommand::Shutdown { ticket } => {
            let mut result = runtime.shutdown();
            if let Err(error) = scheduler.discard_all(runtime) {
                if result.is_ok() {
                    result = Err(error);
                } else {
                    runtime.record_maintenance_error(error);
                }
            }
            let stop = if result.is_err() && runtime.owns_backend_resources() {
                WorkerStop::PreserveRuntime
            } else {
                WorkerStop::DropRuntime
            };
            (RuntimeEvent::Shutdown { ticket, result }, Some(stop))
        }
    }
}

fn dispatch_prefill<L>(
    runtime: &mut InferenceRuntime<L>,
    ticket: CommandTicket,
    request_id: domain_contracts::RequestId,
    tokens: &[domain_contracts::TokenId],
    emit_logits: bool,
    mut logits: Vec<f32>,
) -> (RuntimeEvent, Option<WorkerStop>)
where
    L: ModelLoader,
{
    let result = runtime.prefill(request_id, tokens, emit_logits, logits.as_mut_slice());
    (
        RuntimeEvent::PrefillCompleted {
            ticket,
            request_id,
            result,
            logits,
        },
        None,
    )
}

fn dispatch_decode<L>(
    runtime: &mut InferenceRuntime<L>,
    ticket: CommandTicket,
    request_id: domain_contracts::RequestId,
    token: domain_contracts::TokenId,
    mut logits: Vec<f32>,
) -> (RuntimeEvent, Option<WorkerStop>)
where
    L: ModelLoader,
{
    let result = runtime.decode(request_id, token, logits.as_mut_slice());
    (
        RuntimeEvent::DecodeCompleted {
            ticket,
            request_id,
            result,
            logits,
        },
        None,
    )
}
