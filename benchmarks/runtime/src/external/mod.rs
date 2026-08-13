//! External real-product CPU/CUDA baseline policy, execution, and report assembly.

mod cli;
mod generation;
mod lifecycle;
mod model;
mod observation;
mod report;

use std::ffi::OsString;

use crate::error::{BenchmarkError, BenchmarkResult};
use crate::evidence::application_scalar_type_label;
use crate::metadata;
use crate::support::digest::sha256_hex;
use crate::workspace::{TemporaryWorkspace, repository_root};

use observation::DeviceObserver;
use report::{
    ArtifactLayoutMetadata, CancellationWorkloadMetadata, ChatWorkloadMetadata,
    DirectCompletionWorkloadMetadata, ExternalBaselineReport, LifecycleCounts, ModelMetadata,
    Provenance, Results, WorkloadMetadata,
};

pub(crate) use cli::{Action, HELP};

pub(crate) const UPSTREAM_DECLARED_LICENSE: &str = "apache-2.0";
pub(crate) const LICENSE_METADATA_SOURCE: &str = "https://huggingface.co/TinyLlama/TinyLlama-1.1B-Chat-v1.0/raw/fe8a4ea1ffedaf415f4da2f062534de366a451e6/README.md";
const PRIMARY_CYCLE_COUNT: u32 = 1;

pub(crate) fn parse(arguments: impl IntoIterator<Item = OsString>) -> BenchmarkResult<Action> {
    cli::parse(arguments)
}

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

    let cuda_stability_cycle_count = u32::try_from(lifecycle.cuda_stability_cycles.len())
        .map_err(|_| BenchmarkError::new("CUDA stability-cycle count conversion failed"))?;
    let execution = observer.execution_metadata(application_scalar_type_label(
        lifecycle.actual_execution_scalar,
    ));
    let model = model_metadata(&lifecycle, initial_cache_state);
    let workload = workload_metadata(cuda_stability_cycle_count)?;

    Ok(ExternalBaselineReport {
        schema_version: report::SCHEMA_VERSION,
        provenance: Provenance {
            git: environment.git,
            toolchain: environment.toolchain,
            system: environment.system,
            cache_location: configuration.cache_location.label(),
            cuda_environment,
        },
        execution,
        model,
        workload,
        results: Results {
            primary_cycle: lifecycle.primary_cycle,
            cuda_stability_cycles: lifecycle.cuda_stability_cycles,
            stability_summary: lifecycle.stability_summary,
        },
    })
}

fn model_metadata(
    lifecycle: &lifecycle::LifecycleEvidence,
    initial_cache_state: cli::CacheState,
) -> ModelMetadata {
    ModelMetadata {
        repository: model::MODEL_REPOSITORY,
        requested_revision: model::MODEL_REVISION,
        resolved_commit: lifecycle.resolved_commit.clone(),
        upstream_declared_license: UPSTREAM_DECLARED_LICENSE,
        license_metadata_source: LICENSE_METADATA_SOURCE,
        engine: "Candle",
        source: "Hugging Face Hub",
        format: "Safetensors",
        architecture: model::MODEL_ARCHITECTURE,
        artifact_layout: ArtifactLayoutMetadata {
            configuration_file: "config.json",
            tokenizer_file: "tokenizer.json",
            safetensors_layout: "single_file",
            weight_files: vec!["model.safetensors"],
        },
        configuration_declared_scalar: lifecycle
            .configuration_declared_scalar
            .map(application_scalar_type_label),
        vocabulary_size: lifecycle.vocabulary_size,
        maximum_context_tokens: lifecycle.maximum_context_tokens,
        maximum_prefill_batch: lifecycle.maximum_prefill_batch,
        cache_state_before_resolution: initial_cache_state.label(),
    }
}

fn workload_metadata(cuda_stability_cycles: u32) -> BenchmarkResult<WorkloadMetadata> {
    let sampling = generation::sampling_metadata();
    let total_cycles = PRIMARY_CYCLE_COUNT
        .checked_add(cuda_stability_cycles)
        .ok_or_else(|| BenchmarkError::new("external lifecycle-cycle count overflowed"))?;
    Ok(WorkloadMetadata {
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

fn device_label(device: crate::report::DeviceIdentity) -> &'static str {
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

#[cfg(test)]
mod tests {
    use super::report::{
        CancellationResult, CancellationSubmissionTimings, DirectCompletionSample,
        GenerationSubmissionTimings,
    };

    #[test]
    fn external_generation_report_payload_excludes_text_and_token_identifiers() -> Result<(), String>
    {
        let payload = (
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
}
