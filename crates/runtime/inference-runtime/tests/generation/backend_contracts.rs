use super::support::*;

#[test]
fn decode_sampling_and_generation_capacity_failures_are_stable() -> TestResult {
    let mut decode_source = FakeSource::scripted([1, 2, 3, 0, 0, 0, 0, 0], 3);
    decode_source.fail_decode_call = Some(1);
    let (hosted, thread, _, handle) = hosted(decode_source, 8, 16)?;
    submit_generation(&hosted, handle, request(60, 600, 4, &[], &[]))?;
    let decoded = collect_until_released(&hosted, RequestId::new(60), Duration::from_secs(2))?;
    assert_eq!(decoded.tokens, vec![TokenId::new(1)]);
    assert!(decoded.states.iter().any(|state| matches!(
        state,
        GenerationOutputState::Terminal(GenerationOutcome::Failed(
            inference_runtime::RuntimeError::Sequence(_)
        ))
    )));
    shutdown(hosted, thread)?;

    let mut sampling_source = FakeSource::scripted([0; 8], 0);
    sampling_source.no_candidate = true;
    let (hosted, thread, _, handle) = self::hosted(sampling_source, 8, 16)?;
    submit_generation(&hosted, handle, request(61, 601, 4, &[], &[]))?;
    let sampled = collect_until_released(&hosted, RequestId::new(61), Duration::from_secs(2))?;
    assert!(sampled.states.iter().any(|state| matches!(
        state,
        GenerationOutputState::Terminal(GenerationOutcome::Failed(
            inference_runtime::RuntimeError::Sampling(_)
        ))
    )));
    shutdown(hosted, thread)?;

    let source = FakeSource::scripted([1; 8], 8);
    let (hosted, thread, _, handle) = self::hosted(source, 8, 16)?;
    hosted
        .try_submit(RuntimeCommand::Generate {
            ticket: CommandTicket::new(62),
            handle,
            request: request(62, 602, 40, &[], &[]),
        })
        .map_err(|_| "capacity generation command rejected")?;
    assert!(matches!(
        hosted
            .receive_timeout(Duration::from_secs(2))
            .map_err(|error| format!("capacity admission event: {error:?}"))?,
        RuntimeEvent::GenerationAdmitted {
            result: Err(inference_runtime::RuntimeError::CapacityExhausted(_)),
            ..
        }
    ));
    shutdown(hosted, thread)
}

#[test]
fn short_prefill_logits_fail_before_sampling_and_cleanup_the_sequence() -> TestResult {
    let mut source = FakeSource::scripted([3; 8], 8);
    source.contract_faults = ContractFaults::SHORT_PREFILL_LOGITS;
    source.destroy_failures = 1;
    let (hosted, thread, counters, handle) = hosted(source, 8, 16)?;
    submit_generation(&hosted, handle, request(93, 903, 4, &[], &[]))?;

    let output = collect_until_released(&hosted, RequestId::new(93), Duration::from_secs(2))?;
    assert!(output.tokens.is_empty());
    assert_backend_contract_failure(&output);
    assert!(output.states.iter().any(|state| matches!(
        state,
        GenerationOutputState::CleanupPending { failure, .. }
            if failure.primary_operation() == inference_runtime::RuntimeOperation::Prefill
                && failure.primary_failure() == FailureClass::BackendContract
    )));
    {
        let counters = counters.lock().map_err(|_| "counter mutex poisoned")?;
        assert_eq!(counters.prefill_calls, 1);
        assert_eq!(counters.decode_calls, 0);
        assert_eq!(counters.destruction_attempts, 2);
        assert_eq!(counters.successful_destructions, 1);
        drop(counters);
    }
    shutdown(hosted, thread)
}

#[test]
fn short_decode_logits_fail_before_sampling_and_cleanup_the_sequence() -> TestResult {
    let mut source = FakeSource::scripted([1, 3, 0, 0, 0, 0, 0, 0], 2);
    source.contract_faults = ContractFaults::SHORT_DECODE_LOGITS;
    source.destroy_failures = 1;
    let (hosted, thread, counters, handle) = hosted(source, 8, 16)?;
    submit_generation(&hosted, handle, request(94, 904, 4, &[], &[]))?;

    let output = collect_until_released(&hosted, RequestId::new(94), Duration::from_secs(2))?;
    assert_eq!(output.tokens, vec![TokenId::new(1)]);
    assert_backend_contract_failure(&output);
    assert!(output.states.iter().any(|state| matches!(
        state,
        GenerationOutputState::CleanupPending { failure, .. }
            if failure.primary_operation() == inference_runtime::RuntimeOperation::Decode
                && failure.primary_failure() == FailureClass::BackendContract
    )));
    {
        let counters = counters.lock().map_err(|_| "counter mutex poisoned")?;
        assert_eq!(counters.prefill_calls, 1);
        assert_eq!(counters.decode_calls, 1);
        assert_eq!(counters.destruction_attempts, 2);
        assert_eq!(counters.successful_destructions, 1);
        drop(counters);
    }
    shutdown(hosted, thread)
}

#[test]
fn invalid_prefill_and_decode_positions_fail_the_generation_contract() -> TestResult {
    for (fault, request_id, expected_tokens) in [
        (ContractFaults::INVALID_PREFILL_POSITION, 95, 0),
        (ContractFaults::INVALID_DECODE_POSITION, 96, 1),
    ] {
        let mut source = FakeSource::scripted([1, 2, 0, 0, 0, 0, 0, 0], 2);
        source.contract_faults = fault;
        let (hosted, thread, counters, handle) = hosted(source, 8, 16)?;
        submit_generation(
            &hosted,
            handle,
            request(request_id, request_id + 1_000, 4, &[], &[]),
        )?;

        let output =
            collect_until_released(&hosted, RequestId::new(request_id), Duration::from_secs(2))?;
        assert_eq!(output.tokens.len(), expected_tokens);
        assert_backend_contract_failure(&output);
        assert_eq!(
            counters
                .lock()
                .map_err(|_| "counter mutex poisoned")?
                .successful_destructions,
            1
        );
        shutdown(hosted, thread)?;
    }
    Ok(())
}

#[test]
fn invalid_prefill_consumed_tokens_fail_the_generation_contract() -> TestResult {
    let mut source = FakeSource::scripted([1; 8], 8);
    source.contract_faults = ContractFaults::INVALID_CONSUMED_TOKENS;
    let (hosted, thread, counters, handle) = hosted(source, 8, 16)?;
    submit_generation(&hosted, handle, request(97, 907, 4, &[], &[]))?;

    let output = collect_until_released(&hosted, RequestId::new(97), Duration::from_secs(2))?;
    assert!(output.tokens.is_empty());
    assert_backend_contract_failure(&output);
    assert_eq!(
        counters
            .lock()
            .map_err(|_| "counter mutex poisoned")?
            .successful_destructions,
        1
    );
    shutdown(hosted, thread)
}
