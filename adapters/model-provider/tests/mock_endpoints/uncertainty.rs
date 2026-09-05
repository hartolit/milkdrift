use super::*;

#[test]
fn streaming_cancellation_is_cooperative_and_does_not_claim_remote_termination() -> TestResult {
    let (address, ready, server) = serve_delayed_stream(format!(
        "data: {}\n\n",
        json!({"choices":[{"delta":{"content":"partial"},"finish_reason":null}]})
    ))?;
    let profile = profile(
        &address,
        "openai-cancel",
        ProviderProtocol::OpenAiCompatible {
            path: "v1/chat/completions".to_owned(),
        },
        AuthMode::NoAuth,
        BTreeSet::from([ModelFeature::Streaming, ModelFeature::SystemRole]),
    )?;
    let task = ModelTaskRequest::new(
        vec![Message::new(
            MessageRole::User,
            vec![ContentPart::Text {
                text: "wait".to_owned(),
            }],
            None,
        )?],
        Vec::new(),
        None,
        SessionSelection::Fresh,
        None,
        64,
        true,
        BTreeMap::new(),
    )?;
    let capability = CapabilityId::new("model-mock")?;
    let descriptor = descriptor_for_profile(capability.clone(), &profile)?;
    let snapshot = ResolvedCapabilitySnapshot::from_descriptor(
        &descriptor,
        &OperationId::new("model.generate")?,
    )?;
    let revision = revision()?;
    let data = Arc::new(MockData::default());
    let (manifest, context) = manifest(&data, &revision)?;
    let request = request(&capability, &profile, manifest, task, Vec::new())?;
    let invocation = request.invocation().clone();
    let adapter = Arc::new(ModelEndpointAdapter::new(
        capability,
        profile,
        Arc::new(InMemorySecretResolver::new()),
        data,
    )?);
    adapter.start()?;
    let reporter = Arc::new(Reporter::default());
    let worker_adapter = adapter.clone();
    let worker_reporter = reporter.clone();
    let worker = thread::spawn(move || {
        worker_adapter.execute(
            &AdapterInvocation::with_context(&snapshot, &request, &context),
            worker_reporter.as_ref(),
        )
    });
    ready.recv_timeout(Duration::from_secs(2))?;
    let acknowledgement = adapter.cancel(&CancellationRequest::new(invocation, 1, "stop")?)?;
    assert!(acknowledgement.accepted());
    assert!(!acknowledgement.terminal_boundary());
    worker.join().map_err(|_| "adapter worker panicked")??;
    server.join().map_err(|_| "server panicked")??;
    let events = reporter.events()?;
    assert_eq!(
        events
            .last()
            .and_then(|event| event.kind().terminal())
            .map(|terminal| terminal.status()),
        Some(TerminalStatus::Uncertain)
    );
    let terminal = events
        .last()
        .and_then(|event| event.kind().terminal())
        .ok_or("cancellation omitted its uncertain terminal")?;
    assert_eq!(terminal.side_effect(), SideEffectClass::Unknown);
    assert_eq!(
        terminal.failure().map(|failure| failure.code()),
        Some("model_cancellation_unconfirmed")
    );
    Ok(())
}

#[test]
fn malformed_and_truncated_provider_responses_remain_uncertain_without_partial_artifacts()
-> TestResult {
    let cases = [
        ("malformed-json", "application/json", "{".to_owned(), false),
        (
            "truncated-sse",
            "text/event-stream",
            format!(
                "data: {}\n\ndata: {{\"choices\"",
                json!({"choices":[{"delta":{"content":"partial"},"finish_reason":null}]})
            ),
            true,
        ),
    ];
    for (identity, media_type, body, streaming) in cases {
        let (address, server) = serve(body, media_type)?;
        let profile = profile(
            &address,
            identity,
            ProviderProtocol::OpenAiCompatible {
                path: "v1/chat/completions".to_owned(),
            },
            AuthMode::NoAuth,
            BTreeSet::from([ModelFeature::Streaming, ModelFeature::SystemRole]),
        )?;
        let data = Arc::new(MockData::default());
        let events = execute(
            profile,
            ordinary_task(streaming)?,
            data.clone(),
            Arc::new(InMemorySecretResolver::new()),
        )?;
        server.join().map_err(|_| "server panicked")??;
        assert_uncertain_without_outputs(&events, data.as_ref())?;
        if streaming {
            assert!(events.iter().any(|event| event.kind().progress().is_some()));
        }
    }
    Ok(())
}

#[test]
fn response_bounds_and_idle_timeout_remain_uncertain() -> TestResult {
    let (address, oversized_server) = serve("x".repeat(2_048), "application/json")?;
    let mut bounded_limits = limits();
    bounded_limits.max_response_bytes = 1_024;
    let bounded_profile = profile_with_limits(
        &address,
        "oversized-response",
        ProviderProtocol::OpenAiCompatible {
            path: "v1/chat/completions".to_owned(),
        },
        AuthMode::NoAuth,
        BTreeSet::from([ModelFeature::SystemRole]),
        bounded_limits,
    )?;
    let bounded_data = Arc::new(MockData::default());
    let bounded_events = execute(
        bounded_profile,
        ordinary_task(false)?,
        bounded_data.clone(),
        Arc::new(InMemorySecretResolver::new()),
    )?;
    oversized_server.join().map_err(|_| "server panicked")??;
    assert_uncertain_without_outputs(&bounded_events, bounded_data.as_ref())?;

    let (address, stalled_server) = serve_stalled_body()?;
    let mut timeout_limits = limits();
    timeout_limits.connect_timeout_ms = 50;
    timeout_limits.request_timeout_ms = 100;
    timeout_limits.idle_timeout_ms = 50;
    let stalled_profile = profile_with_limits(
        &address,
        "stalled-response",
        ProviderProtocol::OpenAiCompatible {
            path: "v1/chat/completions".to_owned(),
        },
        AuthMode::NoAuth,
        BTreeSet::from([ModelFeature::SystemRole]),
        timeout_limits,
    )?;
    let stalled_data = Arc::new(MockData::default());
    let stalled_events = execute(
        stalled_profile,
        ordinary_task(false)?,
        stalled_data.clone(),
        Arc::new(InMemorySecretResolver::new()),
    )?;
    stalled_server.join().map_err(|_| "server panicked")??;
    assert_uncertain_without_outputs(&stalled_events, stalled_data.as_ref())?;
    Ok(())
}

#[test]
fn connection_close_after_request_entry_is_a_bounded_external_failure() -> TestResult {
    let (address, server) = serve_drop_after_request()?;
    let profile = profile(
        &address,
        "post-entry-close",
        ProviderProtocol::OpenAiCompatible {
            path: "v1/chat/completions".to_owned(),
        },
        AuthMode::NoAuth,
        BTreeSet::from([ModelFeature::SystemRole]),
    )?;
    let error = execute(
        profile,
        ordinary_task(false)?,
        Arc::new(MockData::default()),
        Arc::new(InMemorySecretResolver::new()),
    )
    .err()
    .ok_or("closed provider connection unexpectedly completed")?;
    server.join().map_err(|_| "server panicked")??;
    let message = error.to_string();
    assert!(message.contains("transport failed after request entry"));
    assert!(!message.contains("HTTP/1.1"));
    Ok(())
}

fn ordinary_task(streaming: bool) -> TestResult<ModelTaskRequest> {
    Ok(ModelTaskRequest::new(
        vec![Message::new(
            MessageRole::User,
            vec![ContentPart::Text {
                text: "bounded hostile endpoint".to_owned(),
            }],
            None,
        )?],
        Vec::new(),
        None,
        SessionSelection::Fresh,
        None,
        64,
        streaming,
        BTreeMap::new(),
    )?)
}

fn assert_uncertain_without_outputs(events: &[InvocationEvent], data: &MockData) -> TestResult {
    let terminal = events
        .last()
        .and_then(|event| event.kind().terminal())
        .ok_or("hostile response omitted terminal evidence")?;
    assert_eq!(terminal.status(), TerminalStatus::Uncertain);
    assert_eq!(terminal.side_effect(), SideEffectClass::Unknown);
    assert!(events.iter().all(|event| event.kind().output().is_none()));
    assert!(
        data.published
            .lock()
            .map_err(|_| "published lock")?
            .is_empty()
    );
    Ok(())
}
