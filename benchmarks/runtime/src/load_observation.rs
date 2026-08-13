//! Stable report conversion for the benchmark-only Candle load observer.

use std::time::Duration;

use candle_backend::{
    CandleLoadCleanupOutcome, CandleLoadObservationOutcome, CandleLoadObservationSnapshot,
};
use domain_contracts::{DeviceKind, LoadPlan, MemoryFootprint};
use inference_runtime::LoadReceipt;
use serde::Serialize;

use crate::error::{BenchmarkError, BenchmarkResult};

/// Serializable correctness and timing facts from one actual hosted load.
///
/// This record is intentionally separate from sampled process RSS and device
/// telemetry. Its planned footprints come from the observed loader transaction;
/// actual final ownership comes only from E0's accepted [`LoadReceipt`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct CandleLoaderObservationRecord {
    preparation_duration_ns: Option<u64>,
    materialization_duration_ns: Option<u64>,
    required_bytes_read: u64,
    whole_file_verification_bytes_read: u64,
    transfer_batches: u64,
    loading_device_synchronizations: u64,
    planned_final_footprint: Option<ObservedFootprintRecord>,
    planned_loading_peak_footprint: Option<ObservedFootprintRecord>,
    actual_e0_final_ownership: Option<ObservedFootprintRecord>,
    outcome: LoadOutcomeRecord,
    cleanup_outcome: CleanupOutcomeRecord,
    cleanup_attempts: u64,
    cleanup_failures: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum LoadOutcomeRecord {
    Succeeded,
    PreparationFailed,
    MaterializationFailed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum CleanupOutcomeRecord {
    NotRequired,
    Pending,
    Succeeded,
    Failed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[expect(
    clippy::struct_field_names,
    reason = "serialized footprint fields preserve the domain MemoryFootprint byte units"
)]
struct ObservedFootprintRecord {
    host_weight_bytes: u64,
    device_weight_bytes: u64,
    host_working_bytes: u64,
    device_working_bytes: u64,
}

/// Validates and converts one fixed-size loader snapshot into report evidence.
///
/// A successful loader observation requires the matching public E0 receipt.
/// Failed observations must not carry a receipt, and invalid or saturated
/// instrumentation is rejected rather than serialized as trustworthy evidence.
///
/// # Errors
///
/// Returns an error when phase timing, byte accounting, plan/receipt identity,
/// cleanup state, or observer transitions are incomplete or inconsistent.
pub fn candle_loader_observation_record(
    snapshot: CandleLoadObservationSnapshot,
    receipt: Option<&LoadReceipt>,
) -> BenchmarkResult<CandleLoaderObservationRecord> {
    validate_common(snapshot)?;
    let plan = snapshot.plan.as_ref();
    let (outcome, cleanup_outcome) = validate_outcome(snapshot, receipt)?;
    let actual_e0_final_ownership = match (plan, receipt) {
        (Some(plan), Some(receipt)) => {
            validate_receipt(plan, receipt)?;
            Some(footprint_record(receipt.reserved_footprint))
        }
        (None | Some(_), None) => None,
        (None, Some(_)) => {
            return Err(BenchmarkError::new(
                "Candle load observation received an E0 receipt without an observed plan",
            ));
        }
    };

    Ok(CandleLoaderObservationRecord {
        preparation_duration_ns: optional_duration_ns(
            snapshot.preparation_duration,
            "Candle preparation",
        )?,
        materialization_duration_ns: optional_duration_ns(
            snapshot.materialization_duration,
            "Candle materialization",
        )?,
        required_bytes_read: snapshot.required_bytes_read,
        whole_file_verification_bytes_read: snapshot.whole_file_verification_bytes_read,
        transfer_batches: snapshot.transfer_batches,
        loading_device_synchronizations: snapshot.loading_device_synchronizations,
        planned_final_footprint: plan.map(|plan| footprint_record(plan.final_footprint)),
        planned_loading_peak_footprint: plan
            .map(|plan| footprint_record(plan.loading_peak_footprint)),
        actual_e0_final_ownership,
        outcome,
        cleanup_outcome,
        cleanup_attempts: snapshot.cleanup_attempts,
        cleanup_failures: snapshot.cleanup_failures,
    })
}

fn validate_common(snapshot: CandleLoadObservationSnapshot) -> BenchmarkResult {
    if snapshot.recording_errors != 0 {
        return Err(BenchmarkError::new(format!(
            "Candle load observation recorded {} invalid transition or counter errors",
            snapshot.recording_errors
        )));
    }
    if snapshot.whole_file_verification_bytes_read < snapshot.required_bytes_read {
        return Err(BenchmarkError::new(
            "Candle load observation reported fewer whole-file verification bytes than required tensor bytes",
        ));
    }
    if snapshot.cleanup_failures > snapshot.cleanup_attempts {
        return Err(BenchmarkError::new(
            "Candle load observation reported more cleanup failures than attempts",
        ));
    }
    Ok(())
}

fn validate_outcome(
    snapshot: CandleLoadObservationSnapshot,
    receipt: Option<&LoadReceipt>,
) -> BenchmarkResult<(LoadOutcomeRecord, CleanupOutcomeRecord)> {
    let cleanup = cleanup_record(snapshot.cleanup_outcome);
    match snapshot.outcome {
        CandleLoadObservationOutcome::Succeeded
            if snapshot.preparation_duration.is_some()
                && snapshot.materialization_duration.is_some()
                && snapshot.plan.is_some()
                && receipt.is_some()
                && snapshot.cleanup_outcome == CandleLoadCleanupOutcome::NotRequired
                && snapshot.cleanup_attempts == 0
                && snapshot.cleanup_failures == 0 =>
        {
            validate_successful_loading_counts(snapshot)?;
            Ok((LoadOutcomeRecord::Succeeded, cleanup))
        }
        CandleLoadObservationOutcome::PreparationFailed
            if snapshot.preparation_duration.is_some()
                && snapshot.materialization_duration.is_none()
                && snapshot.plan.is_none()
                && receipt.is_none()
                && snapshot.cleanup_outcome == CandleLoadCleanupOutcome::NotRequired
                && snapshot.cleanup_attempts == 0
                && snapshot.cleanup_failures == 0 =>
        {
            Ok((LoadOutcomeRecord::PreparationFailed, cleanup))
        }
        CandleLoadObservationOutcome::MaterializationFailed
            if snapshot.preparation_duration.is_some()
                && snapshot.materialization_duration.is_some()
                && snapshot.plan.is_some()
                && receipt.is_none()
                && snapshot.cleanup_outcome != CandleLoadCleanupOutcome::NotRequired =>
        {
            validate_cleanup_counts(snapshot)?;
            Ok((LoadOutcomeRecord::MaterializationFailed, cleanup))
        }
        _ => Err(BenchmarkError::new(
            "Candle load observation was not a coherent terminal success, preparation failure, or materialization failure",
        )),
    }
}

fn validate_cleanup_counts(snapshot: CandleLoadObservationSnapshot) -> BenchmarkResult {
    let one_success_or_active_attempt = snapshot
        .cleanup_failures
        .checked_add(1)
        .is_some_and(|attempts| attempts == snapshot.cleanup_attempts);
    match snapshot.cleanup_outcome {
        CandleLoadCleanupOutcome::Pending
            if (snapshot.cleanup_attempts == 0 && snapshot.cleanup_failures == 0)
                || one_success_or_active_attempt =>
        {
            Ok(())
        }
        CandleLoadCleanupOutcome::Succeeded if one_success_or_active_attempt => Ok(()),
        CandleLoadCleanupOutcome::Failed
            if snapshot.cleanup_attempts > 0
                && snapshot.cleanup_failures == snapshot.cleanup_attempts =>
        {
            Ok(())
        }
        _ => Err(BenchmarkError::new(
            "Candle materialization failure carried inconsistent cleanup attempt evidence",
        )),
    }
}

fn validate_successful_loading_counts(snapshot: CandleLoadObservationSnapshot) -> BenchmarkResult {
    let device_kind = snapshot
        .plan
        .as_ref()
        .map(|plan| plan.accepted_configuration.execution_device.kind)
        .ok_or_else(|| BenchmarkError::new("successful Candle observation omitted its plan"))?;
    let coherent = match device_kind {
        DeviceKind::Cpu => {
            snapshot.transfer_batches == 0 && snapshot.loading_device_synchronizations == 0
        }
        DeviceKind::Cuda => {
            snapshot.transfer_batches > 0
                && snapshot.loading_device_synchronizations == snapshot.transfer_batches
        }
        _ => false,
    };
    if coherent {
        Ok(())
    } else {
        Err(BenchmarkError::new(
            "successful Candle observation carried impossible transfer-batch or loading-synchronization counts",
        ))
    }
}

const fn cleanup_record(value: CandleLoadCleanupOutcome) -> CleanupOutcomeRecord {
    match value {
        CandleLoadCleanupOutcome::NotRequired => CleanupOutcomeRecord::NotRequired,
        CandleLoadCleanupOutcome::Pending => CleanupOutcomeRecord::Pending,
        CandleLoadCleanupOutcome::Succeeded => CleanupOutcomeRecord::Succeeded,
        CandleLoadCleanupOutcome::Failed => CleanupOutcomeRecord::Failed,
    }
}

fn validate_receipt(plan: &LoadPlan, receipt: &LoadReceipt) -> BenchmarkResult {
    if receipt.handle != plan.accepted_configuration.handle
        || receipt.execution_device != plan.accepted_configuration.execution_device
        || receipt.execution_scalar_type != plan.execution_scalar_type
        || receipt.descriptor != plan.descriptor
        || receipt.reserved_footprint != plan.final_footprint
    {
        return Err(BenchmarkError::new(
            "actual E0 final ownership did not match the observed Candle load plan",
        ));
    }
    Ok(())
}

fn optional_duration_ns(
    duration: Option<Duration>,
    label: &'static str,
) -> BenchmarkResult<Option<u64>> {
    duration
        .map(|duration| {
            u64::try_from(duration.as_nanos()).map_err(|_| {
                BenchmarkError::new(format!(
                    "{label} duration exceeded the report's u64 nanosecond range"
                ))
            })
        })
        .transpose()
}

const fn footprint_record(value: MemoryFootprint) -> ObservedFootprintRecord {
    ObservedFootprintRecord {
        host_weight_bytes: value.host_weight_bytes,
        device_weight_bytes: value.device_weight_bytes,
        host_working_bytes: value.host_working_bytes,
        device_working_bytes: value.device_working_bytes,
    }
}

#[cfg(test)]
pub(crate) const fn successful_test_record() -> CandleLoaderObservationRecord {
    CandleLoaderObservationRecord {
        preparation_duration_ns: Some(11),
        materialization_duration_ns: Some(17),
        required_bytes_read: 4_000,
        whole_file_verification_bytes_read: 4_096,
        transfer_batches: 0,
        loading_device_synchronizations: 0,
        planned_final_footprint: Some(ObservedFootprintRecord {
            host_weight_bytes: 4_000,
            device_weight_bytes: 0,
            host_working_bytes: 0,
            device_working_bytes: 0,
        }),
        planned_loading_peak_footprint: Some(ObservedFootprintRecord {
            host_weight_bytes: 4_000,
            device_weight_bytes: 0,
            host_working_bytes: 800,
            device_working_bytes: 0,
        }),
        actual_e0_final_ownership: Some(ObservedFootprintRecord {
            host_weight_bytes: 4_000,
            device_weight_bytes: 0,
            host_working_bytes: 0,
            device_working_bytes: 0,
        }),
        outcome: LoadOutcomeRecord::Succeeded,
        cleanup_outcome: CleanupOutcomeRecord::NotRequired,
        cleanup_attempts: 0,
        cleanup_failures: 0,
    }
}

#[cfg(test)]
mod tests {
    use candle_backend::CandleLoadObservation;
    use domain_contracts::{
        BackendId, CapabilitySet, DeviceId, DeviceKind, ExecutionDevice, LoadConfiguration,
        LoadPlan, MemoryBudget, MemoryFootprint, ModelArchitecture, ModelCapabilities,
        ModelDescriptor, ModelGeneration, ModelHandle, ModelId, ModelMetadata, QuantizationFormat,
        ScalarType, ScalarTypeSet,
    };
    use inference_runtime::LoadReceipt;
    use serde_json::Value;

    use super::candle_loader_observation_record;
    use candle_backend::{CandleLoadCleanupOutcome, CandleLoadObservationOutcome};

    #[test]
    fn successful_record_keeps_planned_and_actual_ownership_distinct() -> Result<(), String> {
        let plan = fixture_plan();
        let receipt = fixture_receipt(plan);
        let (observation, recorder) = CandleLoadObservation::channel();
        recorder.preparation_started();
        recorder.verification_only_bytes_read(200);
        recorder.preparation_succeeded(&plan);
        recorder.materialization_started();
        recorder.required_and_verified_bytes_read(100);
        recorder.transfer_batches_started(2);
        recorder.loading_device_synchronizations_started(2);
        recorder.materialization_succeeded();

        let record = candle_loader_observation_record(observation.snapshot(), Some(&receipt))
            .map_err(|error| error.to_string())?;
        let value = serde_json::to_value(record).map_err(|error| error.to_string())?;
        assert_eq!(value_at(&value, "required_bytes_read")?.as_u64(), Some(100));
        assert_eq!(
            value_at(&value, "whole_file_verification_bytes_read")?.as_u64(),
            Some(300)
        );
        assert_eq!(value_at(&value, "transfer_batches")?.as_u64(), Some(2));
        assert_eq!(
            value_at(&value, "loading_device_synchronizations")?.as_u64(),
            Some(2)
        );
        assert_eq!(
            nested_u64(&value, "planned_final_footprint", "device_weight_bytes")?,
            4_096
        );
        assert_eq!(
            nested_u64(
                &value,
                "planned_loading_peak_footprint",
                "host_working_bytes",
            )?,
            1_024
        );
        assert_eq!(
            nested_u64(&value, "actual_e0_final_ownership", "device_weight_bytes",)?,
            4_096
        );
        assert_eq!(value_at(&value, "outcome")?.as_str(), Some("succeeded"));
        assert_eq!(
            value_at(&value, "cleanup_outcome")?.as_str(),
            Some("not_required")
        );
        Ok(())
    }

    #[test]
    fn successful_record_rejects_impossible_device_batch_and_sync_counts() -> Result<(), String> {
        let cuda_plan = fixture_plan();
        let cuda_receipt = fixture_receipt(cuda_plan);
        let (cuda_observation, cuda_recorder) = CandleLoadObservation::channel();
        cuda_recorder.preparation_started();
        cuda_recorder.preparation_succeeded(&cuda_plan);
        cuda_recorder.materialization_started();
        cuda_recorder.transfer_batches_started(2);
        cuda_recorder.loading_device_synchronizations_started(3);
        cuda_recorder.materialization_succeeded();
        let cuda_error =
            candle_loader_observation_record(cuda_observation.snapshot(), Some(&cuda_receipt))
                .err()
                .ok_or_else(|| {
                    "mismatched CUDA batch/sync counts unexpectedly serialized".to_owned()
                })?;
        assert!(cuda_error.to_string().contains("impossible transfer-batch"));

        let cpu_plan = cpu_fixture_plan();
        let cpu_receipt = fixture_receipt(cpu_plan);
        let (cpu_observation, cpu_recorder) = CandleLoadObservation::channel();
        cpu_recorder.preparation_started();
        cpu_recorder.preparation_succeeded(&cpu_plan);
        cpu_recorder.materialization_started();
        cpu_recorder.transfer_batches_started(1);
        cpu_recorder.loading_device_synchronizations_started(1);
        cpu_recorder.materialization_succeeded();
        let cpu_error =
            candle_loader_observation_record(cpu_observation.snapshot(), Some(&cpu_receipt))
                .err()
                .ok_or_else(|| {
                    "nonzero CPU batch/sync counts unexpectedly serialized".to_owned()
                })?;
        assert!(cpu_error.to_string().contains("impossible transfer-batch"));
        Ok(())
    }

    #[test]
    fn materialization_failure_cleanup_states_serialize_only_when_reachable() -> Result<(), String>
    {
        let plan = fixture_plan();
        let (observation, recorder) = CandleLoadObservation::channel();
        recorder.preparation_started();
        recorder.preparation_succeeded(&plan);
        recorder.materialization_started();
        recorder.materialization_failed();

        assert_cleanup_record(observation.snapshot(), "pending", 0, 0)?;
        recorder.cleanup_started();
        let active = observation.snapshot();
        assert_cleanup_record(active, "pending", 1, 0)?;
        recorder.cleanup_failed();
        assert_cleanup_record(observation.snapshot(), "failed", 1, 1)?;
        recorder.cleanup_started();
        recorder.cleanup_succeeded();
        assert_cleanup_record(observation.snapshot(), "succeeded", 2, 1)?;

        for (outcome, attempts, failures) in [
            (CandleLoadCleanupOutcome::Pending, 1, 1),
            (CandleLoadCleanupOutcome::Succeeded, 3, 1),
            (CandleLoadCleanupOutcome::Failed, 3, 1),
        ] {
            let mut impossible = observation.snapshot();
            impossible.cleanup_outcome = outcome;
            impossible.cleanup_attempts = attempts;
            impossible.cleanup_failures = failures;
            assert!(candle_loader_observation_record(impossible, None).is_err());
        }
        Ok(())
    }

    #[test]
    fn preparation_failure_serializes_without_fabricated_plan_or_receipt() -> Result<(), String> {
        let (observation, recorder) = CandleLoadObservation::channel();
        recorder.preparation_started();
        recorder.verification_only_bytes_read(17);
        recorder.preparation_failed();

        let record = candle_loader_observation_record(observation.snapshot(), None)
            .map_err(|error| error.to_string())?;
        let value = serde_json::to_value(record).map_err(|error| error.to_string())?;
        assert_eq!(
            value_at(&value, "outcome")?.as_str(),
            Some("preparation_failed")
        );
        assert!(value_at(&value, "planned_final_footprint")?.is_null());
        assert!(value_at(&value, "actual_e0_final_ownership")?.is_null());
        Ok(())
    }

    #[test]
    fn invalid_observer_state_is_rejected_as_evidence() -> Result<(), String> {
        let (observation, recorder) = CandleLoadObservation::channel();
        recorder.materialization_started();
        let error = candle_loader_observation_record(observation.snapshot(), None)
            .err()
            .ok_or_else(|| "invalid observation unexpectedly serialized".to_owned())?;
        assert!(error.to_string().contains("invalid transition"));
        Ok(())
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

    fn cpu_fixture_plan() -> LoadPlan {
        let mut plan = fixture_plan();
        plan.accepted_configuration.execution_device =
            ExecutionDevice::new(DeviceId::new(0), DeviceKind::Cpu);
        plan
    }

    fn fixture_receipt(plan: LoadPlan) -> LoadReceipt {
        LoadReceipt {
            handle: plan.accepted_configuration.handle,
            execution_device: plan.accepted_configuration.execution_device,
            execution_scalar_type: plan.execution_scalar_type,
            descriptor: plan.descriptor,
            reserved_footprint: plan.final_footprint,
        }
    }

    fn assert_cleanup_record(
        snapshot: candle_backend::CandleLoadObservationSnapshot,
        expected: &str,
        attempts: u64,
        failures: u64,
    ) -> Result<(), String> {
        assert_eq!(
            snapshot.outcome,
            CandleLoadObservationOutcome::MaterializationFailed
        );
        let value = serde_json::to_value(
            candle_loader_observation_record(snapshot, None).map_err(|error| error.to_string())?,
        )
        .map_err(|error| error.to_string())?;
        assert_eq!(
            value_at(&value, "cleanup_outcome")?.as_str(),
            Some(expected)
        );
        assert_eq!(
            value_at(&value, "cleanup_attempts")?.as_u64(),
            Some(attempts)
        );
        assert_eq!(
            value_at(&value, "cleanup_failures")?.as_u64(),
            Some(failures)
        );
        Ok(())
    }

    fn value_at<'a>(value: &'a Value, key: &str) -> Result<&'a Value, String> {
        value
            .get(key)
            .ok_or_else(|| format!("serialized observation omitted {key}"))
    }

    fn nested_u64(value: &Value, outer: &str, inner: &str) -> Result<u64, String> {
        value_at(value, outer)?
            .get(inner)
            .and_then(Value::as_u64)
            .ok_or_else(|| format!("serialized observation omitted {outer}.{inner}"))
    }
}
