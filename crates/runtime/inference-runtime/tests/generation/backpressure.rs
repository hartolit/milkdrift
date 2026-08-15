use super::support::*;

#[test]
fn backpressure_resumes_without_duplicate_tokens_and_cancellation_stays_responsive() -> TestResult {
    let source = FakeSource::scripted([1, 2, 3, 1, 2, 3, 1, 2], 8);
    let (hosted, thread, counters, handle) = hosted(source, 1, 4)?;
    submit_generation(&hosted, handle, request(30, 300, 8, &[], &[]))?;
    std::thread::sleep(Duration::from_millis(20));
    hosted
        .try_submit(RuntimeCommand::CancelRequest {
            ticket: CommandTicket::new(4),
            request_id: RequestId::new(30),
            reason: domain_contracts::CancellationReason::UserRequested,
        })
        .map_err(|_| "cancel command rejected")?;
    let cancellation_event = hosted
        .receive_timeout(Duration::from_secs(2))
        .map_err(|error| format!("cancel event: {error:?}"))?;
    assert!(matches!(
        cancellation_event,
        RuntimeEvent::GenerationCancellationRequested { result: Ok(()), .. }
    ));
    let output = collect_until_released(&hosted, RequestId::new(30), Duration::from_secs(2))?;
    assert!(output.states.iter().any(|state| matches!(
        state,
        GenerationOutputState::Yielded(domain_contracts::YieldReason::OutputBackpressure(_))
    )));
    assert!(output.states.contains(&GenerationOutputState::Terminal(
        GenerationOutcome::Finished(FinishReason::Cancelled(
            domain_contracts::CancellationReason::UserRequested
        ))
    )));
    let mut deduplicated = output.tokens.clone();
    deduplicated.dedup();
    assert_eq!(deduplicated, output.tokens);
    assert!(
        counters
            .lock()
            .map_err(|_| "counter mutex poisoned")?
            .decode_calls
            <= 1
    );
    shutdown(hosted, thread)
}
