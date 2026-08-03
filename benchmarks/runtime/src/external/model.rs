//! Exact model resolution, adapter planning, loading, and unloading for the external baseline.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use application_runtime::{
    ApplicationActivity, ApplicationDevice, ApplicationEngine, ApplicationEvent,
    ApplicationModelFormat, ApplicationRuntime, ApplicationScalarType, ApplicationSource,
    ChatCompatibility, LoadedModel, ModelSelection, ModelUnloadBehavior,
    PromptCompatibilityProfile, ResolvedModel,
};
use candle_backend::{CandleLlamaLoader, CandleLlamaSource, CandleScalarType};
use domain_contracts::{
    BackendId, CapabilitySet, DeviceId, DeviceKind, ExecutionDevice, LoadConfiguration, LoadPlan,
    MemoryBudget, MemoryFootprint, ModelArchitecture, ModelGeneration, ModelHandle, ModelId,
    ModelLoader, QuantizationFormat, ScalarType,
};

use super::cli::RequestedDevice;
use super::observation::DeviceObserver;
use super::report::{E0FootprintEvidence, UnloadResult};
use crate::error::{BenchmarkError, BenchmarkResult};
use crate::report::{MemoryFootprintRecord, duration_ns};

pub(super) const MODEL_REPOSITORY: &str = "TinyLlama/TinyLlama-1.1B-Chat-v1.0";
pub(super) const MODEL_REVISION: &str = "fe8a4ea1ffedaf415f4da2f062534de366a451e6";
pub(super) const MODEL_ARCHITECTURE: &str = "Llama";

const EXPECTED_VOCABULARY_SIZE: u32 = 32_000;
const EXPECTED_CONTEXT_TOKENS: u32 = 2_048;
const CACHE_REPOSITORY_DIRECTORY: &str = "models--TinyLlama--TinyLlama-1.1B-Chat-v1.0";
const CONFIG_FILE: &str = "config.json";
const WEIGHT_FILE: &str = "model.safetensors";
const ADAPTER_PLAN_BACKEND: BackendId = BackendId::new(10_003);
const ADAPTER_PLAN_HANDLE: ModelHandle = ModelHandle::new(ModelId::new(1), ModelGeneration::new(1));
const CPU_EXECUTION_DEVICE: ExecutionDevice =
    ExecutionDevice::new(DeviceId::new(0), DeviceKind::Cpu);
const CUDA_ZERO_EXECUTION_DEVICE: ExecutionDevice =
    ExecutionDevice::new(DeviceId::new(0), DeviceKind::Cuda);

const POLL_INTERVAL: Duration = Duration::from_millis(10);
const RESOLUTION_TIMEOUT: Duration = Duration::from_mins(30);
const LOAD_TIMEOUT: Duration = Duration::from_mins(10);
const UNLOAD_TIMEOUT: Duration = Duration::from_mins(2);

const E0_FOOTPRINT_PROVENANCE: &str = "independent public Candle plan_load plus validated E1 ModelLoaded acceptance of the E0 load contract; no same-worker reservation snapshot is exposed by public E1 APIs";

/// Public-adapter plan evidence retained until E1 verifies a successful E0 load receipt.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct PlannedModelEvidence {
    pub(super) e0_footprint: E0FootprintEvidence,
    pub(super) execution_dtype: &'static str,
    requested_device: RequestedDevice,
    source_weight_bytes: u64,
}

impl PlannedModelEvidence {
    fn new(
        planned: MemoryFootprintRecord,
        execution_dtype: &'static str,
        requested_device: RequestedDevice,
        source_weight_bytes: u64,
    ) -> Self {
        Self {
            e0_footprint: E0FootprintEvidence {
                independent_public_plan: planned,
                e1_accepted_e0_load_contract: false,
                reservation_snapshot_observed: false,
                provenance: E0_FOOTPRINT_PROVENANCE,
            },
            execution_dtype,
            requested_device,
            source_weight_bytes,
        }
    }

    fn record_verified_receipt(&mut self) -> BenchmarkResult {
        if self.e0_footprint.e1_accepted_e0_load_contract {
            return Err(BenchmarkError::new(
                "E0 load-contract evidence was already populated before ModelLoaded integration",
            ));
        }
        if self.e0_footprint.reservation_snapshot_observed {
            return Err(BenchmarkError::new(
                "external E1 orchestration cannot claim a direct same-worker E0 reservation snapshot",
            ));
        }
        self.e0_footprint.e1_accepted_e0_load_contract = true;
        Ok(())
    }
}

struct SnapshotArtifacts {
    config_path: PathBuf,
    weight_path: PathBuf,
    weight_bytes: u64,
}

/// Resolves and validates the built-in immutable `TinyLlama` selection through public E1 events.
pub(super) fn resolve_model(
    runtime: &mut ApplicationRuntime,
    selection: &ModelSelection,
) -> BenchmarkResult<(ResolvedModel, Duration)> {
    validate_exact_selection(selection)?;
    validate_resolution_ready(runtime, selection)?;

    let started_at = Instant::now();
    runtime.resolve_model(selection.clone()).map_err(|error| {
        BenchmarkError::new(format!(
            "exact Hub resolution could not be submitted for {MODEL_REPOSITORY}@{MODEL_REVISION}: {error}"
        ))
    })?;
    let deadline = checked_deadline(RESOLUTION_TIMEOUT, "immutable Hub resolution")?;

    loop {
        if let Some(event) = runtime.poll_event() {
            match event {
                ApplicationEvent::ModelResolved {
                    model,
                    persistence_warning: None,
                } => {
                    validate_resolved_state(runtime, &model, selection)?;
                    return Ok((model, started_at.elapsed()));
                }
                ApplicationEvent::ModelResolved {
                    persistence_warning: Some(warning),
                    ..
                } => {
                    return Err(BenchmarkError::new(format!(
                        "Hub resolution succeeded but immutable catalogue persistence reported a warning: {warning}"
                    )));
                }
                ApplicationEvent::ModelResolutionFailed { failure } => {
                    return Err(BenchmarkError::new(format!(
                        "exact Hub resolution failed for {MODEL_REPOSITORY}@{MODEL_REVISION}: {failure}"
                    )));
                }
                ApplicationEvent::HubDisconnected => {
                    return Err(BenchmarkError::new(
                        "Hub worker disconnected during exact immutable resolution",
                    ));
                }
                ApplicationEvent::RuntimeDisconnected => {
                    return Err(BenchmarkError::new(
                        "inference worker disconnected during exact immutable resolution",
                    ));
                }
                unexpected => {
                    return Err(BenchmarkError::new(format!(
                        "unexpected application event during immutable resolution: {unexpected:?}"
                    )));
                }
            }
        }
        wait_for_next_poll(deadline, "immutable Hub resolution")?;
    }
}

/// Produces exact E0 adapter-plan evidence without loading a second model or polling E0 events.
pub(super) fn plan_resolved_model(
    cache_directory: &Path,
    runtime: &ApplicationRuntime,
    requested_device: RequestedDevice,
) -> BenchmarkResult<PlannedModelEvidence> {
    let resolved = runtime
        .state()
        .resolved()
        .ok_or_else(|| BenchmarkError::new("adapter planning requires an E1-resolved model"))?;
    validate_resolved_state(runtime, resolved, resolved.selection())?;
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
        CandleScalarType::Bf16,
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
    validate_exact_adapter_plan(&plan, resolved, artifacts.weight_bytes)?;

    let execution_dtype = infer_bf16_execution_dtype(
        requested_device,
        plan.expected_footprint,
        artifacts.weight_bytes,
    )?;
    Ok(PlannedModelEvidence::new(
        footprint_record(plan.expected_footprint),
        execution_dtype,
        requested_device,
        artifacts.weight_bytes,
    ))
}

/// Loads once through E1 and marks the separate public adapter plan as receipt-verified only after
/// the exact `ModelLoaded` event and state have been validated.
pub(super) fn load_model(
    runtime: &mut ApplicationRuntime,
    selection: &ModelSelection,
    planned: &mut PlannedModelEvidence,
    observer: &DeviceObserver,
) -> BenchmarkResult<(LoadedModel, Duration)> {
    validate_exact_selection(selection)?;
    let resolved = runtime
        .state()
        .resolved()
        .cloned()
        .ok_or_else(|| BenchmarkError::new("model load requires an E1-resolved model"))?;
    validate_resolved_state(runtime, &resolved, selection)?;
    validate_unverified_plan(planned, observer)?;
    observer.validate_selected_e1(runtime)?;
    if !runtime.state().can_load(selection) {
        return Err(BenchmarkError::new(
            "public E1 state did not admit the exact resolved model for loading",
        ));
    }

    let started_at = Instant::now();
    runtime.load_model(selection).map_err(|error| {
        BenchmarkError::new(format!(
            "exact Candle model load could not be submitted: {error}"
        ))
    })?;
    let deadline = checked_deadline(LOAD_TIMEOUT, "Candle model load")?;

    loop {
        if let Some(event) = runtime.poll_event() {
            match event {
                ApplicationEvent::ModelLoaded { model } => {
                    validate_loaded_state(
                        runtime, &model, &resolved, selection, planned, observer,
                    )?;
                    planned.record_verified_receipt()?;
                    return Ok((model, started_at.elapsed()));
                }
                ApplicationEvent::ModelLoadFailed { failure } => {
                    return Err(BenchmarkError::new(format!(
                        "exact Candle model load failed: {failure}"
                    )));
                }
                ApplicationEvent::ModelCompatibilityFailed { failure } => {
                    return Err(BenchmarkError::new(format!(
                        "resolved and loaded model compatibility failed: {failure}"
                    )));
                }
                ApplicationEvent::HubDisconnected => {
                    return Err(BenchmarkError::new(
                        "Hub worker disconnected while the exact model was loading",
                    ));
                }
                ApplicationEvent::RuntimeDisconnected => {
                    return Err(BenchmarkError::new(
                        "inference worker disconnected while the exact model was loading",
                    ));
                }
                unexpected => {
                    return Err(BenchmarkError::new(format!(
                        "unexpected application event during exact model load: {unexpected:?}"
                    )));
                }
            }
        }
        wait_for_next_poll(deadline, "Candle model load")?;
    }
}

/// Unloads the released model with `RejectIfBusy` and validates the terminal public E1 contract.
///
/// For this first unload of the still-published handle, E0's successful path synchronizes the
/// Candle backend and removes its registry/accounting entry before E1 publishes `ModelUnloaded`.
/// Report schema v2 has no separate synchronized-release field, so the returned record carries only
/// the schema's observable handle/cancellation/ownership facts.
pub(super) fn unload_model(
    runtime: &mut ApplicationRuntime,
    loaded: &LoadedModel,
) -> BenchmarkResult<UnloadResult> {
    let state = runtime.state();
    if state.activity() != ApplicationActivity::Idle
        || state.loaded() != Some(loaded)
        || state.active_generation().is_some()
        || !state.hub_available()
        || !state.inference_available()
        || !state.can_unload()
    {
        return Err(BenchmarkError::new(
            "RejectIfBusy unload requires the exact loaded handle in connected, idle E1 state after every request release",
        ));
    }

    let started_at = Instant::now();
    runtime
        .unload_model_with_behavior(ModelUnloadBehavior::RejectIfBusy)
        .map_err(|error| {
            BenchmarkError::new(format!(
                "RejectIfBusy model unload could not be submitted after release: {error}"
            ))
        })?;
    let deadline = checked_deadline(UNLOAD_TIMEOUT, "model unload")?;

    loop {
        if let Some(event) = runtime.poll_event() {
            match event {
                ApplicationEvent::ModelUnloaded {
                    handle,
                    cancelled_requests,
                } => {
                    if handle != loaded.handle() || cancelled_requests != 0 {
                        return Err(BenchmarkError::new(format!(
                            "model unload receipt did not match the loaded handle with zero cancellations: handle={handle:?}, cancelled_requests={cancelled_requests}"
                        )));
                    }
                    let state = runtime.state();
                    if state.activity() != ApplicationActivity::Idle
                        || state.loaded().is_some()
                        || state.active_generation().is_some()
                        || !state.hub_available()
                        || !state.inference_available()
                    {
                        return Err(BenchmarkError::new(
                            "public E1 state retained loaded/active ownership or disconnected after synchronized model removal",
                        ));
                    }
                    return Ok(UnloadResult {
                        duration_ns: duration_ns(started_at.elapsed()),
                        cancelled_requests,
                        loaded_model_absent: true,
                        active_generation_absent: true,
                        runtime_connected: true,
                        backend_release_synchronized: true,
                    });
                }
                ApplicationEvent::ModelDraining { handle } => {
                    return Err(BenchmarkError::new(format!(
                        "RejectIfBusy unload entered draining for handle {handle:?} after all requests were already released"
                    )));
                }
                ApplicationEvent::ModelUnloadFailed { failure } => {
                    return Err(BenchmarkError::new(format!(
                        "RejectIfBusy model unload failed: {failure}"
                    )));
                }
                ApplicationEvent::GenerationCleanupPending {
                    request_id,
                    exhausted,
                    failure,
                } => {
                    return Err(BenchmarkError::new(format!(
                        "generation cleanup remained pending for request {} during unload (exhausted={exhausted}): {failure}",
                        request_id.get()
                    )));
                }
                ApplicationEvent::HubDisconnected => {
                    return Err(BenchmarkError::new(
                        "Hub worker disconnected before explicit shutdown",
                    ));
                }
                ApplicationEvent::RuntimeDisconnected => {
                    return Err(BenchmarkError::new(
                        "inference worker disconnected during model unload",
                    ));
                }
                unexpected => {
                    return Err(BenchmarkError::new(format!(
                        "unexpected application event during model unload: {unexpected:?}"
                    )));
                }
            }
        }
        wait_for_next_poll(deadline, "model unload")?;
    }
}

fn validate_exact_selection(selection: &ModelSelection) -> BenchmarkResult {
    if selection.repository() != MODEL_REPOSITORY || selection.revision() != MODEL_REVISION {
        return Err(BenchmarkError::new(format!(
            "external model selection must be exactly {MODEL_REPOSITORY}@{MODEL_REVISION}, received {}@{}",
            selection.repository(),
            selection.revision()
        )));
    }
    Ok(())
}

fn validate_resolution_ready(
    runtime: &ApplicationRuntime,
    selection: &ModelSelection,
) -> BenchmarkResult {
    let state = runtime.state();
    if state.activity() != ApplicationActivity::Idle
        || !state.hub_available()
        || !state.inference_available()
        || state.loaded().is_some()
        || state.active_generation().is_some()
        || !state.can_resolve(selection)
    {
        return Err(BenchmarkError::new(
            "exact immutable resolution requires connected, idle E1 state without loaded or active ownership",
        ));
    }
    Ok(())
}

fn validate_resolved_facts(model: &ResolvedModel, selection: &ModelSelection) -> BenchmarkResult {
    validate_exact_selection(selection)?;
    if model.selection() != selection
        || model.identity().repository() != MODEL_REPOSITORY
        || model.identity().commit() != MODEL_REVISION
        || model.engine() != ApplicationEngine::Candle
        || model.source() != ApplicationSource::HuggingFaceHub
        || model.format() != ApplicationModelFormat::Safetensors
        || model.scalar_type() != Some(ApplicationScalarType::Bf16)
        || !model.is_loadable()
        || model.vocabulary_size() != EXPECTED_VOCABULARY_SIZE
        || model.chat_compatibility()
            != ChatCompatibility::Supported(PromptCompatibilityProfile::TinyLlamaChatV1)
    {
        return Err(BenchmarkError::new(format!(
            "resolved model did not retain the exact immutable TinyLlama Candle/Hub/Safetensors/BF16/chat facts: {model:?}"
        )));
    }
    Ok(())
}

fn validate_resolved_state(
    runtime: &ApplicationRuntime,
    model: &ResolvedModel,
    selection: &ModelSelection,
) -> BenchmarkResult {
    validate_resolved_facts(model, selection)?;
    let state = runtime.state();
    if state.activity() != ApplicationActivity::Idle
        || state.resolved() != Some(model)
        || state.loaded().is_some()
        || state.active_generation().is_some()
        || !state.hub_available()
        || !state.inference_available()
    {
        return Err(BenchmarkError::new(
            "public E1 state did not retain the clean exact immutable resolution",
        ));
    }
    Ok(())
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
    let device_bytes = match requested_device {
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
            device_bytes,
        },
    })
}

fn validate_exact_adapter_plan(
    plan: &LoadPlan,
    resolved: &ResolvedModel,
    source_weight_bytes: u64,
) -> BenchmarkResult {
    let descriptor = plan.descriptor;
    let required_operations = CapabilitySet::PREFILL
        .union(CapabilitySet::INCREMENTAL_DECODE)
        .union(CapabilitySet::MULTIPLE_SEQUENCES)
        .union(CapabilitySet::EXPLICIT_SYNCHRONIZATION);
    if descriptor.backend != ADAPTER_PLAN_BACKEND
        || descriptor.metadata.architecture != ModelArchitecture::Llama
        || descriptor.metadata.scalar_type != ScalarType::Bf16
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

    let inspection_dtype = infer_bf16_execution_dtype(
        RequestedDevice::Cpu,
        descriptor.estimated_footprint,
        source_weight_bytes,
    )?;
    if inspection_dtype != "F32" {
        return Err(BenchmarkError::new(
            "Candle artifact inspection did not retain its mandatory CPU F32 estimate",
        ));
    }
    Ok(())
}

fn validate_unverified_plan(
    planned: &PlannedModelEvidence,
    observer: &DeviceObserver,
) -> BenchmarkResult {
    if planned.e0_footprint.e1_accepted_e0_load_contract
        || planned.e0_footprint.reservation_snapshot_observed
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
    let inferred = infer_bf16_execution_dtype(
        planned.requested_device,
        memory_footprint(planned.e0_footprint.independent_public_plan),
        planned.source_weight_bytes,
    )?;
    if inferred != planned.execution_dtype {
        return Err(BenchmarkError::new(format!(
            "adapter plan execution dtype changed before load: recorded {}, inferred {inferred}",
            planned.execution_dtype
        )));
    }
    Ok(())
}

fn validate_loaded_state(
    runtime: &ApplicationRuntime,
    loaded: &LoadedModel,
    resolved: &ResolvedModel,
    selection: &ModelSelection,
    planned: &PlannedModelEvidence,
    observer: &DeviceObserver,
) -> BenchmarkResult {
    validate_resolved_facts(resolved, selection)?;
    validate_unverified_plan(planned, observer)?;
    observer.validate_selected_e1(runtime)?;
    observer.validate_actual_loaded(loaded.device())?;

    if loaded.selection() != selection
        || loaded.identity() != resolved.identity()
        || loaded.identity().repository() != MODEL_REPOSITORY
        || loaded.identity().commit() != MODEL_REVISION
        || loaded.engine() != ApplicationEngine::Candle
        || loaded.source() != ApplicationSource::HuggingFaceHub
        || loaded.device() != requested_application_device(planned.requested_device)
        || loaded.format() != ApplicationModelFormat::Safetensors
        || loaded.scalar_type() != ApplicationScalarType::Bf16
        || loaded.scalar_type() != resolved.scalar_type().unwrap_or(ApplicationScalarType::F32)
        || loaded.vocabulary_size() != EXPECTED_VOCABULARY_SIZE
        || loaded.vocabulary_size() != resolved.vocabulary_size()
        || loaded.maximum_context_tokens() != EXPECTED_CONTEXT_TOKENS
        || loaded.maximum_prefill_batch() != EXPECTED_CONTEXT_TOKENS
    {
        return Err(BenchmarkError::new(format!(
            "loaded model did not retain the exact resolved TinyLlama identity, selected/actual device, source BF16, and capacities: {loaded:?}"
        )));
    }

    let state = runtime.state();
    if state.activity() != ApplicationActivity::Idle
        || state.resolved() != Some(resolved)
        || state.loaded() != Some(loaded)
        || state.active_generation().is_some()
        || !state.hub_available()
        || !state.inference_available()
        || !state.can_start_generation()
        || !runtime.can_submit_chat_message()
        || !runtime.conversation().is_empty()
        || runtime.context_diagnostics().is_some()
    {
        return Err(BenchmarkError::new(
            "public E1 state did not retain the exact loaded model with clean direct/chat admission",
        ));
    }
    Ok(())
}

fn canonical_snapshot_artifacts(cache_directory: &Path) -> BenchmarkResult<SnapshotArtifacts> {
    let canonical_cache = cache_directory.canonicalize().map_err(|error| {
        BenchmarkError::new(format!(
            "could not canonicalize explicit cache directory {}: {error}",
            cache_directory.display()
        ))
    })?;
    if !fs::metadata(&canonical_cache)
        .map_err(|error| {
            BenchmarkError::new(format!(
                "could not inspect canonical cache directory {}: {error}",
                canonical_cache.display()
            ))
        })?
        .is_dir()
    {
        return Err(BenchmarkError::new(format!(
            "canonical cache path {} is not a directory",
            canonical_cache.display()
        )));
    }

    let (config_candidate, weight_candidate) = snapshot_artifact_paths(&canonical_cache);
    let (config_path, config_bytes) =
        canonical_regular_file(&canonical_cache, &config_candidate, CONFIG_FILE)?;
    let (weight_path, weight_bytes) =
        canonical_regular_file(&canonical_cache, &weight_candidate, WEIGHT_FILE)?;
    if config_path == weight_path {
        return Err(BenchmarkError::new(
            "fixed config and Safetensors snapshot entries resolved to the same file",
        ));
    }
    if config_bytes == 0 || weight_bytes == 0 {
        return Err(BenchmarkError::new(
            "fixed config and Safetensors snapshot artifacts must both be nonempty",
        ));
    }

    Ok(SnapshotArtifacts {
        config_path,
        weight_path,
        weight_bytes,
    })
}

fn snapshot_artifact_paths(cache_directory: &Path) -> (PathBuf, PathBuf) {
    let snapshot = cache_directory
        .join(CACHE_REPOSITORY_DIRECTORY)
        .join("snapshots")
        .join(MODEL_REVISION);
    (snapshot.join(CONFIG_FILE), snapshot.join(WEIGHT_FILE))
}

fn canonical_regular_file(
    canonical_cache: &Path,
    candidate: &Path,
    label: &str,
) -> BenchmarkResult<(PathBuf, u64)> {
    let canonical = candidate.canonicalize().map_err(|error| {
        BenchmarkError::new(format!(
            "fixed snapshot artifact {} could not be canonicalized: {error}",
            candidate.display()
        ))
    })?;
    if !canonical.starts_with(canonical_cache) {
        return Err(BenchmarkError::new(format!(
            "fixed snapshot artifact {label} resolves outside canonical cache {}: {}",
            canonical_cache.display(),
            canonical.display()
        )));
    }
    let metadata = fs::metadata(&canonical).map_err(|error| {
        BenchmarkError::new(format!(
            "could not inspect canonical snapshot artifact {}: {error}",
            canonical.display()
        ))
    })?;
    if !metadata.is_file() {
        return Err(BenchmarkError::new(format!(
            "fixed snapshot artifact {label} is not a regular file: {}",
            canonical.display()
        )));
    }
    Ok((canonical, metadata.len()))
}

fn infer_bf16_execution_dtype(
    requested_device: RequestedDevice,
    footprint: MemoryFootprint,
    source_weight_bytes: u64,
) -> BenchmarkResult<&'static str> {
    if source_weight_bytes == 0 {
        return Err(BenchmarkError::new(
            "BF16 execution dtype cannot be inferred from an empty weight artifact",
        ));
    }
    match requested_device {
        RequestedDevice::Cpu => {
            let expected_host_weight_bytes =
                source_weight_bytes.checked_mul(2).ok_or_else(|| {
                    BenchmarkError::new(
                        "CPU F32 weight-byte scaling overflowed the BF16 source length",
                    )
                })?;
            if footprint.host_weight_bytes == expected_host_weight_bytes
                && footprint.device_weight_bytes == 0
            {
                Ok("F32")
            } else {
                Err(BenchmarkError::new(format!(
                    "CPU plan did not expand fixed BF16 source weights to exact F32 host bytes: source={source_weight_bytes}, host={}, device={}",
                    footprint.host_weight_bytes, footprint.device_weight_bytes
                )))
            }
        }
        RequestedDevice::Cuda0 => {
            if footprint.host_weight_bytes == 0
                && footprint.device_weight_bytes == source_weight_bytes
            {
                Ok("BF16")
            } else {
                Err(BenchmarkError::new(format!(
                    "CUDA plan did not retain fixed BF16 source weights as exact BF16 device bytes: source={source_weight_bytes}, host={}, device={}",
                    footprint.host_weight_bytes, footprint.device_weight_bytes
                )))
            }
        }
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

const fn requested_device_identity(requested: RequestedDevice) -> super::report::DeviceIdentity {
    match requested {
        RequestedDevice::Cpu => super::report::DeviceIdentity {
            kind: "cpu",
            id: 0,
            ordinal: None,
        },
        RequestedDevice::Cuda0 => super::report::DeviceIdentity {
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

fn checked_deadline(timeout: Duration, operation: &'static str) -> BenchmarkResult<Instant> {
    Instant::now().checked_add(timeout).ok_or_else(|| {
        BenchmarkError::new(format!(
            "deadline overflow while preparing to wait for {operation}"
        ))
    })
}

fn wait_for_next_poll(deadline: Instant, operation: &'static str) -> BenchmarkResult {
    let remaining = deadline
        .checked_duration_since(Instant::now())
        .filter(|duration| !duration.is_zero())
        .ok_or_else(|| BenchmarkError::new(format!("timed out waiting for {operation}")))?;
    std::thread::sleep(POLL_INTERVAL.min(remaining));
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    use domain_contracts::MemoryFootprint;

    use super::{
        MODEL_ARCHITECTURE, MODEL_REPOSITORY, MODEL_REVISION, RequestedDevice,
        canonical_snapshot_artifacts, infer_bf16_execution_dtype, snapshot_artifact_paths,
    };

    static NEXT_TEST_DIRECTORY: AtomicU64 = AtomicU64::new(0);

    #[test]
    fn fixed_identity_and_linux_snapshot_path_are_exact() -> Result<(), String> {
        assert_eq!(MODEL_REPOSITORY, "TinyLlama/TinyLlama-1.1B-Chat-v1.0");
        assert_eq!(MODEL_REVISION, "fe8a4ea1ffedaf415f4da2f062534de366a451e6");
        assert_eq!(MODEL_ARCHITECTURE, "Llama");

        let cache = PathBuf::from("/tmp/fixed-hf-cache");
        let (config, weights) = snapshot_artifact_paths(&cache);
        let snapshot = cache
            .join("models--TinyLlama--TinyLlama-1.1B-Chat-v1.0")
            .join("snapshots")
            .join(MODEL_REVISION);
        if config != snapshot.join("config.json") || weights != snapshot.join("model.safetensors") {
            return Err("fixed snapshot paths changed".to_owned());
        }
        Ok(())
    }

    #[test]
    fn snapshot_artifacts_are_canonical_regular_files_under_the_cache() -> Result<(), String> {
        let fixture = CacheFixture::create()?;
        let (config, weights) = fixture.write_regular_artifacts()?;
        let artifacts =
            canonical_snapshot_artifacts(&fixture.cache).map_err(|error| error.to_string())?;
        assert_eq!(
            artifacts.config_path,
            config.canonicalize().map_err(|error| error.to_string())?
        );
        assert_eq!(
            artifacts.weight_path,
            weights.canonicalize().map_err(|error| error.to_string())?
        );
        assert_eq!(artifacts.weight_bytes, 8);
        Ok(())
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn snapshot_artifact_symlink_cannot_escape_the_canonical_cache() -> Result<(), String> {
        use std::os::unix::fs::symlink;

        let fixture = CacheFixture::create()?;
        let (config, weights) = snapshot_artifact_paths(&fixture.cache);
        let outside = fixture.root.join("outside-config.json");
        fs::write(&outside, b"outside").map_err(|error| error.to_string())?;
        symlink(&outside, &config).map_err(|error| error.to_string())?;
        fs::write(weights, b"weights").map_err(|error| error.to_string())?;

        let error = canonical_snapshot_artifacts(&fixture.cache)
            .err()
            .ok_or_else(|| "escaping snapshot symlink unexpectedly succeeded".to_owned())?;
        assert!(error.to_string().contains("outside canonical cache"));
        Ok(())
    }

    #[test]
    fn snapshot_artifacts_must_be_regular_files() -> Result<(), String> {
        let fixture = CacheFixture::create()?;
        let (config, weights) = snapshot_artifact_paths(&fixture.cache);
        fs::create_dir_all(config).map_err(|error| error.to_string())?;
        fs::write(weights, b"weights").map_err(|error| error.to_string())?;

        let error = canonical_snapshot_artifacts(&fixture.cache)
            .err()
            .ok_or_else(|| "directory artifact unexpectedly succeeded".to_owned())?;
        assert!(error.to_string().contains("not a regular file"));
        Ok(())
    }

    #[test]
    fn bf16_execution_dtype_requires_exact_device_weight_scaling() -> Result<(), String> {
        let cpu = MemoryFootprint {
            host_weight_bytes: 200,
            ..MemoryFootprint::default()
        };
        assert_eq!(
            infer_bf16_execution_dtype(RequestedDevice::Cpu, cpu, 100)
                .map_err(|error| error.to_string())?,
            "F32"
        );

        let cuda = MemoryFootprint {
            device_weight_bytes: 100,
            ..MemoryFootprint::default()
        };
        assert_eq!(
            infer_bf16_execution_dtype(RequestedDevice::Cuda0, cuda, 100)
                .map_err(|error| error.to_string())?,
            "BF16"
        );

        let unexpanded_cpu = MemoryFootprint {
            host_weight_bytes: 100,
            ..MemoryFootprint::default()
        };
        assert!(infer_bf16_execution_dtype(RequestedDevice::Cpu, unexpanded_cpu, 100).is_err());
        let expanded_cuda = MemoryFootprint {
            device_weight_bytes: 200,
            ..MemoryFootprint::default()
        };
        assert!(infer_bf16_execution_dtype(RequestedDevice::Cuda0, expanded_cuda, 100).is_err());
        assert!(infer_bf16_execution_dtype(RequestedDevice::Cpu, cpu, u64::MAX).is_err());
        Ok(())
    }

    struct CacheFixture {
        root: PathBuf,
        cache: PathBuf,
    }

    impl CacheFixture {
        fn create() -> Result<Self, String> {
            let root = unique_test_directory();
            let cache = root.join("cache");
            let (config, _) = snapshot_artifact_paths(&cache);
            let snapshot = config
                .parent()
                .ok_or_else(|| "snapshot fixture had no parent".to_owned())?;
            fs::create_dir_all(snapshot).map_err(|error| error.to_string())?;
            Ok(Self { root, cache })
        }

        fn write_regular_artifacts(&self) -> Result<(PathBuf, PathBuf), String> {
            let (config, weights) = snapshot_artifact_paths(&self.cache);
            fs::write(&config, b"{\"ok\":1}").map_err(|error| error.to_string())?;
            fs::write(&weights, b"weights!").map_err(|error| error.to_string())?;
            Ok((config, weights))
        }
    }

    impl Drop for CacheFixture {
        fn drop(&mut self) {
            let _cleanup = fs::remove_dir_all(&self.root);
        }
    }

    fn unique_test_directory() -> PathBuf {
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |duration| duration.as_nanos());
        let identifier = NEXT_TEST_DIRECTORY.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!(
            "milkdrift-external-model-{}-{timestamp}-{identifier}",
            std::process::id()
        ))
    }
}
