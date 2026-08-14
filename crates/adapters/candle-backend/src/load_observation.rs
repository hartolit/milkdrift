//! Bounded, non-production observation state for one loader transaction.
//!
//! This module is compiled only through the non-default `benchmark-observation`
//! or `cuda-hardware-tests` feature. The observer owns no model resources and
//! cannot influence loader results. A benchmark or hardware test retains the
//! read handle while the loader and any failed-preparation owner carry the
//! recorder across the execution boundary.

use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use domain_contracts::LoadPlan;

/// Terminal or in-progress stage observed for one Candle load attempt.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CandleLoadObservationOutcome {
    /// No preparation attempt has started.
    NotStarted,
    /// Source inspection, identity establishment, and planning are running.
    Preparing,
    /// Preparation succeeded and the exact plan is retained.
    Prepared,
    /// Tensor materialization and model construction are running.
    Materializing,
    /// Materialization completed successfully.
    Succeeded,
    /// Preparation failed before a materialization owner existed.
    PreparationFailed,
    /// Materialization failed and produced an explicit cleanup owner.
    MaterializationFailed,
}

/// Cleanup state associated with the observed load attempt.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CandleLoadCleanupOutcome {
    /// The attempt has not produced a failed materialization owner.
    NotRequired,
    /// A failed materialization owner exists or a cleanup attempt is running.
    Pending,
    /// Explicit cleanup completed successfully.
    Succeeded,
    /// The latest explicit cleanup attempt failed and remains retryable.
    Failed,
}

/// Fixed-size observation snapshot for one Candle load attempt.
///
/// Byte counters include successful reads reported by the loader. Required
/// tensor reads are also whole-file verification reads because the same bytes
/// feed the shard digest. A locally established identity baseline therefore
/// contributes an additional whole-file pass without contributing required
/// tensor bytes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CandleLoadObservationSnapshot {
    /// Source inspection, identity establishment, and plan preparation time.
    pub preparation_duration: Option<Duration>,
    /// Tensor materialization, model construction, and loading sync time.
    pub materialization_duration: Option<Duration>,
    /// Required tensor payload bytes successfully read for materialization.
    pub required_bytes_read: u64,
    /// All bytes successfully read into a whole-file identity digest.
    pub whole_file_verification_bytes_read: u64,
    /// Accelerator transfer batches started by materialization.
    pub transfer_batches: u64,
    /// Device synchronization calls started inside materialization.
    pub loading_device_synchronizations: u64,
    /// Exact plan produced by this same observed preparation attempt.
    pub plan: Option<LoadPlan>,
    /// Current or terminal load outcome.
    pub outcome: CandleLoadObservationOutcome,
    /// Current or terminal cleanup outcome.
    pub cleanup_outcome: CandleLoadCleanupOutcome,
    /// Explicit cleanup calls started for the failed owner.
    pub cleanup_attempts: u64,
    /// Explicit cleanup calls that returned failure while retaining the owner.
    pub cleanup_failures: u64,
    /// Invalid transitions or saturated counters observed by instrumentation.
    ///
    /// A non-zero value makes the snapshot unsuitable as benchmark evidence;
    /// instrumentation never converts this condition into a loader failure.
    pub recording_errors: u64,
}

impl Default for CandleLoadObservationSnapshot {
    fn default() -> Self {
        Self {
            preparation_duration: None,
            materialization_duration: None,
            required_bytes_read: 0,
            whole_file_verification_bytes_read: 0,
            transfer_batches: 0,
            loading_device_synchronizations: 0,
            plan: None,
            outcome: CandleLoadObservationOutcome::NotStarted,
            cleanup_outcome: CandleLoadCleanupOutcome::NotRequired,
            cleanup_attempts: 0,
            cleanup_failures: 0,
            recording_errors: 0,
        }
    }
}

#[derive(Debug, Default)]
struct ObservationState {
    snapshot: CandleLoadObservationSnapshot,
    preparation_started: Option<Instant>,
    materialization_started: Option<Instant>,
}

impl ObservationState {
    fn record_error(&mut self) {
        self.snapshot.recording_errors = self.snapshot.recording_errors.saturating_add(1);
    }

    fn require_outcome(&mut self, expected: CandleLoadObservationOutcome) -> bool {
        if self.snapshot.outcome == expected {
            true
        } else {
            self.record_error();
            false
        }
    }

    fn add_counter(counter: &mut u64, amount: u64) -> bool {
        if let Some(total) = counter.checked_add(amount) {
            *counter = total;
            false
        } else {
            *counter = u64::MAX;
            true
        }
    }
}

type SharedObservation = Arc<Mutex<ObservationState>>;

/// Consumer-owned read handle for a single benchmark or hardware-test observation.
///
/// [`Self::channel`] creates this handle together with the recorder that must
/// be carried by the loader transaction. The state contains no event list and
/// remains fixed-size regardless of model tensor count.
#[derive(Clone, Debug)]
pub struct CandleLoadObservation {
    shared: SharedObservation,
}

impl CandleLoadObservation {
    /// Creates a one-attempt observation channel.
    #[must_use]
    pub fn channel() -> (Self, CandleLoadObservationRecorder) {
        let shared = Arc::new(Mutex::new(ObservationState::default()));
        (
            Self {
                shared: Arc::clone(&shared),
            },
            CandleLoadObservationRecorder { shared },
        )
    }

    /// Copies the current fixed-size evidence snapshot.
    #[must_use]
    pub fn snapshot(&self) -> CandleLoadObservationSnapshot {
        lock(&self.shared).snapshot
    }
}

/// Loader-owned writer for one benchmark observation.
///
/// The methods deliberately return no failure. Invalid ordering and numeric
/// saturation are recorded in [`CandleLoadObservationSnapshot::recording_errors`]
/// so benchmark validation can reject the evidence without changing product
/// loading behavior.
#[derive(Clone, Debug)]
pub struct CandleLoadObservationRecorder {
    shared: SharedObservation,
}

impl CandleLoadObservationRecorder {
    /// Marks the start of source inspection and plan preparation.
    pub(crate) fn preparation_started(&self) {
        let mut state = lock(&self.shared);
        if !state.require_outcome(CandleLoadObservationOutcome::NotStarted) {
            return;
        }
        state.preparation_started = Some(Instant::now());
        state.snapshot.outcome = CandleLoadObservationOutcome::Preparing;
    }

    /// Finishes preparation and retains the exact plan from that attempt.
    pub(crate) fn preparation_succeeded(&self, plan: &LoadPlan) {
        let mut state = lock(&self.shared);
        if !state.require_outcome(CandleLoadObservationOutcome::Preparing) {
            return;
        }
        let Some(started) = state.preparation_started.take() else {
            state.record_error();
            return;
        };
        state.snapshot.preparation_duration = Some(started.elapsed());
        state.snapshot.plan = Some(*plan);
        state.snapshot.outcome = CandleLoadObservationOutcome::Prepared;
    }

    /// Finishes a preparation attempt that returned failure.
    pub(crate) fn preparation_failed(&self) {
        let mut state = lock(&self.shared);
        if !state.require_outcome(CandleLoadObservationOutcome::Preparing) {
            return;
        }
        let Some(started) = state.preparation_started.take() else {
            state.record_error();
            return;
        };
        state.snapshot.preparation_duration = Some(started.elapsed());
        state.snapshot.outcome = CandleLoadObservationOutcome::PreparationFailed;
    }

    /// Marks the start of tensor materialization and model construction.
    pub(crate) fn materialization_started(&self) {
        let mut state = lock(&self.shared);
        if !state.require_outcome(CandleLoadObservationOutcome::Prepared) {
            return;
        }
        state.materialization_started = Some(Instant::now());
        state.snapshot.outcome = CandleLoadObservationOutcome::Materializing;
    }

    /// Finishes a successful materialization attempt.
    pub(crate) fn materialization_succeeded(&self) {
        let mut state = lock(&self.shared);
        if !state.require_outcome(CandleLoadObservationOutcome::Materializing) {
            return;
        }
        let Some(started) = state.materialization_started.take() else {
            state.record_error();
            return;
        };
        state.snapshot.materialization_duration = Some(started.elapsed());
        state.snapshot.outcome = CandleLoadObservationOutcome::Succeeded;
    }

    /// Finishes a failed materialization attempt and marks cleanup pending.
    pub(crate) fn materialization_failed(&self) {
        let mut state = lock(&self.shared);
        if !state.require_outcome(CandleLoadObservationOutcome::Materializing) {
            return;
        }
        let Some(started) = state.materialization_started.take() else {
            state.record_error();
            return;
        };
        state.snapshot.materialization_duration = Some(started.elapsed());
        state.snapshot.outcome = CandleLoadObservationOutcome::MaterializationFailed;
        state.snapshot.cleanup_outcome = CandleLoadCleanupOutcome::Pending;
    }

    /// Adds bytes read for both required materialization and whole-file hashing.
    pub(crate) fn required_and_verified_bytes_read(&self, bytes: u64) {
        let mut state = lock(&self.shared);
        if !state.require_outcome(CandleLoadObservationOutcome::Materializing) {
            return;
        }
        let required_overflow =
            ObservationState::add_counter(&mut state.snapshot.required_bytes_read, bytes);
        let verification_overflow = ObservationState::add_counter(
            &mut state.snapshot.whole_file_verification_bytes_read,
            bytes,
        );
        if required_overflow || verification_overflow {
            state.record_error();
        }
    }

    /// Adds non-required bytes read into a whole-file identity digest.
    ///
    /// This includes headers, ignored tensor ranges, and any pre-materialization
    /// local identity baseline.
    pub(crate) fn verification_only_bytes_read(&self, bytes: u64) {
        let mut state = lock(&self.shared);
        if !matches!(
            state.snapshot.outcome,
            CandleLoadObservationOutcome::Preparing | CandleLoadObservationOutcome::Materializing
        ) {
            state.record_error();
            return;
        }
        if ObservationState::add_counter(
            &mut state.snapshot.whole_file_verification_bytes_read,
            bytes,
        ) {
            state.record_error();
        }
    }

    /// Adds transfer batches started during accelerator materialization.
    pub(crate) fn transfer_batches_started(&self, count: u64) {
        let mut state = lock(&self.shared);
        if !state.require_outcome(CandleLoadObservationOutcome::Materializing) {
            return;
        }
        if ObservationState::add_counter(&mut state.snapshot.transfer_batches, count) {
            state.record_error();
        }
    }

    /// Adds device synchronization calls started during materialization.
    pub(crate) fn loading_device_synchronizations_started(&self, count: u64) {
        let mut state = lock(&self.shared);
        if !state.require_outcome(CandleLoadObservationOutcome::Materializing) {
            return;
        }
        if ObservationState::add_counter(&mut state.snapshot.loading_device_synchronizations, count)
        {
            state.record_error();
        }
    }

    /// Marks the start of one explicit failed-owner cleanup call.
    pub(crate) fn cleanup_started(&self) {
        let mut state = lock(&self.shared);
        if state.snapshot.outcome != CandleLoadObservationOutcome::MaterializationFailed
            || !matches!(
                state.snapshot.cleanup_outcome,
                CandleLoadCleanupOutcome::Pending | CandleLoadCleanupOutcome::Failed
            )
        {
            state.record_error();
            return;
        }
        if ObservationState::add_counter(&mut state.snapshot.cleanup_attempts, 1) {
            state.record_error();
        }
        state.snapshot.cleanup_outcome = CandleLoadCleanupOutcome::Pending;
    }

    /// Marks successful explicit cleanup of the failed owner.
    pub(crate) fn cleanup_succeeded(&self) {
        let mut state = lock(&self.shared);
        if state.snapshot.outcome != CandleLoadObservationOutcome::MaterializationFailed
            || state.snapshot.cleanup_outcome != CandleLoadCleanupOutcome::Pending
            || state.snapshot.cleanup_attempts == 0
        {
            state.record_error();
            return;
        }
        state.snapshot.cleanup_outcome = CandleLoadCleanupOutcome::Succeeded;
    }

    /// Marks a retryable explicit cleanup failure.
    pub(crate) fn cleanup_failed(&self) {
        let mut state = lock(&self.shared);
        if state.snapshot.outcome != CandleLoadObservationOutcome::MaterializationFailed
            || state.snapshot.cleanup_outcome != CandleLoadCleanupOutcome::Pending
            || state.snapshot.cleanup_attempts == 0
        {
            state.record_error();
            return;
        }
        if ObservationState::add_counter(&mut state.snapshot.cleanup_failures, 1) {
            state.record_error();
        }
        state.snapshot.cleanup_outcome = CandleLoadCleanupOutcome::Failed;
    }
}

fn lock(shared: &SharedObservation) -> MutexGuard<'_, ObservationState> {
    match shared.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

#[cfg(test)]
mod tests {
    use domain_contracts::{
        BackendId, CapabilitySet, DeviceId, DeviceKind, ExecutionDevice, LoadConfiguration,
        LoadPlan, MemoryBudget, MemoryFootprint, ModelArchitecture, ModelCapabilities,
        ModelDescriptor, ModelGeneration, ModelHandle, ModelId, ModelMetadata, QuantizationFormat,
        ScalarType, ScalarTypeSet,
    };

    use super::{CandleLoadCleanupOutcome, CandleLoadObservation, CandleLoadObservationOutcome};

    #[test]
    fn preparation_failure_is_terminal_without_cleanup() {
        let (observation, recorder) = CandleLoadObservation::channel();
        recorder.preparation_started();
        recorder.verification_only_bytes_read(41);
        recorder.preparation_failed();

        let snapshot = observation.snapshot();
        assert!(snapshot.preparation_duration.is_some());
        assert!(snapshot.materialization_duration.is_none());
        assert_eq!(snapshot.whole_file_verification_bytes_read, 41);
        assert_eq!(
            snapshot.outcome,
            CandleLoadObservationOutcome::PreparationFailed
        );
        assert_eq!(
            snapshot.cleanup_outcome,
            CandleLoadCleanupOutcome::NotRequired
        );
        assert_eq!(snapshot.recording_errors, 0);
    }

    #[test]
    fn failed_cleanup_retry_preserves_counters_and_reaches_success() {
        let plan = fixture_plan();
        let (observation, recorder) = CandleLoadObservation::channel();
        recorder.preparation_started();
        recorder.preparation_succeeded(&plan);
        recorder.materialization_started();
        recorder.required_and_verified_bytes_read(80);
        recorder.transfer_batches_started(2);
        recorder.loading_device_synchronizations_started(2);
        recorder.materialization_failed();
        recorder.cleanup_started();
        recorder.cleanup_failed();
        let after_failure = observation.snapshot();
        recorder.cleanup_started();
        recorder.cleanup_succeeded();
        let after_success = observation.snapshot();

        assert_eq!(after_failure.required_bytes_read, 80);
        assert_eq!(after_failure.transfer_batches, 2);
        assert_eq!(after_failure.loading_device_synchronizations, 2);
        assert_eq!(after_failure.cleanup_attempts, 1);
        assert_eq!(after_failure.cleanup_failures, 1);
        assert_eq!(
            after_failure.cleanup_outcome,
            CandleLoadCleanupOutcome::Failed
        );
        assert_eq!(after_success.required_bytes_read, 80);
        assert_eq!(after_success.transfer_batches, 2);
        assert_eq!(after_success.loading_device_synchronizations, 2);
        assert_eq!(after_success.cleanup_attempts, 2);
        assert_eq!(after_success.cleanup_failures, 1);
        assert_eq!(
            after_success.cleanup_outcome,
            CandleLoadCleanupOutcome::Succeeded
        );
        assert_eq!(after_success.recording_errors, 0);
    }

    #[test]
    fn counters_saturate_and_invalidate_evidence_without_panicking() {
        let (observation, recorder) = CandleLoadObservation::channel();
        recorder.preparation_started();
        recorder.verification_only_bytes_read(u64::MAX);
        recorder.verification_only_bytes_read(1);

        let snapshot = observation.snapshot();
        assert_eq!(snapshot.whole_file_verification_bytes_read, u64::MAX);
        assert_eq!(snapshot.recording_errors, 1);
    }

    #[test]
    fn snapshot_shape_is_fixed_and_small() {
        assert!(
            std::mem::size_of::<super::CandleLoadObservationSnapshot>() <= 512,
            "one observation snapshot must remain bounded independently of tensor count"
        );
    }

    fn fixture_plan() -> LoadPlan {
        let execution_device = ExecutionDevice::new(DeviceId::new(0), DeviceKind::Cuda);
        let final_footprint = MemoryFootprint {
            host_weight_bytes: 0,
            device_weight_bytes: 4_096,
            host_working_bytes: 0,
            device_working_bytes: 0,
        };
        LoadPlan {
            accepted_configuration: LoadConfiguration {
                handle: ModelHandle::new(ModelId::new(7), ModelGeneration::new(1)),
                execution_device,
                memory_budget: MemoryBudget {
                    host_bytes: u64::MAX,
                    device_bytes: u64::MAX,
                },
            },
            descriptor: ModelDescriptor {
                backend: BackendId::new(10_001),
                metadata: ModelMetadata {
                    architecture: ModelArchitecture::Llama,
                    configuration_declared_scalar_type: Some(ScalarType::F32),
                    observed_tensor_scalar_types: ScalarTypeSet::from_scalar(ScalarType::F32),
                    quantization: QuantizationFormat::None,
                    vocabulary_size: 16,
                    context_length: 32,
                },
                capabilities: ModelCapabilities {
                    operations: CapabilitySet::PREFILL,
                    maximum_context_tokens: 32,
                    maximum_sequences: 1,
                    maximum_prefill_batch: 32,
                },
                estimated_footprint: final_footprint,
                sequence_cache_bytes_per_token: 64,
            },
            execution_scalar_type: ScalarType::F32,
            final_footprint,
            loading_peak_footprint: MemoryFootprint {
                host_weight_bytes: 0,
                device_weight_bytes: 4_096,
                host_working_bytes: 1_024,
                device_working_bytes: 0,
            },
        }
    }
}
