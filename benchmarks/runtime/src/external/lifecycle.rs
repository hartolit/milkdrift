//! Shared CPU/CUDA lifecycle coordination for the external product baseline.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use application_runtime::{
    ApplicationActivity, ApplicationDevice, ApplicationRuntime, ApplicationRuntimeConfiguration,
    ApplicationScalarType, GenerationTerminalOutcome, LoadedModel, ModelSelection,
};

use super::cli::{self, RequestedDevice};
use super::generation::{self, PrimaryWorkloadEvidence};
use super::model;
use super::observation::DeviceObserver;
use super::report::{
    CancellationResult, DeviceIdentity, DirectCompletionSample, E0FootprintEvidence,
    LifecycleResult, PrimaryCycleResult, ResourceCheckpoint, ShutdownOwnershipState,
    ShutdownResult, ShutdownWorkerState, StabilityCycleResult, StabilitySummary, UnloadResult,
};
use crate::e1::cleanup_runtime_after_failure;
use crate::error::{BenchmarkError, BenchmarkResult};
use crate::report::{MemoryFootprintRecord, duration_ns};
use crate::workspace::TemporaryWorkspace;

const PRIMARY_CYCLE_ORDINAL: u32 = 1;
const CUDA_STABILITY_CYCLE_ORDINALS: [u32; 2] = [2, 3];
const NO_STABILITY_CYCLE_ORDINALS: [u32; 0] = [];

const HUB_RETRIES: usize = 2;
const POLL_INTERVAL: Duration = Duration::from_millis(10);
const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(30);
const DRAIN_TIMEOUT_MILLISECONDS: u64 = 10_000;
const RESOURCE_CHECKPOINT_CAPACITY: usize = 16;

const BEFORE_APPLICATION_START_CHECKPOINT: &str = "after-observer-context-before-application-start";
const AFTER_APPLICATION_START_CHECKPOINT: &str = "after-application-start-and-device-selection";
const AFTER_RESOLUTION_CHECKPOINT: &str = "after-model-resolution";
const BEFORE_LOAD_CHECKPOINT: &str = "immediately-before-model-load";
const AFTER_LOAD_CHECKPOINT: &str = "after-model-load";
const AFTER_UNLOAD_CHECKPOINT: &str = "after-synchronized-model-unload";
const AFTER_SHUTDOWN_RETURN_CHECKPOINT: &str = "after-application-shutdown-return";
const AFTER_OWNER_DROP_CHECKPOINT: &str = "after-application-shutdown-owner-drop";

const POST_UNLOAD_ACCOUNTING_SCOPE: &str = "complete same-worker E0 RuntimeSnapshot accounting is not exposed by public E1 APIs and is not inferred here; this report records Released output, no cleanup-pending/exhausted event, synchronized ModelUnloaded acceptance, and zero E1 ownership only; direct E0 snapshot validation is external to this report and must be executed and recorded separately";
const CPU_STABILITY_ASSESSMENT: &str =
    "not-applicable: CPU execution has no CUDA stability cycles or CUDA memory observations";
const CUDA_GROWTH_ASSESSMENT: &str = "review-required: at least one retained CUDA used-byte delta series strictly increased across all cycle windows relative to each cycle's own pre-load baseline; observations are device-global and can be affected by other GPU processes, so this finite result is neither proof of a process leak nor proof of unbounded growth";
const CUDA_NO_GROWTH_ASSESSMENT: &str = "no retained CUDA used-byte delta series strictly increased across every cycle window relative to each cycle's own pre-load baseline; observations are device-global, lifecycle contracts passed, and no leak conclusion is drawn";

pub(super) struct LifecycleEvidence {
    pub(super) resolved_commit: String,
    pub(super) source_scalar: &'static str,
    pub(super) vocabulary_size: u32,
    pub(super) maximum_context_tokens: u32,
    pub(super) maximum_prefill_batch: u32,
    pub(super) execution_dtype: &'static str,
    pub(super) primary_cycle: PrimaryCycleResult,
    pub(super) cuda_stability_cycles: Vec<StabilityCycleResult>,
    pub(super) stability_summary: StabilitySummary,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct CycleModelFacts {
    resolved_commit: String,
    source_scalar: &'static str,
    vocabulary_size: u32,
    maximum_context_tokens: u32,
    maximum_prefill_batch: u32,
    execution_dtype: &'static str,
    e0_footprint: E0FootprintEvidence,
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct CycleStabilityObservation {
    unload_used: Option<u64>,
    owner_drop_used: Option<u64>,
    unload_delta: Option<i64>,
    owner_drop_delta: Option<i64>,
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
    let primary_stability = primary.stability;
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
    let mut stability_observations = Vec::new();
    stability_observations
        .try_reserve_exact(1_usize.saturating_add(ordinals.len()))
        .map_err(|error| {
            BenchmarkError::new(format!("stability observation allocation failed: {error}"))
        })?;
    stability_observations.push(primary_stability);

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

    let stability_summary = summarize_stability(requested, &stability_observations)?;
    let CycleModelFacts {
        resolved_commit,
        source_scalar,
        vocabulary_size,
        maximum_context_tokens,
        maximum_prefill_batch,
        execution_dtype,
        e0_footprint: _,
    } = primary_model;

    Ok(LifecycleEvidence {
        resolved_commit,
        source_scalar,
        vocabulary_size,
        maximum_context_tokens,
        maximum_prefill_batch,
        execution_dtype,
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
    let mut resource_checkpoints = Vec::new();
    resource_checkpoints
        .try_reserve_exact(RESOURCE_CHECKPOINT_CAPACITY)
        .map_err(|error| {
            BenchmarkError::new(format!(
                "cycle {ordinal} resource-checkpoint allocation failed: {error}"
            ))
        })?;
    capture_checkpoint(
        observer,
        &mut resource_checkpoints,
        BEFORE_APPLICATION_START_CHECKPOINT,
    )?;

    let database_path = workspace.database_path("external-application", ordinal);
    let configuration = application_configuration(database_path, cache_directory, requested);
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
        requested,
        observer,
        ordinal,
        workload,
        start_elapsed,
        resource_checkpoints,
    );
    let mut completed = match result {
        Ok(completed) => completed,
        Err(error) => return Err(cleanup_runtime_after_failure(runtime, error)),
    };

    drop(runtime);
    let after_owner_drop = observer.capture(AFTER_OWNER_DROP_CHECKPOINT)?;
    let (post_owner_drop_cuda_used_bytes, post_owner_drop_cuda_delta_bytes) =
        checkpoint_cuda_usage(
            requested,
            &after_owner_drop,
            true,
            "post-application-shutdown owner-drop checkpoint",
        )?;
    completed
        .lifecycle
        .resource_checkpoints
        .push(after_owner_drop);
    completed.stability.owner_drop_used = post_owner_drop_cuda_used_bytes;
    completed.stability.owner_drop_delta = post_owner_drop_cuda_delta_bytes;
    validate_complete_stability_observation(requested, completed.stability)?;
    Ok(completed)
}

#[allow(clippy::too_many_arguments, clippy::too_many_lines)]
fn execute_started_cycle(
    runtime: &mut ApplicationRuntime,
    cache_directory: &Path,
    requested: RequestedDevice,
    observer: &mut DeviceObserver,
    ordinal: u32,
    workload: CycleWorkload,
    start_elapsed: Duration,
    mut resource_checkpoints: Vec<ResourceCheckpoint>,
) -> BenchmarkResult<CompletedCycle> {
    select_requested_device(runtime, requested, ordinal)?;
    validate_started(runtime)?;
    observer.validate_selected_e1(runtime)?;
    capture_checkpoint(
        observer,
        &mut resource_checkpoints,
        AFTER_APPLICATION_START_CHECKPOINT,
    )?;

    let cache_state = cli::inspect_cache_state(cache_directory)?;
    eprintln!(
        "cycle {ordinal} cache is {} before exact immutable resolution",
        cache_state.label()
    );
    let selection = ModelSelection::new(model::MODEL_REPOSITORY, model::MODEL_REVISION);
    let (resolved, resolve_elapsed) = model::resolve_model(runtime, &selection)?;
    capture_checkpoint(
        observer,
        &mut resource_checkpoints,
        AFTER_RESOLUTION_CHECKPOINT,
    )?;

    let mut planned = model::plan_resolved_model(cache_directory, runtime, requested)?;
    let before_load = observer.capture_pre_load(BEFORE_LOAD_CHECKPOINT)?;
    let (loaded, load_elapsed) = model::load_model(runtime, &selection, &mut planned, observer)?;
    validate_pre_load_checkpoint(requested, &before_load)?;
    resource_checkpoints.push(before_load);
    capture_checkpoint(observer, &mut resource_checkpoints, AFTER_LOAD_CHECKPOINT)?;

    let (selected_e1_device, actual_loaded_e0_device) =
        validated_device_identities(runtime, &loaded, observer)?;
    validate_footprint_evidence(&planned.e0_footprint)?;
    let model_facts = CycleModelFacts {
        resolved_commit: resolved.identity().commit().to_owned(),
        source_scalar: scalar_label(loaded.scalar_type()),
        vocabulary_size: loaded.vocabulary_size(),
        maximum_context_tokens: loaded.maximum_context_tokens(),
        maximum_prefill_batch: loaded.maximum_prefill_batch(),
        execution_dtype: planned.execution_dtype,
        e0_footprint: planned.e0_footprint,
    };

    let workload_evidence = {
        let mut observe = |checkpoint: &'static str| -> BenchmarkResult {
            capture_checkpoint(observer, &mut resource_checkpoints, checkpoint)
        };
        match workload {
            CycleWorkload::Primary => CycleWorkloadEvidence::Primary(
                generation::run_primary_workload(runtime, &loaded, &mut observe)?,
            ),
            CycleWorkload::Stability => {
                let (direct_completion, cancellation) =
                    generation::run_stability_workload(runtime, &mut observe)?;
                CycleWorkloadEvidence::Stability {
                    direct_completion,
                    cancellation,
                }
            }
        }
    };
    validate_released_workload_state(runtime, &loaded)?;

    let unload = model::unload_model(runtime, &loaded)?;
    validate_unload_contract(&unload)?;
    let after_unload = observer.capture(AFTER_UNLOAD_CHECKPOINT)?;
    let (post_unload_cuda_used_bytes, post_unload_cuda_delta_bytes) =
        checkpoint_cuda_usage(requested, &after_unload, true, "post-unload checkpoint")?;
    resource_checkpoints.push(after_unload);

    eprintln!("shutting down external ApplicationRuntime cycle {ordinal}");
    let shutdown_started_at = Instant::now();
    runtime.shutdown().map_err(|error| {
        BenchmarkError::new(format!(
            "external ApplicationRuntime cycle {ordinal} bounded shutdown failed: {error}"
        ))
    })?;
    let shutdown_elapsed = shutdown_started_at.elapsed();
    validate_stopped(runtime)?;
    capture_checkpoint(
        observer,
        &mut resource_checkpoints,
        AFTER_SHUTDOWN_RETURN_CHECKPOINT,
    )?;

    Ok(CompletedCycle {
        model: model_facts,
        lifecycle: LifecycleResult {
            ordinal,
            cache_state_before_resolution: cache_state.label(),
            start_ns: duration_ns(start_elapsed),
            resolve_ns: duration_ns(resolve_elapsed),
            load_ns: duration_ns(load_elapsed),
            selected_e1_device,
            actual_loaded_e0_device,
            e0_footprint: planned.e0_footprint,
            post_unload_e0_accounting_scope: POST_UNLOAD_ACCOUNTING_SCOPE,
            unload,
            shutdown: ShutdownResult {
                duration_ns: duration_ns(shutdown_elapsed),
                shutdown_returned_cleanly: true,
                workers: ShutdownWorkerState {
                    hub_unavailable: true,
                    inference_unavailable: true,
                },
                ownership: ShutdownOwnershipState {
                    loaded_model_absent: true,
                    active_generation_absent: true,
                },
            },
            resource_checkpoints,
        },
        workload: workload_evidence,
        stability: CycleStabilityObservation {
            unload_used: post_unload_cuda_used_bytes,
            owner_drop_used: None,
            unload_delta: post_unload_cuda_delta_bytes,
            owner_drop_delta: None,
        },
    })
}

fn application_configuration(
    database_path: PathBuf,
    cache_directory: &Path,
    requested: RequestedDevice,
) -> ApplicationRuntimeConfiguration {
    let mut configuration = ApplicationRuntimeConfiguration::desktop(database_path);
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

fn validated_device_identities(
    runtime: &ApplicationRuntime,
    loaded: &LoadedModel,
    observer: &DeviceObserver,
) -> BenchmarkResult<(DeviceIdentity, DeviceIdentity)> {
    observer.validate_selected_e1(runtime)?;
    observer.validate_actual_loaded(loaded.device())?;

    let expected = observer.requested_identity();
    let selected = application_device_identity(runtime.state().selected_device());
    let actual = application_device_identity(loaded.device());
    if selected != expected || actual != expected {
        return Err(BenchmarkError::new(format!(
            "validated device identities changed before recording: requested={expected:?}, selected={selected:?}, actual={actual:?}"
        )));
    }
    Ok((selected, actual))
}

fn validate_observer_request(
    requested: RequestedDevice,
    observer: &DeviceObserver,
) -> BenchmarkResult {
    let expected = application_device_identity(requested_application_device(requested));
    let observer_identity = observer.requested_identity();
    if observer_identity != expected {
        return Err(BenchmarkError::new(format!(
            "lifecycle requested {expected:?}, but the supplied observer addresses {observer_identity:?}"
        )));
    }
    Ok(())
}

fn validate_footprint_evidence(evidence: &E0FootprintEvidence) -> BenchmarkResult {
    if !evidence.e1_accepted_e0_load_contract
        || evidence.reservation_snapshot_observed
        || footprint_is_zero(&evidence.independent_public_plan)
        || evidence.provenance.is_empty()
    {
        return Err(BenchmarkError::new(
            "E0 footprint evidence must be a nonzero independent public plan plus validated E1 acceptance, without claiming a same-worker reservation snapshot",
        ));
    }
    Ok(())
}

fn validate_unload_contract(unload: &UnloadResult) -> BenchmarkResult {
    if unload.cancelled_requests != 0
        || !unload.loaded_model_absent
        || !unload.active_generation_absent
        || !unload.runtime_connected
        || !unload.backend_release_synchronized
    {
        return Err(BenchmarkError::new(
            "synchronized E1 ModelUnloaded evidence did not retain zero cancellations, connected workers, and released model/generation ownership",
        ));
    }
    Ok(())
}

fn validate_pre_load_checkpoint(
    requested: RequestedDevice,
    checkpoint: &ResourceCheckpoint,
) -> BenchmarkResult {
    let (used_bytes, delta) = checkpoint_cuda_usage(
        requested,
        checkpoint,
        true,
        "immediately-before-load checkpoint",
    )?;
    match requested {
        RequestedDevice::Cpu if used_bytes.is_none() && delta.is_none() => Ok(()),
        RequestedDevice::Cuda0 if used_bytes.is_some() && delta == Some(0) => Ok(()),
        _ => Err(BenchmarkError::new(
            "pre-load resource checkpoint did not establish the exact zero CUDA baseline delta",
        )),
    }
}

fn capture_checkpoint(
    observer: &DeviceObserver,
    checkpoints: &mut Vec<ResourceCheckpoint>,
    label: &'static str,
) -> BenchmarkResult {
    checkpoints.push(observer.capture(label)?);
    Ok(())
}

fn checkpoint_cuda_usage(
    requested: RequestedDevice,
    checkpoint: &ResourceCheckpoint,
    require_pre_load_delta: bool,
    context: &'static str,
) -> BenchmarkResult<(Option<u64>, Option<i64>)> {
    match (requested, checkpoint.cuda_memory) {
        (RequestedDevice::Cpu, None) => Ok((None, None)),
        (RequestedDevice::Cpu, Some(_)) => Err(BenchmarkError::new(format!(
            "{context} unexpectedly contained CUDA memory for a CPU cycle"
        ))),
        (RequestedDevice::Cuda0, None) => Err(BenchmarkError::new(format!(
            "{context} omitted CUDA memory for a CUDA cycle"
        ))),
        (RequestedDevice::Cuda0, Some(memory)) => {
            if require_pre_load_delta && memory.used_delta_from_pre_load_bytes.is_none() {
                return Err(BenchmarkError::new(format!(
                    "{context} omitted its delta from the current cycle's pre-load baseline"
                )));
            }
            Ok((
                Some(memory.used_bytes),
                memory.used_delta_from_pre_load_bytes,
            ))
        }
    }
}

fn validate_model_consistency(
    primary: &CycleModelFacts,
    observed: &CycleModelFacts,
    ordinal: u32,
) -> BenchmarkResult {
    if observed != primary {
        return Err(BenchmarkError::new(format!(
            "cycle {ordinal} changed immutable model facts, execution dtype, or E0 footprint semantics relative to primary cycle 1: primary={primary:?}, observed={observed:?}"
        )));
    }
    Ok(())
}

fn validate_complete_stability_observation(
    requested: RequestedDevice,
    observation: CycleStabilityObservation,
) -> BenchmarkResult {
    let complete = match requested {
        RequestedDevice::Cpu => {
            observation.unload_used.is_none()
                && observation.owner_drop_used.is_none()
                && observation.unload_delta.is_none()
                && observation.owner_drop_delta.is_none()
        }
        RequestedDevice::Cuda0 => {
            observation.unload_used.is_some()
                && observation.owner_drop_used.is_some()
                && observation.unload_delta.is_some()
                && observation.owner_drop_delta.is_some()
        }
    };
    if !complete {
        return Err(BenchmarkError::new(
            "completed cycle did not retain the requested device's full stability observations",
        ));
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

fn summarize_stability(
    requested: RequestedDevice,
    observations: &[CycleStabilityObservation],
) -> BenchmarkResult<StabilitySummary> {
    let expected_cycles = 1_usize.saturating_add(stability_cycle_ordinals(requested).len());
    if observations.len() != expected_cycles {
        return Err(BenchmarkError::new(format!(
            "stability summary received {} complete cycles, expected {expected_cycles}",
            observations.len()
        )));
    }

    let cuda_stability_cycle_count = u32::try_from(stability_cycle_ordinals(requested).len())
        .map_err(|_| BenchmarkError::new("CUDA stability-cycle count conversion failed"))?;
    if requested == RequestedDevice::Cpu {
        if observations.iter().any(|observation| {
            observation.unload_used.is_some()
                || observation.owner_drop_used.is_some()
                || observation.unload_delta.is_some()
                || observation.owner_drop_delta.is_some()
        }) {
            return Err(BenchmarkError::new(
                "CPU stability summary received unexpected CUDA observations",
            ));
        }
        return Ok(StabilitySummary {
            primary_cycle_count: 1,
            cuda_stability_cycle_count,
            post_unload_cuda_used_bytes: Vec::new(),
            post_owner_drop_cuda_used_bytes: Vec::new(),
            post_unload_cuda_delta_from_pre_load_bytes: Vec::new(),
            post_owner_drop_cuda_delta_from_pre_load_bytes: Vec::new(),
            strict_monotonic_retained_growth_observed: false,
            max_retained_cuda_delta_bytes: None,
            assessment: CPU_STABILITY_ASSESSMENT.to_owned(),
        });
    }

    let mut post_unload_cuda_used_bytes = Vec::new();
    let mut post_owner_drop_cuda_used_bytes = Vec::new();
    let mut post_unload_deltas = Vec::new();
    let mut post_owner_drop_deltas = Vec::new();
    for observation in observations {
        validate_complete_stability_observation(requested, *observation)?;
        post_unload_cuda_used_bytes.push(observation.unload_used.ok_or_else(|| {
            BenchmarkError::new("CUDA post-unload used bytes disappeared during summary")
        })?);
        post_owner_drop_cuda_used_bytes.push(observation.owner_drop_used.ok_or_else(|| {
            BenchmarkError::new(
                "CUDA post-application-shutdown owner-drop used bytes disappeared during summary",
            )
        })?);
        post_unload_deltas.push(observation.unload_delta.ok_or_else(|| {
            BenchmarkError::new(
                "CUDA post-unload pre-load-baseline delta disappeared during summary",
            )
        })?);
        post_owner_drop_deltas.push(observation.owner_drop_delta.ok_or_else(|| {
            BenchmarkError::new(
                "CUDA post-owner-drop pre-load-baseline delta disappeared during summary",
            )
        })?);
    }

    let strict_monotonic_retained_growth_observed =
        strictly_increases(&post_unload_deltas) || strictly_increases(&post_owner_drop_deltas);
    let max_retained_cuda_delta_bytes = post_unload_deltas
        .iter()
        .chain(&post_owner_drop_deltas)
        .copied()
        .max();
    let assessment = if strict_monotonic_retained_growth_observed {
        CUDA_GROWTH_ASSESSMENT
    } else {
        CUDA_NO_GROWTH_ASSESSMENT
    };

    Ok(StabilitySummary {
        primary_cycle_count: 1,
        cuda_stability_cycle_count,
        post_unload_cuda_used_bytes,
        post_owner_drop_cuda_used_bytes,
        post_unload_cuda_delta_from_pre_load_bytes: post_unload_deltas,
        post_owner_drop_cuda_delta_from_pre_load_bytes: post_owner_drop_deltas,
        strict_monotonic_retained_growth_observed,
        max_retained_cuda_delta_bytes,
        assessment: assessment.to_owned(),
    })
}

fn strictly_increases(values: &[i64]) -> bool {
    values.len() > 1
        && values.windows(2).all(|window| {
            window
                .first()
                .zip(window.get(1))
                .is_some_and(|(previous, current)| current > previous)
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

const fn application_device_identity(device: ApplicationDevice) -> DeviceIdentity {
    match device {
        ApplicationDevice::Cpu => DeviceIdentity {
            kind: "cpu",
            id: 0,
            ordinal: None,
        },
        ApplicationDevice::Cuda { ordinal } => DeviceIdentity {
            kind: "cuda",
            id: ordinal as u64,
            ordinal: Some(ordinal),
        },
    }
}

const fn requested_device_label(requested: RequestedDevice) -> &'static str {
    match requested {
        RequestedDevice::Cpu => "CPU",
        RequestedDevice::Cuda0 => "CUDA ordinal 0",
    }
}

const fn scalar_label(scalar: ApplicationScalarType) -> &'static str {
    match scalar {
        ApplicationScalarType::F32 => "F32",
        ApplicationScalarType::F16 => "F16",
        ApplicationScalarType::Bf16 => "BF16",
    }
}

const fn footprint_is_zero(footprint: &MemoryFootprintRecord) -> bool {
    footprint.host_weight_bytes == 0
        && footprint.device_weight_bytes == 0
        && footprint.host_working_bytes == 0
        && footprint.device_working_bytes == 0
        && footprint.cache_bytes_per_token == 0
}

#[cfg(test)]
mod tests {
    use super::{
        CPU_STABILITY_ASSESSMENT, CycleStabilityObservation, RequestedDevice,
        stability_cycle_ordinals, strictly_increases, summarize_stability,
    };

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
    fn strict_growth_requires_every_adjacent_window_to_increase() {
        assert!(strictly_increases(&[10, 11, 12]));
        assert!(!strictly_increases(&[]));
        assert!(!strictly_increases(&[10]));
        assert!(!strictly_increases(&[10, 10, 11]));
        assert!(!strictly_increases(&[10, 12, 11]));
    }

    #[test]
    fn cpu_summary_is_not_applicable_and_has_no_cuda_arrays() -> Result<(), String> {
        let summary = summarize_stability(
            RequestedDevice::Cpu,
            &[CycleStabilityObservation {
                unload_used: None,
                owner_drop_used: None,
                unload_delta: None,
                owner_drop_delta: None,
            }],
        )
        .map_err(|error| error.to_string())?;
        assert_eq!(summary.primary_cycle_count, 1);
        assert_eq!(summary.cuda_stability_cycle_count, 0);
        assert!(summary.post_unload_cuda_used_bytes.is_empty());
        assert!(summary.post_owner_drop_cuda_used_bytes.is_empty());
        assert!(
            summary
                .post_unload_cuda_delta_from_pre_load_bytes
                .is_empty()
        );
        assert!(
            summary
                .post_owner_drop_cuda_delta_from_pre_load_bytes
                .is_empty()
        );
        assert!(!summary.strict_monotonic_retained_growth_observed);
        assert_eq!(summary.max_retained_cuda_delta_bytes, None);
        assert_eq!(summary.assessment, CPU_STABILITY_ASSESSMENT);
        Ok(())
    }

    #[test]
    fn cuda_summary_flags_growth_without_calling_it_a_leak() -> Result<(), String> {
        let observations = [
            CycleStabilityObservation {
                unload_used: Some(100),
                owner_drop_used: Some(90),
                unload_delta: Some(20),
                owner_drop_delta: Some(10),
            },
            CycleStabilityObservation {
                unload_used: Some(90),
                owner_drop_used: Some(80),
                unload_delta: Some(30),
                owner_drop_delta: Some(20),
            },
            CycleStabilityObservation {
                unload_used: Some(80),
                owner_drop_used: Some(70),
                unload_delta: Some(40),
                owner_drop_delta: Some(30),
            },
        ];
        let summary = summarize_stability(RequestedDevice::Cuda0, &observations)
            .map_err(|error| error.to_string())?;
        assert_eq!(summary.primary_cycle_count, 1);
        assert_eq!(summary.cuda_stability_cycle_count, 2);
        assert_eq!(summary.post_unload_cuda_used_bytes, [100, 90, 80]);
        assert_eq!(summary.post_owner_drop_cuda_used_bytes, [90, 80, 70]);
        assert_eq!(
            summary.post_unload_cuda_delta_from_pre_load_bytes,
            [20, 30, 40]
        );
        assert_eq!(
            summary.post_owner_drop_cuda_delta_from_pre_load_bytes,
            [10, 20, 30]
        );
        assert!(summary.strict_monotonic_retained_growth_observed);
        assert_eq!(summary.max_retained_cuda_delta_bytes, Some(40));
        assert!(summary.assessment.contains("review-required"));
        assert!(
            summary
                .assessment
                .contains("neither proof of a process leak")
        );
        Ok(())
    }
}
