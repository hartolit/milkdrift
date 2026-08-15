use super::*;

pub(crate) fn submit_generation(
    hosted: &CandleRuntime,
    handle: ModelHandle,
    ticket: CommandTicket,
    request: &GenerationRequest,
) -> TestResult {
    let request_id = request.request_id;
    let sequence_id = request.sequence_id;
    hosted
        .try_submit(RuntimeCommand::Generate {
            ticket,
            handle,
            request: request.clone(),
        })
        .map_err(|error| format!("generation command rejected: {error:?}"))?;
    match hosted
        .receive_timeout(EVENT_TIMEOUT)
        .map_err(|error| format!("generation admission event failed: {error:?}"))?
    {
        RuntimeEvent::GenerationAdmitted {
            ticket: event_ticket,
            result: Ok(admission),
        } if event_ticket == ticket => {
            assert_eq!(admission.request.request_id, request_id);
            assert_eq!(admission.request.sequence_id, sequence_id);
            assert_eq!(admission.request.logits_capacity, VOCABULARY_SIZE as usize);
            Ok(())
        }
        RuntimeEvent::GenerationAdmitted {
            result: Err(error), ..
        } => Err(format!("generation admission failed: {error:?}")),
        event => Err(format!(
            "unexpected generation event for ticket {:?}",
            event.ticket()
        )),
    }
}

pub(crate) fn request_cancellation(
    hosted: &CandleRuntime,
    request_id: RequestId,
    ticket: CommandTicket,
) -> TestResult {
    hosted
        .try_submit(RuntimeCommand::CancelRequest {
            ticket,
            request_id,
            reason: CancellationReason::UserRequested,
        })
        .map_err(|error| format!("cancellation command rejected: {error:?}"))?;
    match hosted
        .receive_timeout(EVENT_TIMEOUT)
        .map_err(|error| format!("cancellation event failed: {error:?}"))?
    {
        RuntimeEvent::GenerationCancellationRequested {
            ticket: event_ticket,
            request_id: event_request,
            result: Ok(()),
        } if event_ticket == ticket && event_request == request_id => Ok(()),
        RuntimeEvent::GenerationCancellationRequested {
            result: Err(error), ..
        } => Err(format!("generation cancellation failed: {error:?}")),
        event => Err(format!(
            "unexpected cancellation event for ticket {:?}",
            event.ticket()
        )),
    }
}

pub(crate) fn generation_request(
    request: u64,
    sequence: u64,
    maximum_generated_tokens: u32,
    sampling: SamplingConfig,
    seed: u64,
    eos_tokens: Box<[TokenId]>,
) -> TestResult<GenerationRequest> {
    Ok(GenerationRequest {
        request_id: RequestId::new(request),
        sequence_id: SequenceId::new(sequence),
        prompt_tokens: vec![TokenId::new(1), EXPECTED_GREEDY_TOKEN].into_boxed_slice(),
        sequence: SequenceConfiguration::new(nonzero_u32(CONTEXT_LENGTH)?, nonzero_u32(8)?),
        maximum_generated_tokens: nonzero_u32(maximum_generated_tokens)?,
        sampling,
        seed,
        eos_tokens,
        stop_sequences: Box::new([]),
        output_capacity: GenerationOutputCapacityPolicy::new(NonZeroUsize::MIN, NonZeroUsize::MIN),
    })
}

pub(crate) const fn stochastic_sampling() -> SamplingConfig {
    SamplingConfig {
        temperature: 8.0,
        top_k: 0,
        top_p: 1.0,
        min_p: 0.0,
        repetition_penalty: 1.0,
        repetition_window: 0,
    }
}

#[derive(Default)]
pub(crate) struct CollectedOutput {
    pub(crate) tokens: Vec<TokenId>,
    pub(crate) states: Vec<GenerationOutputState>,
}

pub(crate) fn pull_output(
    hosted: &CandleRuntime,
    request_id: RequestId,
    output: &mut CollectedOutput,
) -> TestResult {
    hosted
        .pull_token_output(|batch| {
            for record in batch.records {
                if record.request_id != request_id {
                    continue;
                }
                match record.kind {
                    TokenOutputRecordKind::Tokens(range) => {
                        if let Some(tokens) = batch.tokens_for(range) {
                            output.tokens.extend_from_slice(tokens);
                        }
                    }
                    TokenOutputRecordKind::State(state) => output.states.push(state),
                }
            }
        })
        .map_err(|error| format!("output pull failed: {error:?}"))
}

pub(crate) fn collect_until_backpressure(
    hosted: &CandleRuntime,
    request_id: RequestId,
    timeout: Duration,
) -> TestResult<CollectedOutput> {
    let deadline = deadline(timeout)?;
    let mut output = CollectedOutput::default();
    loop {
        // Leave the one-token accumulator full long enough for the scheduler to
        // attempt the next publication and emit an observable yield record.
        std::thread::sleep(Duration::from_millis(100));
        pull_output(hosted, request_id, &mut output)?;
        if has_output_backpressure(&output) {
            return Ok(output);
        }
        if output
            .states
            .iter()
            .any(|state| matches!(state, GenerationOutputState::Released(_)))
        {
            return Err("request released before output backpressure was observed".into());
        }
        if Instant::now() >= deadline {
            return Err(format!(
                "output backpressure timed out after {} tokens and states {:?}",
                output.tokens.len(),
                output.states
            ));
        }
    }
}

pub(crate) fn collect_until_released(
    hosted: &CandleRuntime,
    request_id: RequestId,
    timeout: Duration,
    mut output: CollectedOutput,
) -> TestResult<CollectedOutput> {
    let deadline = deadline(timeout)?;
    loop {
        pull_output(hosted, request_id, &mut output)?;
        if output
            .states
            .iter()
            .any(|state| matches!(state, GenerationOutputState::Released(_)))
        {
            return Ok(output);
        }
        if Instant::now() >= deadline {
            return Err(format!(
                "generation release timed out after {} tokens and states {:?}",
                output.tokens.len(),
                output.states
            ));
        }
        std::thread::sleep(Duration::from_millis(1));
    }
}

pub(crate) fn assert_finished(output: &CollectedOutput, reason: FinishReason) {
    let outcome = GenerationOutcome::Finished(reason);
    assert!(
        output
            .states
            .contains(&GenerationOutputState::Terminal(outcome))
    );
    assert!(
        output
            .states
            .contains(&GenerationOutputState::Released(outcome))
    );
    assert!(!output.states.iter().any(|state| matches!(
        state,
        GenerationOutputState::CleanupPending { .. }
            | GenerationOutputState::CleanupExhausted { .. }
    )));
}

pub(crate) fn has_output_backpressure(output: &CollectedOutput) -> bool {
    output.states.iter().any(|state| {
        matches!(
            state,
            GenerationOutputState::Yielded(YieldReason::OutputBackpressure(_))
        )
    })
}

pub(crate) fn assert_output_backpressure(output: &CollectedOutput) {
    assert!(
        has_output_backpressure(output),
        "shared output accumulator never reported an explicit backpressure yield"
    );
}

pub(crate) fn assert_released_snapshot(
    hosted: &CandleRuntime,
    loaded: &LoadReceipt,
    ticket: CommandTicket,
) -> TestResult {
    hosted
        .try_submit(RuntimeCommand::Snapshot { ticket })
        .map_err(|error| format!("snapshot command rejected: {error:?}"))?;
    match hosted
        .receive_timeout(EVENT_TIMEOUT)
        .map_err(|error| format!("snapshot event failed: {error:?}"))?
    {
        RuntimeEvent::Snapshot {
            ticket: event_ticket,
            runtime,
            models,
            retained_models,
        } if event_ticket == ticket => {
            assert_eq!(runtime.loaded_models, 1);
            assert_eq!(runtime.active_requests, 0);
            assert_eq!(runtime.reserved_footprint, loaded.reserved_footprint);
            assert!(runtime.unverified_ownership.is_none());
            assert!(!runtime.admission_blocked);
            assert_eq!(runtime.generation_workspaces, 0);
            assert_eq!(
                runtime.reserved_generation_workspace,
                MemoryFootprint::default()
            );
            assert_eq!(runtime.pending_cleanup_models, 0);
            assert_eq!(runtime.pending_cleanup_sequences, 0);
            assert_eq!(runtime.exhausted_cleanup_models, 0);
            assert_eq!(runtime.exhausted_cleanup_sequences, 0);
            assert!(runtime.maintenance_error.is_none());
            assert!(retained_models.is_empty());
            assert_eq!(models.len(), 1);
            let model = models.first().ok_or("loaded model snapshot missing")?;
            assert_eq!(model.handle, loaded.handle);
            assert_eq!(model.execution_device, loaded.execution_device);
            assert_eq!(model.execution_scalar_type, loaded.execution_scalar_type);
            assert_eq!(model.descriptor, loaded.descriptor);
            assert_eq!(model.reserved_footprint, loaded.reserved_footprint);
            assert_eq!(model.active_requests, 0);
            assert_eq!(model.pending_cleanup_sequences, 0);
            assert_eq!(model.exhausted_cleanup_sequences, 0);
            assert!(!model.degraded);
            Ok(())
        }
        event => Err(format!(
            "unexpected released snapshot event for ticket {:?}",
            event.ticket()
        )),
    }
}

pub(crate) fn deadline(timeout: Duration) -> TestResult<Instant> {
    Instant::now()
        .checked_add(timeout)
        .ok_or_else(|| "test deadline overflow".into())
}

pub(crate) fn nonzero_u32(value: u32) -> TestResult<NonZeroU32> {
    NonZeroU32::new(value).ok_or_else(|| "value must be a non-zero u32".into())
}

pub(crate) fn nonzero_usize(value: usize) -> TestResult<NonZeroUsize> {
    NonZeroUsize::new(value).ok_or_else(|| "capacity must be non-zero".into())
}
