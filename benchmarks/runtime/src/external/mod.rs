//! External real-product CPU baseline policy, execution, and report assembly.

mod cli;
mod lifecycle;

use std::ffi::OsString;

use sha2::{Digest, Sha256};

use crate::error::{BenchmarkError, BenchmarkResult};
use crate::metadata;
use crate::report::{
    ChatWorkloadMetadata, DirectCompletionWorkloadMetadata, EXTERNAL_SCHEMA_VERSION,
    ExternalBaselineReport, ExternalModelMetadata, ExternalProvenance, ExternalWorkloadMetadata,
    SamplingMetadata,
};
use crate::workspace::{TemporaryWorkspace, repository_root};

pub(crate) use cli::{Action, HELP};

pub(crate) const UPSTREAM_DECLARED_LICENSE: &str = "apache-2.0";
pub(crate) const LICENSE_METADATA_SOURCE: &str = "https://huggingface.co/TinyLlama/TinyLlama-1.1B-Chat-v1.0/raw/fe8a4ea1ffedaf415f4da2f062534de366a451e6/README.md";
const COMMAND_MODE: &str = "external_cpu_hugging_face_hub";

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
    let cache_state = cli::inspect_cache_state(&configuration.cache_directory)?;
    eprintln!(
        "external cache is {} and resolves {}",
        cache_state.label(),
        configuration.cache_location.label()
    );

    let mut workspace = TemporaryWorkspace::create("external-baseline")?;
    let lifecycle_result = lifecycle::run(
        workspace.database_path("application", 1),
        &configuration.cache_directory,
    );
    let workspace_cleanup = workspace.cleanup();
    let mut lifecycle = match lifecycle_result {
        Ok(evidence) => {
            workspace_cleanup?;
            evidence
        }
        Err(error) => return Err(error.with_cleanup(workspace_cleanup)),
    };
    lifecycle.results.shutdown.temporary_workspace_removed = true;

    let sampling = SamplingMetadata {
        temperature: 1.0,
        top_k: 1,
        top_p: 1.0,
        min_p: 0.0,
        repetition_penalty: 1.0,
        repetition_window: 0,
        fixed_seed: lifecycle::FIXED_SEED,
    };
    Ok(ExternalBaselineReport {
        schema_version: EXTERNAL_SCHEMA_VERSION,
        provenance: ExternalProvenance {
            git: environment.git,
            toolchain: environment.toolchain,
            system: environment.system,
            command_mode: COMMAND_MODE,
            network_authorized: true,
            cache_location: configuration.cache_location.label(),
        },
        model: ExternalModelMetadata {
            repository: lifecycle::MODEL_REPOSITORY,
            requested_revision: lifecycle::MODEL_REVISION,
            resolved_commit: lifecycle.resolved_commit,
            upstream_declared_license: UPSTREAM_DECLARED_LICENSE,
            license_metadata_source: LICENSE_METADATA_SOURCE,
            engine: "Candle",
            source: "Hugging Face Hub",
            device: "CPU",
            format: "Safetensors",
            architecture: lifecycle::MODEL_ARCHITECTURE,
            scalar_type: lifecycle.scalar_type,
            vocabulary_size: lifecycle.vocabulary_size,
            maximum_context_tokens: lifecycle.maximum_context_tokens,
            maximum_prefill_batch: lifecycle.maximum_prefill_batch,
            cache_state_before_resolution: cache_state.label(),
        },
        workload: ExternalWorkloadMetadata {
            chat_compatibility: ChatWorkloadMetadata {
                message_identifier: lifecycle::CHAT_MESSAGE_IDENTIFIER,
                message_sha256: sha256_hex(lifecycle::CHAT_MESSAGE.as_bytes()),
                message_bytes: byte_len(lifecycle::CHAT_MESSAGE)?,
                maximum_new_tokens: lifecycle::CHAT_MAXIMUM_NEW_TOKENS,
                sampling,
                termination_policy: "exact_chat_compatibility_profile",
            },
            direct_completion: DirectCompletionWorkloadMetadata {
                prompt_identifier: lifecycle::DIRECT_COMPLETION_PROMPT_IDENTIFIER,
                prompt_sha256: sha256_hex(lifecycle::DIRECT_COMPLETION_PROMPT.as_bytes()),
                prompt_bytes: byte_len(lifecycle::DIRECT_COMPLETION_PROMPT)?,
                warmup_count: lifecycle::WARMUP_COUNT,
                sample_count: lifecycle::SAMPLE_COUNT,
                maximum_new_tokens: lifecycle::DIRECT_MAXIMUM_NEW_TOKENS,
                sampling,
                eos_tokens: "none",
                textual_stop_sequences: "none",
            },
        },
        results: lifecycle.results,
    })
}

pub(crate) fn print_human_summary(report: &ExternalBaselineReport) {
    let summary = &report.results.direct_completion.summary;
    eprintln!(
        "external CPU baseline complete: load={} ns, median first decoded output={} ns, median release={} ns, median effective throughput={:.3} generated tokens/s, unload={} ns, shutdown={} ns",
        report.results.load_ns,
        summary.median_submission_to_first_decoded_output_ns,
        summary.median_submission_to_release_ns,
        summary.median_effective_generated_tokens_per_second,
        report.results.unload.duration_ns,
        report.results.shutdown.duration_ns,
    );
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
    use crate::memory::ProcessMemory;
    use crate::report::{
        ChatProofResult, ConversationProof, DirectCompletionSample, GenerationOutcomeMatch,
    };

    #[test]
    fn external_generation_report_payload_excludes_text_and_token_identifiers() -> Result<(), String>
    {
        let payload = (
            ChatProofResult {
                decoded_byte_count: 12,
                prompt_tokens: 9,
                generated_tokens: 3,
                terminal_kind: "end_of_sequence",
                outcome_match: GenerationOutcomeMatch {
                    terminal_state_matched: true,
                    released_state_matched: true,
                    terminal_event_matched: true,
                },
                conversation: ConversationProof {
                    validated: true,
                    cleared: true,
                },
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
                process_memory_after_release: ProcessMemory::default(),
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
