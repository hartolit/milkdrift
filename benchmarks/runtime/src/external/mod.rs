//! External real-product CPU/CUDA baseline policy, execution, and report assembly.

mod cli;
mod generation;
mod lifecycle;
mod model;
mod observation;
mod report;

use std::ffi::OsString;

use sha2::{Digest, Sha256};

use crate::error::{BenchmarkError, BenchmarkResult};
use crate::metadata;
use crate::workspace::{TemporaryWorkspace, repository_root};

use observation::DeviceObserver;
use report::{
    CancellationWorkloadMetadata, ChatWorkloadMetadata, DirectCompletionWorkloadMetadata,
    ExternalBaselineReport, LifecycleCounts, ModelMetadata, Provenance, Results, SamplingMetadata,
    WorkloadMetadata,
};

pub(crate) use cli::{Action, HELP};

pub(crate) const UPSTREAM_DECLARED_LICENSE: &str = "apache-2.0";
pub(crate) const LICENSE_METADATA_SOURCE: &str = "https://huggingface.co/TinyLlama/TinyLlama-1.1B-Chat-v1.0/raw/fe8a4ea1ffedaf415f4da2f062534de366a451e6/README.md";
const COMMAND_MODE: &str = "external_e1_hugging_face_hub";
const PRIMARY_CYCLE_COUNT: u32 = 1;
const CUDA_STABILITY_CYCLE_COUNT: u32 = 2;

pub(crate) fn parse(arguments: impl IntoIterator<Item = OsString>) -> BenchmarkResult<Action> {
    cli::parse(arguments)
}

#[expect(
    clippy::too_many_lines,
    reason = "one cold-path function assembles the complete versioned external evidence record"
)]
pub(crate) fn run_configuration(
    configuration: &cli::Configuration,
) -> BenchmarkResult<ExternalBaselineReport> {
    let repository_root = repository_root()?;
    let configuration = cli::validate_cache_directory(configuration, &repository_root)?;
    let environment = metadata::collect(&repository_root)?;
    if environment.git.dirty {
        return Err(BenchmarkError::new(
            "external baseline requires a clean Git worktree before runtime startup",
        ));
    }

    let initial_cache_state = cli::inspect_cache_state(&configuration.cache_directory)?;
    eprintln!(
        "external cache is {} and resolves {}",
        initial_cache_state.label(),
        configuration.cache_location.label()
    );

    let mut observer = DeviceObserver::new(configuration.device)?;
    let cuda_environment = observer.collect_cuda_environment()?;
    let mut workspace = TemporaryWorkspace::create("external-baseline")?;
    let lifecycle_result = lifecycle::run(
        &workspace,
        &configuration.cache_directory,
        configuration.device,
        &mut observer,
    );
    let workspace_cleanup = workspace.cleanup();
    let lifecycle = match lifecycle_result {
        Ok(evidence) => {
            workspace_cleanup?;
            evidence
        }
        Err(error) => return Err(error.with_cleanup(workspace_cleanup)),
    };

    let sampling = SamplingMetadata {
        temperature: 1.0,
        top_k: 1,
        top_p: 1.0,
        min_p: 0.0,
        repetition_penalty: 1.0,
        repetition_window: 0,
        fixed_seed: generation::FIXED_SEED,
    };
    let cuda_stability_cycles = match configuration.device {
        cli::RequestedDevice::Cpu => 0,
        cli::RequestedDevice::Cuda0 => CUDA_STABILITY_CYCLE_COUNT,
    };
    let total_cycles = PRIMARY_CYCLE_COUNT
        .checked_add(cuda_stability_cycles)
        .ok_or_else(|| BenchmarkError::new("external lifecycle-cycle count overflowed"))?;

    Ok(ExternalBaselineReport {
        schema_version: report::SCHEMA_VERSION,
        provenance: Provenance {
            git: environment.git,
            toolchain: environment.toolchain,
            system: environment.system,
            command_mode: COMMAND_MODE,
            network_authorized: true,
            cache_location: configuration.cache_location.label(),
            cuda_environment,
        },
        execution: observer.execution_metadata(lifecycle.execution_dtype),
        model: ModelMetadata {
            repository: model::MODEL_REPOSITORY,
            requested_revision: model::MODEL_REVISION,
            resolved_commit: lifecycle.resolved_commit,
            upstream_declared_license: UPSTREAM_DECLARED_LICENSE,
            license_metadata_source: LICENSE_METADATA_SOURCE,
            engine: "Candle",
            source: "Hugging Face Hub",
            format: "Safetensors",
            architecture: model::MODEL_ARCHITECTURE,
            source_scalar: lifecycle.source_scalar,
            vocabulary_size: lifecycle.vocabulary_size,
            maximum_context_tokens: lifecycle.maximum_context_tokens,
            maximum_prefill_batch: lifecycle.maximum_prefill_batch,
            cache_state_before_resolution: initial_cache_state.label(),
        },
        workload: WorkloadMetadata {
            chat_compatibility: ChatWorkloadMetadata {
                message_identifier: generation::CHAT_MESSAGE_IDENTIFIER,
                message_sha256: sha256_hex(generation::CHAT_MESSAGE.as_bytes()),
                message_bytes: byte_len(generation::CHAT_MESSAGE)?,
                maximum_new_tokens: generation::CHAT_MAXIMUM_NEW_TOKENS,
                sampling,
                termination_policy: "exact_chat_compatibility_profile",
            },
            direct_completion: DirectCompletionWorkloadMetadata {
                prompt_identifier: generation::DIRECT_COMPLETION_PROMPT_IDENTIFIER,
                prompt_sha256: sha256_hex(generation::DIRECT_COMPLETION_PROMPT.as_bytes()),
                prompt_bytes: byte_len(generation::DIRECT_COMPLETION_PROMPT)?,
                warmup_count: generation::WARMUP_COUNT,
                sample_count: generation::SAMPLE_COUNT,
                maximum_new_tokens: generation::DIRECT_MAXIMUM_NEW_TOKENS,
                sampling,
                eos_tokens: "none",
                textual_stop_sequences: "none",
            },
            cancellation: CancellationWorkloadMetadata {
                prompt_identifier: generation::DIRECT_COMPLETION_PROMPT_IDENTIFIER,
                prompt_sha256: sha256_hex(generation::DIRECT_COMPLETION_PROMPT.as_bytes()),
                prompt_bytes: byte_len(generation::DIRECT_COMPLETION_PROMPT)?,
                maximum_new_tokens: generation::CANCELLATION_MAXIMUM_NEW_TOKENS,
                sampling,
                cancellation_trigger: "GenerationStarted plus first non-empty decoded output",
                cancellation_reason: "user_requested",
            },
            lifecycle: LifecycleCounts {
                primary_full_workload_cycles: PRIMARY_CYCLE_COUNT,
                cuda_stability_cycles,
                total_cycles,
            },
        },
        results: Results {
            primary_cycle: lifecycle.primary_cycle,
            cuda_stability_cycles: lifecycle.cuda_stability_cycles,
            stability_summary: lifecycle.stability_summary,
            temporary_workspace_removed: true,
        },
    })
}

pub(crate) fn print_human_summary(report: &ExternalBaselineReport) {
    let primary = &report.results.primary_cycle;
    let summary = &primary.direct_completion.summary;
    eprintln!(
        "external {} baseline complete: cycles={}, load={} ns, median first decoded output={} ns, median release={} ns, median effective throughput={:.3} generated tokens/s, cancellation release={} ns, unload={} ns, shutdown={} ns",
        device_label(report.execution.requested_device),
        report.workload.lifecycle.total_cycles,
        primary.lifecycle.load_ns,
        summary.median_submission_to_first_decoded_output_ns,
        summary.median_submission_to_release_ns,
        summary.median_effective_generated_tokens_per_second,
        primary.cancellation.cancellation_submission.to_release_ns,
        primary.lifecycle.unload.duration_ns,
        primary.lifecycle.shutdown.duration_ns,
    );
}

fn device_label(device: report::DeviceIdentity) -> &'static str {
    match (device.kind, device.ordinal) {
        ("cpu", None) => "CPU",
        ("cuda", Some(0)) => "CUDA 0",
        _ => "unsupported device",
    }
}

fn byte_len(value: &str) -> BenchmarkResult<u64> {
    u64::try_from(value.len())
        .map_err(|_| BenchmarkError::new("workload byte length conversion to u64 failed"))
}

fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut encoded = String::with_capacity(digest.len().saturating_mul(2));
    for byte in digest {
        encoded.push(hex_digit(byte >> 4));
        encoded.push(hex_digit(byte & 0x0f));
    }
    encoded
}

fn hex_digit(value: u8) -> char {
    match value {
        0..=9 => char::from(b'0' + value),
        10..=15 => char::from(b'a' + (value - 10)),
        _ => '?',
    }
}

#[cfg(test)]
mod tests {
    use super::report::{
        CancellationResult, CancellationSubmissionTimings, ConversationProof,
        DirectCompletionSample, GenerationOutcomeMatch, GenerationSubmissionTimings,
    };

    #[test]
    fn external_generation_report_payload_excludes_text_and_token_identifiers() -> Result<(), String>
    {
        let outcomes = GenerationOutcomeMatch {
            terminal_state_matched: true,
            released_state_matched: true,
            terminal_event_matched: true,
        };
        let payload = (
            ConversationProof {
                validated: true,
                cleared: true,
            },
            DirectCompletionSample {
                ordinal: 1,
                submission_to_generation_started_ns: 1,
                submission_to_first_decoded_output_ns: 2,
                submission_to_terminal_event_ns: 3,
                submission_to_release_ns: 4,
                prompt_tokens: 5,
                generated_tokens: 32,
                decoded_byte_count: 64,
                terminal_kind: "token_limit",
                terminal_state_matched: true,
                released_state_matched: true,
                terminal_event_matched: true,
                effective_generated_tokens_per_second: 8.0,
            },
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
                decoded_byte_count: 12,
                prompt_tokens: 5,
                generated_tokens: 2,
                terminal_kind: "cancelled",
                cancellation_acknowledged: true,
                outcome_match: outcomes,
            },
        );
        let json = serde_json::to_string(&payload).map_err(|error| error.to_string())?;
        for prohibited in [
            "decoded_text",
            "generated_text",
            "token_ids",
            "generated_token_ids",
        ] {
            assert!(!json.contains(prohibited), "unexpected field {prohibited}");
        }
        Ok(())
    }

    #[test]
    fn prompt_hash_uses_standard_sha256_encoding() {
        assert_eq!(
            super::sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }
}
