//! Device-neutral JSON report schema for the external product baseline.

use serde::Serialize;

use crate::memory::ProcessMemory;
use crate::report::{GitMetadata, MemoryFootprintRecord, SystemMetadata, ToolchainMetadata};

pub(super) const SCHEMA_VERSION: u32 = 5;

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
    pub(super) planned_execution_scalar: &'static str,
    pub(super) actual_execution_scalar: &'static str,
    pub(super) host_sampling: bool,
    pub(super) cuda_logits_to_host_limitation: Option<&'static str>,
    pub(super) cuda_memory_observation_scope: Option<&'static str>,
    pub(super) cuda_context_observation: Option<CudaContextObservation>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub(super) struct CudaContextObservation {
    pub(super) device_discovery_calls: u64,
    pub(super) initialization_scope: &'static str,
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
    pub(super) artifact_layout: ArtifactLayoutMetadata,
    pub(super) configuration_declared_scalar: Option<&'static str>,
    pub(super) observed_tensor_scalars: Vec<&'static str>,
    pub(super) vocabulary_size: u32,
    pub(super) maximum_context_tokens: u32,
    pub(super) maximum_prefill_batch: u32,
    pub(super) cache_state_before_resolution: &'static str,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub(super) struct ArtifactLayoutMetadata {
    pub(super) configuration_file: &'static str,
    pub(super) tokenizer_file: &'static str,
    pub(super) safetensors_layout: &'static str,
    pub(super) weight_files: Vec<&'static str>,
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
    pub(super) prepared_load: PreparedLoadEvidence,
    pub(super) post_unload_e0_accounting_scope: &'static str,
    pub(super) unload: UnloadResult,
    pub(super) shutdown: ShutdownResult,
    pub(super) resource_checkpoints: Vec<ResourceCheckpoint>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub(super) struct PreparedLoadEvidence {
    pub(super) planned_execution_device: DeviceIdentity,
    pub(super) exact_final_footprint: MemoryFootprintRecord,
    pub(super) loading_peak_footprint: MemoryFootprintRecord,
    pub(super) e1_load_accepted: bool,
    pub(super) e0_reserved_ownership_observed: bool,
    pub(super) provenance: &'static str,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub(super) struct ResourceCheckpoint {
    pub(super) checkpoint: &'static str,
    pub(super) process_memory: ProcessMemory,
    pub(super) whole_device_cuda_memory: Option<CudaMemoryObservation>,
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

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use serde_json::Value;

    use super::*;
    use crate::report::{GitMetadata, MemoryFootprintRecord, SystemMetadata, ToolchainMetadata};

    const CUDA_IDENTITY: DeviceIdentity = DeviceIdentity {
        kind: "cuda",
        id: 0,
        ordinal: Some(0),
    };

    fn final_footprint() -> MemoryFootprintRecord {
        MemoryFootprintRecord {
            host_weight_bytes: 0,
            device_weight_bytes: 2_200,
            host_working_bytes: 0,
            device_working_bytes: 0,
        }
    }

    fn loading_peak_footprint() -> MemoryFootprintRecord {
        MemoryFootprintRecord {
            host_working_bytes: 2_200,
            ..final_footprint()
        }
    }

    fn generation_outcomes() -> GenerationOutcomeMatch {
        GenerationOutcomeMatch {
            terminal_state_matched: true,
            released_state_matched: true,
            terminal_event_matched: true,
        }
    }

    fn cancellation() -> CancellationResult {
        CancellationResult {
            generation_submission: GenerationSubmissionTimings {
                to_generation_started_ns: 1,
                to_first_decoded_output_ns: 2,
                to_cancellation_submission_ns: 3,
            },
            cancellation_submission: CancellationSubmissionTimings {
                to_acknowledgement_ns: 1,
                to_terminal_output_ns: 2,
                to_terminal_event_ns: 3,
                to_release_ns: 4,
            },
            decoded_byte_count: 8,
            prompt_tokens: 4,
            generated_tokens: 1,
            terminal_kind: "cancelled",
            cancellation_acknowledged: true,
            outcome_match: generation_outcomes(),
        }
    }

    fn sample() -> DirectCompletionSample {
        DirectCompletionSample {
            ordinal: 1,
            submission_to_generation_started_ns: 1,
            submission_to_first_decoded_output_ns: 2,
            submission_to_terminal_event_ns: 3,
            submission_to_release_ns: 4,
            prompt_tokens: 4,
            generated_tokens: 32,
            decoded_byte_count: 64,
            terminal_kind: "token_limit",
            terminal_state_matched: true,
            released_state_matched: true,
            terminal_event_matched: true,
            effective_generated_tokens_per_second: 8.0,
        }
    }

    fn lifecycle() -> LifecycleResult {
        LifecycleResult {
            ordinal: 1,
            cache_state_before_resolution: "populated",
            start_ns: 1,
            resolve_ns: 2,
            load_ns: 3,
            selected_e1_device: CUDA_IDENTITY,
            actual_loaded_e0_device: CUDA_IDENTITY,
            prepared_load: PreparedLoadEvidence {
                planned_execution_device: CUDA_IDENTITY,
                exact_final_footprint: final_footprint(),
                loading_peak_footprint: loading_peak_footprint(),
                e1_load_accepted: true,
                e0_reserved_ownership_observed: false,
                provenance: "observer prepare_load followed by E1 acceptance",
            },
            post_unload_e0_accounting_scope: "separate direct E0 fixture proof",
            unload: UnloadResult {
                duration_ns: 4,
                cancelled_requests: 0,
                loaded_model_absent: true,
                active_generation_absent: true,
                runtime_connected: true,
                backend_release_synchronized: true,
            },
            shutdown: ShutdownResult {
                duration_ns: 5,
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
            resource_checkpoints: vec![ResourceCheckpoint {
                checkpoint: "after-model-load",
                process_memory: ProcessMemory {
                    vm_rss_bytes: Some(10_000),
                    vm_hwm_bytes: Some(12_000),
                },
                whole_device_cuda_memory: Some(CudaMemoryObservation {
                    total_bytes: 16_000,
                    free_bytes: 10_000,
                    used_bytes: 6_000,
                    used_delta_from_pre_load_bytes: Some(2_000),
                }),
            }],
        }
    }

    const fn sampling() -> SamplingMetadata {
        SamplingMetadata {
            temperature: 1.0,
            top_k: 1,
            top_p: 1.0,
            min_p: 0.0,
            repetition_penalty: 1.0,
            repetition_window: 0,
            fixed_seed: 39,
        }
    }

    fn provenance() -> Provenance {
        Provenance {
            git: GitMetadata {
                head: "commit".to_owned(),
                head_tree: "tree".to_owned(),
                dirty: false,
            },
            toolchain: ToolchainMetadata {
                rust_version: "rustc".to_owned(),
                cargo_version: "cargo".to_owned(),
                llvm_version: Some("llvm".to_owned()),
                rustc_host: "x86_64-unknown-linux-gnu".to_owned(),
                build_profile: "release",
            },
            system: SystemMetadata {
                os: "linux",
                kernel: "kernel".to_owned(),
                cpu_model: Some("cpu".to_owned()),
                physical_cpu_count: Some(1),
                logical_cpu_count: Some(1),
                total_memory_bytes: Some(32_000),
                thread_environment: BTreeMap::new(),
            },
            command_mode: "external_e1_hugging_face_hub",
            network_authorized: true,
            cache_location: "repository_root_target",
            cuda_environment: Some(CudaEnvironmentMetadata {
                driver_version: "600.0".to_owned(),
                toolkit_release: "12.8".to_owned(),
                toolkit_compiler_version: "V12.8.0".to_owned(),
                build_compute_capability: "120".to_owned(),
                cuda_visible_devices: Some("0".to_owned()),
            }),
        }
    }

    fn execution() -> ExecutionMetadata {
        ExecutionMetadata {
            cuda_enabled: true,
            requested_device: CUDA_IDENTITY,
            cuda_device: Some(CudaDeviceMetadata {
                name: "NVIDIA GeForce RTX 5070 Ti".to_owned(),
                compute_capability: CudaComputeCapability {
                    major: 12,
                    minor: 0,
                },
                total_memory_bytes: 16_000,
            }),
            planned_execution_scalar: "BF16",
            actual_execution_scalar: "BF16",
            host_sampling: true,
            cuda_logits_to_host_limitation: Some("host sampling"),
            cuda_memory_observation_scope: Some("whole device"),
            cuda_context_observation: Some(CudaContextObservation {
                device_discovery_calls: 17,
                initialization_scope: "cold checkpoints; never per token",
            }),
        }
    }

    fn model() -> ModelMetadata {
        ModelMetadata {
            repository: "model",
            requested_revision: "revision",
            resolved_commit: "revision".to_owned(),
            upstream_declared_license: "apache-2.0",
            license_metadata_source: "model-card",
            engine: "Candle",
            source: "Hugging Face Hub",
            format: "Safetensors",
            architecture: "Llama",
            artifact_layout: ArtifactLayoutMetadata {
                configuration_file: "config.json",
                tokenizer_file: "tokenizer.json",
                safetensors_layout: "single_file",
                weight_files: vec!["model.safetensors"],
            },
            configuration_declared_scalar: Some("BF16"),
            observed_tensor_scalars: vec!["BF16"],
            vocabulary_size: 32_000,
            maximum_context_tokens: 2_048,
            maximum_prefill_batch: 2_048,
            cache_state_before_resolution: "populated",
        }
    }

    fn workload() -> WorkloadMetadata {
        WorkloadMetadata {
            chat_compatibility: ChatWorkloadMetadata {
                message_identifier: "chat",
                message_sha256: "hash".to_owned(),
                message_bytes: 10,
                maximum_new_tokens: 24,
                sampling: sampling(),
                termination_policy: "profile",
            },
            direct_completion: DirectCompletionWorkloadMetadata {
                prompt_identifier: "direct",
                prompt_sha256: "hash".to_owned(),
                prompt_bytes: 10,
                warmup_count: 1,
                sample_count: 3,
                maximum_new_tokens: 32,
                sampling: sampling(),
                eos_tokens: "none",
                textual_stop_sequences: "none",
            },
            cancellation: CancellationWorkloadMetadata {
                prompt_identifier: "direct",
                prompt_sha256: "hash".to_owned(),
                prompt_bytes: 10,
                maximum_new_tokens: 128,
                sampling: sampling(),
                cancellation_trigger: "progress",
                cancellation_reason: "user_requested",
            },
            lifecycle: LifecycleCounts {
                primary_full_workload_cycles: 1,
                cuda_stability_cycles: 0,
                total_cycles: 1,
            },
        }
    }

    fn primary_cycle() -> PrimaryCycleResult {
        PrimaryCycleResult {
            lifecycle: lifecycle(),
            chat_compatibility: ChatProofResult {
                submission_to_generation_started_ns: 1,
                submission_to_first_decoded_output_ns: 2,
                submission_to_terminal_event_ns: 3,
                submission_to_release_ns: 4,
                decoded_byte_count: 8,
                prompt_tokens: 4,
                generated_tokens: 2,
                terminal_kind: "end_of_sequence",
                outcome_match: generation_outcomes(),
                conversation: ConversationProof {
                    validated: true,
                    cleared: true,
                },
            },
            direct_completion: DirectCompletionResults {
                warmup: DirectCompletionWarmupResult {
                    decoded_byte_count: 64,
                    prompt_tokens: 4,
                    generated_tokens: 32,
                    terminal_kind: "token_limit",
                    clean_release: true,
                },
                samples: vec![sample()],
                summary: DirectCompletionSummary {
                    sample_count: 1,
                    median_submission_to_generation_started_ns: 1,
                    median_submission_to_first_decoded_output_ns: 2,
                    median_submission_to_terminal_event_ns: 3,
                    median_submission_to_release_ns: 4,
                    median_effective_generated_tokens_per_second: 8.0,
                },
            },
            cancellation: cancellation(),
        }
    }

    fn results() -> Results {
        Results {
            primary_cycle: primary_cycle(),
            cuda_stability_cycles: Vec::new(),
            stability_summary: StabilitySummary {
                primary_cycle_count: 1,
                cuda_stability_cycle_count: 0,
                post_unload_cuda_used_bytes: vec![4_000],
                post_owner_drop_cuda_used_bytes: vec![4_000],
                post_unload_cuda_delta_from_pre_load_bytes: vec![0],
                post_owner_drop_cuda_delta_from_pre_load_bytes: vec![0],
                strict_monotonic_retained_growth_observed: false,
                max_retained_cuda_delta_bytes: Some(0),
                assessment: "finite whole-device observation".to_owned(),
            },
            temporary_workspace_removed: true,
        }
    }

    fn fixture_report() -> ExternalBaselineReport {
        ExternalBaselineReport {
            schema_version: SCHEMA_VERSION,
            provenance: provenance(),
            execution: execution(),
            model: model(),
            workload: workload(),
            results: results(),
        }
    }

    fn assert_no_prohibited_keys(value: &Value) {
        match value {
            Value::Object(fields) => {
                for (key, nested) in fields {
                    assert!(
                        ![
                            "scalar_type",
                            "source_scalar",
                            "execution_scalar",
                            "execution_dtype",
                            "independent_public_plan",
                            "decoded_text",
                            "generated_text",
                            "token_ids",
                            "generated_token_ids",
                        ]
                        .contains(&key.as_str()),
                        "prohibited serialized field {key}"
                    );
                    assert_no_prohibited_keys(nested);
                }
            }
            Value::Array(values) => {
                for nested in values {
                    assert_no_prohibited_keys(nested);
                }
            }
            _ => {}
        }
    }

    fn value_at<'a>(value: &'a Value, path: &[&str]) -> Result<&'a Value, String> {
        let mut current = value;
        for key in path {
            current = current
                .get(*key)
                .ok_or_else(|| format!("serialized report omitted key {key:?} in {path:?}"))?;
        }
        Ok(current)
    }

    #[test]
    fn schema_five_serializes_declared_observed_planned_and_actual_facts() -> Result<(), String> {
        let value = serde_json::to_value(fixture_report()).map_err(|error| error.to_string())?;
        assert_eq!(value_at(&value, &["schema_version"])?.as_u64(), Some(5));
        assert_eq!(
            value_at(&value, &["model", "configuration_declared_scalar"])?.as_str(),
            Some("BF16")
        );
        let observed = value_at(&value, &["model", "observed_tensor_scalars"])?
            .as_array()
            .ok_or_else(|| "observed tensor scalars were not an array".to_owned())?;
        assert_eq!(observed.first().and_then(Value::as_str), Some("BF16"));
        assert_eq!(
            value_at(&value, &["execution", "planned_execution_scalar"])?.as_str(),
            Some("BF16")
        );
        assert_eq!(
            value_at(&value, &["execution", "actual_execution_scalar"])?.as_str(),
            Some("BF16")
        );
        assert_eq!(
            value_at(&value, &["execution", "requested_device", "kind"])?.as_str(),
            Some("cuda")
        );
        assert_eq!(
            value_at(
                &value,
                &[
                    "results",
                    "primary_cycle",
                    "lifecycle",
                    "selected_e1_device",
                    "ordinal"
                ],
            )?
            .as_u64(),
            Some(0)
        );
        assert_eq!(
            value_at(
                &value,
                &[
                    "results",
                    "primary_cycle",
                    "lifecycle",
                    "actual_loaded_e0_device",
                    "ordinal",
                ],
            )?
            .as_u64(),
            Some(0)
        );
        assert_eq!(
            value_at(
                &value,
                &[
                    "execution",
                    "cuda_context_observation",
                    "device_discovery_calls",
                ],
            )?
            .as_u64(),
            Some(17)
        );
        assert_no_prohibited_keys(&value);
        Ok(())
    }

    #[test]
    fn plan_reserved_ownership_rss_and_whole_device_cuda_remain_distinct() -> Result<(), String> {
        let value = serde_json::to_value(fixture_report()).map_err(|error| error.to_string())?;
        let lifecycle = value_at(&value, &["results", "primary_cycle", "lifecycle"])?;
        let prepared = value_at(lifecycle, &["prepared_load"])?;
        let checkpoints = value_at(lifecycle, &["resource_checkpoints"])?
            .as_array()
            .ok_or_else(|| "resource checkpoints were not an array".to_owned())?;
        let first = checkpoints
            .first()
            .ok_or_else(|| "resource checkpoints were empty".to_owned())?;
        let process = value_at(first, &["process_memory"])?;
        let whole_device = value_at(first, &["whole_device_cuda_memory"])?;
        assert_eq!(
            value_at(prepared, &["exact_final_footprint", "device_weight_bytes"])?.as_u64(),
            Some(2_200)
        );
        assert_eq!(
            value_at(prepared, &["loading_peak_footprint", "host_working_bytes"])?.as_u64(),
            Some(2_200)
        );
        assert_eq!(
            value_at(prepared, &["e0_reserved_ownership_observed"])?.as_bool(),
            Some(false)
        );
        assert_eq!(value_at(process, &["vm_rss_bytes"])?.as_u64(), Some(10_000));
        assert_eq!(
            value_at(whole_device, &["used_bytes"])?.as_u64(),
            Some(6_000)
        );
        assert!(prepared.get("vm_rss_bytes").is_none());
        assert!(prepared.get("used_bytes").is_none());
        assert!(process.get("device_weight_bytes").is_none());
        assert!(whole_device.get("device_weight_bytes").is_none());
        Ok(())
    }

    #[test]
    fn external_schema_history_remains_documented_without_a_legacy_parser() {
        let readme = include_str!("../../README.md");
        assert!(readme.contains("**External schema 1 (historical):**"));
        assert!(readme.contains("**External schema 2 (historical):**"));
        assert!(readme.contains("**External schema 3 (historical):**"));
        assert!(readme.contains("**External schema 4 (historical):**"));
        assert!(readme.contains("**External schema 5 (current):**"));
    }
}
