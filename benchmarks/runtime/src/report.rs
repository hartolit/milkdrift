//! Typed JSON report schema, separate from benchmark execution state.

use std::collections::BTreeMap;
use std::time::Duration;

use serde::Serialize;

use crate::fixture::FixtureIdentity;
use crate::memory::ProcessMemory;

pub(crate) const SCHEMA_VERSION: u32 = 2;
pub(crate) const EXTERNAL_SCHEMA_VERSION: u32 = 1;

#[derive(Serialize)]
pub(crate) struct BaselineReport {
    pub(crate) schema_version: u32,
    pub(crate) metadata: RunMetadata,
    pub(crate) results: BaselineResults,
}

#[derive(Serialize)]
pub(crate) struct ExternalBaselineReport {
    pub(crate) schema_version: u32,
    pub(crate) provenance: ExternalProvenance,
    pub(crate) model: ExternalModelMetadata,
    pub(crate) workload: ExternalWorkloadMetadata,
    pub(crate) results: ExternalResults,
}

#[derive(Serialize)]
pub(crate) struct ExternalProvenance {
    pub(crate) git: GitMetadata,
    pub(crate) toolchain: ToolchainMetadata,
    pub(crate) system: SystemMetadata,
    pub(crate) command_mode: &'static str,
    pub(crate) network_authorized: bool,
    pub(crate) cache_location: &'static str,
}

#[derive(Serialize)]
pub(crate) struct ExternalModelMetadata {
    pub(crate) repository: &'static str,
    pub(crate) requested_revision: &'static str,
    pub(crate) resolved_commit: String,
    pub(crate) upstream_declared_license: &'static str,
    pub(crate) license_metadata_source: &'static str,
    pub(crate) engine: &'static str,
    pub(crate) source: &'static str,
    pub(crate) device: &'static str,
    pub(crate) format: &'static str,
    pub(crate) architecture: &'static str,
    pub(crate) scalar_type: &'static str,
    pub(crate) vocabulary_size: u32,
    pub(crate) maximum_context_tokens: u32,
    pub(crate) maximum_prefill_batch: u32,
    pub(crate) cache_state_before_resolution: &'static str,
}

#[derive(Serialize)]
pub(crate) struct ExternalWorkloadMetadata {
    pub(crate) chat_compatibility: ChatWorkloadMetadata,
    pub(crate) direct_completion: DirectCompletionWorkloadMetadata,
}

#[derive(Serialize)]
pub(crate) struct ChatWorkloadMetadata {
    pub(crate) message_identifier: &'static str,
    pub(crate) message_sha256: String,
    pub(crate) message_bytes: u64,
    pub(crate) maximum_new_tokens: u32,
    pub(crate) sampling: SamplingMetadata,
    pub(crate) termination_policy: &'static str,
}

#[derive(Serialize)]
pub(crate) struct DirectCompletionWorkloadMetadata {
    pub(crate) prompt_identifier: &'static str,
    pub(crate) prompt_sha256: String,
    pub(crate) prompt_bytes: u64,
    pub(crate) warmup_count: u32,
    pub(crate) sample_count: u32,
    pub(crate) maximum_new_tokens: u32,
    pub(crate) sampling: SamplingMetadata,
    pub(crate) eos_tokens: &'static str,
    pub(crate) textual_stop_sequences: &'static str,
}

#[derive(Clone, Copy, Serialize)]
pub(crate) struct SamplingMetadata {
    pub(crate) temperature: f32,
    pub(crate) top_k: u32,
    pub(crate) top_p: f32,
    pub(crate) min_p: f32,
    pub(crate) repetition_penalty: f32,
    pub(crate) repetition_window: u32,
    pub(crate) fixed_seed: u64,
}

#[derive(Serialize)]
pub(crate) struct ExternalResults {
    pub(crate) application_startup_ns: u64,
    pub(crate) resolution_ns: u64,
    pub(crate) load_ns: u64,
    pub(crate) chat_compatibility: ChatProofResult,
    pub(crate) direct_completion: DirectCompletionResults,
    pub(crate) unload: ExternalUnloadResult,
    pub(crate) shutdown: ExternalShutdownResult,
    pub(crate) memory: ExternalMemoryCheckpoints,
}

#[derive(Serialize)]
pub(crate) struct ChatProofResult {
    pub(crate) decoded_byte_count: u64,
    pub(crate) prompt_tokens: u64,
    pub(crate) generated_tokens: u64,
    pub(crate) terminal_kind: &'static str,
    pub(crate) outcome_match: GenerationOutcomeMatch,
    pub(crate) conversation: ConversationProof,
}

#[derive(Serialize)]
pub(crate) struct GenerationOutcomeMatch {
    pub(crate) terminal_state_matched: bool,
    pub(crate) released_state_matched: bool,
    pub(crate) terminal_event_matched: bool,
}

#[derive(Serialize)]
pub(crate) struct ConversationProof {
    pub(crate) validated: bool,
    pub(crate) cleared: bool,
}

#[derive(Serialize)]
pub(crate) struct DirectCompletionResults {
    pub(crate) warmup: DirectCompletionWarmupResult,
    pub(crate) samples: Vec<DirectCompletionSample>,
    pub(crate) summary: DirectCompletionSummary,
}

#[derive(Serialize)]
pub(crate) struct DirectCompletionWarmupResult {
    pub(crate) decoded_byte_count: u64,
    pub(crate) prompt_tokens: u64,
    pub(crate) generated_tokens: u64,
    pub(crate) terminal_kind: &'static str,
    pub(crate) clean_release: bool,
}

#[derive(Serialize)]
pub(crate) struct DirectCompletionSample {
    pub(crate) ordinal: u32,
    pub(crate) submission_to_generation_started_ns: u64,
    pub(crate) submission_to_first_decoded_output_ns: u64,
    pub(crate) submission_to_terminal_event_ns: u64,
    pub(crate) submission_to_release_ns: u64,
    pub(crate) prompt_tokens: u64,
    pub(crate) generated_tokens: u64,
    pub(crate) decoded_byte_count: u64,
    pub(crate) terminal_kind: &'static str,
    pub(crate) terminal_state_matched: bool,
    pub(crate) released_state_matched: bool,
    pub(crate) terminal_event_matched: bool,
    pub(crate) effective_generated_tokens_per_second: f64,
    pub(crate) process_memory_after_release: ProcessMemory,
}

#[derive(Serialize)]
pub(crate) struct DirectCompletionSummary {
    pub(crate) sample_count: u32,
    pub(crate) median_submission_to_generation_started_ns: u64,
    pub(crate) median_submission_to_first_decoded_output_ns: u64,
    pub(crate) median_submission_to_terminal_event_ns: u64,
    pub(crate) median_submission_to_release_ns: u64,
    pub(crate) median_effective_generated_tokens_per_second: f64,
}

#[derive(Serialize)]
pub(crate) struct ExternalUnloadResult {
    pub(crate) duration_ns: u64,
    pub(crate) cancelled_requests: u32,
    pub(crate) loaded_model_absent: bool,
    pub(crate) active_generation_absent: bool,
    pub(crate) runtime_connected: bool,
}

#[derive(Serialize)]
pub(crate) struct ExternalShutdownResult {
    pub(crate) duration_ns: u64,
    pub(crate) shutdown_returned_cleanly: bool,
    pub(crate) workers: ShutdownWorkerState,
    pub(crate) ownership: ShutdownOwnershipState,
    pub(crate) temporary_workspace_removed: bool,
}

#[derive(Serialize)]
pub(crate) struct ShutdownWorkerState {
    pub(crate) hub_unavailable: bool,
    pub(crate) inference_unavailable: bool,
}

#[derive(Serialize)]
pub(crate) struct ShutdownOwnershipState {
    pub(crate) loaded_model_absent: bool,
    pub(crate) active_generation_absent: bool,
}

#[derive(Serialize)]
pub(crate) struct ExternalMemoryCheckpoints {
    pub(crate) before_application_start: ProcessMemory,
    pub(crate) after_application_start: ProcessMemory,
    pub(crate) after_resolution: ProcessMemory,
    pub(crate) after_load: ProcessMemory,
    pub(crate) after_warmup_release: ProcessMemory,
    pub(crate) after_unload: ProcessMemory,
    pub(crate) after_shutdown: ProcessMemory,
}

#[derive(Serialize)]
pub(crate) struct RunMetadata {
    pub(crate) git: GitMetadata,
    pub(crate) toolchain: ToolchainMetadata,
    pub(crate) system: SystemMetadata,
    pub(crate) fixture: SyntheticFixtureMetadata,
    pub(crate) workload: WorkloadMetadata,
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
    pub(crate) rustc_host: String,
    pub(crate) build_profile: &'static str,
}

#[derive(Serialize)]
pub(crate) struct SystemMetadata {
    pub(crate) os: &'static str,
    pub(crate) kernel: String,
    pub(crate) cpu_model: Option<String>,
    pub(crate) physical_cpu_count: Option<usize>,
    pub(crate) logical_cpu_count: Option<usize>,
    pub(crate) total_memory_bytes: Option<u64>,
    pub(crate) thread_environment: BTreeMap<String, String>,
}

#[derive(Serialize)]
pub(crate) struct SyntheticFixtureMetadata {
    pub(crate) identity: FixtureIdentity,
    pub(crate) backend: &'static str,
    pub(crate) architecture: &'static str,
    pub(crate) format: &'static str,
    pub(crate) scalar_type: &'static str,
    pub(crate) vocabulary_size: u32,
    pub(crate) context_capacity: u32,
}

#[derive(Serialize)]
pub(crate) struct WorkloadMetadata {
    pub(crate) warmup_cycles: u32,
    pub(crate) sample_cycles: u32,
    pub(crate) checked_prefill_prompt_tokens: u32,
    pub(crate) generation_prompt_tokens: u32,
    pub(crate) first_token_generation_limit: u32,
    pub(crate) post_first_token_window: u32,
    pub(crate) backpressure_generation_limit: u32,
    pub(crate) backpressure_hold_milliseconds: u64,
    pub(crate) cancellation_generation_limit: u32,
    pub(crate) cancellation_hold_milliseconds: u64,
    pub(crate) sampling_strategy: &'static str,
}

#[derive(Serialize)]
pub(crate) struct BaselineResults {
    pub(crate) synthetic_e0: CycleSet<SyntheticCycle>,
    pub(crate) application_lifecycle: CycleSet<ApplicationLifecycleCycle>,
}

#[derive(Serialize)]
pub(crate) struct CycleSet<T> {
    pub(crate) warmups: Vec<T>,
    pub(crate) samples: Vec<T>,
}

#[derive(Serialize)]
pub(crate) struct ApplicationLifecycleCycle {
    pub(crate) start_ns: u64,
    pub(crate) shutdown_ns: u64,
    pub(crate) rss_before_start: ProcessMemory,
    pub(crate) rss_after_start: ProcessMemory,
    pub(crate) rss_after_shutdown: ProcessMemory,
}

#[derive(Serialize)]
pub(crate) struct SyntheticCycle {
    pub(crate) e0_start_ns: u64,
    pub(crate) model_load_ns: u64,
    pub(crate) checked_prefill_ns: u64,
    pub(crate) first_token_ns: u64,
    pub(crate) post_first_token_proxy_ns: u64,
    pub(crate) backpressure: BackpressureMeasurement,
    pub(crate) cancellation: CancellationMeasurement,
    pub(crate) model_unload_ns: u64,
    pub(crate) shutdown: ShutdownMeasurement,
    pub(crate) snapshots: Vec<SnapshotCheckpoint>,
}

#[derive(Serialize)]
pub(crate) struct BackpressureMeasurement {
    pub(crate) controlled_hold_ns: u64,
    pub(crate) recovery_to_next_token_ns: u64,
}

#[derive(Serialize)]
pub(crate) struct CancellationMeasurement {
    pub(crate) generated_tokens: u32,
    pub(crate) acknowledgement_ns: u64,
    pub(crate) terminal_ns: u64,
    pub(crate) released_ns: u64,
}

#[expect(
    clippy::struct_field_names,
    reason = "nanosecond suffixes are explicit serialized units"
)]
#[derive(Serialize)]
pub(crate) struct ShutdownMeasurement {
    pub(crate) event_ns: u64,
    pub(crate) join_ns: u64,
    pub(crate) total_ns: u64,
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

pub(crate) fn duration_ns(duration: Duration) -> u64 {
    u64::try_from(duration.as_nanos()).unwrap_or(u64::MAX)
}
