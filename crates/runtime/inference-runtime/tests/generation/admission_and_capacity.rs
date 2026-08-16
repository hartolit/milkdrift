use super::support::*;

#[test]
fn generation_admission_rejects_oversized_prefill_before_native_creation() -> TestResult {
    let source = FakeSource::scripted([1; 8], 8);
    let (hosted, thread, counters, handle) = hosted(source, 8, 16)?;
    let mut generation = request(80, 800, 4, &[], &[]);
    generation.prompt_tokens = vec![TokenId::new(0); 9].into_boxed_slice();

    let error = submit_generation_error(&hosted, handle, generation, CommandTicket::new(80))?;
    assert!(matches!(
        error,
        inference_runtime::RuntimeError::CapacityExhausted(_)
    ));
    {
        let counters = counters.lock().map_err(|_| "counter mutex poisoned")?;
        assert_eq!(counters.sequence_creations, 0);
        assert_eq!(counters.retained_memory_bytes, MODEL_HOST_BYTES);
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

#[test]
fn generation_admission_rejects_insufficient_output_capacity_before_native_creation() -> TestResult
{
    let source = FakeSource::scripted([1; 8], 8);
    let (hosted, thread, counters, handle) = hosted(source, 1, 4)?;
    let mut generation = request(81, 801, 4, &[], &[]);
    generation.output_capacity =
        GenerationOutputCapacityPolicy::new(nonzero_usize(2)?, NonZeroUsize::MIN);

    let error = submit_generation_error(&hosted, handle, generation, CommandTicket::new(81))?;
    assert!(matches!(
        error,
        inference_runtime::RuntimeError::CapacityExhausted(_)
    ));
    assert_eq!(
        counters
            .lock()
            .map_err(|_| "counter mutex poisoned")?
            .sequence_creations,
        0
    );
    shutdown(hosted, thread)
}

#[test]
fn generation_workspace_bytes_are_admitted_before_native_sequence_creation() -> TestResult {
    let source = FakeSource::scripted([1; 8], 8);
    let (hosted, thread, counters, handle) = hosted_with_budget(source, 8, 16, 219)?;

    let error = submit_generation_error(
        &hosted,
        handle,
        request(82, 802, 4, &[], &[]),
        CommandTicket::new(82),
    )?;
    assert!(matches!(
        error,
        inference_runtime::RuntimeError::InsufficientMemory {
            kind: inference_runtime::MemoryKind::Host,
            required_bytes: 220,
            available_bytes: 219,
        }
    ));
    {
        let counters = counters.lock().map_err(|_| "counter mutex poisoned")?;
        assert_eq!(counters.sequence_creations, 0);
        assert_eq!(counters.retained_memory_bytes, MODEL_HOST_BYTES);
        drop(counters);
    }
    shutdown(hosted, thread)
}

#[test]
fn generation_workspace_accounting_is_retained_until_terminal_output_release() -> TestResult {
    let source = FakeSource::scripted([1; 8], 8);
    let (hosted, thread, counters, handle) = hosted(source, 1, 1)?;
    submit_generation(&hosted, handle, request(89, 809, 1, &[], &[]))?;

    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        if counters
            .lock()
            .map_err(|_| "counter mutex poisoned")?
            .successful_destructions
            == 1
        {
            break;
        }
        if Instant::now() >= deadline {
            return Err("sequence cleanup did not complete".into());
        }
        std::thread::yield_now();
    }

    hosted
        .try_submit(RuntimeCommand::Snapshot {
            ticket: CommandTicket::new(90),
        })
        .map_err(|_| "snapshot command rejected")?;
    match hosted
        .receive_timeout(Duration::from_secs(2))
        .map_err(|error| format!("snapshot event failed: {error:?}"))?
    {
        RuntimeEvent::Snapshot {
            runtime,
            retained_models,
            ..
        } => {
            assert_eq!(runtime.active_requests, 0);
            assert_eq!(runtime.generation_workspaces, 1);
            assert_eq!(
                runtime.reserved_generation_workspace,
                MemoryFootprint {
                    host_weight_bytes: 0,
                    device_weight_bytes: 0,
                    host_working_bytes: 64,
                    device_working_bytes: 0,
                }
            );
            assert_eq!(
                runtime.reserved_footprint,
                MemoryFootprint {
                    host_weight_bytes: MODEL_HOST_BYTES,
                    device_weight_bytes: 0,
                    host_working_bytes: 64,
                    device_working_bytes: 0,
                }
            );
            assert!(runtime.unverified_ownership.is_none());
            assert!(!runtime.admission_blocked);
            assert!(retained_models.is_empty());
        }
        _ => return Err("unexpected snapshot event".into()),
    }

    hosted
        .try_submit(RuntimeCommand::StartRequest {
            ticket: CommandTicket::new(92),
            handle,
            request_id: RequestId::new(89),
            sequence_id: SequenceId::new(899),
            configuration: SequenceConfiguration::new(
                NonZeroU32::new(32).unwrap_or(NonZeroU32::MIN),
                NonZeroU32::new(8).unwrap_or(NonZeroU32::MIN),
            ),
        })
        .map_err(|_| "request reuse command rejected")?;
    assert!(matches!(
        hosted
            .receive_timeout(Duration::from_secs(2))
            .map_err(|error| format!("request reuse event failed: {error:?}"))?,
        RuntimeEvent::RequestStarted {
            result: Err(inference_runtime::RuntimeError::RequestAlreadyActive(request_id)),
            ..
        } if request_id == RequestId::new(89)
    ));

    let output = collect_until_released(&hosted, RequestId::new(89), Duration::from_secs(2))?;
    assert_eq!(output.tokens, vec![TokenId::new(1)]);

    hosted
        .try_submit(RuntimeCommand::Snapshot {
            ticket: CommandTicket::new(91),
        })
        .map_err(|_| "snapshot command rejected")?;
    match hosted
        .receive_timeout(Duration::from_secs(2))
        .map_err(|error| format!("snapshot event failed: {error:?}"))?
    {
        RuntimeEvent::Snapshot {
            runtime,
            retained_models,
            ..
        } => {
            assert_eq!(runtime.generation_workspaces, 0);
            assert_eq!(
                runtime.reserved_generation_workspace,
                MemoryFootprint::default()
            );
            assert_eq!(runtime.reserved_footprint, model_footprint());
            assert!(runtime.unverified_ownership.is_none());
            assert!(!runtime.admission_blocked);
            assert!(retained_models.is_empty());
        }
        _ => return Err("unexpected snapshot event".into()),
    }
    shutdown(hosted, thread)
}

#[test]
fn generation_admission_requires_exact_full_vocabulary_logits() -> TestResult {
    let mut source = FakeSource::scripted([1; 8], 8);
    source.logits_capacity = 3;
    let (hosted, thread, counters, handle) = hosted(source, 8, 16)?;

    let error = submit_generation_error(
        &hosted,
        handle,
        request(83, 803, 4, &[], &[]),
        CommandTicket::new(83),
    )?;
    assert_eq!(
        error,
        inference_runtime::RuntimeError::BackendContractViolation
    );
    assert_eq!(
        counters
            .lock()
            .map_err(|_| "counter mutex poisoned")?
            .sequence_creations,
        0
    );
    shutdown(hosted, thread)
}

#[test]
fn scheduled_generation_requires_prefill_and_incremental_decode_capabilities() -> TestResult {
    let common = CapabilitySet::MULTIPLE_SEQUENCES.union(CapabilitySet::EXPLICIT_SYNCHRONIZATION);
    for (operations, request_id) in [
        (common.union(CapabilitySet::INCREMENTAL_DECODE), 83),
        (common.union(CapabilitySet::PREFILL), 84),
    ] {
        let mut source = FakeSource::scripted([1; 8], 8);
        source.operations = operations;
        let (hosted, thread, counters, handle) = hosted(source, 8, 16)?;

        let error = submit_generation_error(
            &hosted,
            handle,
            request(request_id, request_id + 1_000, 4, &[], &[]),
            CommandTicket::new(request_id),
        )?;
        assert_eq!(error, RuntimeError::Model(ModelError::Unsupported));
        assert_eq!(
            counters
                .lock()
                .map_err(|_| "counter mutex poisoned")?
                .sequence_creations,
            0
        );
        shutdown(hosted, thread)?;
    }
    Ok(())
}

#[test]
fn scheduled_generation_configuration_respects_advertised_limits() -> TestResult {
    let source = FakeSource::scripted([1; 8], 8);
    let (hosted, thread, counters, handle) = hosted(source, 8, 16)?;
    let configurations = [
        (
            85,
            SequenceConfiguration::new(
                NonZeroU32::new(65).unwrap_or(NonZeroU32::MIN),
                NonZeroU32::new(8).unwrap_or(NonZeroU32::MIN),
            ),
        ),
        (
            86,
            SequenceConfiguration::new(
                NonZeroU32::new(32).unwrap_or(NonZeroU32::MIN),
                NonZeroU32::new(9).unwrap_or(NonZeroU32::MIN),
            ),
        ),
        (
            87,
            SequenceConfiguration::new(
                NonZeroU32::new(4).unwrap_or(NonZeroU32::MIN),
                NonZeroU32::new(5).unwrap_or(NonZeroU32::MIN),
            ),
        ),
    ];

    for (request_id, configuration) in configurations {
        let mut generation = request(request_id, request_id + 1_000, 1, &[], &[]);
        generation.sequence = configuration;
        let error =
            submit_generation_error(&hosted, handle, generation, CommandTicket::new(request_id))?;
        assert_eq!(error, RuntimeError::Model(ModelError::Unsupported));
    }
    assert_eq!(
        counters
            .lock()
            .map_err(|_| "counter mutex poisoned")?
            .sequence_creations,
        0
    );
    shutdown(hosted, thread)
}

#[test]
fn direct_prefill_and_decode_require_advertised_capabilities() -> TestResult {
    let common = CapabilitySet::MULTIPLE_SEQUENCES.union(CapabilitySet::EXPLICIT_SYNCHRONIZATION);

    let mut source = FakeSource::scripted([1; 8], 8);
    source.operations = common.union(CapabilitySet::INCREMENTAL_DECODE);
    let (mut runtime, counters, handle) = synchronous_runtime(&source)?;
    let request_id = RequestId::new(88);
    runtime
        .start_request(
            handle,
            request_id,
            SequenceId::new(888),
            direct_sequence_configuration(),
        )
        .map_err(debug_error)?;
    assert_eq!(
        runtime.prefill(request_id, &[TokenId::new(0)], true, &mut [0.0; 4]),
        Err(RuntimeError::Sequence(SequenceError::Unsupported))
    );
    assert!(!runtime.is_request_active(request_id));
    runtime.shutdown().map_err(debug_error)?;
    {
        let counters = counters.lock().map_err(|_| "counter mutex poisoned")?;
        assert_eq!(counters.prefill_calls, 0);
        assert_eq!(counters.successful_destructions, 1);
        assert_eq!(counters.retained_memory_bytes, 0);
        drop(counters);
    }

    let mut source = FakeSource::scripted([1; 8], 8);
    source.operations = common.union(CapabilitySet::PREFILL);
    let (mut runtime, counters, handle) = synchronous_runtime(&source)?;
    let request_id = RequestId::new(89);
    runtime
        .start_request(
            handle,
            request_id,
            SequenceId::new(889),
            direct_sequence_configuration(),
        )
        .map_err(debug_error)?;
    runtime
        .prefill(request_id, &[TokenId::new(0)], true, &mut [0.0; 4])
        .map_err(debug_error)?;
    assert_eq!(
        runtime.decode(request_id, TokenId::new(1), &mut [0.0; 4]),
        Err(RuntimeError::Sequence(SequenceError::Unsupported))
    );
    assert!(!runtime.is_request_active(request_id));
    runtime.shutdown().map_err(debug_error)?;
    {
        let counters = counters.lock().map_err(|_| "counter mutex poisoned")?;
        assert_eq!(counters.prefill_calls, 1);
        assert_eq!(counters.decode_calls, 0);
        assert_eq!(counters.successful_destructions, 1);
        assert_eq!(counters.retained_memory_bytes, 0);
        drop(counters);
    }
    Ok(())
}

#[test]
fn ready_results_preserve_sequence_state_identity_and_capacity() -> TestResult {
    let cases = [
        (ContractFaults::PREFILL_INVALID_STATE, false, false, 110),
        (ContractFaults::PREFILL_MUTATED_IDENTITY, false, true, 111),
        (ContractFaults::PREFILL_MUTATED_CAPACITY, false, false, 112),
        (ContractFaults::DECODE_INVALID_STATE, true, false, 113),
        (ContractFaults::DECODE_MUTATED_IDENTITY, true, false, 114),
        (ContractFaults::DECODE_MUTATED_CAPACITY, true, false, 115),
    ];

    for (fault, decode_fault, quarantine, value) in cases {
        let mut source = FakeSource::scripted([1; 8], 8);
        source.contract_faults = fault;
        source.destroy_failures = u32::from(quarantine);
        let (mut runtime, counters, handle) = synchronous_runtime(&source)?;
        let request_id = RequestId::new(value);
        let sequence_id = SequenceId::new(value + 1_000);
        runtime
            .start_request(
                handle,
                request_id,
                sequence_id,
                direct_sequence_configuration(),
            )
            .map_err(debug_error)?;
        let mut logits = [0.0; 4];
        if decode_fault {
            runtime
                .prefill(request_id, &[TokenId::new(0)], true, &mut logits)
                .map_err(debug_error)?;
        }
        let result = if decode_fault {
            runtime
                .decode(request_id, TokenId::new(1), &mut logits)
                .map(|_| ())
        } else {
            runtime
                .prefill(request_id, &[TokenId::new(0)], true, &mut logits)
                .map(|_| ())
        };
        assert_eq!(result, Err(RuntimeError::BackendContractViolation));
        assert!(!runtime.is_request_active(request_id));

        if quarantine {
            let cleanup = runtime
                .request_cleanup_state(request_id)
                .ok_or("missing quarantined sequence")?;
            assert_eq!(
                cleanup.failure.primary_failure,
                FailureClass::BackendContract
            );
            assert!(matches!(
                cleanup.resource,
                CleanupResource::Sequence {
                    handle: retained_handle,
                    request_id: retained_request,
                    sequence_id: retained_sequence,
                } if retained_handle == handle
                    && retained_request == request_id
                    && retained_sequence == sequence_id
            ));
            assert_eq!(
                cleanup.ownership,
                RetainedOwnership::Unverified {
                    accepted_footprint: sequence_footprint(),
                    reported_footprint: sequence_footprint(),
                    conservative_footprint: inference_runtime::ConservativeFootprint::Known(
                        sequence_footprint(),
                    ),
                }
            );
            assert!(matches!(
                runtime.poll_cleanup().map_err(debug_error)?,
                CleanupPoll::Released(_)
            ));
        } else {
            assert!(!runtime.is_request_cleanup_pending(request_id));
        }

        let reuse_request = RequestId::new(value + 2_000);
        runtime
            .start_request(
                handle,
                reuse_request,
                sequence_id,
                direct_sequence_configuration(),
            )
            .map_err(debug_error)?;
        runtime
            .cancel_request(
                reuse_request,
                domain_contracts::CancellationReason::UserRequested,
            )
            .map_err(debug_error)?;
        runtime.shutdown().map_err(debug_error)?;
        let counters = counters.lock().map_err(|_| "counter mutex poisoned")?;
        assert_eq!(counters.successful_destructions, 2);
        assert_eq!(counters.active_sequences, 0);
        assert_eq!(counters.retained_memory_bytes, 0);
        drop(counters);
    }
    Ok(())
}
