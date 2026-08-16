use super::*;

pub(crate) struct CollectedOutput {
    pub(crate) tokens: Vec<TokenId>,
    pub(crate) states: Vec<GenerationOutputState>,
}

pub(crate) fn assert_backend_contract_failure(output: &CollectedOutput) {
    assert!(
        output
            .states
            .contains(&GenerationOutputState::Terminal(GenerationOutcome::Failed(
                inference_runtime::RuntimeError::BackendContractViolation
            )))
    );
}

pub(crate) fn collect_until_released(
    hosted: &HostedRuntime<FakeSource>,
    request_id: RequestId,
    timeout: Duration,
) -> TestResult<CollectedOutput> {
    let deadline = Instant::now()
        .checked_add(timeout)
        .ok_or("deadline overflow")?;
    let mut output = CollectedOutput {
        tokens: Vec::new(),
        states: Vec::new(),
    };
    loop {
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
            .map_err(|error| format!("token pull failed: {error:?}"))?;
        if output
            .states
            .iter()
            .any(|state| matches!(state, GenerationOutputState::Released(_)))
        {
            return Ok(output);
        }
        if Instant::now() >= deadline {
            return Err(format!("generation timed out: {:?}", output.states));
        }
        std::thread::sleep(Duration::from_millis(1));
    }
}

pub(crate) fn collect_until_all_released(
    hosted: &HostedRuntime<FakeSource>,
    request_ids: &[RequestId],
    timeout: Duration,
) -> TestResult<BTreeMap<RequestId, CollectedOutput>> {
    let deadline = Instant::now()
        .checked_add(timeout)
        .ok_or("deadline overflow")?;
    let requested = request_ids.iter().copied().collect::<BTreeSet<_>>();
    let mut outputs = request_ids
        .iter()
        .copied()
        .map(|request_id| {
            (
                request_id,
                CollectedOutput {
                    tokens: Vec::new(),
                    states: Vec::new(),
                },
            )
        })
        .collect::<BTreeMap<_, _>>();
    loop {
        hosted
            .pull_token_output(|batch| {
                for record in batch.records {
                    let Some(output) = outputs.get_mut(&record.request_id) else {
                        continue;
                    };
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
            .map_err(|error| format!("token pull failed: {error:?}"))?;
        let released = outputs
            .iter()
            .filter(|(request_id, output)| {
                requested.contains(request_id)
                    && output
                        .states
                        .iter()
                        .any(|state| matches!(state, GenerationOutputState::Released(_)))
            })
            .count();
        if released == requested.len() {
            return Ok(outputs);
        }
        if Instant::now() >= deadline {
            return Err("multi-request generation timed out".into());
        }
        std::thread::sleep(Duration::from_millis(1));
    }
}

pub(crate) fn synchronous_runtime(source: &FakeSource) -> TestResult<SynchronousParts> {
    let counters = Arc::new(Mutex::new(Counters::default()));
    let mut runtime = InferenceRuntime::new(
        FakeLoader {
            counters: Arc::clone(&counters),
        },
        RuntimeLimits::new(
            NonZeroU32::MIN,
            NonZeroU32::new(4).unwrap_or(NonZeroU32::MIN),
            MemoryBudget::ZERO.with_host_bytes(ByteCount::from_u64(10_000)),
        ),
    );
    let handle = runtime
        .load_model(MODEL, source, cpu_device())
        .map_err(debug_error)?
        .handle;
    Ok((runtime, counters, handle))
}

pub(crate) fn direct_sequence_configuration() -> SequenceConfiguration {
    SequenceConfiguration::new(
        NonZeroU32::new(8).unwrap_or(NonZeroU32::MIN),
        NonZeroU32::new(4).unwrap_or(NonZeroU32::MIN),
    )
}

pub(crate) fn debug_error(error: impl core::fmt::Debug) -> String {
    format!("{error:?}")
}

pub(crate) fn hosted(
    source: FakeSource,
    token_capacity: usize,
    record_capacity: usize,
) -> TestResult<HostedParts> {
    hosted_with_budget(source, token_capacity, record_capacity, 10_000)
}

pub(crate) fn hosted_with_budget(
    source: FakeSource,
    token_capacity: usize,
    record_capacity: usize,
    host_bytes: u64,
) -> TestResult<HostedParts> {
    let (hosted, thread, counters) = start_hosted(
        token_capacity,
        record_capacity,
        NonZeroU32::MIN,
        NonZeroU32::new(4).ok_or("request limit")?,
        host_bytes,
    )?;
    let handle = load_model(&hosted, MODEL, source, CommandTicket::new(1))?;
    Ok((hosted, thread, counters, handle))
}

pub(crate) fn start_hosted(
    token_capacity: usize,
    record_capacity: usize,
    maximum_loaded_models: NonZeroU32,
    maximum_active_requests: NonZeroU32,
    host_bytes: u64,
) -> TestResult<(
    HostedRuntime<FakeSource>,
    RuntimeThread,
    Arc<Mutex<Counters>>,
)> {
    let counters = Arc::new(Mutex::new(Counters::default()));
    let configuration =
        HostedRuntimeConfiguration::new(nonzero_usize(8)?, nonzero_usize(8)?, NonZeroU64::MIN)
            .with_token_output_capacity(
                nonzero_usize(token_capacity)?,
                nonzero_usize(record_capacity)?,
            );
    let (hosted, thread) = start_hosted_runtime(
        FakeLoader {
            counters: Arc::clone(&counters),
        },
        RuntimeLimits::new(
            maximum_loaded_models,
            maximum_active_requests,
            MemoryBudget::ZERO.with_host_bytes(ByteCount::from_u64(host_bytes)),
        ),
        configuration,
    )
    .map_err(|error| error.to_string())?;
    Ok((hosted, thread, counters))
}

pub(crate) fn load_model(
    hosted: &HostedRuntime<FakeSource>,
    model_id: ModelId,
    source: FakeSource,
    ticket: CommandTicket,
) -> TestResult<ModelHandle> {
    hosted
        .try_submit(RuntimeCommand::LoadModel {
            ticket,
            model_id,
            source,
            execution_device: cpu_device(),
        })
        .map_err(|_| "load command rejected")?;
    let event = hosted
        .receive_timeout(Duration::from_secs(2))
        .map_err(|error| format!("load event: {error:?}"))?;
    let RuntimeEvent::ModelLoaded {
        ticket: event_ticket,
        result: Ok(receipt),
    } = event
    else {
        return Err("model load failed".into());
    };
    if event_ticket != ticket {
        return Err("model load ticket mismatch".into());
    }
    Ok(receipt.handle)
}

pub(crate) fn submit_generation(
    hosted: &HostedRuntime<FakeSource>,
    handle: ModelHandle,
    request: GenerationRequest,
) -> TestResult {
    submit_generation_with_ticket(hosted, handle, request, CommandTicket::new(2))
}

pub(crate) fn submit_generation_with_ticket(
    hosted: &HostedRuntime<FakeSource>,
    handle: ModelHandle,
    request: GenerationRequest,
    ticket: CommandTicket,
) -> TestResult {
    hosted
        .try_submit(RuntimeCommand::Generate {
            ticket,
            handle,
            request,
        })
        .map_err(|_| "generation command rejected")?;
    match hosted
        .receive_timeout(Duration::from_secs(2))
        .map_err(|error| format!("generation admission event: {error:?}"))?
    {
        RuntimeEvent::GenerationAdmitted {
            ticket: event_ticket,
            result: Ok(_),
        } if event_ticket == ticket => Ok(()),
        RuntimeEvent::GenerationAdmitted {
            result: Err(error), ..
        } => Err(format!("generation admission failed: {error:?}")),
        _ => Err("unexpected generation admission event".into()),
    }
}

pub(crate) fn submit_generation_error(
    hosted: &HostedRuntime<FakeSource>,
    handle: ModelHandle,
    request: GenerationRequest,
    ticket: CommandTicket,
) -> TestResult<inference_runtime::RuntimeError> {
    hosted
        .try_submit(RuntimeCommand::Generate {
            ticket,
            handle,
            request,
        })
        .map_err(|_| "generation command rejected")?;
    match hosted
        .receive_timeout(Duration::from_secs(2))
        .map_err(|error| format!("generation admission event: {error:?}"))?
    {
        RuntimeEvent::GenerationAdmitted {
            ticket: event_ticket,
            result: Err(error),
        } if event_ticket == ticket => Ok(error),
        _ => Err("generation admission unexpectedly succeeded".into()),
    }
}

pub(crate) fn shutdown(hosted: HostedRuntime<FakeSource>, thread: RuntimeThread) -> TestResult {
    hosted
        .try_submit(RuntimeCommand::Shutdown {
            ticket: CommandTicket::new(99),
        })
        .map_err(|_| "shutdown command rejected")?;
    match hosted
        .receive_timeout(Duration::from_secs(2))
        .map_err(|error| format!("shutdown event: {error:?}"))?
    {
        RuntimeEvent::Shutdown { result: Ok(_), .. } => {}
        _ => return Err("unexpected shutdown event".into()),
    }
    thread.join().map_err(|error| error.to_string())
}

pub(crate) fn request(
    request: u64,
    sequence: u64,
    maximum_generated_tokens: u32,
    eos_tokens: &[TokenId],
    stops: &[GenerationStopSequence],
) -> GenerationRequest {
    GenerationRequest {
        request_id: RequestId::new(request),
        sequence_id: SequenceId::new(sequence),
        prompt_tokens: vec![TokenId::new(0)].into_boxed_slice(),
        sequence: SequenceConfiguration::new(
            NonZeroU32::new(32).unwrap_or(NonZeroU32::MIN),
            NonZeroU32::new(8).unwrap_or(NonZeroU32::MIN),
        ),
        maximum_generated_tokens: NonZeroU32::new(maximum_generated_tokens)
            .unwrap_or(NonZeroU32::MIN),
        sampling: SamplingConfig::greedy(),
        seed: 17,
        eos_tokens: eos_tokens.to_vec().into_boxed_slice(),
        stop_sequences: stops.to_vec().into_boxed_slice(),
        output_capacity: GenerationOutputCapacityPolicy::default(),
    }
}

pub(crate) fn write_logits(source: &FakeSource, generated: usize, logits: &mut [f32]) {
    if source.no_candidate {
        logits.fill(f32::NEG_INFINITY);
        return;
    }
    if source.uniform_logits {
        logits.fill(0.0);
        return;
    }
    logits.fill(-100.0);
    let token = if generated < source.script_len {
        source.script.get(generated).copied().unwrap_or(0)
    } else {
        0
    };
    if let Some(logit) = logits.get_mut(token as usize) {
        *logit = 100.0;
    }
}

pub(crate) const fn descriptor(operations: CapabilitySet) -> ModelDescriptor {
    ModelDescriptor {
        backend: BACKEND,
        metadata: ModelMetadata {
            architecture: ModelArchitecture::Llama,
            configuration_declared_scalar_type: Some(ScalarType::F32),
            observed_tensor_scalar_types: ScalarTypeSet::from_scalar(ScalarType::F32),
            quantization: QuantizationFormat::None,
            vocabulary_size: 4,
            context_length: 64,
        },
        capabilities: ModelCapabilities {
            operations,
            maximum_context_tokens: 64,
            maximum_sequences: 4,
            maximum_prefill_batch: 8,
        },
        estimated_footprint: model_footprint(),
    }
}

pub(crate) const fn model_footprint() -> MemoryFootprint {
    MemoryFootprint::host_weights(ByteCount::from_u64(MODEL_HOST_BYTES))
}

pub(crate) const fn sequence_footprint() -> MemoryFootprint {
    MemoryFootprint::host_working(ByteCount::from_u64(SEQUENCE_HOST_BYTES))
}

pub(crate) const fn failure(code: u32) -> BackendFailure {
    BackendFailure::new(BACKEND, BackendFailureKind::Internal, code)
}

pub(crate) fn nonzero_usize(value: usize) -> TestResult<NonZeroUsize> {
    NonZeroUsize::new(value).ok_or_else(|| "non-zero capacity required".into())
}
