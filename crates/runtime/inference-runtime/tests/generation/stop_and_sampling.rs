use super::support::*;

fn stochastic_run(seed: u64) -> TestResult<Vec<TokenId>> {
    let mut source = FakeSource::scripted([0; 8], 0);
    source.uniform_logits = true;
    let (hosted, thread, _, handle) = hosted(source, 16, 32)?;
    let mut generation = request(50, 500, 5, &[], &[]);
    generation.seed = seed;
    generation.sampling = SamplingConfig::new(1.0, 0, 1.0, 0.0, 1.0, 0)
        .map_err(|error| format!("sampling configuration failed: {error:?}"))?;
    submit_generation(&hosted, handle, generation)?;
    let output = collect_until_released(&hosted, RequestId::new(50), Duration::from_secs(2))?;
    shutdown(hosted, thread)?;
    Ok(output.tokens)
}

#[test]
fn token_limit_stop_sequence_and_seeded_sampling_are_deterministic() -> TestResult {
    let scripted = FakeSource::scripted([1, 2, 3, 0, 0, 0, 0, 0], 3);
    let (hosted, thread, _, handle) = hosted(scripted, 8, 16)?;
    let stop = GenerationStopSequence {
        code: 7,
        tokens: vec![TokenId::new(1), TokenId::new(2)].into_boxed_slice(),
    };
    submit_generation(&hosted, handle, request(20, 200, 6, &[], &[stop]))?;
    let stopped = collect_until_released(&hosted, RequestId::new(20), Duration::from_secs(2))?;
    assert_eq!(stopped.tokens, vec![TokenId::new(1), TokenId::new(2)]);
    assert!(stopped.states.contains(&GenerationOutputState::Terminal(
        GenerationOutcome::Finished(FinishReason::StopCondition)
    )));
    shutdown(hosted, thread)?;

    let first = stochastic_run(55)?;
    let second = stochastic_run(55)?;
    assert_eq!(first, second);
    assert_eq!(first.len(), 5);
    Ok(())
}
