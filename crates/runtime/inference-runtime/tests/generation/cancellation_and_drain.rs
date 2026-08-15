use super::support::*;

#[test]
fn shutdown_terminates_without_draining_backpressured_output() -> TestResult {
    let source = FakeSource::scripted([1, 2, 3, 1, 2, 3, 1, 2], 8);
    let (hosted, thread, counters, handle) = hosted(source, 1, 1)?;
    submit_generation(&hosted, handle, request(31, 301, 8, &[], &[]))?;

    let deadline = Instant::now()
        .checked_add(Duration::from_secs(2))
        .ok_or("backpressure deadline overflow")?;
    loop {
        let sampling_opportunities = counters
            .lock()
            .map_err(|_| "counter mutex poisoned")?
            .sampling_opportunities;
        if sampling_opportunities >= 2 {
            break;
        }
        if Instant::now() >= deadline {
            return Err("generation did not reach output backpressure".into());
        }
        std::thread::sleep(Duration::from_millis(1));
    }

    hosted
        .try_submit(RuntimeCommand::Shutdown {
            ticket: CommandTicket::new(5),
        })
        .map_err(|_| "shutdown command rejected")?;
    match hosted
        .receive_timeout(Duration::from_secs(2))
        .map_err(|error| format!("shutdown event: {error:?}"))?
    {
        RuntimeEvent::Shutdown { result: Ok(_), .. } => {}
        _ => return Err("unexpected shutdown event".into()),
    }
    thread.join().map_err(|error| error.to_string())?;

    let counters = counters.lock().map_err(|_| "counter mutex poisoned")?;
    assert_eq!(counters.successful_destructions, 1);
    assert_eq!(counters.active_sequences, 0);
    assert_eq!(counters.retained_memory_bytes, 0);
    drop(counters);

    Ok(())
}

#[test]
fn cancellation_queued_with_admission_is_observed_before_prefill() -> TestResult {
    let (gate, entered, release) = blocking_gate();
    let mut source = FakeSource::scripted([1; 8], 8);
    source.load_gate = Some(gate);
    let (hosted, thread, counters) = start_hosted(
        8,
        16,
        NonZeroU32::MIN,
        NonZeroU32::new(4).ok_or("request limit")?,
        10_000,
    )?;
    hosted
        .try_submit(RuntimeCommand::LoadModel {
            ticket: CommandTicket::new(1),
            model_id: MODEL,
            source,
            execution_device: cpu_device(),
        })
        .map_err(|_| "load command rejected")?;
    entered
        .recv_timeout(Duration::from_secs(2))
        .map_err(|error| format!("load gate was not entered: {error:?}"))?;

    let handle = ModelHandle::new(MODEL, ModelGeneration::new(1));
    hosted
        .try_submit(RuntimeCommand::Generate {
            ticket: CommandTicket::new(2),
            handle,
            request: request(84, 804, 4, &[], &[]),
        })
        .map_err(|_| "generation command rejected")?;
    hosted
        .try_submit(RuntimeCommand::CancelRequest {
            ticket: CommandTicket::new(3),
            request_id: RequestId::new(84),
            reason: domain_contracts::CancellationReason::UserRequested,
        })
        .map_err(|_| "cancel command rejected")?;
    release.send(()).map_err(|_| "load gate release failed")?;

    let mut loaded = false;
    let mut admitted = false;
    let mut cancellation_recorded = false;
    for _ in 0..3 {
        match hosted
            .receive_timeout(Duration::from_secs(2))
            .map_err(|error| format!("queued command event failed: {error:?}"))?
        {
            RuntimeEvent::ModelLoaded { result: Ok(_), .. } => loaded = true,
            RuntimeEvent::GenerationAdmitted { result: Ok(_), .. } => admitted = true,
            RuntimeEvent::GenerationCancellationRequested { result: Ok(()), .. } => {
                cancellation_recorded = true;
            }
            _ => return Err("unexpected queued command event".into()),
        }
    }
    assert!(loaded && admitted && cancellation_recorded);

    let output = collect_until_released(&hosted, RequestId::new(84), Duration::from_secs(2))?;
    assert!(output.tokens.is_empty());
    assert!(output.states.contains(&GenerationOutputState::Terminal(
        GenerationOutcome::Finished(FinishReason::Cancelled(
            domain_contracts::CancellationReason::UserRequested
        ))
    )));
    {
        let counters = counters.lock().map_err(|_| "counter mutex poisoned")?;
        assert_eq!(counters.prefill_calls, 0);
        assert_eq!(counters.sampling_opportunities, 0);
        assert_eq!(counters.successful_destructions, 1);
        drop(counters);
    }
    shutdown(hosted, thread)
}

#[test]
fn cancellation_arriving_during_prefill_is_observed_before_decode() -> TestResult {
    let (gate, entered, release) = blocking_gate();
    let mut source = FakeSource::scripted([1; 8], 8);
    source.prefill_gate = Some(gate);
    let (hosted, thread, counters, handle) = hosted(source, 8, 16)?;
    submit_generation(&hosted, handle, request(184, 1_804, 4, &[], &[]))?;
    entered
        .recv_timeout(Duration::from_secs(2))
        .map_err(|error| format!("prefill gate was not entered: {error:?}"))?;
    hosted
        .try_submit(RuntimeCommand::CancelRequest {
            ticket: CommandTicket::new(184),
            request_id: RequestId::new(184),
            reason: domain_contracts::CancellationReason::UserRequested,
        })
        .map_err(|_| "cancel command rejected")?;
    release
        .send(())
        .map_err(|_| "prefill gate release failed")?;
    assert!(matches!(
        hosted
            .receive_timeout(Duration::from_secs(2))
            .map_err(|error| format!("cancel event failed: {error:?}"))?,
        RuntimeEvent::GenerationCancellationRequested { result: Ok(()), .. }
    ));

    let output = collect_until_released(&hosted, RequestId::new(184), Duration::from_secs(2))?;
    assert!(output.tokens.is_empty());
    assert!(output.states.contains(&GenerationOutputState::Terminal(
        GenerationOutcome::Finished(FinishReason::Cancelled(
            domain_contracts::CancellationReason::UserRequested
        ))
    )));
    let counters = counters.lock().map_err(|_| "counter mutex poisoned")?;
    assert_eq!(counters.prefill_calls, 1);
    assert_eq!(counters.decode_calls, 0);
    drop(counters);
    shutdown(hosted, thread)
}

#[test]
fn scheduled_generation_escalates_at_the_drain_timeout() -> TestResult {
    let source = FakeSource::scripted([1, 2, 3, 1, 2, 3, 1, 2], 8);
    let (hosted, thread, counters, handle) = hosted(source, 1, 16)?;
    submit_generation(&hosted, handle, request(85, 805, 31, &[], &[]))?;

    let timeout = DrainTimeout::from_millis(2).map_err(|error| format!("{error:?}"))?;
    hosted
        .try_submit(RuntimeCommand::UnloadModel {
            ticket: CommandTicket::new(85),
            handle,
            policy: UnloadPolicy::Drain { timeout },
        })
        .map_err(|_| "drain command rejected")?;
    match hosted
        .receive_timeout(Duration::from_secs(2))
        .map_err(|error| format!("initial drain event failed: {error:?}"))?
    {
        RuntimeEvent::ModelUnload {
            result: Ok(receipt),
            ..
        } if receipt.status == inference_runtime::UnloadStatus::Draining => {}
        _ => return Err("unexpected initial drain event".into()),
    }
    match hosted
        .receive_timeout(Duration::from_secs(2))
        .map_err(|error| format!("terminal drain event failed: {error:?}"))?
    {
        RuntimeEvent::ModelUnload {
            result: Ok(receipt),
            ..
        } if receipt.status == inference_runtime::UnloadStatus::Unloaded
            && receipt.cancelled_requests == 1 => {}
        _ => return Err("unexpected terminal drain event".into()),
    }

    let output = collect_until_released(&hosted, RequestId::new(85), Duration::from_secs(2))?;
    assert!(output.states.contains(&GenerationOutputState::Terminal(
        GenerationOutcome::Finished(FinishReason::Cancelled(
            domain_contracts::CancellationReason::DrainTimeout
        ))
    )));
    {
        let counters = counters.lock().map_err(|_| "counter mutex poisoned")?;
        assert_eq!(counters.successful_destructions, 1);
        assert_eq!(counters.active_sequences, 0);
        assert_eq!(counters.retained_memory_bytes, 0);
        drop(counters);
    }
    shutdown(hosted, thread)
}
