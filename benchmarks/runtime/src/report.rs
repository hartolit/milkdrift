//! Stable typed JSON records shared by both normal-runner modes.

use std::collections::BTreeMap;
use std::time::Duration;

use serde::Serialize;

use crate::cli::Mode;
use crate::error::{BenchmarkError, BenchmarkResult};
use crate::fixture::FixtureIdentity;
use crate::memory::ProcessMemory;

pub(crate) const SCHEMA_VERSION: u32 = 1;

#[derive(Serialize)]
pub(crate) struct BaselineReport {
    pub(crate) schema_version: u32,
    pub(crate) metadata: RunMetadata,
    pub(crate) results: Results,
}

#[derive(Serialize)]
pub(crate) struct RunMetadata {
    pub(crate) git: GitMetadata,
    pub(crate) toolchain: ToolchainMetadata,
    pub(crate) system: SystemMetadata,
    pub(crate) model: ModelMetadata,
    pub(crate) workload: WorkloadMetadata,
    pub(crate) execution_policy: ExecutionPolicy,
}

#[derive(Serialize)]
pub(crate) struct GitMetadata {
    pub(crate) head: String,
    pub(crate) head_tree: String,
    pub(crate) dirty: bool,
}

#[derive(Serialize)]
pub(crate) struct ToolchainMetadata {
    pub(crate) rust_version: String,
    pub(crate) cargo_version: String,
    pub(crate) llvm_version: Option<String>,
    pub(crate) criterion_version: &'static str,
    pub(crate) target_triple: String,
    pub(crate) build_profile: &'static str,
    pub(crate) enabled_features: Vec<String>,
}

#[derive(Serialize)]
pub(crate) struct SystemMetadata {
    pub(crate) os: &'static str,
    pub(crate) kernel: String,
    pub(crate) cpu_model: Option<String>,
    pub(crate) physical_cpu_count: Option<usize>,
    pub(crate) logical_cpu_count: Option<usize>,
    pub(crate) total_memory_bytes: Option<u64>,
    pub(crate) thread_environment: BTreeMap<String, Option<String>>,
}

#[derive(Serialize)]
pub(crate) struct ModelMetadata {
    pub(crate) identity: String,
    pub(crate) repository: Option<String>,
    pub(crate) revision: String,
    pub(crate) architecture: &'static str,
    pub(crate) format: &'static str,
    pub(crate) scalar_type: &'static str,
    pub(crate) vocabulary_size: u32,
    pub(crate) context_capacity: u32,
    pub(crate) fixture: Option<FixtureIdentity>,
}

#[derive(Serialize)]
pub(crate) struct WorkloadMetadata {
    pub(crate) mode: Mode,
    pub(crate) warmup_cycles: u32,
    pub(crate) sample_cycles: u32,
    pub(crate) application_runtime_startup_warmup_cycles: u32,
    pub(crate) application_runtime_startup_sample_cycles: u32,
    pub(crate) prompt_token_count: u64,
    pub(crate) checked_prefill_prompt_token_count: Option<u32>,
    pub(crate) generation_token_count: u32,
    pub(crate) post_first_token_window: u32,
    pub(crate) sampling: SamplingMetadata,
}

#[derive(Serialize)]
pub(crate) struct SamplingMetadata {
    pub(crate) policy: &'static str,
    pub(crate) temperature: f32,
    pub(crate) top_k: u32,
    pub(crate) top_p: f32,
    pub(crate) min_p: f32,
    pub(crate) repetition_penalty: f32,
    pub(crate) repetition_window: u32,
    pub(crate) seed: u64,
    pub(crate) eos_token_count: usize,
    pub(crate) stop_sequence_count: usize,
}

#[expect(
    clippy::struct_excessive_bools,
    reason = "these independent booleans are stable machine-readable JSON policy facts"
)]
#[derive(Serialize)]
pub(crate) struct ExecutionPolicy {
    pub(crate) network_allowed: bool,
    pub(crate) hugging_face_offline: bool,
    pub(crate) cache_directory: Option<String>,
    pub(crate) operational_timeouts_are_thresholds: bool,
    pub(crate) result_files_written_by_runner: bool,
}

#[derive(Serialize)]
#[serde(tag = "mode", rename_all = "kebab-case")]
pub(crate) enum Results {
    Synthetic {
        warmups: Vec<SyntheticCycle>,
        samples: Vec<SyntheticCycle>,
        application_runtime_startup: LifecycleCycleSet,
        summary: Box<SyntheticSummary>,
    },
    RealProduct {
        warmups: Vec<RealProductCycle>,
        samples: Vec<RealProductCycle>,
        summary: Box<RealProductSummary>,
    },
}

#[derive(Serialize)]
pub(crate) struct LifecycleCycleSet {
    pub(crate) warmups: Vec<ApplicationStartupCycle>,
    pub(crate) samples: Vec<ApplicationStartupCycle>,
}

#[derive(Serialize)]
pub(crate) struct ApplicationStartupCycle {
    pub(crate) ordinal: u32,
    pub(crate) start_ns: u64,
    pub(crate) shutdown_ns: u64,
    pub(crate) rss_before_start: ProcessMemory,
    pub(crate) rss_after_start: ProcessMemory,
    pub(crate) rss_after_shutdown: ProcessMemory,
}

#[derive(Serialize)]
pub(crate) struct SyntheticCycle {
    pub(crate) ordinal: u32,
    pub(crate) e0_start_ns: u64,
    pub(crate) model_load_ns: u64,
    pub(crate) checked_prefill: ThroughputMeasurement,
    pub(crate) first_token_ns: u64,
    pub(crate) post_first_token_proxy: ProxyThroughputMeasurement,
    pub(crate) backpressure: BackpressureMeasurement,
    pub(crate) cancellation: CancellationMeasurement,
    pub(crate) model_unload_ns: u64,
    pub(crate) shutdown: ShutdownMeasurement,
    pub(crate) generations: SyntheticGenerationEvidence,
    pub(crate) snapshots: Vec<SnapshotCheckpoint>,
}

#[derive(Serialize)]
pub(crate) struct ThroughputMeasurement {
    pub(crate) duration_ns: u64,
    pub(crate) token_count: u32,
    pub(crate) tokens_per_second: Option<f64>,
}

#[derive(Serialize)]
pub(crate) struct ProxyThroughputMeasurement {
    pub(crate) label: &'static str,
    pub(crate) duration_ns: u64,
    pub(crate) token_count: u32,
    pub(crate) tokens_per_second: Option<f64>,
}

#[derive(Serialize)]
pub(crate) struct BackpressureMeasurement {
    pub(crate) controlled_hold_ns: u64,
    pub(crate) recovery_to_next_token_ns: u64,
    pub(crate) output_backpressure_observed: bool,
}

#[expect(
    clippy::struct_field_names,
    reason = "nanosecond suffixes are required stable JSON unit annotations"
)]
#[derive(Serialize)]
pub(crate) struct CancellationMeasurement {
    pub(crate) acknowledgement_ns: u64,
    pub(crate) terminal_ns: u64,
    pub(crate) released_ns: u64,
}

#[expect(
    clippy::struct_field_names,
    reason = "nanosecond suffixes are required stable JSON unit annotations"
)]
#[derive(Serialize)]
pub(crate) struct ShutdownMeasurement {
    pub(crate) event_ns: u64,
    pub(crate) join_ns: u64,
    pub(crate) total_ns: u64,
}

#[derive(Serialize)]
pub(crate) struct SyntheticGenerationEvidence {
    pub(crate) first_token_and_proxy: GenerationValidation,
    pub(crate) backpressure: GenerationValidation,
    pub(crate) cancellation: GenerationValidation,
}

#[derive(Serialize)]
pub(crate) struct GenerationValidation {
    pub(crate) generated_token_count: u32,
    pub(crate) terminal: &'static str,
    pub(crate) released: &'static str,
    pub(crate) cleanup_pending_observed: bool,
    pub(crate) cleanup_exhausted_observed: bool,
}

#[derive(Serialize)]
pub(crate) struct SnapshotCheckpoint {
    pub(crate) checkpoint: &'static str,
    pub(crate) process_memory: ProcessMemory,
    pub(crate) runtime: RuntimeAccounting,
    pub(crate) models: Vec<ModelAccounting>,
}

#[derive(Serialize)]
pub(crate) struct RuntimeAccounting {
    pub(crate) loaded_models: u32,
    pub(crate) active_requests: u32,
    pub(crate) reserved_footprint: MemoryFootprintRecord,
    pub(crate) generation_workspaces: u32,
    pub(crate) reserved_generation_workspace: MemoryFootprintRecord,
    pub(crate) pending_cleanup_models: u32,
    pub(crate) pending_cleanup_sequences: u32,
    pub(crate) exhausted_cleanup_models: u32,
    pub(crate) exhausted_cleanup_sequences: u32,
    pub(crate) last_cleanup_present: bool,
    pub(crate) maintenance_error_present: bool,
    pub(crate) shutting_down: bool,
}

#[derive(Clone, Copy, Serialize)]
pub(crate) struct MemoryFootprintRecord {
    pub(crate) host_weight_bytes: u64,
    pub(crate) device_weight_bytes: u64,
    pub(crate) host_working_bytes: u64,
    pub(crate) device_working_bytes: u64,
    pub(crate) cache_bytes_per_token: u64,
}

#[derive(Serialize)]
pub(crate) struct ModelAccounting {
    pub(crate) model_id: u64,
    pub(crate) generation: u64,
    pub(crate) lifecycle: &'static str,
    pub(crate) reserved_footprint: MemoryFootprintRecord,
    pub(crate) active_requests: u32,
    pub(crate) pending_cleanup_sequences: u32,
    pub(crate) exhausted_cleanup_sequences: u32,
    pub(crate) degraded: bool,
}

#[derive(Serialize)]
pub(crate) struct RealProductCycle {
    pub(crate) ordinal: u32,
    pub(crate) application_start_ns: u64,
    pub(crate) resolution_or_download_ns: u64,
    pub(crate) model_load_ns: u64,
    pub(crate) first_decoded_output: FirstDecodedOutputMeasurement,
    pub(crate) post_first_generated_token_proxy: Option<ProxyThroughputMeasurement>,
    pub(crate) generation: RealGenerationEvidence,
    pub(crate) model_unload_ns: u64,
    pub(crate) application_shutdown_ns: u64,
    pub(crate) model: RealModelEvidence,
    pub(crate) process_memory: RealProcessMemory,
}

#[derive(Serialize)]
pub(crate) struct FirstDecodedOutputMeasurement {
    pub(crate) duration_ns: u64,
    pub(crate) first_fragment_bytes: usize,
    pub(crate) usage_at_observation: UsageRecord,
}

#[derive(Clone, Copy, Serialize)]
pub(crate) struct UsageRecord {
    pub(crate) prompt_tokens: u64,
    pub(crate) generated_tokens: u64,
}

#[derive(Serialize)]
pub(crate) struct RealGenerationEvidence {
    pub(crate) decoded_byte_count: usize,
    pub(crate) decoded_text_record_count: u32,
    pub(crate) terminal: &'static str,
    pub(crate) released: &'static str,
    pub(crate) terminal_usage: UsageRecord,
    pub(crate) cleanup_pending_observed: bool,
    pub(crate) cleanup_exhausted_observed: bool,
}

#[derive(Serialize)]
pub(crate) struct RealModelEvidence {
    pub(crate) repository: String,
    pub(crate) requested_revision: String,
    pub(crate) immutable_commit: String,
    pub(crate) engine: &'static str,
    pub(crate) source: &'static str,
    pub(crate) device: &'static str,
    pub(crate) format: &'static str,
    pub(crate) scalar_type: &'static str,
    pub(crate) vocabulary_size: u32,
    pub(crate) maximum_context_tokens: u32,
    pub(crate) maximum_prefill_batch: u32,
}

#[derive(Serialize)]
pub(crate) struct RealProcessMemory {
    pub(crate) before_start: ProcessMemory,
    pub(crate) after_start: ProcessMemory,
    pub(crate) after_resolution: ProcessMemory,
    pub(crate) after_load: ProcessMemory,
    pub(crate) after_generation_release: ProcessMemory,
    pub(crate) after_unload: ProcessMemory,
    pub(crate) after_shutdown: ProcessMemory,
}

#[expect(
    clippy::struct_field_names,
    reason = "nanosecond suffixes are required stable JSON unit annotations"
)]
#[derive(Serialize)]
pub(crate) struct SyntheticSummary {
    pub(crate) model_load_ns: MetricSummary,
    pub(crate) checked_prefill_ns: MetricSummary,
    pub(crate) first_token_ns: MetricSummary,
    pub(crate) post_first_token_proxy_ns: MetricSummary,
    pub(crate) backpressure_recovery_ns: MetricSummary,
    pub(crate) cancellation_terminal_ns: MetricSummary,
    pub(crate) model_unload_ns: MetricSummary,
    pub(crate) e0_shutdown_total_ns: MetricSummary,
    pub(crate) application_start_ns: MetricSummary,
    pub(crate) application_shutdown_ns: MetricSummary,
}

#[expect(
    clippy::struct_field_names,
    reason = "nanosecond suffixes are required stable JSON unit annotations"
)]
#[derive(Serialize)]
pub(crate) struct RealProductSummary {
    pub(crate) application_start_ns: MetricSummary,
    pub(crate) resolution_or_download_ns: MetricSummary,
    pub(crate) model_load_ns: MetricSummary,
    pub(crate) first_decoded_output_ns: MetricSummary,
    pub(crate) model_unload_ns: MetricSummary,
    pub(crate) application_shutdown_ns: MetricSummary,
}

#[derive(Serialize)]
pub(crate) struct MetricSummary {
    pub(crate) sample_count: usize,
    pub(crate) minimum: u64,
    pub(crate) median: u64,
    pub(crate) maximum: u64,
    pub(crate) arithmetic_mean: u64,
}

pub(crate) fn duration_ns(duration: Duration) -> u64 {
    u64::try_from(duration.as_nanos()).unwrap_or(u64::MAX)
}

pub(crate) fn throughput(token_count: u32, duration: Duration) -> Option<f64> {
    let seconds = duration.as_secs_f64();
    if seconds > 0.0 {
        Some(f64::from(token_count) / seconds)
    } else {
        None
    }
}

pub(crate) fn metric_summary(
    values: impl IntoIterator<Item = u64>,
) -> BenchmarkResult<MetricSummary> {
    let mut values = values.into_iter().collect::<Vec<_>>();
    if values.is_empty() {
        return Err(BenchmarkError::new(
            "cannot summarize an empty normal-sample set",
        ));
    }
    values.sort_unstable();
    let sample_count = values.len();
    let minimum = values
        .first()
        .copied()
        .ok_or_else(|| BenchmarkError::new("summary minimum disappeared"))?;
    let maximum = values
        .last()
        .copied()
        .ok_or_else(|| BenchmarkError::new("summary maximum disappeared"))?;
    let middle = sample_count / 2;
    let median = if sample_count % 2 == 0 {
        let lower = values
            .get(middle.saturating_sub(1))
            .copied()
            .ok_or_else(|| BenchmarkError::new("summary lower median disappeared"))?;
        let upper = values
            .get(middle)
            .copied()
            .ok_or_else(|| BenchmarkError::new("summary upper median disappeared"))?;
        lower.saturating_add(upper) / 2
    } else {
        values
            .get(middle)
            .copied()
            .ok_or_else(|| BenchmarkError::new("summary median disappeared"))?
    };
    let sum = values.iter().fold(0_u128, |total, value| {
        total.saturating_add(u128::from(*value))
    });
    let divisor = u128::try_from(sample_count)
        .map_err(|_| BenchmarkError::new("summary sample count conversion failed"))?;
    let mean = sum / divisor;
    let arithmetic_mean = u64::try_from(mean).unwrap_or(u64::MAX);
    Ok(MetricSummary {
        sample_count,
        minimum,
        median,
        maximum,
        arithmetic_mean,
    })
}
