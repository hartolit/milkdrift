//! Fixed prompts, settings, and workload ordering for external E1 generation.

use std::time::Instant;

use application_runtime::{
    ApplicationRuntime, GenerationSeed, GenerationSettings, LoadedModel, SamplingConfig,
};

use super::observer::{GenerationEvidence, drive_generation};
use super::summary;
use super::validation::{self, GenerationExpectation};
use crate::error::{BenchmarkError, BenchmarkResult};
use crate::external::report::{
    CancellationResult, ChatProofResult, DirectCompletionResults, DirectCompletionSample,
};

pub(in crate::external) const CHAT_MESSAGE: &str =
    "Reply with one short sentence confirming that local inference is working.";
pub(in crate::external) const CHAT_MESSAGE_IDENTIFIER: &str =
    "tinyllama-local-inference-chat-proof-v1";
pub(in crate::external) const DIRECT_COMPLETION_PROMPT: &str =
    "The following is a concise explanation of deterministic resource cleanup in systems software:";
pub(in crate::external) const DIRECT_COMPLETION_PROMPT_IDENTIFIER: &str =
    "deterministic-resource-cleanup-completion-v1";
pub(in crate::external) const CHAT_MAXIMUM_NEW_TOKENS: u32 = 24;
pub(in crate::external) const DIRECT_MAXIMUM_NEW_TOKENS: u32 = 32;
pub(in crate::external) const CANCELLATION_MAXIMUM_NEW_TOKENS: u32 = 128;
pub(in crate::external) const WARMUP_COUNT: u32 = 1;
pub(in crate::external) const SAMPLE_COUNT: u32 = 3;
pub(in crate::external) const TEMPERATURE: f32 = 1.0;
pub(in crate::external) const TOP_K: u32 = 1;
pub(in crate::external) const TOP_P: f32 = 1.0;
pub(in crate::external) const MIN_P: f32 = 0.0;
pub(in crate::external) const REPETITION_PENALTY: f32 = 1.0;
pub(in crate::external) const REPETITION_WINDOW: u32 = 0;
pub(in crate::external) const FIXED_SEED: u64 = 39;

const AFTER_CHAT_RELEASE_CHECKPOINT: &str = "after-chat-release";
const AFTER_WARMUP_RELEASE_CHECKPOINT: &str = "after-warmup-release";
const PRIMARY_DIRECT_RELEASE_CHECKPOINTS: [&str; 3] = [
    "after-direct-sample-1-release",
    "after-direct-sample-2-release",
    "after-direct-sample-3-release",
];
const AFTER_STABILITY_DIRECT_RELEASE_CHECKPOINT: &str = "after-stability-direct-release";
const BEFORE_CANCELLATION_REQUEST_CHECKPOINT: &str = "before-cancellation-request";
const AFTER_CANCELLATION_RELEASE_CHECKPOINT: &str = "after-cancellation-release";

pub(in crate::external) struct PrimaryWorkloadEvidence {
    pub(in crate::external) chat_compatibility: ChatProofResult,
    pub(in crate::external) direct_completion: DirectCompletionResults,
    pub(in crate::external) cancellation: CancellationResult,
}

pub(in crate::external) fn run_primary_workload(
    runtime: &mut ApplicationRuntime,
    loaded: &LoadedModel,
    observe: &mut impl FnMut(&'static str) -> BenchmarkResult,
) -> BenchmarkResult<PrimaryWorkloadEvidence> {
    eprintln!("running the exact compatible-chat proof");
    let chat_compatibility = run_chat_proof(runtime, loaded, observe)?;
    observe(AFTER_CHAT_RELEASE_CHECKPOINT)?;

    eprintln!("running one controlled direct-completion warmup");
    let warmup_evidence = run_direct_completion(
        runtime,
        GenerationExpectation::direct_token_limit(DIRECT_MAXIMUM_NEW_TOKENS),
        None,
        observe,
    )?;
    let warmup = summary::warmup_record(&warmup_evidence);
    observe(AFTER_WARMUP_RELEASE_CHECKPOINT)?;

    let expected_sample_count = usize::try_from(SAMPLE_COUNT)
        .map_err(|_| BenchmarkError::new("external sample count conversion to usize failed"))?;
    if PRIMARY_DIRECT_RELEASE_CHECKPOINTS.len() != expected_sample_count {
        return Err(BenchmarkError::new(
            "direct-completion checkpoint labels did not match the configured sample count",
        ));
    }

    eprintln!("running three sequential controlled direct-completion samples");
    let mut samples = Vec::new();
    samples
        .try_reserve_exact(expected_sample_count)
        .map_err(|error| BenchmarkError::new(format!("sample allocation failed: {error}")))?;
    for (index, checkpoint) in PRIMARY_DIRECT_RELEASE_CHECKPOINTS
        .iter()
        .copied()
        .enumerate()
    {
        let ordinal = u32::try_from(index.saturating_add(1)).map_err(|_| {
            BenchmarkError::new("direct-completion sample ordinal conversion failed")
        })?;
        eprintln!("running direct-completion sample {ordinal} of {SAMPLE_COUNT}");
        let evidence = run_direct_completion(
            runtime,
            GenerationExpectation::direct_token_limit(DIRECT_MAXIMUM_NEW_TOKENS),
            None,
            observe,
        )?;
        let sample = summary::sample_record(ordinal, &evidence)?;
        observe(checkpoint)?;
        samples.push(sample);
    }
    let direct_summary = summary::summarize_samples(&samples, SAMPLE_COUNT)?;

    eprintln!("running cancellation after observable generation progress");
    let cancellation = run_cancellation(runtime, observe)?;

    Ok(PrimaryWorkloadEvidence {
        chat_compatibility,
        direct_completion: DirectCompletionResults {
            warmup,
            samples,
            summary: direct_summary,
        },
        cancellation,
    })
}

pub(in crate::external) fn run_stability_workload(
    runtime: &mut ApplicationRuntime,
    observe: &mut impl FnMut(&'static str) -> BenchmarkResult,
) -> BenchmarkResult<(DirectCompletionSample, CancellationResult)> {
    eprintln!("running one exact direct completion for the stability cycle");
    let evidence = run_direct_completion(
        runtime,
        GenerationExpectation::direct_token_limit(DIRECT_MAXIMUM_NEW_TOKENS),
        None,
        observe,
    )?;
    let direct_completion = summary::sample_record(1, &evidence)?;
    observe(AFTER_STABILITY_DIRECT_RELEASE_CHECKPOINT)?;

    eprintln!("running one cancellation for the stability cycle");
    let cancellation = run_cancellation(runtime, observe)?;
    Ok((direct_completion, cancellation))
}

fn run_chat_proof(
    runtime: &mut ApplicationRuntime,
    loaded: &LoadedModel,
    observe: &mut impl FnMut(&'static str) -> BenchmarkResult,
) -> BenchmarkResult<ChatProofResult> {
    validation::validate_chat_ready(runtime, loaded)?;
    let submitted_at = Instant::now();
    let request_id = runtime
        .submit_user_message(CHAT_MESSAGE, generation_settings(CHAT_MAXIMUM_NEW_TOKENS)?)
        .map_err(|error| {
            BenchmarkError::new(format!(
                "exact compatible-chat request could not be submitted: {error}"
            ))
        })?;
    let evidence = drive_generation(
        runtime,
        request_id,
        submitted_at,
        GenerationExpectation::chat(CHAT_MAXIMUM_NEW_TOKENS),
        None,
        observe,
    )?;
    validation::validate_chat_conversation(
        runtime,
        loaded,
        evidence.finish_reason,
        evidence.usage,
        evidence.decoded_byte_count,
        CHAT_MAXIMUM_NEW_TOKENS,
        CHAT_MESSAGE,
    )?;
    validation::clear_chat_conversation(runtime)?;
    summary::chat_proof_record(&evidence)
}

fn run_direct_completion(
    runtime: &mut ApplicationRuntime,
    expectation: GenerationExpectation,
    before_cancellation_checkpoint: Option<&'static str>,
    observe: &mut impl FnMut(&'static str) -> BenchmarkResult,
) -> BenchmarkResult<GenerationEvidence> {
    validation::validate_direct_ready(runtime)?;
    let maximum_new_tokens = expectation.maximum_new_tokens();
    let submitted_at = Instant::now();
    let request_id = runtime
        .start_generation(
            DIRECT_COMPLETION_PROMPT,
            generation_settings(maximum_new_tokens)?,
        )
        .map_err(|error| {
            BenchmarkError::new(format!(
                "controlled direct completion could not be submitted: {error}"
            ))
        })?;
    let evidence = drive_generation(
        runtime,
        request_id,
        submitted_at,
        expectation,
        before_cancellation_checkpoint,
        observe,
    )?;
    validation::validate_direct_conversation_state(runtime)?;
    Ok(evidence)
}

fn run_cancellation(
    runtime: &mut ApplicationRuntime,
    observe: &mut impl FnMut(&'static str) -> BenchmarkResult,
) -> BenchmarkResult<CancellationResult> {
    let evidence = run_direct_completion(
        runtime,
        GenerationExpectation::cancellation(CANCELLATION_MAXIMUM_NEW_TOKENS),
        Some(BEFORE_CANCELLATION_REQUEST_CHECKPOINT),
        observe,
    )?;
    let result = summary::cancellation_record(&evidence)?;
    observe(AFTER_CANCELLATION_RELEASE_CHECKPOINT)?;
    Ok(result)
}

fn generation_settings(maximum_new_tokens: u32) -> BenchmarkResult<GenerationSettings> {
    let sampling = SamplingConfig::new(
        TEMPERATURE,
        TOP_K,
        TOP_P,
        MIN_P,
        REPETITION_PENALTY,
        REPETITION_WINDOW,
    )
    .map_err(|error| BenchmarkError::new(format!("invalid sampling policy: {error:?}")))?;
    GenerationSettings::new(maximum_new_tokens, sampling)
        .map(|settings| settings.with_seed(GenerationSeed::Fixed(FIXED_SEED)))
        .map_err(|error| BenchmarkError::new(format!("invalid generation settings: {error}")))
}
