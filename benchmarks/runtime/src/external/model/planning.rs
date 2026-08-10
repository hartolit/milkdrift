//! Observer-owned Candle preparation and exact public load-plan validation.

use std::path::Path;

use application_runtime::{
    ApplicationDevice, ApplicationRuntime, ApplicationScalarType, ResolvedModel,
};
use candle_backend::{CandleLlamaLoader, CandleLlamaSource};
use domain_contracts::{
    BackendId, CapabilitySet, DeviceId, DeviceKind, ExecutionDevice, LoadConfiguration, LoadPlan,
    MemoryBudget, MemoryFootprint, ModelArchitecture, ModelGeneration, ModelHandle, ModelId,
    ModelLoader, PreparedLoad, QuantizationFormat, ScalarType, ScalarTypeSet,
};

use super::super::cli::RequestedDevice;
use super::super::observation::DeviceObserver;
use super::super::report::{DeviceIdentity, PreparedLoadEvidence};
use crate::error::{BenchmarkError, BenchmarkResult};
use crate::evidence::{application_scalar_type, footprint_record, validate_prepared_load_plan};
use crate::report::MemoryFootprintRecord;

use super::PlannedModelEvidence;
use super::identity::{
    EXPECTED_CONTEXT_TOKENS, EXPECTED_VOCABULARY_SIZE, MODEL_CONFIGURATION_DECLARED_SCALAR,
    MODEL_OBSERVED_TENSOR_SCALARS, canonical_snapshot_artifacts,
};
use super::resolution::validate_resolved_state;

const ADAPTER_PREPARATION_BACKEND: BackendId = BackendId::new(10_003);
const ADAPTER_PREPARATION_HANDLE: ModelHandle =
    ModelHandle::new(ModelId::new(1), ModelGeneration::new(1));
const CPU_EXECUTION_DEVICE: ExecutionDevice =
    ExecutionDevice::new(DeviceId::new(0), DeviceKind::Cpu);
const CUDA_ZERO_EXECUTION_DEVICE: ExecutionDevice =
    ExecutionDevice::new(DeviceId::new(0), DeviceKind::Cuda);

const PREPARED_LOAD_PROVENANCE: &str = "observer-owned unmaterialized Candle prepare_load plan plus public E1 ModelLoaded acceptance; E1 exposes actual execution scalar/device but no same-worker E0 reserved-ownership snapshot";

pub(super) fn plan_resolved_model(
    cache_directory: &Path,
    runtime: &ApplicationRuntime,
    requested_device: RequestedDevice,
) -> BenchmarkResult<PlannedModelEvidence> {
    let resolved = runtime
        .state()
        .resolved()
        .ok_or_else(|| BenchmarkError::new("adapter preparation requires an E1-resolved model"))?;
    let resolved_declaration = validate_resolved_state(runtime, resolved, resolved.selection())?;
    let configuration = planning_configuration(runtime, requested_device)?;
    if !runtime.state().can_load(resolved.selection()) {
        return Err(BenchmarkError::new(
            "public E1 state did not admit the exact resolved model for observer preparation",
        ));
    }

    let artifacts = canonical_snapshot_artifacts(cache_directory)?;
    let source = CandleLlamaSource::from_local_files(
        artifacts.config_path,
        vec![artifacts.weight_path],
    )
    .map_err(|error| {
        BenchmarkError::new(format!(
            "fixed TinyLlama Candle source could not be constructed for observer preparation: {error}"
        ))
    })?;
    let mut loader = CandleLlamaLoader::new(ADAPTER_PREPARATION_BACKEND);
    let prepared = loader
        .prepare_load(&source, &configuration)
        .map_err(|error| {
            BenchmarkError::new(format!(
                "public Candle prepare_load failed for the exact {} device: {error:?}",
                requested_device_label(requested_device)
            ))
        })?;
    let plan = *prepared.plan();
    drop(prepared);

    let scalar_facts = validate_exact_adapter_plan(
        &plan,
        &configuration,
        resolved,
        resolved_declaration,
        requested_device,
    )?;
    let planned = PlannedModelEvidence {
        prepared_load: PreparedLoadEvidence {
            planned_execution_device: requested_device_identity(requested_device),
            exact_final_footprint: footprint_record(plan.final_footprint),
            loading_peak_footprint: footprint_record(plan.loading_peak_footprint),
            e1_load_accepted: false,
            e0_reserved_ownership_observed: false,
            provenance: PREPARED_LOAD_PROVENANCE,
        },
        configuration_declared_scalar_type: scalar_facts.configuration_declared_scalar_type,
        observed_tensor_scalar_types: scalar_facts.observed_tensor_scalar_types,
        planned_execution_scalar_type: scalar_facts.planned_execution_scalar_type,
        requested_device,
    };
    validate_plan_state(&planned, false)?;
    Ok(planned)
}

impl PlannedModelEvidence {
    pub(super) fn record_e1_load_acceptance(&mut self) -> BenchmarkResult {
        if self.prepared_load.e1_load_accepted {
            return Err(BenchmarkError::new(
                "E1 load acceptance was already recorded for this observer preparation",
            ));
        }
        if self.prepared_load.e0_reserved_ownership_observed {
            return Err(BenchmarkError::new(
                "external E1 orchestration cannot claim direct same-worker E0 reserved ownership",
            ));
        }
        self.prepared_load.e1_load_accepted = true;
        validate_plan_state(self, true)
    }
}

pub(super) fn validate_unverified_plan(
    planned: &PlannedModelEvidence,
    observer: &DeviceObserver,
) -> BenchmarkResult {
    validate_observer_identity(planned, observer)?;
    validate_plan_state(planned, false)
}

pub(super) fn validate_verified_plan(
    planned: &PlannedModelEvidence,
    observer: &DeviceObserver,
) -> BenchmarkResult {
    validate_observer_identity(planned, observer)?;
    validate_plan_state(planned, true)
}

fn validate_observer_identity(
    planned: &PlannedModelEvidence,
    observer: &DeviceObserver,
) -> BenchmarkResult {
    if observer.requested_identity() != requested_device_identity(planned.requested_device) {
        return Err(BenchmarkError::new(
            "observer preparation and device observer address different requested execution devices",
        ));
    }
    Ok(())
}

fn validate_plan_state(
    planned: &PlannedModelEvidence,
    expected_e1_acceptance: bool,
) -> BenchmarkResult {
    let expected_device = requested_device_identity(planned.requested_device);
    if planned.prepared_load.e1_load_accepted != expected_e1_acceptance
        || planned.prepared_load.e0_reserved_ownership_observed
        || planned.prepared_load.planned_execution_device != expected_device
        || planned.configuration_declared_scalar_type != MODEL_CONFIGURATION_DECLARED_SCALAR
        || planned.observed_tensor_scalar_types != MODEL_OBSERVED_TENSOR_SCALARS
        || planned.planned_execution_scalar_type
            != expected_execution_scalar(planned.requested_device)
        || planned.prepared_load.provenance.is_empty()
    {
        return Err(BenchmarkError::new(
            "retained observer preparation changed its declared, observed, planned-execution, acceptance, device, or ownership-scope facts",
        ));
    }
    validate_recorded_footprints(
        planned.requested_device,
        planned.prepared_load.exact_final_footprint,
        planned.prepared_load.loading_peak_footprint,
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
            "observer preparation requested {expected_application_device:?}, but E1 preferences/state selected {:?}/{:?}",
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
            "runtime host-memory limit was zero during observer preparation",
        ));
    }
    let observed_device_capacity_bytes = match requested_device {
        RequestedDevice::Cpu => 0,
        RequestedDevice::Cuda0 => summary
            .total_memory_bytes()
            .filter(|capacity| *capacity > 0)
            .ok_or_else(|| {
                BenchmarkError::new(
                    "E1 CUDA ordinal 0 summary omitted a nonzero total capacity for observer preparation",
                )
            })?,
    };

    Ok(LoadConfiguration {
        handle: ADAPTER_PREPARATION_HANDLE,
        execution_device: requested_execution_device(requested_device),
        memory_budget: MemoryBudget {
            host_bytes,
            device_bytes: observed_device_capacity_bytes,
        },
    })
}

struct ValidatedScalarFacts {
    configuration_declared_scalar_type: Option<ApplicationScalarType>,
    observed_tensor_scalar_types: ScalarTypeSet,
    planned_execution_scalar_type: ApplicationScalarType,
}

fn validate_exact_adapter_plan(
    plan: &LoadPlan,
    configuration: &LoadConfiguration,
    resolved: &ResolvedModel,
    resolved_declaration: Option<ApplicationScalarType>,
    requested_device: RequestedDevice,
) -> BenchmarkResult<ValidatedScalarFacts> {
    validate_prepared_load_plan(plan)?;
    let descriptor = plan.descriptor;
    let required_operations = CapabilitySet::PREFILL
        .union(CapabilitySet::INCREMENTAL_DECODE)
        .union(CapabilitySet::MULTIPLE_SEQUENCES)
        .union(CapabilitySet::EXPLICIT_SYNCHRONIZATION);
    if plan.accepted_configuration != *configuration
        || descriptor.backend != ADAPTER_PREPARATION_BACKEND
        || descriptor.metadata.architecture != ModelArchitecture::Llama
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
            "public Candle prepare_load did not retain the exact unquantized TinyLlama descriptor, configuration, and capabilities: {plan:?}"
        )));
    }

    let scalar_facts = validate_scalar_facts(
        resolved_declaration,
        descriptor.metadata.configuration_declared_scalar_type,
        descriptor.metadata.observed_tensor_scalar_types,
        plan.execution_scalar_type,
        requested_device,
    )?;
    validate_inspection_footprint(
        descriptor.estimated_footprint,
        descriptor.sequence_cache_bytes_per_token,
    )?;
    if requested_device == RequestedDevice::Cpu
        && descriptor.estimated_footprint != plan.final_footprint
    {
        return Err(BenchmarkError::new(
            "CPU prepare_load exact final footprint differed from the same source's exact CPU inspection footprint",
        ));
    }
    Ok(scalar_facts)
}

fn validate_scalar_facts(
    resolved_declaration: Option<ApplicationScalarType>,
    plan_declaration: Option<ScalarType>,
    observed_tensor_scalar_types: ScalarTypeSet,
    plan_execution_scalar: ScalarType,
    requested_device: RequestedDevice,
) -> BenchmarkResult<ValidatedScalarFacts> {
    let plan_declaration = plan_declaration
        .map(|scalar| application_scalar_type(scalar, "prepared configuration declaration"))
        .transpose()?;
    let planned_execution_scalar_type =
        application_scalar_type(plan_execution_scalar, "prepared execution scalar")?;
    if resolved_declaration != MODEL_CONFIGURATION_DECLARED_SCALAR
        || plan_declaration != MODEL_CONFIGURATION_DECLARED_SCALAR
        || plan_declaration != resolved_declaration
        || observed_tensor_scalar_types != MODEL_OBSERVED_TENSOR_SCALARS
        || planned_execution_scalar_type != expected_execution_scalar(requested_device)
    {
        return Err(BenchmarkError::new(format!(
            "fixed TinyLlama preparation did not preserve optional declared BF16, homogeneous observed {{BF16}}, and expected {} execution: resolved={resolved_declaration:?}, prepared_declaration={plan_declaration:?}, observed={observed_tensor_scalar_types:?}, execution={planned_execution_scalar_type:?}",
            requested_device_label(requested_device)
        )));
    }
    Ok(ValidatedScalarFacts {
        configuration_declared_scalar_type: plan_declaration,
        observed_tensor_scalar_types,
        planned_execution_scalar_type,
    })
}

fn validate_inspection_footprint(
    footprint: MemoryFootprint,
    sequence_cache_bytes_per_token: u64,
) -> BenchmarkResult {
    if footprint.checked_host_bytes().is_none()
        || footprint.checked_device_bytes().is_none()
        || footprint.host_weight_bytes == 0
        || footprint.device_weight_bytes != 0
        || footprint.device_working_bytes != 0
        || sequence_cache_bytes_per_token == 0
    {
        return Err(BenchmarkError::new(
            "device-independent inspection did not expose a nonzero exact CPU tensor footprint and separate sequence-cache planning rate",
        ));
    }
    Ok(())
}

fn validate_recorded_footprints(
    requested_device: RequestedDevice,
    exact_final: MemoryFootprintRecord,
    loading_peak: MemoryFootprintRecord,
) -> BenchmarkResult {
    let final_footprint = memory_footprint(exact_final);
    let loading_footprint = memory_footprint(loading_peak);
    let contains_final = loading_footprint.contains_components(final_footprint);
    let placement_matches = match requested_device {
        RequestedDevice::Cpu => {
            final_footprint.host_weight_bytes > 0
                && final_footprint.device_weight_bytes == 0
                && final_footprint.device_working_bytes == 0
        }
        RequestedDevice::Cuda0 => {
            final_footprint.host_weight_bytes == 0 && final_footprint.device_weight_bytes > 0
        }
    };
    if final_footprint.checked_host_bytes().is_none()
        || final_footprint.checked_device_bytes().is_none()
        || loading_footprint.checked_host_bytes().is_none()
        || loading_footprint.checked_device_bytes().is_none()
        || !contains_final
        || !placement_matches
    {
        return Err(BenchmarkError::new(
            "serialized prepare_load final and loading-peak footprints lost exact plan coherence",
        ));
    }
    Ok(())
}

const fn expected_execution_scalar(requested: RequestedDevice) -> ApplicationScalarType {
    match requested {
        RequestedDevice::Cpu => ApplicationScalarType::F32,
        RequestedDevice::Cuda0 => ApplicationScalarType::Bf16,
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
        RequestedDevice::Cpu => "CPU/F32",
        RequestedDevice::Cuda0 => "CUDA ordinal 0/BF16",
    }
}

const fn memory_footprint(value: MemoryFootprintRecord) -> MemoryFootprint {
    MemoryFootprint {
        host_weight_bytes: value.host_weight_bytes,
        device_weight_bytes: value.device_weight_bytes,
        host_working_bytes: value.host_working_bytes,
        device_working_bytes: value.device_working_bytes,
    }
}

#[cfg(test)]
mod tests {
    use application_runtime::ApplicationScalarType;
    use domain_contracts::{ScalarType, ScalarTypeSet};

    use super::super::super::cli::RequestedDevice;
    use super::{MODEL_OBSERVED_TENSOR_SCALARS, validate_scalar_facts};

    #[test]
    fn fixed_profile_preserves_optional_declaration_observation_and_cpu_execution()
    -> Result<(), String> {
        let facts = validate_scalar_facts(
            Some(ApplicationScalarType::Bf16),
            Some(ScalarType::Bf16),
            MODEL_OBSERVED_TENSOR_SCALARS,
            ScalarType::F32,
            RequestedDevice::Cpu,
        )
        .map_err(|error| error.to_string())?;
        assert_eq!(
            facts.configuration_declared_scalar_type,
            Some(ApplicationScalarType::Bf16)
        );
        assert_eq!(
            facts.observed_tensor_scalar_types,
            ScalarTypeSet::from_scalar(ScalarType::Bf16)
        );
        assert_eq!(
            facts.planned_execution_scalar_type,
            ApplicationScalarType::F32
        );
        Ok(())
    }

    #[test]
    fn fixed_profile_preserves_optional_declaration_observation_and_cuda_execution()
    -> Result<(), String> {
        let facts = validate_scalar_facts(
            Some(ApplicationScalarType::Bf16),
            Some(ScalarType::Bf16),
            MODEL_OBSERVED_TENSOR_SCALARS,
            ScalarType::Bf16,
            RequestedDevice::Cuda0,
        )
        .map_err(|error| error.to_string())?;
        assert_eq!(
            facts.planned_execution_scalar_type,
            ApplicationScalarType::Bf16
        );
        Ok(())
    }

    #[test]
    fn homogeneous_tinyllama_profile_is_not_mixed_checkpoint_evidence() {
        let mixed =
            MODEL_OBSERVED_TENSOR_SCALARS.union(ScalarTypeSet::from_scalar(ScalarType::F32));
        assert!(
            validate_scalar_facts(
                Some(ApplicationScalarType::Bf16),
                Some(ScalarType::Bf16),
                mixed,
                ScalarType::F32,
                RequestedDevice::Cpu,
            )
            .is_err()
        );
    }
}
