//! Report projection, checked duration conversion, throughput, and summaries.

use std::time::Duration;

use domain_contracts::FinishReason;

use super::observer::GenerationEvidence;
use crate::error::{BenchmarkError, BenchmarkResult};
use crate::external::report::{
    CancellationResult, CancellationSubmissionTimings, ChatProofResult, ConversationProof,
    DirectCompletionSample, DirectCompletionSummary, DirectCompletionWarmupResult,
    GenerationOutcomeMatch, GenerationSubmissionTimings, SamplingMetadata,
};

pub(in crate::external) const fn sampling_metadata() -> SamplingMetadata {
    SamplingMetadata {
        temperature: super::workload::TEMPERATURE,
        top_k: super::workload::TOP_K,
        top_p: super::workload::TOP_P,
        min_p: super::workload::MIN_P,
        repetition_penalty: super::workload::REPETITION_PENALTY,
        repetition_window: super::workload::REPETITION_WINDOW,
        fixed_seed: super::workload::FIXED_SEED,
    }
}

pub(super) fn chat_proof_record(evidence: &GenerationEvidence) -> BenchmarkResult<ChatProofResult> {
    Ok(ChatProofResult {
        submission_to_generation_started_ns: duration_ns(
            evidence.normal_timings.started,
            "chat submission to GenerationStarted",
        )?,
        submission_to_first_decoded_output_ns: duration_ns(
            evidence.normal_timings.first_decoded,
            "chat submission to first decoded output",
        )?,
        submission_to_terminal_event_ns: duration_ns(
            evidence.normal_timings.terminal_event,
            "chat submission to terminal event",
        )?,
        submission_to_release_ns: duration_ns(
            evidence.normal_timings.release,
            "chat submission to release",
        )?,
        decoded_byte_count: evidence.decoded_byte_count,
        prompt_tokens: evidence.usage.prompt_tokens,
        generated_tokens: evidence.usage.generated_tokens,
        terminal_kind: finish_reason_label(evidence.finish_reason),
        outcome_match: matched_outcomes(),
        conversation: ConversationProof {
            validated: true,
            cleared: true,
        },
    })
}

pub(super) fn warmup_record(evidence: &GenerationEvidence) -> DirectCompletionWarmupResult {
    DirectCompletionWarmupResult {
        decoded_byte_count: evidence.decoded_byte_count,
        prompt_tokens: evidence.usage.prompt_tokens,
        generated_tokens: evidence.usage.generated_tokens,
        terminal_kind: finish_reason_label(evidence.finish_reason),
        clean_release: true,
    }
}

pub(super) fn sample_record(
    ordinal: u32,
    evidence: &GenerationEvidence,
) -> BenchmarkResult<DirectCompletionSample> {
    let release_seconds = evidence.normal_timings.release.as_secs_f64();
    if release_seconds <= 0.0 || !release_seconds.is_finite() {
        return Err(BenchmarkError::new(
            "submission-to-release duration could not support a finite throughput calculation",
        ));
    }
    let generated_tokens = u32::try_from(evidence.usage.generated_tokens).map_err(|_| {
        BenchmarkError::new("generated-token count was too large for exact f64 conversion")
    })?;
    let effective_generated_tokens_per_second = f64::from(generated_tokens) / release_seconds;
    if !effective_generated_tokens_per_second.is_finite() {
        return Err(BenchmarkError::new(
            "effective generated-token throughput was not finite",
        ));
    }

    Ok(DirectCompletionSample {
        ordinal,
        submission_to_generation_started_ns: duration_ns(
            evidence.normal_timings.started,
            "direct-completion submission to GenerationStarted",
        )?,
        submission_to_first_decoded_output_ns: duration_ns(
            evidence.normal_timings.first_decoded,
            "direct-completion submission to first decoded output",
        )?,
        submission_to_terminal_event_ns: duration_ns(
            evidence.normal_timings.terminal_event,
            "direct-completion submission to terminal event",
        )?,
        submission_to_release_ns: duration_ns(
            evidence.normal_timings.release,
            "direct-completion submission to release",
        )?,
        prompt_tokens: evidence.usage.prompt_tokens,
        generated_tokens: evidence.usage.generated_tokens,
        decoded_byte_count: evidence.decoded_byte_count,
        terminal_kind: finish_reason_label(evidence.finish_reason),
        terminal_state_matched: true,
        released_state_matched: true,
        terminal_event_matched: true,
        effective_generated_tokens_per_second,
    })
}

pub(super) fn cancellation_record(
    evidence: &GenerationEvidence,
) -> BenchmarkResult<CancellationResult> {
    let timings = evidence.cancellation_timings.ok_or_else(|| {
        BenchmarkError::new("completed cancellation did not retain cancellation timings")
    })?;
    Ok(CancellationResult {
        generation_submission: GenerationSubmissionTimings {
            to_generation_started_ns: duration_ns(
                evidence.normal_timings.started,
                "cancellation generation submission to GenerationStarted",
            )?,
            to_first_decoded_output_ns: duration_ns(
                evidence.normal_timings.first_decoded,
                "cancellation generation submission to first decoded output",
            )?,
            to_cancellation_submission_ns: duration_ns(
                timings.generation_submission_to_cancellation,
                "generation submission to cancellation submission",
            )?,
        },
        cancellation_submission: CancellationSubmissionTimings {
            to_acknowledgement_ns: duration_ns(
                timings.cancellation_submission_to_acknowledgement,
                "cancellation submission to acknowledgement",
            )?,
            to_terminal_output_ns: duration_ns(
                timings.cancellation_submission_to_terminal_output,
                "cancellation submission to terminal output",
            )?,
            to_terminal_event_ns: duration_ns(
                timings.cancellation_submission_to_terminal_event,
                "cancellation submission to terminal event",
            )?,
            to_release_ns: duration_ns(
                timings.cancellation_submission_to_release,
                "cancellation submission to release",
            )?,
        },
        decoded_byte_count: evidence.decoded_byte_count,
        prompt_tokens: evidence.usage.prompt_tokens,
        generated_tokens: evidence.usage.generated_tokens,
        terminal_kind: finish_reason_label(evidence.finish_reason),
        cancellation_acknowledged: true,
        outcome_match: matched_outcomes(),
    })
}

pub(super) fn summarize_samples(
    samples: &[DirectCompletionSample],
    expected_sample_count: u32,
) -> BenchmarkResult<DirectCompletionSummary> {
    let sample_count = u32::try_from(samples.len())
        .map_err(|_| BenchmarkError::new("sample count conversion to u32 failed"))?;
    if sample_count != expected_sample_count {
        return Err(BenchmarkError::new(format!(
            "external baseline collected {sample_count} samples instead of {expected_sample_count}"
        )));
    }
    Ok(DirectCompletionSummary {
        sample_count,
        median_submission_to_generation_started_ns: median_u64(
            samples
                .iter()
                .map(|sample| sample.submission_to_generation_started_ns),
        )?,
        median_submission_to_first_decoded_output_ns: median_u64(
            samples
                .iter()
                .map(|sample| sample.submission_to_first_decoded_output_ns),
        )?,
        median_submission_to_terminal_event_ns: median_u64(
            samples
                .iter()
                .map(|sample| sample.submission_to_terminal_event_ns),
        )?,
        median_submission_to_release_ns: median_u64(
            samples.iter().map(|sample| sample.submission_to_release_ns),
        )?,
        median_effective_generated_tokens_per_second: median_f64(
            samples
                .iter()
                .map(|sample| sample.effective_generated_tokens_per_second),
        )?,
    })
}

pub(in crate::external) fn duration_ns(
    duration: Duration,
    label: &'static str,
) -> BenchmarkResult<u64> {
    u64::try_from(duration.as_nanos()).map_err(|_| {
        BenchmarkError::new(format!(
            "{label} duration exceeded the report's u64 nanosecond range"
        ))
    })
}

fn median_u64(values: impl IntoIterator<Item = u64>) -> BenchmarkResult<u64> {
    let mut values = values.into_iter().collect::<Vec<_>>();
    if values.is_empty() {
        return Err(BenchmarkError::new("cannot summarize an empty sample set"));
    }
    values.sort_unstable();
    let middle = values.len() / 2;
    let upper = values
        .get(middle)
        .copied()
        .ok_or_else(|| BenchmarkError::new("summary upper median disappeared"))?;
    if values.len() % 2 == 0 {
        let lower = values
            .get(middle.saturating_sub(1))
            .copied()
            .ok_or_else(|| BenchmarkError::new("summary lower median disappeared"))?;
        Ok(lower + (upper - lower) / 2)
    } else {
        Ok(upper)
    }
}

fn median_f64(values: impl IntoIterator<Item = f64>) -> BenchmarkResult<f64> {
    let mut values = values.into_iter().collect::<Vec<_>>();
    if values.is_empty() || values.iter().any(|value| !value.is_finite()) {
        return Err(BenchmarkError::new(
            "cannot summarize empty or non-finite throughput samples",
        ));
    }
    values.sort_by(f64::total_cmp);
    let middle = values.len() / 2;
    let upper = values
        .get(middle)
        .copied()
        .ok_or_else(|| BenchmarkError::new("throughput upper median disappeared"))?;
    if values.len() % 2 == 0 {
        let lower = values
            .get(middle.saturating_sub(1))
            .copied()
            .ok_or_else(|| BenchmarkError::new("throughput lower median disappeared"))?;
        Ok(f64::midpoint(lower, upper))
    } else {
        Ok(upper)
    }
}

const fn matched_outcomes() -> GenerationOutcomeMatch {
    GenerationOutcomeMatch {
        terminal_state_matched: true,
        released_state_matched: true,
        terminal_event_matched: true,
    }
}

const fn finish_reason_label(reason: FinishReason) -> &'static str {
    match reason {
        FinishReason::EndOfSequence(_) => "end_of_sequence",
        FinishReason::TokenLimit => "token_limit",
        FinishReason::StopCondition => "stop_condition",
        FinishReason::BufferExhausted(_) => "buffer_exhausted",
        FinishReason::Cancelled(_) => "cancelled",
        _ => "other",
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use application_runtime::{GenerationTerminal, GenerationTerminalOutcome};
    use domain_contracts::{FinishReason, GenerationUsage, RequestId};

    use super::{duration_ns, median_u64, sample_record, summarize_samples};
    use crate::external::generation::observer::{GenerationEvidence, NormalTimings};
    use crate::external::report::DirectCompletionSample;

    fn evidence(release: Duration) -> GenerationEvidence {
        let usage = GenerationUsage {
            prompt_tokens: 12,
            generated_tokens: 32,
        };
        GenerationEvidence {
            terminal: GenerationTerminal {
                request_id: RequestId::new(1),
                outcome: GenerationTerminalOutcome::Finished(FinishReason::TokenLimit),
                usage,
            },
            finish_reason: FinishReason::TokenLimit,
            usage,
            decoded_byte_count: 64,
            normal_timings: NormalTimings {
                started: Duration::from_millis(1),
                first_decoded: Duration::from_millis(2),
                terminal_event: Duration::from_millis(3),
                release,
            },
            cancellation_timings: None,
        }
    }

    fn sample(
        ordinal: u32,
        started: u64,
        first_decoded: u64,
        terminal_event: u64,
        release: u64,
        throughput: f64,
    ) -> DirectCompletionSample {
        DirectCompletionSample {
            ordinal,
            submission_to_generation_started_ns: started,
            submission_to_first_decoded_output_ns: first_decoded,
            submission_to_terminal_event_ns: terminal_event,
            submission_to_release_ns: release,
            prompt_tokens: 12,
            generated_tokens: 32,
            decoded_byte_count: 64,
            terminal_kind: "token_limit",
            terminal_state_matched: true,
            released_state_matched: true,
            terminal_event_matched: true,
            effective_generated_tokens_per_second: throughput,
        }
    }

    #[test]
    fn report_conversion_uses_checked_nanoseconds_and_effective_throughput() -> Result<(), String> {
        let sample = sample_record(1, &evidence(Duration::from_secs(4)))
            .map_err(|error| error.to_string())?;
        assert_eq!(sample.submission_to_generation_started_ns, 1_000_000);
        assert_eq!(sample.submission_to_first_decoded_output_ns, 2_000_000);
        assert_eq!(sample.submission_to_terminal_event_ns, 3_000_000);
        assert_eq!(sample.submission_to_release_ns, 4_000_000_000);
        assert!((sample.effective_generated_tokens_per_second - 8.0).abs() < f64::EPSILON);
        assert!(duration_ns(Duration::MAX, "synthetic overflow").is_err());
        Ok(())
    }

    #[test]
    fn integer_median_avoids_overflow() -> Result<(), String> {
        assert_eq!(
            median_u64([u64::MAX - 1, u64::MAX]).map_err(|error| error.to_string())?,
            u64::MAX - 1
        );
        Ok(())
    }

    #[test]
    fn three_sample_summary_uses_sorted_medians() -> Result<(), String> {
        let samples = [
            sample(1, 3, 30, 300, 3_000, 3.0),
            sample(2, 1, 10, 100, 1_000, 1.0),
            sample(3, 2, 20, 200, 2_000, 2.0),
        ];
        let summary = summarize_samples(&samples, 3).map_err(|error| error.to_string())?;
        assert_eq!(summary.sample_count, 3);
        assert_eq!(summary.median_submission_to_generation_started_ns, 2);
        assert_eq!(summary.median_submission_to_first_decoded_output_ns, 20);
        assert_eq!(summary.median_submission_to_terminal_event_ns, 200);
        assert_eq!(summary.median_submission_to_release_ns, 2_000);
        assert!((summary.median_effective_generated_tokens_per_second - 2.0).abs() < f64::EPSILON);
        Ok(())
    }
}
