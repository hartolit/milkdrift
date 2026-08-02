//! One-shot execution coverage for every configured component benchmark case.

#![forbid(unsafe_code)]

#[path = "../benches/support/mod.rs"]
mod support;

use sampling::SamplingError;
use support::{SAMPLING_CASES, STOP_CASES, SamplingFixture, VOCABULARY_SIZES};

#[test]
fn every_sampling_benchmark_case_executes_at_every_vocabulary_size() -> Result<(), SamplingError> {
    for case in SAMPLING_CASES {
        for &(vocabulary_size, vocabulary_elements) in &VOCABULARY_SIZES {
            let mut fixture = SamplingFixture::new(case, vocabulary_size)?;
            fixture.restore_logits();

            let sample = fixture.sample()?;

            assert!(
                u64::from(sample.token.get()) < vocabulary_elements,
                "case {} returned token {} outside vocabulary {}",
                case.name(),
                sample.token.get(),
                vocabulary_size
            );
            assert!(
                sample.probability.is_finite()
                    && sample.probability > 0.0
                    && sample.probability <= 1.0,
                "case {} returned invalid probability {} at vocabulary {}",
                case.name(),
                sample.probability,
                vocabulary_size
            );
        }
    }

    Ok(())
}

#[test]
fn every_stop_matching_benchmark_case_executes_once() {
    for case in STOP_CASES {
        let fixture = case.build();
        let observed = fixture
            .match_suffix()
            .map(|matched| (matched.code, matched.matched_tokens));

        assert_eq!(
            observed,
            case.expected(),
            "stop benchmark case {}/{} returned an unexpected match",
            case.name(),
            case.parameter()
        );
    }
}
