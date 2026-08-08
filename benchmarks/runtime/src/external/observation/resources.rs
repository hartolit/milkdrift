//! Host RSS, whole-device CUDA memory, cycle baselines, and retained-growth summaries.

use super::super::cli::RequestedDevice;
use super::super::report::{CudaMemoryObservation, ResourceCheckpoint, StabilitySummary};
use super::device::{DeviceState, DiscoveryCounter, ValidatedCudaProbe};
use crate::error::{BenchmarkError, BenchmarkResult};
use crate::memory::process_memory;

const CPU_STABILITY_ASSESSMENT: &str =
    "not-applicable: CPU execution has no CUDA stability cycles or CUDA memory observations";
const CUDA_GROWTH_ASSESSMENT: &str = "review-required: at least one retained CUDA used-byte delta series strictly increased across all cycle windows relative to each cycle's own pre-load baseline; observations are device-global and can be affected by other GPU processes, so this finite result is neither proof of a process leak nor proof of unbounded growth";
const CUDA_NO_GROWTH_ASSESSMENT: &str = "no retained CUDA used-byte delta series strictly increased across every cycle window relative to each cycle's own pre-load baseline; observations are device-global, lifecycle contracts passed, and no leak conclusion is drawn";

pub(super) const CUDA_MEMORY_OBSERVATION_SCOPE: &str = "safe CUDA driver total/free observations for the whole device, not process-attributed usage; desktop and other GPU processes can affect absolute values and deltas";

#[derive(Default)]
pub(super) struct ResourceState {
    pre_load_used_bytes: Option<u64>,
}

impl ResourceState {
    pub(super) fn begin_cycle(&mut self) {
        self.pre_load_used_bytes = None;
    }

    pub(super) fn capture(
        &self,
        device: &DeviceState,
        discovery_counter: &DiscoveryCounter,
        checkpoint: &'static str,
    ) -> BenchmarkResult<ResourceCheckpoint> {
        let host_memory = process_memory()?;
        let cuda_memory = match device.requested() {
            RequestedDevice::Cpu => None,
            RequestedDevice::Cuda0 => {
                let probe = device.validated_cuda_probe(discovery_counter)?;
                Some(cuda_memory_observation(&probe, self.pre_load_used_bytes)?)
            }
        };
        Ok(ResourceCheckpoint {
            checkpoint,
            process_memory: host_memory,
            whole_device_cuda_memory: cuda_memory,
        })
    }

    pub(super) fn capture_pre_load(
        &mut self,
        device: &DeviceState,
        discovery_counter: &DiscoveryCounter,
        checkpoint: &'static str,
    ) -> BenchmarkResult<ResourceCheckpoint> {
        let host_memory = process_memory()?;
        let cuda_memory = match device.requested() {
            RequestedDevice::Cpu => None,
            RequestedDevice::Cuda0 => {
                let probe = device.validated_cuda_probe(discovery_counter)?;
                let used_bytes = probe.used_bytes()?;
                let observation = CudaMemoryObservation {
                    total_bytes: probe.total_bytes,
                    free_bytes: probe.free_bytes,
                    used_bytes,
                    used_delta_from_pre_load_bytes: Some(0),
                };
                self.pre_load_used_bytes = Some(used_bytes);
                Some(observation)
            }
        };
        Ok(ResourceCheckpoint {
            checkpoint,
            process_memory: host_memory,
            whole_device_cuda_memory: cuda_memory,
        })
    }
}

fn cuda_memory_observation(
    probe: &ValidatedCudaProbe,
    pre_load_used_bytes: Option<u64>,
) -> BenchmarkResult<CudaMemoryObservation> {
    let used_bytes = probe.used_bytes()?;
    let used_delta_from_pre_load_bytes = pre_load_used_bytes
        .map(|baseline| signed_used_delta(used_bytes, baseline))
        .transpose()?;
    Ok(CudaMemoryObservation {
        total_bytes: probe.total_bytes,
        free_bytes: probe.free_bytes,
        used_bytes,
        used_delta_from_pre_load_bytes,
    })
}

fn signed_used_delta(current: u64, pre_load: u64) -> BenchmarkResult<i64> {
    let delta = i128::from(current) - i128::from(pre_load);
    i64::try_from(delta).map_err(|_| {
        BenchmarkError::new(format!(
            "CUDA used-memory delta does not fit i64: current {current} bytes, pre-load baseline {pre_load} bytes"
        ))
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::external) struct CycleStabilityObservation {
    pub(in crate::external) unload_used: Option<u64>,
    pub(in crate::external) owner_drop_used: Option<u64>,
    pub(in crate::external) unload_delta: Option<i64>,
    pub(in crate::external) owner_drop_delta: Option<i64>,
}

pub(in crate::external) fn validate_pre_load_checkpoint(
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

pub(in crate::external) fn stability_after_unload(
    requested: RequestedDevice,
    checkpoint: &ResourceCheckpoint,
) -> BenchmarkResult<CycleStabilityObservation> {
    let (unload_used, unload_delta) =
        checkpoint_cuda_usage(requested, checkpoint, true, "post-unload checkpoint")?;
    Ok(CycleStabilityObservation {
        unload_used,
        owner_drop_used: None,
        unload_delta,
        owner_drop_delta: None,
    })
}

pub(in crate::external) fn record_owner_drop(
    requested: RequestedDevice,
    checkpoint: &ResourceCheckpoint,
    observation: &mut CycleStabilityObservation,
) -> BenchmarkResult {
    let (owner_drop_used, owner_drop_delta) = checkpoint_cuda_usage(
        requested,
        checkpoint,
        true,
        "post-application-shutdown owner-drop checkpoint",
    )?;
    observation.owner_drop_used = owner_drop_used;
    observation.owner_drop_delta = owner_drop_delta;
    validate_complete_stability_observation(requested, *observation)
}

pub(in crate::external) fn validate_complete_stability_observation(
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

pub(in crate::external) fn summarize_stability(
    requested: RequestedDevice,
    cuda_stability_cycle_count: u32,
    observations: &[CycleStabilityObservation],
) -> BenchmarkResult<StabilitySummary> {
    let expected_cycles = usize::try_from(cuda_stability_cycle_count)
        .ok()
        .and_then(|count| count.checked_add(1))
        .ok_or_else(|| BenchmarkError::new("stability-cycle count conversion overflowed"))?;
    if observations.len() != expected_cycles {
        return Err(BenchmarkError::new(format!(
            "stability summary received {} complete cycles, expected {expected_cycles}",
            observations.len()
        )));
    }

    if requested == RequestedDevice::Cpu {
        if cuda_stability_cycle_count != 0 {
            return Err(BenchmarkError::new(
                "CPU stability summary received a nonzero CUDA stability-cycle count",
            ));
        }
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

fn checkpoint_cuda_usage(
    requested: RequestedDevice,
    checkpoint: &ResourceCheckpoint,
    require_pre_load_delta: bool,
    context: &'static str,
) -> BenchmarkResult<(Option<u64>, Option<i64>)> {
    match (requested, checkpoint.whole_device_cuda_memory) {
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

pub(in crate::external) fn strictly_increases(values: &[i64]) -> bool {
    values.len() > 1
        && values.windows(2).all(|window| {
            window
                .first()
                .zip(window.get(1))
                .is_some_and(|(previous, current)| current > previous)
        })
}

#[cfg(test)]
mod tests {
    use super::{
        CPU_STABILITY_ASSESSMENT, CycleStabilityObservation, ResourceState,
        cuda_memory_observation, record_owner_drop, signed_used_delta, stability_after_unload,
        strictly_increases, summarize_stability, validate_pre_load_checkpoint,
    };
    use crate::external::cli::RequestedDevice;
    use crate::external::observation::device::ValidatedCudaProbe;
    use crate::external::report::{CudaComputeCapability, CudaMemoryObservation};

    fn valid_probe() -> ValidatedCudaProbe {
        ValidatedCudaProbe {
            name: "NVIDIA GeForce RTX 5070 Ti".to_owned(),
            compute_capability: CudaComputeCapability {
                major: 12,
                minor: 0,
            },
            total_bytes: 16_000,
            free_bytes: 12_000,
            supports_bf16: true,
        }
    }

    #[test]
    fn cycle_reset_clears_the_previous_pre_load_baseline() {
        let mut resources = ResourceState {
            pre_load_used_bytes: Some(4_000),
        };
        resources.begin_cycle();
        assert_eq!(resources.pre_load_used_bytes, None);
    }

    #[test]
    fn cuda_memory_delta_is_signed_and_pre_load_can_be_exactly_zero() -> Result<(), String> {
        assert_eq!(
            signed_used_delta(12_000, 10_000).map_err(|error| error.to_string())?,
            2_000
        );
        assert_eq!(
            signed_used_delta(8_000, 10_000).map_err(|error| error.to_string())?,
            -2_000
        );
        assert_eq!(
            signed_used_delta(10_000, 10_000).map_err(|error| error.to_string())?,
            0
        );
        assert!(signed_used_delta(u64::MAX, 0).is_err());

        assert_eq!(
            cuda_memory_observation(&valid_probe(), Some(4_000))
                .map_err(|error| error.to_string())?,
            CudaMemoryObservation {
                total_bytes: 16_000,
                free_bytes: 12_000,
                used_bytes: 4_000,
                used_delta_from_pre_load_bytes: Some(0),
            }
        );
        Ok(())
    }

    #[test]
    fn checkpoint_interpretation_keeps_cpu_and_cuda_shapes_distinct() -> Result<(), String> {
        let cpu = crate::external::report::ResourceCheckpoint {
            checkpoint: "cpu",
            process_memory: crate::memory::ProcessMemory::default(),
            whole_device_cuda_memory: None,
        };
        validate_pre_load_checkpoint(RequestedDevice::Cpu, &cpu)
            .map_err(|error| error.to_string())?;
        let mut cpu_stability = stability_after_unload(RequestedDevice::Cpu, &cpu)
            .map_err(|error| error.to_string())?;
        record_owner_drop(RequestedDevice::Cpu, &cpu, &mut cpu_stability)
            .map_err(|error| error.to_string())?;

        let cuda = crate::external::report::ResourceCheckpoint {
            checkpoint: "cuda",
            process_memory: crate::memory::ProcessMemory::default(),
            whole_device_cuda_memory: Some(CudaMemoryObservation {
                total_bytes: 16_000,
                free_bytes: 12_000,
                used_bytes: 4_000,
                used_delta_from_pre_load_bytes: Some(0),
            }),
        };
        validate_pre_load_checkpoint(RequestedDevice::Cuda0, &cuda)
            .map_err(|error| error.to_string())?;
        let mut cuda_stability = stability_after_unload(RequestedDevice::Cuda0, &cuda)
            .map_err(|error| error.to_string())?;
        record_owner_drop(RequestedDevice::Cuda0, &cuda, &mut cuda_stability)
            .map_err(|error| error.to_string())?;
        assert_eq!(cuda_stability.unload_used, Some(4_000));
        assert_eq!(cuda_stability.owner_drop_delta, Some(0));
        Ok(())
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
            0,
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
        let summary = summarize_stability(RequestedDevice::Cuda0, 2, &observations)
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
