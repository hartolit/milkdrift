//! Independent public Candle planning and accounted-footprint validation.

use std::path::Path;

use application_runtime::{
    ApplicationDevice, ApplicationRuntime, ApplicationScalarType, ResolvedModel,
};
use candle_backend::{CandleLlamaLoader, CandleLlamaSource};
use domain_contracts::{
    BackendId, CapabilitySet, DeviceId, DeviceKind, ExecutionDevice, LoadConfiguration, LoadPlan,
    MemoryBudget, MemoryFootprint, ModelArchitecture, ModelGeneration, ModelHandle, ModelId,
    ModelLoader, QuantizationFormat, ScalarType,
};

use super::super::cli::RequestedDevice;
use super::super::observation::DeviceObserver;
use super::super::report::{AccountedFootprintEvidence, DeviceIdentity};
use crate::error::{BenchmarkError, BenchmarkResult};
use crate::report::MemoryFootprintRecord;

use super::PlannedModelEvidence;
use super::identity::{
    EXPECTED_CONTEXT_TOKENS, EXPECTED_VOCABULARY_SIZE, MODEL_CANDLE_SOURCE_SCALAR,
    MODEL_DOMAIN_SOURCE_SCALAR, MODEL_SOURCE_SCALAR, canonical_snapshot_artifacts,
};
use super::resolution::validate_resolved_state;

const ADAPTER_PLAN_BACKEND: BackendId = BackendId::new(10_003);
const ADAPTER_PLAN_HANDLE: ModelHandle = ModelHandle::new(ModelId::new(1), ModelGeneration::new(1));
const CPU_EXECUTION_DEVICE: ExecutionDevice =
    ExecutionDevice::new(DeviceId::new(0), DeviceKind::Cpu);
const CUDA_ZERO_EXECUTION_DEVICE: ExecutionDevice =
    ExecutionDevice::new(DeviceId::new(0), DeviceKind::Cuda);

const E0_FOOTPRINT_PROVENANCE: &str = "independent public Candle plan_load plus validated E1 ModelLoaded acceptance of the E0 load contract; no same-worker reservation snapshot is exposed by public E1 APIs";

pub(super) fn plan_resolved_model(
    cache_directory: &Path,
    runtime: &ApplicationRuntime,
    requested_device: RequestedDevice,
) -> BenchmarkResult<PlannedModelEvidence> {
    let resolved = runtime
        .state()
        .resolved()
        .ok_or_else(|| BenchmarkError::new("adapter planning requires an E1-resolved model"))?;
    let resolved_source_scalar = validate_resolved_state(runtime, resolved, resolved.selection())?;
    let configuration = planning_configuration(runtime, requested_device)?;
    if !runtime.state().can_load(resolved.selection()) {
        return Err(BenchmarkError::new(
            "public E1 state did not admit the exact resolved model for adapter planning",
        ));
    }

    let artifacts = canonical_snapshot_artifacts(cache_directory)?;
    let source = CandleLlamaSource::new(
        artifacts.config_path,
        vec![artifacts.weight_path],
        MODEL_CANDLE_SOURCE_SCALAR,
    )
    .map_err(|error| {
        BenchmarkError::new(format!(
            "fixed BF16 Candle Llama source could not be constructed: {error}"
        ))
    })?;
    let loader = CandleLlamaLoader::new(ADAPTER_PLAN_BACKEND);
    let plan = loader.plan_load(&source, &configuration).map_err(|error| {
        BenchmarkError::new(format!(
            "public Candle plan_load failed for the exact {} device: {error:?}",
            requested_device_label(requested_device)
        ))
    })?;
    let (source_scalar_type, execution_scalar_type) = validate_exact_adapter_plan(
        &plan,
        resolved,
        resolved_source_scalar,
        requested_device,
        artifacts.weight_bytes,
    )?;

    Ok(PlannedModelEvidence::new(
        plan.expected_footprint,
        source_scalar_type,
        execution_scalar_type,
        requested_device,
        artifacts.weight_bytes,
    ))
}

impl PlannedModelEvidence {
    fn new(
        planned: MemoryFootprint,
        source_scalar_type: ApplicationScalarType,
        execution_scalar_type: ApplicationScalarType,
        requested_device: RequestedDevice,
        source_weight_bytes: u64,
    ) -> Self {
        Self {
            accounted_footprint: AccountedFootprintEvidence {
                independent_public_plan: footprint_record(planned),
                e1_accepted_e0_load_contract: false,
                reservation_snapshot_observed: false,
                provenance: E0_FOOTPRINT_PROVENANCE,
            },
            source_scalar_type,
            execution_scalar_type,
            requested_device,
            source_weight_bytes,
        }
    }

    pub(super) fn record_verified_receipt(&mut self) -> BenchmarkResult {
        if self.accounted_footprint.e1_accepted_e0_load_contract {
            return Err(BenchmarkError::new(
                "E0 load-contract evidence was already populated before ModelLoaded integration",
            ));
        }
        if self.accounted_footprint.reservation_snapshot_observed {
            return Err(BenchmarkError::new(
                "external E1 orchestration cannot claim a direct same-worker E0 reservation snapshot",
            ));
        }
        self.accounted_footprint.e1_accepted_e0_load_contract = true;
        Ok(())
    }
}

pub(super) fn validate_unverified_plan(
    planned: &PlannedModelEvidence,
    observer: &DeviceObserver,
) -> BenchmarkResult {
    if planned.accounted_footprint.e1_accepted_e0_load_contract
        || planned.accounted_footprint.reservation_snapshot_observed
    {
        return Err(BenchmarkError::new(
            "model load requires an independent plan whose E0 load contract is still unverified and whose reservation has not been misrepresented as directly observed",
        ));
    }
    if observer.requested_identity() != requested_device_identity(planned.requested_device) {
        return Err(BenchmarkError::new(
            "adapter plan and device observer address different requested execution devices",
        ));
    }
    if planned.source_scalar_type != MODEL_SOURCE_SCALAR {
        return Err(BenchmarkError::new(format!(
            "independent adapter plan source scalar changed before load: expected {MODEL_SOURCE_SCALAR:?}, recorded {:?}",
            planned.source_scalar_type
        )));
    }
    let expected_execution = expected_execution_scalar(planned.requested_device);
    if planned.execution_scalar_type != expected_execution {
        return Err(BenchmarkError::new(format!(
            "independent adapter plan execution scalar changed before load: expected {expected_execution:?}, recorded {:?}",
            planned.execution_scalar_type
        )));
    }
    validate_bf16_weight_accounting(
        planned.requested_device,
        memory_footprint(planned.accounted_footprint.independent_public_plan),
        planned.source_weight_bytes,
        "retained independent public Candle plan",
    )
}

fn planning_configuration(
    runtime: &ApplicationRuntime,
    requested_device: RequestedDevice,
) -> BenchmarkResult<LoadConfiguration> {
    let expected_application_device = requested_application_device(requested_device);
    let state = runtime.state();
    if runtime.preferences().selected_device != expected_application_device
        || state.selected_device() != expected_application_device
    {
        return Err(BenchmarkError::new(format!(
            "adapter planning requested {expected_application_device:?}, but E1 preferences/state selected {:?}/{:?}",
            runtime.preferences().selected_device,
            state.selected_device()
        )));
    }
    let summary = state.selected_device_summary().ok_or_else(|| {
        BenchmarkError::new(format!(
            "E1 published no selected-device summary for {expected_application_device:?}"
        ))
    })?;
    if summary.device() != expected_application_device || !summary.available() {
        return Err(BenchmarkError::new(format!(
            "E1 selected-device summary was unavailable or addressed a different device: {summary:?}"
        )));
    }

    let host_bytes = runtime.preferences().maximum_host_memory_bytes;
    if host_bytes == 0 {
        return Err(BenchmarkError::new(
            "runtime host-memory limit was zero during adapter planning",
        ));
    }
    // This is a physical-capacity observation used only to bound admission. It is not the
    // adapter's accounted footprint and is never represented as a same-worker reservation.
    let observed_device_capacity_bytes = match requested_device {
        RequestedDevice::Cpu => 0,
        RequestedDevice::Cuda0 => summary
            .total_memory_bytes()
            .filter(|capacity| *capacity > 0)
            .ok_or_else(|| {
                BenchmarkError::new(
                    "E1 CUDA ordinal 0 summary omitted a nonzero total capacity for adapter planning",
                )
            })?,
    };

    Ok(LoadConfiguration {
        handle: ADAPTER_PLAN_HANDLE,
        execution_device: requested_execution_device(requested_device),
        memory_budget: MemoryBudget {
            host_bytes,
            device_bytes: observed_device_capacity_bytes,
        },
    })
}

fn validate_exact_adapter_plan(
    plan: &LoadPlan,
    resolved: &ResolvedModel,
    resolved_source_scalar: ApplicationScalarType,
    requested_device: RequestedDevice,
    source_weight_bytes: u64,
) -> BenchmarkResult<(ApplicationScalarType, ApplicationScalarType)> {
    let descriptor = plan.descriptor;
    let required_operations = CapabilitySet::PREFILL
        .union(CapabilitySet::INCREMENTAL_DECODE)
        .union(CapabilitySet::MULTIPLE_SEQUENCES)
        .union(CapabilitySet::EXPLICIT_SYNCHRONIZATION);
    if descriptor.backend != ADAPTER_PLAN_BACKEND
        || descriptor.metadata.architecture != ModelArchitecture::Llama
        || descriptor.metadata.scalar_type != MODEL_DOMAIN_SOURCE_SCALAR
        || descriptor.metadata.quantization != QuantizationFormat::None
        || descriptor.metadata.vocabulary_size != EXPECTED_VOCABULARY_SIZE
        || descriptor.metadata.vocabulary_size != resolved.vocabulary_size()
        || descriptor.metadata.context_length != EXPECTED_CONTEXT_TOKENS
        || descriptor.capabilities.maximum_context_tokens != EXPECTED_CONTEXT_TOKENS
        || descriptor.capabilities.maximum_prefill_batch != EXPECTED_CONTEXT_TOKENS
        || descriptor.capabilities.maximum_sequences == 0
        || !descriptor
            .capabilities
            .operations
            .contains(required_operations)
    {
        return Err(BenchmarkError::new(format!(
            "public Candle plan_load did not retain the exact unquantized BF16 Llama descriptor and capabilities: {descriptor:?}"
        )));
    }

    let scalar_facts = validate_explicit_plan_scalars(
        resolved_source_scalar,
        descriptor.metadata.scalar_type,
        plan.execution_scalar_type,
        requested_device,
    )?;

    validate_bf16_weight_accounting(
        RequestedDevice::Cpu,
        descriptor.estimated_footprint,
        source_weight_bytes,
        "Candle artifact inspection",
    )?;
    validate_bf16_weight_accounting(
        requested_device,
        plan.expected_footprint,
        source_weight_bytes,
        "public Candle load plan",
    )?;
    Ok(scalar_facts)
}

fn validate_explicit_plan_scalars(
    resolved_source_scalar: ApplicationScalarType,
    plan_source_scalar: ScalarType,
    plan_execution_scalar: ScalarType,
    requested_device: RequestedDevice,
) -> BenchmarkResult<(ApplicationScalarType, ApplicationScalarType)> {
    let source_scalar_type = application_scalar_type(plan_source_scalar, "plan source")?;
    let execution_scalar_type = application_scalar_type(plan_execution_scalar, "plan execution")?;
    if resolved_source_scalar != MODEL_SOURCE_SCALAR
        || source_scalar_type != MODEL_SOURCE_SCALAR
        || source_scalar_type != resolved_source_scalar
    {
        return Err(BenchmarkError::new(format!(
            "independent plan source scalar did not match explicit resolved BF16 evidence: resolved={resolved_source_scalar:?}, plan={source_scalar_type:?}"
        )));
    }
    let expected_execution = expected_execution_scalar(requested_device);
    if execution_scalar_type != expected_execution {
        return Err(BenchmarkError::new(format!(
            "independent plan execution scalar for {} was {execution_scalar_type:?}, expected {expected_execution:?}",
            requested_device_label(requested_device)
        )));
    }
    Ok((source_scalar_type, execution_scalar_type))
}

fn validate_bf16_weight_accounting(
    requested_device: RequestedDevice,
    footprint: MemoryFootprint,
    source_weight_bytes: u64,
    context: &'static str,
) -> BenchmarkResult {
    if source_weight_bytes == 0 {
        return Err(BenchmarkError::new(format!(
            "{context} cannot validate BF16 weight accounting for an empty artifact"
        )));
    }
    match requested_device {
        RequestedDevice::Cpu => {
            let expected_host_weight_bytes =
                source_weight_bytes.checked_mul(2).ok_or_else(|| {
                    BenchmarkError::new(format!(
                        "{context} CPU F32 weight-byte accounting overflowed the BF16 source length"
                    ))
                })?;
            if footprint.host_weight_bytes != expected_host_weight_bytes
                || footprint.device_weight_bytes != 0
            {
                return Err(BenchmarkError::new(format!(
                    "{context} did not account for fixed BF16 source weights as exact F32 host bytes: source={source_weight_bytes}, host={}, device={}",
                    footprint.host_weight_bytes, footprint.device_weight_bytes
                )));
            }
        }
        RequestedDevice::Cuda0 => {
            if footprint.host_weight_bytes != 0
                || footprint.device_weight_bytes != source_weight_bytes
            {
                return Err(BenchmarkError::new(format!(
                    "{context} did not account for fixed BF16 source weights as exact BF16 device bytes: source={source_weight_bytes}, host={}, device={}",
                    footprint.host_weight_bytes, footprint.device_weight_bytes
                )));
            }
        }
    }
    Ok(())
}

const fn expected_execution_scalar(requested: RequestedDevice) -> ApplicationScalarType {
    match requested {
        RequestedDevice::Cpu => ApplicationScalarType::F32,
        RequestedDevice::Cuda0 => ApplicationScalarType::Bf16,
    }
}

fn application_scalar_type(
    scalar: ScalarType,
    context: &'static str,
) -> BenchmarkResult<ApplicationScalarType> {
    match scalar {
        ScalarType::F32 => Ok(ApplicationScalarType::F32),
        ScalarType::F16 => Ok(ApplicationScalarType::F16),
        ScalarType::Bf16 => Ok(ApplicationScalarType::Bf16),
        unsupported => Err(BenchmarkError::new(format!(
            "{context} used unsupported scalar {unsupported:?}"
        ))),
    }
}

const fn requested_application_device(requested: RequestedDevice) -> ApplicationDevice {
    match requested {
        RequestedDevice::Cpu => ApplicationDevice::Cpu,
        RequestedDevice::Cuda0 => ApplicationDevice::Cuda { ordinal: 0 },
    }
}

const fn requested_execution_device(requested: RequestedDevice) -> ExecutionDevice {
    match requested {
        RequestedDevice::Cpu => CPU_EXECUTION_DEVICE,
        RequestedDevice::Cuda0 => CUDA_ZERO_EXECUTION_DEVICE,
    }
}

const fn requested_device_identity(requested: RequestedDevice) -> DeviceIdentity {
    match requested {
        RequestedDevice::Cpu => DeviceIdentity {
            kind: "cpu",
            id: 0,
            ordinal: None,
        },
        RequestedDevice::Cuda0 => DeviceIdentity {
            kind: "cuda",
            id: 0,
            ordinal: Some(0),
        },
    }
}

const fn requested_device_label(requested: RequestedDevice) -> &'static str {
    match requested {
        RequestedDevice::Cpu => "CPU",
        RequestedDevice::Cuda0 => "CUDA ordinal 0",
    }
}

const fn footprint_record(value: MemoryFootprint) -> MemoryFootprintRecord {
    MemoryFootprintRecord {
        host_weight_bytes: value.host_weight_bytes,
        device_weight_bytes: value.device_weight_bytes,
        host_working_bytes: value.host_working_bytes,
        device_working_bytes: value.device_working_bytes,
        cache_bytes_per_token: value.cache_bytes_per_token,
    }
}

const fn memory_footprint(value: MemoryFootprintRecord) -> MemoryFootprint {
    MemoryFootprint {
        host_weight_bytes: value.host_weight_bytes,
        device_weight_bytes: value.device_weight_bytes,
        host_working_bytes: value.host_working_bytes,
        device_working_bytes: value.device_working_bytes,
        cache_bytes_per_token: value.cache_bytes_per_token,
    }
}

#[cfg(test)]
mod tests {
    use application_runtime::ApplicationScalarType;
    use domain_contracts::{MemoryFootprint, ScalarType};

    use super::super::super::cli::RequestedDevice;
    use super::{
        MODEL_DOMAIN_SOURCE_SCALAR, MODEL_SOURCE_SCALAR, PlannedModelEvidence,
        validate_bf16_weight_accounting, validate_explicit_plan_scalars,
    };

    #[test]
    fn explicit_plan_scalars_preserve_bf16_source_with_cpu_f32_execution() -> Result<(), String> {
        let (source, execution) = validate_explicit_plan_scalars(
            MODEL_SOURCE_SCALAR,
            MODEL_DOMAIN_SOURCE_SCALAR,
            ScalarType::F32,
            RequestedDevice::Cpu,
        )
        .map_err(|error| error.to_string())?;
        assert_eq!(source, ApplicationScalarType::Bf16);
        assert_eq!(execution, ApplicationScalarType::F32);
        Ok(())
    }

    #[test]
    fn explicit_plan_scalars_preserve_bf16_source_with_cuda_bf16_execution() -> Result<(), String> {
        let (source, execution) = validate_explicit_plan_scalars(
            MODEL_SOURCE_SCALAR,
            MODEL_DOMAIN_SOURCE_SCALAR,
            ScalarType::Bf16,
            RequestedDevice::Cuda0,
        )
        .map_err(|error| error.to_string())?;
        assert_eq!(source, ApplicationScalarType::Bf16);
        assert_eq!(execution, ApplicationScalarType::Bf16);
        Ok(())
    }

    #[test]
    fn explicit_plan_scalars_reject_source_or_execution_substitution() {
        assert!(
            validate_explicit_plan_scalars(
                MODEL_SOURCE_SCALAR,
                ScalarType::F32,
                ScalarType::F32,
                RequestedDevice::Cpu,
            )
            .is_err()
        );
        assert!(
            validate_explicit_plan_scalars(
                MODEL_SOURCE_SCALAR,
                MODEL_DOMAIN_SOURCE_SCALAR,
                ScalarType::Bf16,
                RequestedDevice::Cpu,
            )
            .is_err()
        );
        assert!(
            validate_explicit_plan_scalars(
                MODEL_SOURCE_SCALAR,
                MODEL_DOMAIN_SOURCE_SCALAR,
                ScalarType::F32,
                RequestedDevice::Cuda0,
            )
            .is_err()
        );
    }

    #[test]
    fn bf16_accounting_requires_exact_cpu_and_cuda_weight_scaling() -> Result<(), String> {
        let cpu = MemoryFootprint {
            host_weight_bytes: 200,
            ..MemoryFootprint::default()
        };
        validate_bf16_weight_accounting(RequestedDevice::Cpu, cpu, 100, "test CPU accounting")
            .map_err(|error| error.to_string())?;

        let cuda = MemoryFootprint {
            device_weight_bytes: 100,
            ..MemoryFootprint::default()
        };
        validate_bf16_weight_accounting(RequestedDevice::Cuda0, cuda, 100, "test CUDA accounting")
            .map_err(|error| error.to_string())?;

        let unexpanded_cpu = MemoryFootprint {
            host_weight_bytes: 100,
            ..MemoryFootprint::default()
        };
        assert!(
            validate_bf16_weight_accounting(
                RequestedDevice::Cpu,
                unexpanded_cpu,
                100,
                "test CPU accounting",
            )
            .is_err()
        );
        let expanded_cuda = MemoryFootprint {
            device_weight_bytes: 200,
            ..MemoryFootprint::default()
        };
        assert!(
            validate_bf16_weight_accounting(
                RequestedDevice::Cuda0,
                expanded_cuda,
                100,
                "test CUDA accounting",
            )
            .is_err()
        );
        assert!(
            validate_bf16_weight_accounting(RequestedDevice::Cpu, cpu, u64::MAX, "test overflow",)
                .is_err()
        );
        Ok(())
    }

    #[test]
    fn receipt_verification_never_accepts_a_same_worker_snapshot_claim() {
        let mut planned = PlannedModelEvidence::new(
            MemoryFootprint::default(),
            ApplicationScalarType::Bf16,
            ApplicationScalarType::F32,
            RequestedDevice::Cpu,
            1,
        );
        planned.accounted_footprint.reservation_snapshot_observed = true;
        assert!(planned.record_verified_receipt().is_err());
        assert!(!planned.accounted_footprint.e1_accepted_e0_load_contract);
    }
}
