use super::support::*;

#[test]
fn greedy_generation_prefills_once_decodes_and_finishes_on_eos() -> TestResult {
    let source = FakeSource::scripted([1, 2, 3, 0, 0, 0, 0, 0], 3);
    let (hosted, thread, counters, handle) = hosted(source, 8, 16)?;
    submit_generation(
        &hosted,
        handle,
        request(10, 100, 8, &[TokenId::new(3)], &[]),
    )?;
    let output = collect_until_released(&hosted, RequestId::new(10), Duration::from_secs(2))?;
    assert_eq!(
        output.tokens,
        vec![TokenId::new(1), TokenId::new(2), TokenId::new(3)]
    );
    assert!(output.states.contains(&GenerationOutputState::Terminal(
        GenerationOutcome::Finished(FinishReason::EndOfSequence(TokenId::new(3)))
    )));
    let counters = counters.lock().map_err(|_| "counter mutex poisoned")?;
    assert_eq!(counters.prefill_calls, 1);
    assert_eq!(counters.decode_calls, 2);
    assert_eq!(counters.successful_destructions, 1);
    drop(counters);
    shutdown(hosted, thread)
}

#[test]
fn concurrent_runnable_requests_complete_without_starvation() -> TestResult {
    let source = FakeSource::scripted([1, 2, 3, 0, 0, 0, 0, 0], 3);
    let (hosted, thread, _, handle) = hosted(source, 32, 64)?;
    let request_ids = [RequestId::new(70), RequestId::new(71)];
    for (ticket, request_id, sequence_id) in [(70, 70, 700), (71, 71, 701)] {
        hosted
            .try_submit(RuntimeCommand::Generate {
                ticket: CommandTicket::new(ticket),
                handle,
                request: request(request_id, sequence_id, 3, &[], &[]),
            })
            .map_err(|_| "fairness generation command rejected")?;
    }
    for _ in 0..2 {
        assert!(matches!(
            hosted
                .receive_timeout(Duration::from_secs(2))
                .map_err(|error| format!("fairness admission event: {error:?}"))?,
            RuntimeEvent::GenerationAdmitted { result: Ok(_), .. }
        ));
    }

    let outputs = collect_until_all_released(&hosted, &request_ids, Duration::from_secs(2))?;
    for request_id in request_ids {
        let output = outputs
            .get(&request_id)
            .ok_or("missing concurrent generation output")?;
        assert_eq!(output.tokens.len(), 3);
        assert!(output.states.iter().any(|state| matches!(
            state,
            GenerationOutputState::Terminal(GenerationOutcome::Finished(FinishReason::TokenLimit))
        )));
    }
    shutdown(hosted, thread)
}
