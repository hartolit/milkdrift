use super::support::*;

#[test]
fn exhausted_failed_preparation_is_retained_until_process_exit() -> TestResult {
    let mut source = FakeSource::scripted([0; 8], 0);
    source.failed_load_cleanup_failures = Some(u32::MAX);
    let (hosted, thread, counters) = start_hosted(1, 8, NonZeroU32::MIN, NonZeroU32::MIN, 10_000)?;
    hosted
        .try_submit(RuntimeCommand::LoadModel {
            ticket: CommandTicket::new(60),
            model_id: MODEL,
            source,
            execution_device: cpu_device(),
        })
        .map_err(|_| "failed-load command rejected")?;
    assert!(matches!(
        hosted
            .receive_timeout(Duration::from_secs(2))
            .map_err(|error| format!("failed-load event: {error:?}"))?,
        RuntimeEvent::ModelLoaded {
            result: Err(RuntimeError::CleanupFailed(_)),
            ..
        }
    ));

    hosted
        .try_submit(RuntimeCommand::Shutdown {
            ticket: CommandTicket::new(61),
        })
        .map_err(|_| "failed-load shutdown command rejected")?;
    match hosted
        .receive_timeout(Duration::from_secs(2))
        .map_err(|error| format!("failed-load shutdown event: {error:?}"))?
    {
        RuntimeEvent::Shutdown {
            result: Err(RuntimeError::TerminalCleanupRetention { first, summary }),
            ..
        } => {
            assert_eq!(
                first.resource,
                CleanupResource::FailedLoad {
                    handle: ModelHandle::new(MODEL, ModelGeneration::new(1)),
                }
            );
            assert_eq!(first.ownership, RetainedOwnership::Exact(model_footprint()));
            assert_eq!(summary.failed_preparations, 1);
            assert_eq!(summary.verified_models, 0);
            assert_eq!(summary.incompatible_models, 0);
            assert_eq!(summary.sequences, 0);
        }
        _ => return Err("unexpected failed-load shutdown event".into()),
    }
    drop(hosted);
    thread.join().map_err(|error| error.to_string())?;

    let counters = counters.lock().map_err(|_| "counter mutex poisoned")?;
    assert_eq!(counters.loads, 1);
    assert_eq!(counters.failed_load_cleanup_attempts, 3);
    assert_eq!(counters.successful_failed_load_cleanups, 0);
    assert_eq!(counters.retained_memory_bytes, MODEL_HOST_BYTES);
    assert_eq!(counters.prepared_drops, 0);
    assert_eq!(counters.retained_prepared_drops, 0);
    drop(counters);
    Ok(())
}

#[test]
fn backend_failure_and_cleanup_retry_preserve_both_terminal_states() -> TestResult {
    let mut source = FakeSource::scripted([1, 2, 3, 0, 0, 0, 0, 0], 3);
    source.fail_prefill = true;
    source.destroy_failures = 2;
    let (hosted, thread, counters, handle) = hosted(source, 8, 16)?;
    submit_generation(&hosted, handle, request(40, 400, 4, &[], &[]))?;
    let output = collect_until_released(&hosted, RequestId::new(40), Duration::from_secs(2))?;
    assert!(output.states.iter().any(|state| matches!(
        state,
        GenerationOutputState::Terminal(GenerationOutcome::Failed(
            inference_runtime::RuntimeError::Sequence(SequenceError::Backend(_))
        ))
    )));
    assert!(output.states.iter().any(|state| matches!(
        state,
        GenerationOutputState::CleanupPending { failure, .. }
            if failure.primary_failure == inference_runtime::FailureClass::Sequence
                && failure.cleanup_failure == inference_runtime::FailureClass::Sequence
    )));
    assert!(
        output
            .states
            .iter()
            .any(|state| matches!(state, GenerationOutputState::Released(_)))
    );
    let counters = counters.lock().map_err(|_| "counter mutex poisoned")?;
    assert_eq!(counters.destruction_attempts, 3);
    assert_eq!(counters.successful_destructions, 1);
    assert_eq!(counters.active_sequences, 0);
    assert_eq!(counters.retained_memory_bytes, MODEL_HOST_BYTES);
    drop(counters);
    shutdown(hosted, thread)
}

#[test]
fn exhausted_sequence_cleanup_is_published_after_terminal_and_pending() -> TestResult {
    let mut source = FakeSource::scripted([1; 8], 8);
    source.destroy_failures = u32::MAX;
    let (hosted, thread, counters, handle) = hosted(source, 8, 16)?;
    submit_generation(&hosted, handle, request(190, 1_900, 1, &[], &[]))?;

    let deadline = Instant::now() + Duration::from_secs(2);
    let mut states = Vec::new();
    while !states
        .iter()
        .any(|state| matches!(state, GenerationOutputState::CleanupExhausted { .. }))
    {
        hosted
            .pull_token_output(|batch| {
                states.extend(
                    batch
                        .records
                        .iter()
                        .filter_map(|record| {
                            (record.request_id == RequestId::new(190)).then_some(record.kind)
                        })
                        .filter_map(|kind| match kind {
                            TokenOutputRecordKind::State(state) => Some(state),
                            TokenOutputRecordKind::Tokens(_) => None,
                        }),
                );
            })
            .map_err(|error| format!("token pull failed: {error:?}"))?;
        if Instant::now() >= deadline {
            return Err(format!("cleanup exhaustion timed out: {states:?}"));
        }
        std::thread::yield_now();
    }
    let terminal = states
        .iter()
        .position(|state| matches!(state, GenerationOutputState::Terminal(_)))
        .ok_or("missing terminal state")?;
    let pending = states
        .iter()
        .position(|state| matches!(state, GenerationOutputState::CleanupPending { .. }))
        .ok_or("missing cleanup-pending state")?;
    let exhausted = states
        .iter()
        .position(|state| matches!(state, GenerationOutputState::CleanupExhausted { .. }))
        .ok_or("missing cleanup-exhausted state")?;
    assert!(terminal < pending && pending < exhausted);
    assert!(
        !states
            .iter()
            .any(|state| matches!(state, GenerationOutputState::Released(_)))
    );
    assert_eq!(
        counters
            .lock()
            .map_err(|_| "counter mutex poisoned")?
            .destruction_attempts,
        3
    );

    hosted
        .try_submit(RuntimeCommand::Shutdown {
            ticket: CommandTicket::new(191),
        })
        .map_err(|_| "shutdown command rejected")?;
    assert!(matches!(
        hosted
            .receive_timeout(Duration::from_secs(2))
            .map_err(|error| format!("shutdown event failed: {error:?}"))?,
        RuntimeEvent::Shutdown {
            result: Err(RuntimeError::TerminalCleanupRetention { .. }),
            ..
        }
    ));
    drop(hosted);
    thread.join().map_err(|error| error.to_string())
}

#[test]
fn model_unload_retry_recovers_and_releases_accounting_once() -> TestResult {
    let mut source = FakeSource::scripted([1; 8], 8);
    source.unload_failures = 2;
    let (hosted, thread, counters, handle) = hosted(source, 8, 16)?;

    hosted
        .try_submit(RuntimeCommand::UnloadModel {
            ticket: CommandTicket::new(88),
            handle,
            policy: UnloadPolicy::RejectIfBusy,
        })
        .map_err(|_| "unload command rejected")?;

    match hosted
        .receive_timeout(Duration::from_secs(2))
        .map_err(|error| format!("initial unload event failed: {error:?}"))?
    {
        RuntimeEvent::ModelUnload {
            result: Err(inference_runtime::RuntimeError::CleanupFailed(state)),
            ..
        } if state.failure.primary_operation
            == inference_runtime::RuntimeOperation::ModelUnload
            && state.failure.primary_failure == inference_runtime::FailureClass::Completion
            && state.failure.cleanup_failure
                == inference_runtime::FailureClass::Synchronization => {}
        _ => return Err("unexpected initial unload event".into()),
    }

    match hosted
        .receive_timeout(Duration::from_secs(2))
        .map_err(|error| format!("recovered unload event failed: {error:?}"))?
    {
        RuntimeEvent::ModelUnload {
            result: Ok(receipt),
            ..
        } if receipt.status == inference_runtime::UnloadStatus::Unloaded
            && receipt.cancelled_requests == 0 => {}
        _ => return Err("unexpected recovered unload event".into()),
    }

    hosted
        .try_submit(RuntimeCommand::Snapshot {
            ticket: CommandTicket::new(89),
        })
        .map_err(|_| "snapshot command rejected")?;
    match hosted
        .receive_timeout(Duration::from_secs(2))
        .map_err(|error| format!("snapshot event failed: {error:?}"))?
    {
        RuntimeEvent::Snapshot {
            runtime,
            models,
            retained_models,
            ..
        } => {
            assert_eq!(runtime.loaded_models, 0);
            assert_eq!(runtime.pending_cleanup_models, 0);
            assert_eq!(runtime.exhausted_cleanup_models, 0);
            assert_eq!(runtime.reserved_footprint, MemoryFootprint::default());
            assert!(runtime.unverified_ownership.is_none());
            assert!(!runtime.admission_blocked);
            assert!(models.is_empty());
            assert!(retained_models.is_empty());
        }
        _ => return Err("unexpected snapshot event".into()),
    }

    {
        let counters = counters.lock().map_err(|_| "counter mutex poisoned")?;
        assert_eq!(counters.unload_attempts, 3);
        assert_eq!(counters.retained_memory_bytes, 0);
        drop(counters);
    }
    shutdown(hosted, thread)?;
    let counters = counters.lock().map_err(|_| "counter mutex poisoned")?;
    assert_eq!(counters.unload_attempts, 3);
    assert_eq!(counters.retained_memory_bytes, 0);
    drop(counters);
    Ok(())
}

#[test]
#[expect(
    clippy::too_many_lines,
    reason = "the complete two-model isolation scenario is intentionally kept contiguous"
)]
fn healthy_model_progresses_while_another_model_retries_cleanup() -> TestResult {
    let (prefill_gate, entered, release) = blocking_gate();
    let mut failing = FakeSource::scripted([1; 8], 8);
    failing.fail_prefill = true;
    failing.destroy_failures = 2;
    failing.prefill_gate = Some(prefill_gate);
    let healthy = FakeSource::scripted([1, 2, 3, 0, 0, 0, 0, 0], 3);

    let (hosted, thread, counters) = start_hosted(
        32,
        64,
        NonZeroU32::new(2).ok_or("model limit")?,
        NonZeroU32::new(4).ok_or("request limit")?,
        20_000,
    )?;
    let failing_handle = load_model(&hosted, ModelId::new(1), failing, CommandTicket::new(1))?;
    let healthy_handle = load_model(&hosted, ModelId::new(2), healthy, CommandTicket::new(2))?;

    hosted
        .try_submit(RuntimeCommand::Generate {
            ticket: CommandTicket::new(10),
            handle: failing_handle,
            request: request(86, 806, 4, &[], &[]),
        })
        .map_err(|_| "failing generation command rejected")?;
    entered
        .recv_timeout(Duration::from_secs(2))
        .map_err(|error| format!("prefill gate was not entered: {error:?}"))?;
    hosted
        .try_submit(RuntimeCommand::Generate {
            ticket: CommandTicket::new(11),
            handle: healthy_handle,
            request: request(87, 807, 3, &[], &[]),
        })
        .map_err(|_| "healthy generation command rejected")?;
    hosted
        .try_submit(RuntimeCommand::Generate {
            ticket: CommandTicket::new(12),
            handle: failing_handle,
            request: request(88, 808, 3, &[], &[]),
        })
        .map_err(|_| "degraded-model generation command rejected")?;
    release
        .send(())
        .map_err(|_| "prefill gate release failed")?;

    let mut admitted = 0_u32;
    let mut degraded_rejected = false;
    for _ in 0..3 {
        match hosted
            .receive_timeout(Duration::from_secs(2))
            .map_err(|error| format!("isolation admission event failed: {error:?}"))?
        {
            RuntimeEvent::GenerationAdmitted { result: Ok(_), .. } => {
                admitted = admitted.saturating_add(1);
            }
            RuntimeEvent::GenerationAdmitted {
                ticket,
                result: Err(inference_runtime::RuntimeError::ModelDegraded(model_id)),
            } if ticket == CommandTicket::new(12) && model_id == failing_handle.id => {
                degraded_rejected = true;
            }
            _ => return Err("unexpected isolation admission event".into()),
        }
    }
    assert_eq!(admitted, 2);
    assert!(degraded_rejected);
    let mut outputs = collect_until_all_released(
        &hosted,
        &[RequestId::new(86), RequestId::new(87)],
        Duration::from_secs(2),
    )?;
    let failing_output = outputs
        .remove(&RequestId::new(86))
        .ok_or("missing failing output")?;
    let healthy_output = outputs
        .remove(&RequestId::new(87))
        .ok_or("missing healthy output")?;
    assert!(failing_output.states.iter().any(|state| matches!(
        state,
        GenerationOutputState::Terminal(GenerationOutcome::Failed(_))
    )));
    assert_eq!(
        healthy_output.tokens,
        vec![TokenId::new(1), TokenId::new(2), TokenId::new(3)]
    );
    {
        let counters = counters.lock().map_err(|_| "counter mutex poisoned")?;
        assert_eq!(counters.sequence_creations, 2);
        assert_eq!(counters.destruction_attempts, 4);
        assert_eq!(counters.successful_destructions, 2);
        assert_eq!(counters.sampling_opportunities, 3);
        assert_eq!(counters.active_sequences, 0);
        assert_eq!(counters.retained_memory_bytes, 200);
        drop(counters);
    }
    shutdown(hosted, thread)?;
    assert_eq!(
        counters
            .lock()
            .map_err(|_| "counter mutex poisoned")?
            .retained_memory_bytes,
        0
    );
    Ok(())
}
