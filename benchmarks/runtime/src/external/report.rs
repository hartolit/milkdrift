//! Device-neutral JSON report schema for the external product baseline.

use serde::Serialize;

use crate::memory::ProcessMemory;
use crate::report::{DeviceIdentity, GitMetadata, SystemMetadata, ToolchainMetadata};

pub(super) const SCHEMA_VERSION: u32 = 6;

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
    pub(super) actual_execution_scalar: &'static str,
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
    pub(super) unload: UnloadResult,
    pub(super) shutdown: ShutdownResult,
    pub(super) resource_checkpoints: Vec<ResourceCheckpoint>,
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

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub(super) struct UnloadResult {
    pub(super) duration_ns: u64,
    pub(super) cancelled_requests: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub(super) struct ShutdownResult {
    pub(super) duration_ns: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub(super) struct StabilitySummary {
    pub(super) strict_monotonic_retained_growth_observed: bool,
    pub(super) max_retained_cuda_delta_bytes: Option<i64>,
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use serde_json::Value;

    use super::*;
    use crate::report::{DeviceIdentity, GitMetadata, SystemMetadata, ToolchainMetadata};

    const CUDA_IDENTITY: DeviceIdentity = DeviceIdentity {
        kind: "cuda",
        id: 0,
        ordinal: Some(0),
    };

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
            unload: UnloadResult {
                duration_ns: 4,
                cancelled_requests: 0,
            },
            shutdown: ShutdownResult { duration_ns: 5 },
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
            actual_execution_scalar: "BF16",
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
            },
            direct_completion: DirectCompletionResults {
                warmup: DirectCompletionWarmupResult {
                    decoded_byte_count: 64,
                    prompt_tokens: 4,
                    generated_tokens: 32,
                    terminal_kind: "token_limit",
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

    fn fixture_report() -> ExternalBaselineReport {
        ExternalBaselineReport {
            schema_version: SCHEMA_VERSION,
            provenance: provenance(),
            execution: execution(),
            model: model(),
            workload: workload(),
            results: Results {
                primary_cycle: primary_cycle(),
                cuda_stability_cycles: Vec::new(),
                stability_summary: StabilitySummary {
                    strict_monotonic_retained_growth_observed: false,
                    max_retained_cuda_delta_bytes: Some(0),
                },
            },
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

    fn assert_absent_keys(value: &Value, prohibited: &[&str]) {
        match value {
            Value::Object(fields) => {
                for (key, nested) in fields {
                    assert!(
                        !prohibited.contains(&key.as_str()),
                        "prohibited serialized field {key}"
                    );
                    assert_absent_keys(nested, prohibited);
                }
            }
            Value::Array(values) => {
                for nested in values {
                    assert_absent_keys(nested, prohibited);
                }
            }
            _ => {}
        }
    }

    #[test]
    fn schema_six_serializes_public_e1_facts_and_raw_measurements() -> Result<(), String> {
        let value = serde_json::to_value(fixture_report()).map_err(|error| error.to_string())?;
        assert_eq!(value_at(&value, &["schema_version"])?.as_u64(), Some(6));
        assert_eq!(
            value_at(&value, &["model", "configuration_declared_scalar"])?.as_str(),
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
        let lifecycle = value_at(&value, &["results", "primary_cycle", "lifecycle"])?;
        assert_eq!(
            value_at(lifecycle, &["selected_e1_device", "ordinal"])?.as_u64(),
            Some(0)
        );
        assert_eq!(
            value_at(lifecycle, &["actual_loaded_e0_device", "ordinal"])?.as_u64(),
            Some(0)
        );
        assert_eq!(value_at(lifecycle, &["load_ns"])?.as_u64(), Some(3));
        let checkpoints = value_at(lifecycle, &["resource_checkpoints"])?
            .as_array()
            .ok_or_else(|| "resource checkpoints were not an array".to_owned())?;
        let first = checkpoints
            .first()
            .ok_or_else(|| "resource checkpoints were empty".to_owned())?;
        assert_eq!(
            value_at(first, &["process_memory", "vm_rss_bytes"])?.as_u64(),
            Some(10_000)
        );
        assert_eq!(
            value_at(first, &["whole_device_cuda_memory", "used_bytes"])?.as_u64(),
            Some(6_000)
        );
        assert_eq!(
            value_at(
                &value,
                &[
                    "results",
                    "primary_cycle",
                    "cancellation",
                    "cancellation_submission",
                    "to_acknowledgement_ns",
                ],
            )?
            .as_u64(),
            Some(1)
        );
        Ok(())
    }

    #[test]
    fn schema_six_omits_independent_planning_and_tautological_success_payload() -> Result<(), String>
    {
        let value = serde_json::to_value(fixture_report()).map_err(|error| error.to_string())?;
        assert_absent_keys(
            &value,
            &[
                "observed_tensor_scalars",
                "planned_execution_scalar",
                "prepared_load",
                "planned_execution_device",
                "exact_final_footprint",
                "loading_peak_footprint",
                "e1_load_accepted",
                "e0_reserved_ownership_observed",
                "post_unload_e0_accounting_scope",
                "command_mode",
                "network_authorized",
                "host_sampling",
                "cuda_logits_to_host_limitation",
                "cuda_memory_observation_scope",
                "cuda_context_observation",
                "temporary_workspace_removed",
                "cancellation_acknowledged",
                "outcome_match",
                "conversation",
                "clean_release",
                "terminal_state_matched",
                "released_state_matched",
                "terminal_event_matched",
                "shutdown_returned_cleanly",
                "workers",
                "ownership",
                "loaded_model_absent",
                "active_generation_absent",
                "runtime_connected",
                "backend_release_synchronized",
                "assessment",
                "post_unload_cuda_used_bytes",
                "post_owner_drop_cuda_used_bytes",
                "post_unload_cuda_delta_from_pre_load_bytes",
                "post_owner_drop_cuda_delta_from_pre_load_bytes",
            ],
        );
        Ok(())
    }

    #[test]
    fn external_schema_history_remains_documented_without_a_legacy_parser() {
        let readme = include_str!("../../README.md");
        assert!(readme.contains("**External schema 1 (historical):**"));
        assert!(readme.contains("**External schema 2 (historical):**"));
        assert!(readme.contains("**External schema 3 (historical):**"));
        assert!(readme.contains("**External schema 4 (historical):**"));
        assert!(readme.contains("**External schema 5 (historical):**"));
        assert!(readme.contains("**External schema 6 (current):**"));
    }
}
