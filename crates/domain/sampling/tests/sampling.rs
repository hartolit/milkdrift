//! Integration tests for deterministic sampling and stop-sequence matching.

use domain_contracts::{CapacityResource, TokenId};
use sampling::{
    Sampler, SamplingConfig, SamplingError, SamplingWorkspace, StopSequence, match_stop_suffix,
};

#[test]
fn greedy_sampling_selects_highest_logit() -> Result<(), SamplingError> {
    let mut sampler = Sampler::new(SamplingConfig::greedy(), 7)?;
    let mut logits = [0.1_f32, 2.0, 0.5];
    let mut indices = [0_u32; 3];
    let mut seen = [0_u32; 3];

    let sample = sampler.sample(
        &mut logits,
        &[],
        SamplingWorkspace {
            indices: &mut indices,
            seen_tokens: &mut seen,
        },
    )?;

    assert_eq!(sample.token, TokenId::new(1));
    assert_eq!(sample.probability.to_bits(), 1.0_f32.to_bits());
    Ok(())
}

#[test]
fn undersized_workspace_returns_capacity_error() -> Result<(), SamplingError> {
    let mut sampler = Sampler::new(SamplingConfig::greedy(), 7)?;
    let mut logits = [0.1_f32, 2.0, 0.5];
    let mut indices = [0_u32; 2];
    let mut seen = [0_u32; 3];

    let result = sampler.sample(
        &mut logits,
        &[],
        SamplingWorkspace {
            indices: &mut indices,
            seen_tokens: &mut seen,
        },
    );

    assert!(matches!(
        result,
        Err(SamplingError::CapacityExhausted(
            domain_contracts::CapacityExhausted {
                resource: CapacityResource::SamplingIndices,
                required: 3,
                available: 2,
            }
        ))
    ));
    Ok(())
}

#[test]
fn repetition_penalty_is_applied_once_per_token() -> Result<(), SamplingError> {
    let configuration = SamplingConfig {
        repetition_penalty: 2.0,
        ..SamplingConfig::greedy()
    };
    let mut sampler = Sampler::new(configuration, 1)?;
    let mut logits = [2.0_f32, 1.5];
    let history = [TokenId::new(0), TokenId::new(0)];
    let mut indices = [0_u32; 2];
    let mut seen = [0_u32; 2];

    let sample = sampler.sample(
        &mut logits,
        &history,
        SamplingWorkspace {
            indices: &mut indices,
            seen_tokens: &mut seen,
        },
    )?;

    assert_eq!(sample.token, TokenId::new(1));
    Ok(())
}

#[test]
fn stop_matching_uses_token_suffixes() {
    let generated = [TokenId::new(1), TokenId::new(2), TokenId::new(3)];
    let stop_tokens = [TokenId::new(2), TokenId::new(3)];
    let sequences = [StopSequence {
        code: 9,
        tokens: &stop_tokens,
    }];

    let matched = match_stop_suffix(&generated, &sequences);

    assert_eq!(matched.map(|value| value.code), Some(9));
}

#[test]
fn probabilistic_sampling_uses_portable_exponential_math() -> Result<(), SamplingError> {
    let configuration = SamplingConfig {
        temperature: 0.7,
        top_k: 0,
        top_p: 1.0,
        min_p: 0.0,
        repetition_penalty: 1.0,
        repetition_window: 0,
    };
    let mut sampler = Sampler::new(configuration, 11)?;
    let mut logits = [0.0_f32, 1.0, 2.0];
    let mut indices = [0_u32; 3];
    let mut seen = [0_u32; 3];

    let sample = sampler.sample(
        &mut logits,
        &[],
        SamplingWorkspace {
            indices: &mut indices,
            seen_tokens: &mut seen,
        },
    )?;

    assert!(sample.probability.is_finite());
    assert!(sample.probability > 0.0);
    assert!(sample.probability <= 1.0);
    Ok(())
}

#[test]
fn positive_infinity_top_p_rounds_up_candidate_count() -> Result<(), SamplingError> {
    let configuration = SamplingConfig {
        top_k: 0,
        top_p: 0.5,
        ..SamplingConfig::greedy()
    };
    let mut sampler = Sampler::new(configuration, 19)?;
    let mut logits = [f32::INFINITY, f32::INFINITY, f32::INFINITY, 0.0];
    let mut indices = [0_u32; 4];
    let mut seen = [0_u32; 4];

    let sample = sampler.sample(
        &mut logits,
        &[],
        SamplingWorkspace {
            indices: &mut indices,
            seen_tokens: &mut seen,
        },
    )?;

    assert!(matches!(sample.token.get(), 0 | 1));
    assert_eq!(sample.probability.to_bits(), 0.5_f32.to_bits());
    Ok(())
}
