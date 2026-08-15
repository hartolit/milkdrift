use super::support::*;

#[test]
fn hosted_worker_publishes_shutdown_after_a_full_event_queue_drains() -> Result<(), String> {
    let hosted_configuration = HostedRuntimeConfiguration::new(
        non_zero_usize(2)?,
        non_zero_usize(1)?,
        NonZeroU64::new(1).ok_or("non-zero poll interval")?,
    );
    let (hosted, thread_handle) =
        start_hosted_runtime(MockLoader, limits(1, 1, 10_000), hosted_configuration)
            .map_err(|error| error.to_string())?;

    hosted
        .try_submit(RuntimeCommand::Snapshot {
            ticket: CommandTicket::new(70),
        })
        .map_err(|_| "snapshot command rejected")?;
    wait_for_queue_state(&hosted, 1, 0)?;

    hosted
        .try_submit(RuntimeCommand::Shutdown {
            ticket: CommandTicket::new(71),
        })
        .map_err(|_| "shutdown command rejected")?;
    wait_for_queue_state(&hosted, 1, 0)?;
    if thread_handle.is_finished() {
        return Err("worker stopped before publishing its shutdown event".into());
    }

    match hosted
        .receive_timeout(Duration::from_secs(2))
        .map_err(|error| format!("snapshot event failed: {error:?}"))?
    {
        RuntimeEvent::Snapshot { ticket, .. } if ticket == CommandTicket::new(70) => {}
        _ => return Err("unexpected event ahead of shutdown".into()),
    }
    match hosted
        .receive_timeout(Duration::from_secs(2))
        .map_err(|error| format!("shutdown event failed: {error:?}"))?
    {
        RuntimeEvent::Shutdown {
            ticket,
            result: Ok(_),
        } if ticket == CommandTicket::new(71) => {}
        _ => return Err("missing correlated shutdown event".into()),
    }
    thread_handle.join().map_err(|error| error.to_string())?;
    Ok(())
}

#[test]
fn hosted_worker_preserves_accepted_events_ahead_of_shutdown() -> Result<(), String> {
    let hosted_configuration = HostedRuntimeConfiguration::new(
        non_zero_usize(3)?,
        non_zero_usize(1)?,
        NonZeroU64::new(1).ok_or("non-zero poll interval")?,
    );
    let (hosted, thread_handle) =
        start_hosted_runtime(MockLoader, limits(1, 1, 10_000), hosted_configuration)
            .map_err(|error| error.to_string())?;

    hosted
        .try_submit(RuntimeCommand::Snapshot {
            ticket: CommandTicket::new(72),
        })
        .map_err(|_| "first snapshot command rejected")?;
    wait_for_queue_state(&hosted, 1, 0)?;

    hosted
        .try_submit(RuntimeCommand::Snapshot {
            ticket: CommandTicket::new(73),
        })
        .map_err(|_| "second snapshot command rejected")?;
    hosted
        .try_submit(RuntimeCommand::Shutdown {
            ticket: CommandTicket::new(74),
        })
        .map_err(|_| "shutdown command rejected")?;
    wait_for_queue_state(&hosted, 1, 0)?;

    for expected_ticket in [CommandTicket::new(72), CommandTicket::new(73)] {
        match hosted
            .receive_timeout(Duration::from_secs(2))
            .map_err(|error| format!("snapshot event failed: {error:?}"))?
        {
            RuntimeEvent::Snapshot { ticket, .. } if ticket == expected_ticket => {}
            _ => return Err("accepted snapshot was lost or reordered before shutdown".into()),
        }
    }
    match hosted
        .receive_timeout(Duration::from_secs(2))
        .map_err(|error| format!("shutdown event failed: {error:?}"))?
    {
        RuntimeEvent::Shutdown {
            ticket,
            result: Ok(_),
        } if ticket == CommandTicket::new(74) => {}
        _ => return Err("missing correlated shutdown event".into()),
    }
    thread_handle.join().map_err(|error| error.to_string())?;
    Ok(())
}

#[test]
fn hosted_worker_preserves_completed_maintenance_ahead_of_shutdown() -> Result<(), String> {
    let hosted_configuration = HostedRuntimeConfiguration::new(
        non_zero_usize(4)?,
        non_zero_usize(1)?,
        NonZeroU64::new(1).ok_or("non-zero poll interval")?,
    );
    let (hosted, thread_handle) =
        start_hosted_runtime(MockLoader, limits(1, 1, 10_000), hosted_configuration)
            .map_err(|error| error.to_string())?;

    hosted
        .try_submit(RuntimeCommand::LoadModel {
            ticket: CommandTicket::new(75),
            model_id: ModelId::new(75),
            source: MockSource {
                model_bytes: 100,
                vocabulary_size: 4,
            },
            execution_device: cpu_device(),
        })
        .map_err(|_| "load command rejected")?;
    let loaded = receive_load_receipt(&hosted)?;
    hosted
        .try_submit(RuntimeCommand::StartRequest {
            ticket: CommandTicket::new(76),
            handle: loaded.handle,
            request_id: RequestId::new(75),
            sequence_id: SequenceId::new(750),
            configuration: sequence_configuration(8, 4)?,
        })
        .map_err(|_| "start command rejected")?;
    match hosted
        .receive_timeout(Duration::from_secs(2))
        .map_err(|error| format!("start event failed: {error:?}"))?
    {
        RuntimeEvent::RequestStarted { result: Ok(_), .. } => {}
        _ => return Err("unexpected request-start event".into()),
    }

    let timeout = DrainTimeout::from_millis(2).map_err(debug_error)?;
    hosted
        .try_submit(RuntimeCommand::UnloadModel {
            ticket: CommandTicket::new(77),
            handle: loaded.handle,
            policy: UnloadPolicy::Drain { timeout },
        })
        .map_err(|_| "unload command rejected")?;
    wait_for_queue_state(&hosted, 1, 0)?;
    thread::sleep(Duration::from_millis(20));
    hosted
        .try_submit(RuntimeCommand::Shutdown {
            ticket: CommandTicket::new(78),
        })
        .map_err(|_| "shutdown command rejected")?;

    match hosted
        .receive_timeout(Duration::from_secs(2))
        .map_err(|error| format!("initial unload event failed: {error:?}"))?
    {
        RuntimeEvent::ModelUnload {
            ticket,
            result: Ok(receipt),
        } if ticket == CommandTicket::new(77) && receipt.status == UnloadStatus::Draining => {}
        _ => return Err("model did not enter draining".into()),
    }
    match hosted
        .receive_timeout(Duration::from_secs(2))
        .map_err(|error| format!("terminal unload event failed: {error:?}"))?
    {
        RuntimeEvent::ModelUnload {
            ticket,
            result: Ok(receipt),
        } if ticket == CommandTicket::new(77)
            && receipt.status == UnloadStatus::Unloaded
            && receipt.cancelled_requests == 1 => {}
        _ => return Err("completed maintenance event was lost before shutdown".into()),
    }
    match hosted
        .receive_timeout(Duration::from_secs(2))
        .map_err(|error| format!("shutdown event failed: {error:?}"))?
    {
        RuntimeEvent::Shutdown {
            ticket,
            result: Ok(_),
        } if ticket == CommandTicket::new(78) => {}
        _ => return Err("missing correlated shutdown event".into()),
    }
    thread_handle.join().map_err(|error| error.to_string())?;
    Ok(())
}

#[test]
fn hosted_worker_retries_a_failed_forced_release() -> Result<(), String> {
    let hosted_configuration = HostedRuntimeConfiguration::new(
        non_zero_usize(4)?,
        non_zero_usize(4)?,
        NonZeroU64::new(1).ok_or("non-zero poll interval")?,
    );
    let (hosted, thread_handle) =
        start_hosted_runtime(MockLoader, limits(1, 1, 10_000), hosted_configuration)
            .map_err(|error| error.to_string())?;

    hosted
        .try_submit(RuntimeCommand::LoadModel {
            ticket: CommandTicket::new(80),
            model_id: ModelId::new(8),
            source: MockSource {
                model_bytes: 100,
                vocabulary_size: 4,
            },
            execution_device: cpu_device(),
        })
        .map_err(|_| "load command rejected")?;
    let loaded = receive_load_receipt(&hosted)?;

    hosted
        .try_submit(RuntimeCommand::StartRequest {
            ticket: CommandTicket::new(81),
            handle: loaded.handle,
            request_id: RequestId::new(80),
            sequence_id: SequenceId::new(999),
            configuration: sequence_configuration(8, 4)?,
        })
        .map_err(|_| "start command rejected")?;
    match hosted
        .receive_timeout(Duration::from_secs(2))
        .map_err(|error| format!("start event failed: {error:?}"))?
    {
        RuntimeEvent::RequestStarted { result: Ok(_), .. } => {}
        _ => return Err("unexpected request-start event".into()),
    }

    hosted
        .try_submit(RuntimeCommand::UnloadModel {
            ticket: CommandTicket::new(82),
            handle: loaded.handle,
            policy: UnloadPolicy::CancelActive,
        })
        .map_err(|_| "unload command rejected")?;
    match hosted
        .receive_timeout(Duration::from_secs(2))
        .map_err(|error| format!("initial unload event failed: {error:?}"))?
    {
        RuntimeEvent::ModelUnload {
            ticket,
            result: Err(inference_runtime::RuntimeError::CleanupFailed(state)),
        } if state.failure.primary_failure == inference_runtime::FailureClass::Cancellation
            && state.failure.cleanup_failure == inference_runtime::FailureClass::Sequence
            && ticket == CommandTicket::new(82) => {}

        _ => return Err("missing initial sequence-release failure".into()),
    }

    match hosted
        .receive_timeout(Duration::from_secs(2))
        .map_err(|error| format!("retry unload event failed: {error:?}"))?
    {
        RuntimeEvent::ModelUnload {
            ticket,
            result: Ok(receipt),
        } if ticket == CommandTicket::new(82)
            && receipt.status == UnloadStatus::Unloaded
            && receipt.cancelled_requests == 1 => {}
        _ => return Err("failed sequence release was not retried".into()),
    }

    hosted
        .try_submit(RuntimeCommand::Shutdown {
            ticket: CommandTicket::new(83),
        })
        .map_err(|_| "shutdown command rejected")?;
    match hosted
        .receive_timeout(Duration::from_secs(2))
        .map_err(|error| format!("shutdown event failed: {error:?}"))?
    {
        RuntimeEvent::Shutdown { result: Ok(_), .. } => {}
        _ => return Err("unexpected shutdown event".into()),
    }
    thread_handle.join().map_err(|error| error.to_string())?;
    Ok(())
}

#[test]
#[expect(
    clippy::too_many_lines,
    reason = "the hosted regression keeps accepted, duplicate, completion, and terminal ticket ordering explicit"
)]
fn hosted_worker_reports_terminal_unload_after_natural_drain() -> Result<(), String> {
    let hosted_configuration = HostedRuntimeConfiguration::new(
        non_zero_usize(4)?,
        non_zero_usize(4)?,
        NonZeroU64::new(1).ok_or("non-zero poll interval")?,
    );
    let (hosted, thread_handle) =
        start_hosted_runtime(MockLoader, limits(1, 1, 10_000), hosted_configuration)
            .map_err(|error| error.to_string())?;

    hosted
        .try_submit(RuntimeCommand::LoadModel {
            ticket: CommandTicket::new(90),
            model_id: ModelId::new(9),
            source: MockSource {
                model_bytes: 100,
                vocabulary_size: 4,
            },
            execution_device: cpu_device(),
        })
        .map_err(|_| "load command rejected")?;
    let loaded = receive_load_receipt(&hosted)?;

    hosted
        .try_submit(RuntimeCommand::StartRequest {
            ticket: CommandTicket::new(91),
            handle: loaded.handle,
            request_id: RequestId::new(90),
            sequence_id: SequenceId::new(900),
            configuration: sequence_configuration(8, 4)?,
        })
        .map_err(|_| "start command rejected")?;
    match hosted
        .receive_timeout(Duration::from_secs(2))
        .map_err(|error| format!("start event failed: {error:?}"))?
    {
        RuntimeEvent::RequestStarted { result: Ok(_), .. } => {}
        _ => return Err("unexpected request-start event".into()),
    }

    let timeout = DrainTimeout::from_millis(5_000).map_err(debug_error)?;
    hosted
        .try_submit(RuntimeCommand::UnloadModel {
            ticket: CommandTicket::new(92),
            handle: loaded.handle,
            policy: UnloadPolicy::Drain { timeout },
        })
        .map_err(|_| "unload command rejected")?;
    match hosted
        .receive_timeout(Duration::from_secs(2))
        .map_err(|error| format!("drain event failed: {error:?}"))?
    {
        RuntimeEvent::ModelUnload {
            ticket,
            result: Ok(receipt),
        } if ticket == CommandTicket::new(92) && receipt.status == UnloadStatus::Draining => {}
        _ => return Err("model did not enter draining".into()),
    }

    hosted
        .try_submit(RuntimeCommand::UnloadModel {
            ticket: CommandTicket::new(93),
            handle: loaded.handle,
            policy: UnloadPolicy::CancelActive,
        })
        .map_err(|_| "duplicate unload command rejected by the queue")?;
    match hosted
        .receive_timeout(Duration::from_secs(2))
        .map_err(|error| format!("duplicate unload event failed: {error:?}"))?
    {
        RuntimeEvent::ModelUnload {
            ticket,
            result: Err(_),
        } if ticket == CommandTicket::new(93) => {}
        _ => return Err("duplicate unload was not rejected on its own ticket".into()),
    }

    hosted
        .try_submit(RuntimeCommand::CompleteRequest {
            ticket: CommandTicket::new(94),
            request_id: RequestId::new(90),
            reason: FinishReason::StopCondition,
        })
        .map_err(|_| "completion command rejected")?;
    match hosted
        .receive_timeout(Duration::from_secs(2))
        .map_err(|error| format!("request completion event failed: {error:?}"))?
    {
        RuntimeEvent::RequestFinished {
            ticket,
            result: Ok(FinishReason::StopCondition),
            ..
        } if ticket == CommandTicket::new(94) => {}
        _ => return Err("unexpected request completion event".into()),
    }

    match hosted
        .receive_timeout(Duration::from_secs(2))
        .map_err(|error| format!("terminal natural-drain event failed: {error:?}"))?
    {
        RuntimeEvent::ModelUnload {
            ticket,
            result: Ok(receipt),
        } if ticket == CommandTicket::new(92)
            && receipt.status == UnloadStatus::Unloaded
            && receipt.cancelled_requests == 0 => {}
        _ => return Err("natural drain did not emit terminal unload".into()),
    }

    hosted
        .try_submit(RuntimeCommand::Shutdown {
            ticket: CommandTicket::new(95),
        })
        .map_err(|_| "shutdown command rejected")?;
    match hosted
        .receive_timeout(Duration::from_secs(2))
        .map_err(|error| format!("shutdown event failed: {error:?}"))?
    {
        RuntimeEvent::Shutdown { result: Ok(_), .. } => {}
        _ => return Err("unexpected shutdown event".into()),
    }
    thread_handle.join().map_err(|error| error.to_string())?;
    Ok(())
}

#[test]
#[expect(
    clippy::too_many_lines,
    reason = "the bounded-queue deadline scenario keeps event backpressure and unload ordering explicit"
)]
fn hosted_worker_enforces_deadline_while_event_queue_is_full() -> Result<(), String> {
    let hosted_configuration = HostedRuntimeConfiguration::new(
        non_zero_usize(4)?,
        non_zero_usize(1)?,
        NonZeroU64::new(1).ok_or("non-zero poll interval")?,
    );
    let (hosted, thread_handle) =
        start_hosted_runtime(MockLoader, limits(1, 1, 10_000), hosted_configuration)
            .map_err(|error| error.to_string())?;

    hosted
        .try_submit(RuntimeCommand::LoadModel {
            ticket: CommandTicket::new(1),
            model_id: ModelId::new(4),
            source: MockSource {
                model_bytes: 100,
                vocabulary_size: 4,
            },
            execution_device: cpu_device(),
        })
        .map_err(|_| "load command rejected")?;
    let loaded = receive_load_receipt(&hosted)?;

    hosted
        .try_submit(RuntimeCommand::StartRequest {
            ticket: CommandTicket::new(2),
            handle: loaded.handle,
            request_id: RequestId::new(40),
            sequence_id: SequenceId::new(400),
            configuration: sequence_configuration(8, 4)?,
        })
        .map_err(|_| "start command rejected")?;
    match hosted
        .receive_timeout(Duration::from_secs(2))
        .map_err(|error| format!("start event failed: {error:?}"))?
    {
        RuntimeEvent::RequestStarted { result: Ok(_), .. } => {}
        _ => return Err("unexpected request-start event".into()),
    }

    let timeout = DrainTimeout::from_millis(2).map_err(debug_error)?;
    hosted
        .try_submit(RuntimeCommand::UnloadModel {
            ticket: CommandTicket::new(3),
            handle: loaded.handle,
            policy: UnloadPolicy::Drain { timeout },
        })
        .map_err(|_| "unload command rejected")?;
    thread::sleep(Duration::from_millis(20));
    match hosted
        .receive_timeout(Duration::from_secs(2))
        .map_err(|error| format!("unload event failed: {error:?}"))?
    {
        RuntimeEvent::ModelUnload {
            result: Ok(receipt),
            ..
        } if receipt.status == UnloadStatus::Draining => {}
        _ => return Err("unexpected unload event".into()),
    }

    match hosted
        .receive_timeout(Duration::from_secs(2))
        .map_err(|error| format!("terminal unload event failed: {error:?}"))?
    {
        RuntimeEvent::ModelUnload {
            ticket,
            result: Ok(receipt),
        } if ticket == CommandTicket::new(3)
            && receipt.status == UnloadStatus::Unloaded
            && receipt.cancelled_requests == 1 => {}
        _ => return Err("missing terminal unload event after drain timeout".into()),
    }

    hosted
        .try_submit(RuntimeCommand::Snapshot {
            ticket: CommandTicket::new(4),
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
        } if runtime.loaded_models == 0
            && runtime.active_requests == 0
            && runtime.unverified_ownership.is_none()
            && !runtime.admission_blocked
            && models.is_empty()
            && retained_models.is_empty() => {}
        _ => return Err("deadline did not reclaim model under event backpressure".into()),
    }

    hosted
        .try_submit(RuntimeCommand::Shutdown {
            ticket: CommandTicket::new(5),
        })
        .map_err(|_| "shutdown command rejected")?;
    match hosted
        .receive_timeout(Duration::from_secs(2))
        .map_err(|error| format!("shutdown event failed: {error:?}"))?
    {
        RuntimeEvent::Shutdown { result: Ok(_), .. } => {}
        _ => return Err("unexpected shutdown event".into()),
    }
    thread_handle.join().map_err(|error| error.to_string())?;
    Ok(())
}
