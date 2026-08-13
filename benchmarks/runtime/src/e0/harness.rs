//! Shared hosted-E0 worker, transport, cleanup, and bounded-join ownership.

use std::num::{NonZeroU32, NonZeroU64, NonZeroUsize};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use candle_backend::{
    CandleLlamaLoader, CandleLlamaSource, CandleLoadObservation, CandleLoadObservationSnapshot,
};
use domain_contracts::{BackendId, MemoryBudget, ModelHandle, ModelId, RequestId, SequenceId};
use inference_runtime::{
    CommandTicket, HostedRuntime, HostedRuntimeConfiguration, RuntimeCommand, RuntimeEvent,
    RuntimeLimits, RuntimeThread, ShutdownReceipt, start_hosted_runtime,
};

use crate::error::{BenchmarkError, BenchmarkResult};
use crate::support::deadline::Deadline;

pub(super) const CANDLE_BACKEND: BackendId = BackendId::new(10_001);

pub(super) const FIXTURE_MODEL_ID: ModelId = ModelId::new(7);
const EVENT_TIMEOUT: Duration = Duration::from_secs(10);
const JOIN_TIMEOUT: Duration = Duration::from_secs(10);
const COMMAND_CAPACITY: usize = 16;
const EVENT_CAPACITY: usize = 16;
static RETAINED_WORKERS: Mutex<Vec<RuntimeThread>> = Mutex::new(Vec::new());

pub(super) type CandleRuntime = HostedRuntime<CandleLlamaSource>;

/// Raw shutdown timing returned before normal-runner report conversion.
#[derive(Clone, Copy, Debug)]
#[doc(hidden)]
pub struct ShutdownDurations {
    /// Shutdown submission through the matching shutdown event.
    pub event: Duration,
    /// Time spent waiting for completion and joining after the event phase.
    pub join: Duration,
    /// Complete shutdown submission through successful join.
    pub total: Duration,
}

/// One returned event and only its hosted submission-to-matching-event duration.
pub(super) struct TimedEvent {
    pub(super) event: RuntimeEvent,
    pub(super) elapsed: Duration,
}

/// Concrete benchmark-only owner for one hosted Candle E0 worker.
///
/// Both the normal runner and Criterion target use this owner. It has no `Drop`
/// cleanup contract; every call site must pass its result through [`Self::finish`].
#[doc(hidden)]
pub struct HostedE0Harness {
    runtime: Option<CandleRuntime>,
    thread: Option<RuntimeThread>,
    load_observation: CandleLoadObservation,
    loaded_model: Option<ModelHandle>,
    next_ticket: u64,
    next_request: u64,
}

impl HostedE0Harness {
    pub(crate) fn start(
        token_output_capacity: usize,
        token_output_record_capacity: usize,
    ) -> BenchmarkResult<(Self, Duration)> {
        let configuration = HostedRuntimeConfiguration::new(
            nonzero_usize(COMMAND_CAPACITY, "command capacity")?,
            nonzero_usize(EVENT_CAPACITY, "event capacity")?,
            NonZeroU64::MIN,
        )
        .with_token_output_capacity(
            nonzero_usize(token_output_capacity, "token output capacity")?,
            nonzero_usize(token_output_record_capacity, "token output record capacity")?,
        );
        let limits = RuntimeLimits::new(
            NonZeroU32::MIN,
            NonZeroU32::MIN,
            MemoryBudget {
                host_bytes: u64::MAX,
                device_bytes: 0,
            },
        );
        let (load_observation, load_observation_recorder) = CandleLoadObservation::channel();
        let started = Instant::now();
        let (runtime, thread) = start_hosted_runtime(
            CandleLlamaLoader::with_load_observation(CANDLE_BACKEND, load_observation_recorder),
            limits,
            configuration,
        )
        .map_err(|error| BenchmarkError::new(format!("E0 worker start failed: {error}")))?;
        let elapsed = started.elapsed();
        Ok((
            Self {
                runtime: Some(runtime),
                thread: Some(thread),
                load_observation,
                loaded_model: None,
                next_ticket: 1,
                next_request: 1,
            },
            elapsed,
        ))
    }

    pub(super) fn runtime(&self) -> BenchmarkResult<&CandleRuntime> {
        self.runtime
            .as_ref()
            .ok_or_else(|| BenchmarkError::new("hosted E0 endpoint is no longer available"))
    }

    pub(super) fn ticket(&mut self) -> BenchmarkResult<CommandTicket> {
        let ticket = CommandTicket::new(self.next_ticket);
        self.next_ticket = self
            .next_ticket
            .checked_add(1)
            .ok_or_else(|| BenchmarkError::new("E0 command ticket exhausted"))?;
        Ok(ticket)
    }

    pub(super) fn request_identity(&mut self) -> BenchmarkResult<(RequestId, SequenceId)> {
        let value = self.next_request;
        self.next_request = self
            .next_request
            .checked_add(1)
            .ok_or_else(|| BenchmarkError::new("E0 request identity exhausted"))?;
        Ok((RequestId::new(value), SequenceId::new(value)))
    }

    pub(super) fn submit(
        &self,
        command: RuntimeCommand<CandleLlamaSource>,
        operation: &str,
    ) -> BenchmarkResult {
        self.runtime()?.try_submit(command).map_err(|error| {
            BenchmarkError::new(format!("{operation} command was rejected: {error:?}"))
        })
    }

    pub(super) fn receive(
        &self,
        ticket: CommandTicket,
        operation: &str,
    ) -> BenchmarkResult<RuntimeEvent> {
        let event = self
            .runtime()?
            .receive_timeout(EVENT_TIMEOUT)
            .map_err(|error| {
                BenchmarkError::new(format!(
                    "{operation} event did not arrive within the operational timeout: {error:?}"
                ))
            })?;
        validate_ticket(&event, ticket, operation)?;
        Ok(event)
    }

    pub(super) fn timed_exchange(
        &self,
        ticket: CommandTicket,
        command: RuntimeCommand<CandleLlamaSource>,
        operation: &str,
    ) -> BenchmarkResult<TimedEvent> {
        let started = Instant::now();
        self.submit(command, operation)?;
        let event = self.receive(ticket, operation)?;
        let elapsed = started.elapsed();
        Ok(TimedEvent { event, elapsed })
    }

    pub(super) fn record_loaded_model(&mut self, handle: ModelHandle) -> BenchmarkResult {
        if self.loaded_model.replace(handle).is_some() {
            return Err(BenchmarkError::new(
                "hosted E0 harness observed a second resident model",
            ));
        }
        Ok(())
    }

    pub(super) fn loaded_model(&self) -> BenchmarkResult<ModelHandle> {
        self.loaded_model
            .ok_or_else(|| BenchmarkError::new("hosted E0 fixture model is not loaded"))
    }

    pub(super) fn load_observation_snapshot(&self) -> CandleLoadObservationSnapshot {
        self.load_observation.snapshot()
    }

    pub(super) fn record_unloaded_model(&mut self, handle: ModelHandle) -> BenchmarkResult {
        if self.loaded_model != Some(handle) {
            return Err(BenchmarkError::new(
                "hosted E0 unload did not match the tracked model",
            ));
        }
        self.loaded_model = None;
        Ok(())
    }

    /// Combines a scenario result with unload, shutdown, endpoint release, and join.
    ///
    /// Successful scenarios unload a still-resident fixture before requiring a
    /// previously-clean shutdown receipt. Failed scenarios skip reject-if-busy
    /// unload and let E0's terminal shutdown path clean active ownership. Cleanup
    /// failures are appended without replacing the scenario failure.
    #[doc(hidden)]
    pub fn finish<T>(
        mut self,
        primary: BenchmarkResult<T>,
    ) -> BenchmarkResult<(T, ShutdownDurations)> {
        let (value, mut failure) = match primary {
            Ok(value) => (Some(value), None),
            Err(error) => (None, Some(error)),
        };

        if failure.is_none()
            && self.loaded_model.is_some()
            && let Err(error) = super::lifecycle::unload_loaded_model(&mut self)
        {
            append_failure(&mut failure, error);
        }

        let require_previously_clean = failure.is_none();
        let shutdown = match self.shutdown(require_previously_clean) {
            Ok(durations) => Some(durations),
            Err(error) => {
                append_failure(&mut failure, error);
                None
            }
        };

        if let Some(error) = failure {
            return Err(error);
        }
        match (value, shutdown) {
            (Some(value), Some(shutdown)) => Ok((value, shutdown)),
            _ => Err(BenchmarkError::new(
                "hosted E0 finalization lost a successful value or shutdown timing",
            )),
        }
    }

    fn shutdown(&mut self, require_previously_clean: bool) -> BenchmarkResult<ShutdownDurations> {
        let ticket = self.ticket();
        let total_started = Instant::now();
        let event_result = ticket.and_then(|ticket| {
            self.submit(RuntimeCommand::Shutdown { ticket }, "runtime shutdown")?;
            self.receive_shutdown(ticket, !require_previously_clean)
        });
        let event_elapsed = total_started.elapsed();
        let event_validation = event_result
            .and_then(|event| validate_shutdown_event(&event, require_previously_clean));

        drop(self.runtime.take());
        let join_started = Instant::now();
        let join_result = self.join_worker(join_started);
        if join_result.is_err() && self.thread.is_some() {
            self.retain_unjoined_worker();
        }
        let join_elapsed = join_started.elapsed();
        let durations = ShutdownDurations {
            event: event_elapsed,
            join: join_elapsed,
            total: total_started.elapsed(),
        };

        match event_validation {
            Ok(()) => {
                join_result?;
                Ok(durations)
            }
            Err(error) => Err(error.with_cleanup(join_result)),
        }
    }

    fn receive_shutdown(
        &self,
        ticket: CommandTicket,
        tolerate_pending_events: bool,
    ) -> BenchmarkResult<RuntimeEvent> {
        if !tolerate_pending_events {
            return self.receive(ticket, "runtime shutdown");
        }
        let deadline = Deadline::after(EVENT_TIMEOUT, "runtime shutdown event")?;
        loop {
            let remaining = deadline.remaining("matching runtime shutdown event")?;
            let event = self.runtime()?.receive_timeout(remaining).map_err(|error| {
                BenchmarkError::new(format!(
                    "runtime shutdown event did not arrive within the operational timeout: {error:?}"
                ))
            })?;
            if event.ticket() == ticket {
                return Ok(event);
            }
        }
    }

    fn join_worker(&mut self, started: Instant) -> BenchmarkResult {
        let deadline = Deadline::from_start(started, JOIN_TIMEOUT, "runtime worker join")?;
        loop {
            let finished = self
                .thread
                .as_ref()
                .ok_or_else(|| BenchmarkError::new("runtime thread handle is missing"))?
                .is_finished();
            if finished {
                break;
            }
            deadline.wait_for_poll("runtime worker completion before retained-handle join")?;
        }
        let thread = self
            .thread
            .take()
            .ok_or_else(|| BenchmarkError::new("runtime thread handle disappeared before join"))?;
        thread
            .join()
            .map_err(|error| BenchmarkError::new(format!("runtime worker join failed: {error}")))
    }

    fn retain_unjoined_worker(&mut self) {
        let Some(thread) = self.thread.take() else {
            return;
        };
        let mut retained = match RETAINED_WORKERS.lock() {
            Ok(retained) => retained,
            Err(poisoned) => poisoned.into_inner(),
        };
        retained.push(thread);
    }
}

fn validate_ticket(
    event: &RuntimeEvent,
    ticket: CommandTicket,
    operation: &str,
) -> BenchmarkResult {
    if event.ticket() != ticket {
        return Err(BenchmarkError::new(format!(
            "{operation} returned ticket {}, expected {}",
            event.ticket().get(),
            ticket.get()
        )));
    }
    Ok(())
}

fn validate_shutdown_event(
    event: &RuntimeEvent,
    require_previously_clean: bool,
) -> BenchmarkResult {
    let RuntimeEvent::Shutdown { result, .. } = event else {
        return Err(BenchmarkError::new(
            "runtime shutdown command returned a non-shutdown event",
        ));
    };
    let receipt = result.as_ref().map_err(|error| {
        BenchmarkError::new(format!("runtime shutdown reported failure: {error:?}"))
    })?;
    if require_previously_clean {
        validate_clean_shutdown_receipt(*receipt)?;
    }
    Ok(())
}

fn validate_clean_shutdown_receipt(receipt: ShutdownReceipt) -> BenchmarkResult {
    if receipt.unloaded_models != 0 || receipt.cancelled_requests != 0 {
        return Err(BenchmarkError::new(format!(
            "clean shutdown unexpectedly unloaded {} models or cancelled {} requests",
            receipt.unloaded_models, receipt.cancelled_requests
        )));
    }
    Ok(())
}

fn append_failure(failure: &mut Option<BenchmarkError>, cleanup: BenchmarkError) {
    *failure = Some(match failure.take() {
        Some(primary) => primary.with_cleanup(Err(cleanup)),
        None => cleanup,
    });
}

fn nonzero_usize(value: usize, label: &str) -> BenchmarkResult<NonZeroUsize> {
    NonZeroUsize::new(value).ok_or_else(|| BenchmarkError::new(format!("{label} must be non-zero")))
}
