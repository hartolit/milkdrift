//! Bounded hosted-runtime command, snapshot, shutdown, and join ownership.

use std::num::{NonZeroU32, NonZeroU64, NonZeroUsize};
use std::time::{Duration, Instant};

use candle_backend::{CandleLlamaLoader, CandleLlamaSource};
use domain_contracts::{BackendId, MemoryBudget, MemoryFootprint, ModelLifecycleState};
use inference_runtime::{
    CommandTicket, HostedRuntime, HostedRuntimeConfiguration, RuntimeCommand, RuntimeEvent,
    RuntimeLimits, RuntimeSnapshot, RuntimeThread, ShutdownReceipt, start_hosted_runtime,
};

use crate::error::{BenchmarkError, BenchmarkResult};
use crate::memory::process_memory;
use crate::report::{
    MemoryFootprintRecord, ModelAccounting, RuntimeAccounting, ShutdownMeasurement,
    SnapshotCheckpoint, duration_ns,
};

pub(super) const CANDLE_BACKEND: BackendId = BackendId::new(10_001);
const EVENT_TIMEOUT: Duration = Duration::from_secs(10);
const JOIN_TIMEOUT: Duration = Duration::from_secs(10);
const WAIT_INTERVAL: Duration = Duration::from_millis(1);

pub(super) type CandleRuntime = HostedRuntime<CandleLlamaSource>;

pub(super) struct E0Harness {
    pub(super) runtime: CandleRuntime,
    thread: Option<RuntimeThread>,
    next_ticket: u64,
}

pub(super) struct CapturedSnapshot {
    pub(super) raw: RuntimeSnapshot,
    pub(super) models: Vec<inference_runtime::ModelSnapshot>,
    pub(super) record: SnapshotCheckpoint,
}

impl E0Harness {
    pub(super) fn start() -> BenchmarkResult<(Self, Duration)> {
        let configuration = HostedRuntimeConfiguration::new(
            nonzero_usize(16, "command capacity")?,
            nonzero_usize(16, "event capacity")?,
            NonZeroU64::MIN,
        )
        .with_token_output_capacity(
            NonZeroUsize::MIN,
            nonzero_usize(128, "token output record capacity")?,
        );
        let limits = RuntimeLimits::new(
            NonZeroU32::MIN,
            NonZeroU32::MIN,
            MemoryBudget {
                host_bytes: u64::MAX,
                device_bytes: 0,
            },
        );
        let started = Instant::now();
        let (runtime, thread) = start_hosted_runtime(
            CandleLlamaLoader::new(CANDLE_BACKEND),
            limits,
            configuration,
        )
        .map_err(|error| BenchmarkError::new(format!("E0 worker start failed: {error}")))?;
        let elapsed = started.elapsed();
        Ok((
            Self {
                runtime,
                thread: Some(thread),
                next_ticket: 1,
            },
            elapsed,
        ))
    }

    pub(super) fn ticket(&mut self) -> BenchmarkResult<CommandTicket> {
        let ticket = CommandTicket::new(self.next_ticket);
        self.next_ticket = self
            .next_ticket
            .checked_add(1)
            .ok_or_else(|| BenchmarkError::new("E0 command ticket exhausted"))?;
        Ok(ticket)
    }

    pub(super) fn submit(
        &self,
        command: RuntimeCommand<CandleLlamaSource>,
        operation: &str,
    ) -> BenchmarkResult {
        self.runtime.try_submit(command).map_err(|error| {
            BenchmarkError::new(format!("{operation} command was rejected: {error:?}"))
        })
    }

    pub(super) fn receive(
        &self,
        ticket: CommandTicket,
        operation: &str,
    ) -> BenchmarkResult<RuntimeEvent> {
        let event = self
            .runtime
            .receive_timeout(EVENT_TIMEOUT)
            .map_err(|error| {
                BenchmarkError::new(format!(
                    "{operation} event did not arrive within the operational timeout: {error:?}"
                ))
            })?;
        if event.ticket() != ticket {
            return Err(BenchmarkError::new(format!(
                "{operation} returned ticket {}, expected {}",
                event.ticket().get(),
                ticket.get()
            )));
        }
        Ok(event)
    }

    pub(super) fn snapshot(
        &mut self,
        checkpoint: &'static str,
    ) -> BenchmarkResult<CapturedSnapshot> {
        let ticket = self.ticket()?;
        self.submit(RuntimeCommand::Snapshot { ticket }, "runtime snapshot")?;
        let event = self.receive(ticket, "runtime snapshot")?;
        let RuntimeEvent::Snapshot {
            runtime, models, ..
        } = event
        else {
            return Err(BenchmarkError::new(
                "runtime snapshot command returned a non-snapshot event",
            ));
        };
        validate_no_cleanup_failure(&runtime, &models, checkpoint)?;
        let model_records = models.iter().map(model_accounting).collect();
        let record = SnapshotCheckpoint {
            checkpoint,
            process_memory: process_memory()?,
            runtime: runtime_accounting(runtime),
            models: model_records,
        };
        Ok(CapturedSnapshot {
            raw: runtime,
            models,
            record,
        })
    }

    pub(super) fn shutdown(
        &mut self,
        require_previously_clean: bool,
    ) -> BenchmarkResult<ShutdownMeasurement> {
        let ticket = self.ticket()?;
        let total_started = Instant::now();
        let event_result = self
            .submit(RuntimeCommand::Shutdown { ticket }, "runtime shutdown")
            .and_then(|()| self.receive(ticket, "runtime shutdown"));
        let event_elapsed = total_started.elapsed();
        let event_validation = event_result
            .and_then(|event| validate_shutdown_event(&event, require_previously_clean));

        let join_started = Instant::now();
        let join_result = self.join_worker(join_started);
        let join_elapsed = join_started.elapsed();
        let measurement = ShutdownMeasurement {
            event_ns: duration_ns(event_elapsed),
            join_ns: duration_ns(join_elapsed),
            total_ns: duration_ns(total_started.elapsed()),
        };

        match event_validation {
            Ok(()) => {
                join_result?;
                Ok(measurement)
            }
            Err(error) => Err(error.with_cleanup(join_result)),
        }
    }

    fn join_worker(&mut self, started: Instant) -> BenchmarkResult {
        let deadline = started
            .checked_add(JOIN_TIMEOUT)
            .ok_or_else(|| BenchmarkError::new("runtime join deadline overflowed"))?;
        loop {
            let finished = self
                .thread
                .as_ref()
                .ok_or_else(|| BenchmarkError::new("runtime thread handle is missing"))?
                .is_finished();
            if finished {
                break;
            }
            let remaining = deadline
                .checked_duration_since(Instant::now())
                .filter(|duration| !duration.is_zero())
                .ok_or_else(|| {
                    BenchmarkError::new(
                        "runtime worker did not finish within the operational join timeout",
                    )
                })?;
            std::thread::sleep(WAIT_INTERVAL.min(remaining));
        }
        let thread = self
            .thread
            .take()
            .ok_or_else(|| BenchmarkError::new("runtime thread handle disappeared before join"))?;
        thread
            .join()
            .map_err(|error| BenchmarkError::new(format!("runtime worker join failed: {error}")))
    }
}

fn nonzero_usize(value: usize, label: &str) -> BenchmarkResult<NonZeroUsize> {
    NonZeroUsize::new(value).ok_or_else(|| BenchmarkError::new(format!("{label} must be non-zero")))
}

fn validate_no_cleanup_failure(
    runtime: &RuntimeSnapshot,
    models: &[inference_runtime::ModelSnapshot],
    checkpoint: &str,
) -> BenchmarkResult {
    if runtime.pending_cleanup_models != 0
        || runtime.pending_cleanup_sequences != 0
        || runtime.exhausted_cleanup_models != 0
        || runtime.exhausted_cleanup_sequences != 0
        || runtime.maintenance_error.is_some()
        || models.iter().any(|model| {
            model.pending_cleanup_sequences != 0
                || model.exhausted_cleanup_sequences != 0
                || model.degraded
        })
    {
        return Err(BenchmarkError::new(format!(
            "{checkpoint} snapshot contains pending, exhausted, degraded, or failed cleanup accounting"
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

fn runtime_accounting(snapshot: RuntimeSnapshot) -> RuntimeAccounting {
    RuntimeAccounting {
        loaded_models: snapshot.loaded_models,
        active_requests: snapshot.active_requests,
        reserved_footprint: footprint(snapshot.reserved_footprint),
        generation_workspaces: snapshot.generation_workspaces,
        reserved_generation_workspace: footprint(snapshot.reserved_generation_workspace),
        pending_cleanup_models: snapshot.pending_cleanup_models,
        pending_cleanup_sequences: snapshot.pending_cleanup_sequences,
        exhausted_cleanup_models: snapshot.exhausted_cleanup_models,
        exhausted_cleanup_sequences: snapshot.exhausted_cleanup_sequences,
        last_cleanup_present: snapshot.last_cleanup.is_some(),
        maintenance_error_present: snapshot.maintenance_error.is_some(),
        shutting_down: snapshot.shutting_down,
    }
}

fn model_accounting(snapshot: &inference_runtime::ModelSnapshot) -> ModelAccounting {
    ModelAccounting {
        model_id: snapshot.handle.id.get(),
        generation: snapshot.handle.generation.get(),
        lifecycle: lifecycle_label(snapshot.lifecycle),
        reserved_footprint: footprint(snapshot.reserved_footprint),
        active_requests: snapshot.active_requests,
        pending_cleanup_sequences: snapshot.pending_cleanup_sequences,
        exhausted_cleanup_sequences: snapshot.exhausted_cleanup_sequences,
        degraded: snapshot.degraded,
    }
}

const fn lifecycle_label(state: ModelLifecycleState) -> &'static str {
    match state {
        ModelLifecycleState::Absent => "absent",
        ModelLifecycleState::Loading => "loading",
        ModelLifecycleState::Ready => "ready",
        ModelLifecycleState::Active { .. } => "active",
        ModelLifecycleState::Draining { .. } => "draining",
        ModelLifecycleState::Cancelling { .. } => "cancelling",
        ModelLifecycleState::Unloading => "unloading",
        ModelLifecycleState::Failed { .. } => "failed",
    }
}

pub(super) const fn footprint(value: MemoryFootprint) -> MemoryFootprintRecord {
    MemoryFootprintRecord {
        host_weight_bytes: value.host_weight_bytes,
        device_weight_bytes: value.device_weight_bytes,
        host_working_bytes: value.host_working_bytes,
        device_working_bytes: value.device_working_bytes,
        cache_bytes_per_token: value.cache_bytes_per_token,
    }
}
