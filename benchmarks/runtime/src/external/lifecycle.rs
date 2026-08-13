//! Concise shared CPU/CUDA lifecycle coordination for the external product baseline.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use application_runtime::{
    ApplicationActivity, ApplicationDevice, ApplicationRuntime, ApplicationRuntimeConfiguration,
    ApplicationScalarType, GenerationTerminalOutcome, LoadedModel, ModelSelection,
};

use super::cli::{self, RequestedDevice};
use super::generation::{self, PrimaryWorkloadEvidence};
use super::model;
use super::observation::{
    CycleStabilityObservation, DeviceObserver, record_owner_drop, stability_after_unload,
    summarize_stability, validate_pre_load_checkpoint,
};
use super::report::{
    CancellationResult, DirectCompletionSample, LifecycleResult, PrimaryCycleResult,
    ResourceCheckpoint, ShutdownResult, StabilityCycleResult, StabilitySummary,
};
use crate::error::{BenchmarkError, BenchmarkResult};
use crate::evidence::application_device_record;
use crate::support::cleanup::cleanup_runtime_after_failure;
use crate::workspace::TemporaryWorkspace;

const PRIMARY_CYCLE_ORDINAL: u32 = 1;
const CUDA_STABILITY_CYCLE_ORDINALS: [u32; 2] = [2, 3];
const NO_STABILITY_CYCLE_ORDINALS: [u32; 0] = [];

const HUB_RETRIES: usize = 2;
const POLL_INTERVAL: Duration = Duration::from_millis(10);
const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(30);
const DRAIN_TIMEOUT_MILLISECONDS: u64 = 10_000;
const RESOURCE_CHECKPOINT_CAPACITY: usize = 16;

const BEFORE_APPLICATION_START_CHECKPOINT: &str =
    "after-observer-initialization-before-application-start";
const AFTER_APPLICATION_START_CHECKPOINT: &str = "after-application-start-and-device-selection";
const AFTER_RESOLUTION_CHECKPOINT: &str = "after-model-resolution";
const BEFORE_LOAD_CHECKPOINT: &str = "immediately-before-model-load";
const AFTER_LOAD_CHECKPOINT: &str = "after-model-load";
const AFTER_UNLOAD_CHECKPOINT: &str = "after-synchronized-model-unload";
const AFTER_SHUTDOWN_RETURN_CHECKPOINT: &str = "after-application-shutdown-return";
const AFTER_OWNER_DROP_CHECKPOINT: &str = "after-application-shutdown-owner-drop";

pub(super) struct LifecycleEvidence {
    pub(super) resolved_commit: String,
    pub(super) configuration_declared_scalar: Option<ApplicationScalarType>,
    pub(super) actual_execution_scalar: ApplicationScalarType,
    pub(super) vocabulary_size: u32,
    pub(super) maximum_context_tokens: u32,
    pub(super) maximum_prefill_batch: u32,
    pub(super) primary_cycle: PrimaryCycleResult,
    pub(super) cuda_stability_cycles: Vec<StabilityCycleResult>,
    pub(super) stability_summary: StabilitySummary,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct CycleModelFacts {
    resolved_commit: String,
    configuration_declared_scalar: Option<ApplicationScalarType>,
    actual_execution_scalar: ApplicationScalarType,
    vocabulary_size: u32,
    maximum_context_tokens: u32,
    maximum_prefill_batch: u32,
}

#[derive(Clone, Copy)]
enum CycleWorkload {
    Primary,
    Stability,
}

enum CycleWorkloadEvidence {
    Primary(PrimaryWorkloadEvidence),
    Stability {
        direct_completion: DirectCompletionSample,
        cancellation: CancellationResult,
    },
}

struct CompletedCycle {
    model: CycleModelFacts,
    lifecycle: LifecycleResult,
    workload: CycleWorkloadEvidence,
    stability: CycleStabilityObservation,
}

struct CycleRecorder<'a> {
    requested: RequestedDevice,
    observer: &'a mut DeviceObserver,
    checkpoints: Vec<ResourceCheckpoint>,
}

impl<'a> CycleRecorder<'a> {
    fn new(requested: RequestedDevice, observer: &'a mut DeviceObserver) -> BenchmarkResult<Self> {
        let mut checkpoints = Vec::new();
        checkpoints
            .try_reserve_exact(RESOURCE_CHECKPOINT_CAPACITY)
            .map_err(|error| {
                BenchmarkError::new(format!(
                    "resource-checkpoint allocation failed for external cycle: {error}"
                ))
            })?;
        Ok(Self {
            requested,
            observer,
            checkpoints,
        })
    }

    fn capture(&mut self, label: &'static str) -> BenchmarkResult {
        self.checkpoints.push(self.observer.capture(label)?);
        Ok(())
    }

    fn capture_pre_load(&mut self) -> BenchmarkResult {
        let checkpoint = self.observer.capture_pre_load(BEFORE_LOAD_CHECKPOINT)?;
        validate_pre_load_checkpoint(self.requested, &checkpoint)?;
        self.checkpoints.push(checkpoint);
        Ok(())
    }

    fn capture_after_unload(&mut self) -> BenchmarkResult<CycleStabilityObservation> {
        let checkpoint = self.observer.capture(AFTER_UNLOAD_CHECKPOINT)?;
        let stability = stability_after_unload(self.requested, &checkpoint)?;
        self.checkpoints.push(checkpoint);
        Ok(stability)
    }

    fn finish_owner_drop(&mut self, completed: &mut CompletedCycle) -> BenchmarkResult {
        let checkpoint = self.observer.capture(AFTER_OWNER_DROP_CHECKPOINT)?;
        record_owner_drop(self.requested, &checkpoint, &mut completed.stability)?;
        completed.lifecycle.resource_checkpoints.push(checkpoint);
        Ok(())
    }

    fn take_checkpoints(&mut self) -> Vec<ResourceCheckpoint> {
        std::mem::take(&mut self.checkpoints)
    }
}

pub(super) fn run(
    workspace: &TemporaryWorkspace,
    cache_directory: &Path,
    requested: RequestedDevice,
    observer: &mut DeviceObserver,
) -> BenchmarkResult<LifecycleEvidence> {
    validate_observer_request(requested, observer)?;

    let primary = run_cycle(
        workspace,
        cache_directory,
        requested,
        observer,
        PRIMARY_CYCLE_ORDINAL,
        CycleWorkload::Primary,
    )?;
    let primary_model = primary.model.clone();
    let mut stability_observations = vec![primary.stability];
    let primary_cycle = primary_cycle_result(primary)?;

    let ordinals = stability_cycle_ordinals(requested);
    let mut cuda_stability_cycles = Vec::new();
    cuda_stability_cycles
        .try_reserve_exact(ordinals.len())
        .map_err(|error| {
            BenchmarkError::new(format!(
                "CUDA stability-cycle result allocation failed: {error}"
            ))
        })?;
    stability_observations
        .try_reserve_exact(ordinals.len())
        .map_err(|error| {
            BenchmarkError::new(format!("stability observation allocation failed: {error}"))
        })?;

    for &ordinal in ordinals {
        let cycle = run_cycle(
            workspace,
            cache_directory,
            requested,
            observer,
            ordinal,
            CycleWorkload::Stability,
        )?;
        validate_model_consistency(&primary_model, &cycle.model, ordinal)?;
        stability_observations.push(cycle.stability);
        cuda_stability_cycles.push(stability_cycle_result(cycle)?);
    }

    let cuda_cycle_count = u32::try_from(ordinals.len())
        .map_err(|_| BenchmarkError::new("CUDA stability-cycle count conversion failed"))?;
    let stability_summary =
        summarize_stability(requested, cuda_cycle_count, &stability_observations)?;

    Ok(LifecycleEvidence {
        resolved_commit: primary_model.resolved_commit,
        configuration_declared_scalar: primary_model.configuration_declared_scalar,
        actual_execution_scalar: primary_model.actual_execution_scalar,
        vocabulary_size: primary_model.vocabulary_size,
        maximum_context_tokens: primary_model.maximum_context_tokens,
        maximum_prefill_batch: primary_model.maximum_prefill_batch,
        primary_cycle,
        cuda_stability_cycles,
        stability_summary,
    })
}

fn run_cycle(
    workspace: &TemporaryWorkspace,
    cache_directory: &Path,
    requested: RequestedDevice,
    observer: &mut DeviceObserver,
    ordinal: u32,
    workload: CycleWorkload,
) -> BenchmarkResult<CompletedCycle> {
    observer.begin_cycle();
    let mut recorder = CycleRecorder::new(requested, observer)?;
    recorder.capture(BEFORE_APPLICATION_START_CHECKPOINT)?;

    let configuration = application_configuration(
        workspace.database_path("external-application", ordinal),
        cache_directory,
        requested,
    );
    eprintln!(
        "starting external ApplicationRuntime cycle {ordinal} on {}",
        requested_device_label(requested)
    );
    let started_at = Instant::now();
    let mut runtime = ApplicationRuntime::start(configuration).map_err(|error| {
        BenchmarkError::new(format!(
            "external ApplicationRuntime cycle {ordinal} startup failed: {error}"
        ))
    })?;
    let start_elapsed = started_at.elapsed();

    let result = execute_started_cycle(
        &mut runtime,
        cache_directory,
        ordinal,
        workload,
        start_elapsed,
        &mut recorder,
    );
    let mut completed = match result {
        Ok(completed) => completed,
        Err(error) => return Err(cleanup_runtime_after_failure(runtime, error)),
    };

    drop(runtime);
    recorder.finish_owner_drop(&mut completed)?;
    Ok(completed)
}

fn execute_started_cycle(
    runtime: &mut ApplicationRuntime,
    cache_directory: &Path,
    ordinal: u32,
    workload: CycleWorkload,
    start_elapsed: Duration,
    recorder: &mut CycleRecorder<'_>,
) -> BenchmarkResult<CompletedCycle> {
    select_requested_device(runtime, recorder.requested, ordinal)?;
    validate_started(runtime)?;
    recorder.observer.validate_selected_e1(runtime)?;
    recorder.capture(AFTER_APPLICATION_START_CHECKPOINT)?;

    let cache_state = cli::inspect_cache_state(cache_directory)?;
    let selection = ModelSelection::new(model::MODEL_REPOSITORY, model::MODEL_REVISION);
    let (resolved, resolve_elapsed) = model::resolve_model(runtime, &selection)?;
    recorder.capture(AFTER_RESOLUTION_CHECKPOINT)?;

    recorder.capture_pre_load()?;
    let (loaded, load_elapsed) = model::load_model(runtime, &selection, recorder.observer)?;
    recorder.capture(AFTER_LOAD_CHECKPOINT)?;

    let model_facts = validated_model_facts(runtime, &resolved, &loaded, recorder.observer)?;
    let workload_evidence = run_cycle_workload(runtime, &loaded, workload, recorder)?;
    validate_released_workload_state(runtime, &loaded)?;

    let unload = model::unload_model(runtime, &loaded)?;
    let stability = recorder.capture_after_unload()?;
    let shutdown = shutdown_cycle(runtime, ordinal)?;
    recorder.capture(AFTER_SHUTDOWN_RETURN_CHECKPOINT)?;

    Ok(CompletedCycle {
        model: model_facts,
        lifecycle: LifecycleResult {
            ordinal,
            cache_state_before_resolution: cache_state.label(),
            start_ns: generation::duration_ns(start_elapsed, "ApplicationRuntime startup")?,
            resolve_ns: generation::duration_ns(resolve_elapsed, "immutable model resolution")?,
            load_ns: generation::duration_ns(load_elapsed, "E1 model load acceptance")?,
            selected_e1_device: application_device_record(runtime.state().selected_device()),
            actual_loaded_e0_device: application_device_record(loaded.device()),
            unload,
            shutdown,
            resource_checkpoints: recorder.take_checkpoints(),
        },
        workload: workload_evidence,
        stability,
    })
}

fn validated_model_facts(
    runtime: &ApplicationRuntime,
    resolved: &application_runtime::ResolvedModel,
    loaded: &LoadedModel,
    observer: &DeviceObserver,
) -> BenchmarkResult<CycleModelFacts> {
    observer.validate_selected_e1(runtime)?;
    observer.validate_actual_loaded(loaded.device())?;
    Ok(CycleModelFacts {
        resolved_commit: resolved.identity().commit().to_owned(),
        configuration_declared_scalar: resolved.configuration_declared_scalar_type(),
        actual_execution_scalar: loaded.execution_scalar_type(),
        vocabulary_size: loaded.vocabulary_size(),
        maximum_context_tokens: loaded.maximum_context_tokens(),
        maximum_prefill_batch: loaded.maximum_prefill_batch(),
    })
}

fn run_cycle_workload(
    runtime: &mut ApplicationRuntime,
    loaded: &LoadedModel,
    workload: CycleWorkload,
    recorder: &mut CycleRecorder<'_>,
) -> BenchmarkResult<CycleWorkloadEvidence> {
    let mut observe = |checkpoint: &'static str| recorder.capture(checkpoint);
    match workload {
        CycleWorkload::Primary => Ok(CycleWorkloadEvidence::Primary(
            generation::run_primary_workload(runtime, loaded, &mut observe)?,
        )),
        CycleWorkload::Stability => {
            let (direct_completion, cancellation) =
                generation::run_stability_workload(runtime, &mut observe)?;
            Ok(CycleWorkloadEvidence::Stability {
                direct_completion,
                cancellation,
            })
        }
    }
}

fn shutdown_cycle(
    runtime: &mut ApplicationRuntime,
    ordinal: u32,
) -> BenchmarkResult<ShutdownResult> {
    eprintln!("shutting down external ApplicationRuntime cycle {ordinal}");
    let started_at = Instant::now();
    runtime.shutdown().map_err(|error| {
        BenchmarkError::new(format!(
            "external ApplicationRuntime cycle {ordinal} bounded shutdown failed: {error}"
        ))
    })?;
    let elapsed = started_at.elapsed();
    validate_stopped(runtime)?;
    Ok(ShutdownResult {
        duration_ns: generation::duration_ns(elapsed, "ApplicationRuntime shutdown")?,
    })
}

fn application_configuration(
    database_path: PathBuf,
    cache_directory: &Path,
    requested: RequestedDevice,
) -> ApplicationRuntimeConfiguration {
    let mut configuration = ApplicationRuntimeConfiguration::new(database_path);
    model::MODEL_REPOSITORY.clone_into(&mut configuration.defaults.default_repository);
    model::MODEL_REVISION.clone_into(&mut configuration.defaults.default_revision);
    configuration.defaults.selected_device = requested_application_device(requested);
    configuration.defaults.drain_timeout_milliseconds = DRAIN_TIMEOUT_MILLISECONDS;
    configuration.hub.cache_directory = Some(cache_directory.to_path_buf());
    configuration.hub.maximum_retries = HUB_RETRIES;
    configuration.timing.runtime_poll = POLL_INTERVAL;
    configuration.timing.hub_worker_poll = POLL_INTERVAL;
    configuration.timing.hub_event_send_timeout = Duration::from_secs(1);
    configuration.timing.hub_command_shutdown_timeout = SHUTDOWN_TIMEOUT;
    configuration.timing.runtime_shutdown_timeout = SHUTDOWN_TIMEOUT;
    configuration.timing.runtime_shutdown_event_poll = POLL_INTERVAL;
    configuration.timing.runtime_join_timeout = SHUTDOWN_TIMEOUT;
    configuration.timing.runtime_join_poll = POLL_INTERVAL;
    configuration.timing.hub_shutdown_timeout = SHUTDOWN_TIMEOUT;
    configuration.timing.hub_shutdown_poll = POLL_INTERVAL;
    configuration
}

fn select_requested_device(
    runtime: &mut ApplicationRuntime,
    requested: RequestedDevice,
    ordinal: u32,
) -> BenchmarkResult {
    let device = requested_application_device(requested);
    runtime.select_device(device).map_err(|error| {
        BenchmarkError::new(format!(
            "cycle {ordinal} could not explicitly select requested device {device:?}: {error}"
        ))
    })
}

fn validate_started(runtime: &ApplicationRuntime) -> BenchmarkResult {
    let state = runtime.state();
    if state.activity() != ApplicationActivity::Idle
        || !state.hub_available()
        || !state.inference_available()
        || state.resolved().is_some()
        || state.loaded().is_some()
        || state.active_generation().is_some()
        || state.last_generation().is_some()
        || !runtime.conversation().is_empty()
        || runtime.context_diagnostics().is_some()
        || !state.can_select_device()
    {
        return Err(BenchmarkError::new(
            "external ApplicationRuntime start/device selection returned non-clean connected E1 state",
        ));
    }
    Ok(())
}

fn validate_released_workload_state(
    runtime: &ApplicationRuntime,
    loaded: &LoadedModel,
) -> BenchmarkResult {
    let state = runtime.state();
    let finished = state.last_generation().is_some_and(|terminal| {
        matches!(&terminal.outcome, GenerationTerminalOutcome::Finished(_))
    });
    if state.activity() != ApplicationActivity::Idle
        || state.loaded() != Some(loaded)
        || state.active_generation().is_some()
        || !state.hub_available()
        || !state.inference_available()
        || !finished
        || !runtime.conversation().is_empty()
        || runtime.context_diagnostics().is_some()
    {
        return Err(BenchmarkError::new(
            "external workload did not finish with Released ownership, a successful Finished terminal outcome, connected workers, and no conversation state",
        ));
    }
    Ok(())
}

fn validate_stopped(runtime: &ApplicationRuntime) -> BenchmarkResult {
    let state = runtime.state();
    if state.activity() != ApplicationActivity::ShuttingDown
        || state.hub_available()
        || state.inference_available()
        || state.loaded().is_some()
        || state.active_generation().is_some()
        || !runtime.conversation().is_empty()
        || runtime.context_diagnostics().is_some()
    {
        return Err(BenchmarkError::new(
            "ApplicationRuntime shutdown returned without stopped workers, released model/generation ownership, and empty conversation state",
        ));
    }
    Ok(())
}

fn validate_observer_request(
    requested: RequestedDevice,
    observer: &DeviceObserver,
) -> BenchmarkResult {
    let expected = application_device_record(requested_application_device(requested));
    let observer_identity = observer.requested_identity();
    if observer_identity != expected {
        return Err(BenchmarkError::new(format!(
            "lifecycle requested {expected:?}, but the supplied observer addresses {observer_identity:?}"
        )));
    }
    Ok(())
}

fn validate_model_consistency(
    primary: &CycleModelFacts,
    observed: &CycleModelFacts,
    ordinal: u32,
) -> BenchmarkResult {
    if observed != primary {
        return Err(BenchmarkError::new(format!(
            "cycle {ordinal} changed immutable public model facts or actual execution scalar relative to primary cycle 1: primary={primary:?}, observed={observed:?}"
        )));
    }
    Ok(())
}

fn primary_cycle_result(cycle: CompletedCycle) -> BenchmarkResult<PrimaryCycleResult> {
    let CycleWorkloadEvidence::Primary(workload) = cycle.workload else {
        return Err(BenchmarkError::new(
            "primary cycle completed with stability-workload evidence",
        ));
    };
    Ok(PrimaryCycleResult {
        lifecycle: cycle.lifecycle,
        chat_compatibility: workload.chat_compatibility,
        direct_completion: workload.direct_completion,
        cancellation: workload.cancellation,
    })
}

fn stability_cycle_result(cycle: CompletedCycle) -> BenchmarkResult<StabilityCycleResult> {
    let CycleWorkloadEvidence::Stability {
        direct_completion,
        cancellation,
    } = cycle.workload
    else {
        return Err(BenchmarkError::new(
            "CUDA stability cycle completed with primary-workload evidence",
        ));
    };
    Ok(StabilityCycleResult {
        lifecycle: cycle.lifecycle,
        direct_completion,
        cancellation,
    })
}

const fn stability_cycle_ordinals(requested: RequestedDevice) -> &'static [u32] {
    match requested {
        RequestedDevice::Cpu => &NO_STABILITY_CYCLE_ORDINALS,
        RequestedDevice::Cuda0 => &CUDA_STABILITY_CYCLE_ORDINALS,
    }
}

const fn requested_application_device(requested: RequestedDevice) -> ApplicationDevice {
    match requested {
        RequestedDevice::Cpu => ApplicationDevice::Cpu,
        RequestedDevice::Cuda0 => ApplicationDevice::Cuda { ordinal: 0 },
    }
}

const fn requested_device_label(requested: RequestedDevice) -> &'static str {
    match requested {
        RequestedDevice::Cpu => "CPU",
        RequestedDevice::Cuda0 => "CUDA ordinal 0",
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::{
        CompletedCycle, CycleWorkload, DeviceObserver, RequestedDevice, TemporaryWorkspace,
        run_cycle, stability_cycle_ordinals,
    };
    use crate::error::BenchmarkResult;

    type CycleEntryPoint = fn(
        &TemporaryWorkspace,
        &Path,
        RequestedDevice,
        &mut DeviceObserver,
        u32,
        CycleWorkload,
    ) -> BenchmarkResult<CompletedCycle>;

    const fn orchestration_entry_point(_requested: RequestedDevice) -> CycleEntryPoint {
        run_cycle
    }

    #[test]
    fn cpu_and_cuda_cycle_plans_are_exact() {
        assert!(stability_cycle_ordinals(RequestedDevice::Cpu).is_empty());
        assert_eq!(stability_cycle_ordinals(RequestedDevice::Cuda0), &[2, 3]);
        assert_eq!(1 + stability_cycle_ordinals(RequestedDevice::Cpu).len(), 1);
        assert_eq!(
            1 + stability_cycle_ordinals(RequestedDevice::Cuda0).len(),
            3
        );
    }

    #[test]
    fn cpu_and_cuda_share_one_cycle_orchestration_entry_point() {
        assert!(std::ptr::fn_addr_eq(
            orchestration_entry_point(RequestedDevice::Cpu),
            orchestration_entry_point(RequestedDevice::Cuda0),
        ));
    }
}
