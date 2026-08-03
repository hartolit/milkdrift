//! Device-neutral JSON report schema for the external product baseline.

use serde::Serialize;

use crate::memory::ProcessMemory;
use crate::report::{GitMetadata, MemoryFootprintRecord, SystemMetadata, ToolchainMetadata};

pub(super) const SCHEMA_VERSION: u32 = 2;

#[derive(Clone, Debug, PartialEq, Serialize)]
pub(crate) struct ExternalBaselineReport {
    pub(super) schema_version: u32,
    pub(super) provenance: Provenance,
    pub(super) execution: ExecutionMetadata,
    pub(super) model: ModelMetadata,
    pub(super) workload: WorkloadMetadata,
    pub(super) results: Results,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub(super) struct Provenance {
    pub(super) git: GitMetadata,
    pub(super) toolchain: ToolchainMetadata,
    pub(super) system: SystemMetadata,
    pub(super) command_mode: &'static str,
    pub(super) network_authorized: bool,
    pub(super) cache_location: &'static str,
    pub(super) cuda_environment: Option<CudaEnvironmentMetadata>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub(super) struct CudaEnvironmentMetadata {
    pub(super) driver_version: String,
    pub(super) toolkit_release: String,
    pub(super) toolkit_compiler_version: String,
    pub(super) build_compute_capability: String,
    pub(super) cuda_visible_devices: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub(super) struct ExecutionMetadata {
    pub(super) cuda_enabled: bool,
    pub(super) requested_device: DeviceIdentity,
    pub(super) cuda_device: Option<CudaDeviceMetadata>,
    pub(super) execution_dtype: &'static str,
    pub(super) host_sampling: bool,
    pub(super) cuda_logits_to_host_limitation: Option<&'static str>,
    pub(super) cuda_memory_observation_scope: Option<&'static str>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub(super) struct DeviceIdentity {
    pub(super) kind: &'static str,
    pub(super) id: u64,
    pub(super) ordinal: Option<u32>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub(super) struct CudaDeviceMetadata {
    pub(super) name: String,
    pub(super) compute_capability: CudaComputeCapability,
    pub(super) total_memory_bytes: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub(super) struct CudaComputeCapability {
    pub(super) major: u32,
    pub(super) minor: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub(super) struct ModelMetadata {
    pub(super) repository: &'static str,
    pub(super) requested_revision: &'static str,
    pub(super) resolved_commit: String,
    pub(super) upstream_declared_license: &'static str,
    pub(super) license_metadata_source: &'static str,
    pub(super) engine: &'static str,
    pub(super) source: &'static str,
    pub(super) format: &'static str,
    pub(super) architecture: &'static str,
    pub(super) source_scalar: &'static str,
    pub(super) vocabulary_size: u32,
    pub(super) maximum_context_tokens: u32,
    pub(super) maximum_prefill_batch: u32,
    pub(super) cache_state_before_resolution: &'static str,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub(super) struct WorkloadMetadata {
    pub(super) chat_compatibility: ChatWorkloadMetadata,
    pub(super) direct_completion: DirectCompletionWorkloadMetadata,
    pub(super) cancellation: CancellationWorkloadMetadata,
    pub(super) lifecycle: LifecycleCounts,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub(super) struct ChatWorkloadMetadata {
    pub(super) message_identifier: &'static str,
    pub(super) message_sha256: String,
    pub(super) message_bytes: u64,
    pub(super) maximum_new_tokens: u32,
    pub(super) sampling: SamplingMetadata,
    pub(super) termination_policy: &'static str,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub(super) struct DirectCompletionWorkloadMetadata {
    pub(super) prompt_identifier: &'static str,
    pub(super) prompt_sha256: String,
    pub(super) prompt_bytes: u64,
    pub(super) warmup_count: u32,
    pub(super) sample_count: u32,
    pub(super) maximum_new_tokens: u32,
    pub(super) sampling: SamplingMetadata,
    pub(super) eos_tokens: &'static str,
    pub(super) textual_stop_sequences: &'static str,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub(super) struct CancellationWorkloadMetadata {
    pub(super) prompt_identifier: &'static str,
    pub(super) prompt_sha256: String,
    pub(super) prompt_bytes: u64,
    pub(super) maximum_new_tokens: u32,
    pub(super) sampling: SamplingMetadata,
    pub(super) cancellation_trigger: &'static str,
    pub(super) cancellation_reason: &'static str,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub(super) struct SamplingMetadata {
    pub(super) temperature: f32,
    pub(super) top_k: u32,
    pub(super) top_p: f32,
    pub(super) min_p: f32,
    pub(super) repetition_penalty: f32,
    pub(super) repetition_window: u32,
    pub(super) fixed_seed: u64,
}

#[expect(
    clippy::struct_field_names,
    reason = "the cycle suffix makes every serialized count's unit explicit"
)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub(super) struct LifecycleCounts {
    pub(super) primary_full_workload_cycles: u32,
    pub(super) cuda_stability_cycles: u32,
    pub(super) total_cycles: u32,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub(super) struct Results {
    pub(super) primary_cycle: PrimaryCycleResult,
    pub(super) cuda_stability_cycles: Vec<StabilityCycleResult>,
    pub(super) stability_summary: StabilitySummary,
    pub(super) temporary_workspace_removed: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub(super) struct PrimaryCycleResult {
    pub(super) lifecycle: LifecycleResult,
    pub(super) chat_compatibility: ChatProofResult,
    pub(super) direct_completion: DirectCompletionResults,
    pub(super) cancellation: CancellationResult,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub(super) struct StabilityCycleResult {
    pub(super) lifecycle: LifecycleResult,
    pub(super) direct_completion: DirectCompletionSample,
    pub(super) cancellation: CancellationResult,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub(super) struct LifecycleResult {
    pub(super) ordinal: u32,
    pub(super) cache_state_before_resolution: &'static str,
    pub(super) start_ns: u64,
    pub(super) resolve_ns: u64,
    pub(super) load_ns: u64,
    pub(super) selected_e1_device: DeviceIdentity,
    pub(super) actual_loaded_e0_device: DeviceIdentity,
    pub(super) e0_footprint: E0FootprintEvidence,
    pub(super) post_unload_e0_accounting_scope: &'static str,
    pub(super) unload: UnloadResult,
    pub(super) shutdown: ShutdownResult,
    pub(super) resource_checkpoints: Vec<ResourceCheckpoint>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub(super) struct E0FootprintEvidence {
    pub(super) independent_public_plan: MemoryFootprintRecord,
    pub(super) e1_accepted_e0_load_contract: bool,
    pub(super) reservation_snapshot_observed: bool,
    pub(super) provenance: &'static str,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub(super) struct ResourceCheckpoint {
    pub(super) checkpoint: &'static str,
    pub(super) host_memory: ProcessMemory,
    pub(super) cuda_memory: Option<CudaMemoryObservation>,
}

#[expect(
    clippy::struct_field_names,
    reason = "the byte suffix distinguishes every serialized memory quantity's unit"
)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub(super) struct CudaMemoryObservation {
    pub(super) total_bytes: u64,
    pub(super) free_bytes: u64,
    pub(super) used_bytes: u64,
    pub(super) used_delta_from_pre_load_bytes: Option<i64>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub(super) struct CancellationResult {
    pub(super) generation_submission: GenerationSubmissionTimings,
    pub(super) cancellation_submission: CancellationSubmissionTimings,
    pub(super) decoded_byte_count: u64,
    pub(super) prompt_tokens: u64,
    pub(super) generated_tokens: u64,
    pub(super) terminal_kind: &'static str,
    pub(super) cancellation_acknowledged: bool,
    pub(super) outcome_match: GenerationOutcomeMatch,
}

#[expect(
    clippy::struct_field_names,
    reason = "the to prefix identifies the shared generation-submission timing origin"
)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub(super) struct GenerationSubmissionTimings {
    pub(super) to_generation_started_ns: u64,
    pub(super) to_first_decoded_output_ns: u64,
    pub(super) to_cancellation_submission_ns: u64,
}

#[expect(
    clippy::struct_field_names,
    reason = "the to prefix identifies the shared cancellation-submission timing origin"
)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub(super) struct CancellationSubmissionTimings {
    pub(super) to_acknowledgement_ns: u64,
    pub(super) to_terminal_output_ns: u64,
    pub(super) to_terminal_event_ns: u64,
    pub(super) to_release_ns: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub(super) struct ChatProofResult {
    pub(super) submission_to_generation_started_ns: u64,
    pub(super) submission_to_first_decoded_output_ns: u64,
    pub(super) submission_to_terminal_event_ns: u64,
    pub(super) submission_to_release_ns: u64,
    pub(super) decoded_byte_count: u64,
    pub(super) prompt_tokens: u64,
    pub(super) generated_tokens: u64,
    pub(super) terminal_kind: &'static str,
    pub(super) outcome_match: GenerationOutcomeMatch,
    pub(super) conversation: ConversationProof,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub(super) struct GenerationOutcomeMatch {
    pub(super) terminal_state_matched: bool,
    pub(super) released_state_matched: bool,
    pub(super) terminal_event_matched: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub(super) struct ConversationProof {
    pub(super) validated: bool,
    pub(super) cleared: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub(super) struct DirectCompletionResults {
    pub(super) warmup: DirectCompletionWarmupResult,
    pub(super) samples: Vec<DirectCompletionSample>,
    pub(super) summary: DirectCompletionSummary,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub(super) struct DirectCompletionWarmupResult {
    pub(super) decoded_byte_count: u64,
    pub(super) prompt_tokens: u64,
    pub(super) generated_tokens: u64,
    pub(super) terminal_kind: &'static str,
    pub(super) clean_release: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub(super) struct DirectCompletionSample {
    pub(super) ordinal: u32,
    pub(super) submission_to_generation_started_ns: u64,
    pub(super) submission_to_first_decoded_output_ns: u64,
    pub(super) submission_to_terminal_event_ns: u64,
    pub(super) submission_to_release_ns: u64,
    pub(super) prompt_tokens: u64,
    pub(super) generated_tokens: u64,
    pub(super) decoded_byte_count: u64,
    pub(super) terminal_kind: &'static str,
    pub(super) terminal_state_matched: bool,
    pub(super) released_state_matched: bool,
    pub(super) terminal_event_matched: bool,
    pub(super) effective_generated_tokens_per_second: f64,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub(super) struct DirectCompletionSummary {
    pub(super) sample_count: u32,
    pub(super) median_submission_to_generation_started_ns: u64,
    pub(super) median_submission_to_first_decoded_output_ns: u64,
    pub(super) median_submission_to_terminal_event_ns: u64,
    pub(super) median_submission_to_release_ns: u64,
    pub(super) median_effective_generated_tokens_per_second: f64,
}

#[expect(
    clippy::struct_excessive_bools,
    reason = "each flag records one independent required unload invariant"
)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub(super) struct UnloadResult {
    pub(super) duration_ns: u64,
    pub(super) cancelled_requests: u32,
    pub(super) loaded_model_absent: bool,
    pub(super) active_generation_absent: bool,
    pub(super) runtime_connected: bool,
    pub(super) backend_release_synchronized: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub(super) struct ShutdownResult {
    pub(super) duration_ns: u64,
    pub(super) shutdown_returned_cleanly: bool,
    pub(super) workers: ShutdownWorkerState,
    pub(super) ownership: ShutdownOwnershipState,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub(super) struct ShutdownWorkerState {
    pub(super) hub_unavailable: bool,
    pub(super) inference_unavailable: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub(super) struct ShutdownOwnershipState {
    pub(super) loaded_model_absent: bool,
    pub(super) active_generation_absent: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub(super) struct StabilitySummary {
    pub(super) primary_cycle_count: u32,
    pub(super) cuda_stability_cycle_count: u32,
    pub(super) post_unload_cuda_used_bytes: Vec<u64>,
    pub(super) post_owner_drop_cuda_used_bytes: Vec<u64>,
    pub(super) post_unload_cuda_delta_from_pre_load_bytes: Vec<i64>,
    pub(super) post_owner_drop_cuda_delta_from_pre_load_bytes: Vec<i64>,
    pub(super) strict_monotonic_retained_growth_observed: bool,
    pub(super) max_retained_cuda_delta_bytes: Option<i64>,
    pub(super) assessment: String,
}
