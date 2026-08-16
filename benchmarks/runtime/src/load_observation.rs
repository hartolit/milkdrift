//! Stable report conversion for the benchmark-only Candle load observer.

use std::time::Duration;

use candle_backend::{
    CandleLoadCleanupOutcome, CandleLoadObservationOutcome, CandleLoadObservationSnapshot,
};
use domain_contracts::DeviceKind;
use inference_runtime::LoadReceipt;
use serde::Serialize;

use crate::error::{BenchmarkError, BenchmarkResult};
use crate::evidence::validate_load_receipt;
use crate::report::checked_duration_ns;

/// Serializable correctness and timing facts from one actual hosted load.
///
/// This record is intentionally separate from sampled process RSS and device
/// telemetry. The adjacent prepared-plan and E0-receipt records own deterministic
/// planned and accepted ownership, so this stage record does not duplicate them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub(crate) struct CandleLoaderObservationRecord {
    preparation_duration_ns: u64,
    materialization_duration_ns: u64,
    required_bytes_read: u64,
    whole_file_verification_bytes_read: u64,
    transfer_batches: u64,
    loading_device_synchronizations: u64,
}

/// Validates and converts one fixed-size loader snapshot into report evidence.
///
/// The normal writer reaches this conversion only after successful hosted loading and therefore
/// requires the matching public E0 receipt. Failure and retained-cleanup observations abort the
/// synthetic run instead of becoming a performance sample.
///
/// # Errors
///
/// Returns an error when phase timing, byte accounting, plan/receipt identity,
/// cleanup state, or observer transitions are incomplete or inconsistent.
pub(crate) fn candle_loader_observation_record(
    snapshot: CandleLoadObservationSnapshot,
    receipt: &LoadReceipt,
) -> BenchmarkResult<CandleLoaderObservationRecord> {
    validate_common(snapshot)?;
    if snapshot.outcome != CandleLoadObservationOutcome::Succeeded
        || snapshot.cleanup_outcome != CandleLoadCleanupOutcome::NotRequired
        || snapshot.cleanup_attempts != 0
        || snapshot.cleanup_failures != 0
    {
        return Err(BenchmarkError::new(
            "synthetic loader evidence requires successful loading without retained cleanup",
        ));
    }
    let plan = snapshot.plan.as_ref().ok_or_else(|| {
        BenchmarkError::new("successful Candle load observation omitted its plan")
    })?;
    validate_load_receipt(plan, receipt)?;
    validate_successful_loading_counts(snapshot)?;

    Ok(CandleLoaderObservationRecord {
        preparation_duration_ns: required_duration_ns(
            snapshot.preparation_duration,
            "Candle preparation",
        )?,
        materialization_duration_ns: required_duration_ns(
            snapshot.materialization_duration,
            "Candle materialization",
        )?,
        required_bytes_read: snapshot.required_bytes_read.as_u64(),
        whole_file_verification_bytes_read: snapshot.whole_file_verification_bytes_read.as_u64(),
        transfer_batches: snapshot.transfer_batches,
        loading_device_synchronizations: snapshot.loading_device_synchronizations,
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
    Ok(())
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

fn required_duration_ns(duration: Option<Duration>, label: &'static str) -> BenchmarkResult<u64> {
    duration
        .ok_or_else(|| BenchmarkError::new(format!("{label} duration was not observed")))
        .and_then(|duration| checked_duration_ns(duration, label))
}

#[cfg(test)]
mod tests {
    use super::candle_loader_observation_record;
    use candle_backend::{
        CandleLoadCleanupOutcome, CandleLoadObservationOutcome, CandleLoadObservationSnapshot,
    };
    use domain_contracts::{
        BackendId, ByteCount, CapabilitySet, DeviceId, DeviceKind, ExecutionDevice,
        LoadConfiguration, LoadPlan, MemoryBudget, MemoryFootprint, ModelArchitecture,
        ModelCapabilities, ModelDescriptor, ModelGeneration, ModelHandle, ModelId, ModelMetadata,
        QuantizationFormat, ScalarType, ScalarTypeSet,
    };
    use inference_runtime::LoadReceipt;

    #[test]
    fn successful_record_rejects_impossible_device_batch_and_sync_counts() -> Result<(), String> {
        let cuda_plan = fixture_plan();
        let cuda_receipt = fixture_receipt(cuda_plan);
        let cuda_error =
            candle_loader_observation_record(successful_snapshot(cuda_plan, 2, 3), &cuda_receipt)
                .err()
                .ok_or_else(|| {
                    "mismatched CUDA batch/sync counts unexpectedly serialized".to_owned()
                })?;
        assert!(cuda_error.to_string().contains("impossible transfer-batch"));

        let cpu_plan = cpu_fixture_plan();
        let cpu_receipt = fixture_receipt(cpu_plan);
        let cpu_error =
            candle_loader_observation_record(successful_snapshot(cpu_plan, 1, 1), &cpu_receipt)
                .err()
                .ok_or_else(|| {
                    "nonzero CPU batch/sync counts unexpectedly serialized".to_owned()
                })?;
        assert!(cpu_error.to_string().contains("impossible transfer-batch"));
        Ok(())
    }

    fn successful_snapshot(
        plan: LoadPlan,
        transfer_batches: u64,
        loading_device_synchronizations: u64,
    ) -> CandleLoadObservationSnapshot {
        CandleLoadObservationSnapshot {
            preparation_duration: Some(std::time::Duration::from_nanos(1)),
            materialization_duration: Some(std::time::Duration::from_nanos(1)),
            transfer_batches,
            loading_device_synchronizations,
            plan: Some(plan),
            outcome: CandleLoadObservationOutcome::Succeeded,
            cleanup_outcome: CandleLoadCleanupOutcome::NotRequired,
            ..CandleLoadObservationSnapshot::default()
        }
    }

    fn fixture_plan() -> LoadPlan {
        let execution_device = ExecutionDevice::new(DeviceId::new(0), DeviceKind::Cuda);
        let final_footprint = MemoryFootprint::device_weights(ByteCount::from_u64(4_096));
        LoadPlan {
            accepted_configuration: LoadConfiguration {
                handle: ModelHandle::new(ModelId::new(7), ModelGeneration::new(1)),
                execution_device,
                memory_budget: MemoryBudget::UNLIMITED,
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
            },
            execution_scalar_type: ScalarType::F32,
            final_footprint,
            loading_peak_footprint: final_footprint
                .with_host_working_bytes(ByteCount::from_u64(1_024)),
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
}
