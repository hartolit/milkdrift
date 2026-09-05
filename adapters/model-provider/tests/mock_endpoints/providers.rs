use super::*;

#[test]
fn openai_compatible_non_streaming_preserves_tools_structure_usage_and_artifacts() -> TestResult {
    let response = json!({
        "id":"response-1","model":"mock-model",
        "choices":[{"message":{"content":"{\"answer\":42}","tool_calls":[{
            "id":"call-1","type":"function","function":{"name":"lookup","arguments":"{\"id\":\"x\"}"}
        }]},"finish_reason":"tool_calls"}],
        "usage":{"prompt_tokens":12,"completion_tokens":7,"prompt_tokens_details":{"cached_tokens":3}}
    })
    .to_string();
    let (address, server) = serve(response, "application/json")?;
    let profile = profile(
        &address,
        "openai-local",
        ProviderProtocol::OpenAiCompatible {
            path: "v1/chat/completions".to_owned(),
        },
        AuthMode::NoAuth,
        BTreeSet::from([
            ModelFeature::SystemRole,
            ModelFeature::Tools,
            ModelFeature::StructuredOutput,
        ]),
    )?;
    let task = ModelTaskRequest::new(
        vec![Message::new(
            MessageRole::User,
            vec![ContentPart::Text {
                text: "answer".to_owned(),
            }],
            None,
        )?],
        vec![ToolDefinition::new(
            "lookup",
            "lookup",
            BoundedJson::new(json!({"type":"object"}))?,
        )?],
        Some(StructuredOutput::new(
            "answer",
            BoundedJson::new(json!({"type":"object"}))?,
            true,
        )?),
        SessionSelection::Fresh,
        None,
        128,
        false,
        BTreeMap::new(),
    )?;
    let data = Arc::new(MockData::default());
    let events = execute(
        profile,
        task,
        data.clone(),
        Arc::new(InMemorySecretResolver::new()),
    )?;
    let captured = server.join().map_err(|_| "server panicked")??;
    assert!(captured.starts_with("POST /v1/chat/completions HTTP/1.1"));
    assert!(captured.contains("\"response_format\""));
    assert!(captured.contains("\"tools\""));
    assert_eq!(
        events
            .last()
            .and_then(|event| event.kind().terminal())
            .map(|terminal| terminal.status()),
        Some(TerminalStatus::Success),
        "{events:#?}"
    );
    assert!(events.iter().any(|event| {
        event
            .kind()
            .output()
            .is_some_and(|(name, _)| name == "tool_calls")
    }));
    assert!(events.iter().any(|event| {
        event
            .kind()
            .output()
            .is_some_and(|(name, _)| name == "structured_output")
    }));
    assert_eq!(
        data.published.lock().map_err(|_| "published lock")?.len(),
        5
    );
    Ok(())
}

#[test]
fn anthropic_native_streaming_maps_events_and_required_headers() -> TestResult {
    let body = [
        json!({"type":"message_start","message":{"usage":{"input_tokens":9}}}),
        json!({"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}),
        json!({"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"hello"}}),
        json!({"type":"content_block_stop","index":0}),
        json!({"type":"message_delta","delta":{"stop_reason":"end_turn"},"usage":{"output_tokens":2}}),
        json!({"type":"message_stop"}),
    ]
    .into_iter()
    .map(|value| format!("data: {value}\n\n"))
    .collect::<String>();
    let (address, server) = serve(body, "text/event-stream")?;
    let secret_ref = milkdrift_authority::SecretRef::new("secret:anthropic-test")?;
    let resolver = Arc::new(InMemorySecretResolver::new());
    resolver.insert(secret_ref.clone(), b"test-secret-value".to_vec())?;
    let profile = profile(
        &address,
        "anthropic-local",
        ProviderProtocol::Anthropic {
            version: "2023-06-01".to_owned(),
            path: "v1/messages".to_owned(),
        },
        AuthMode::AnthropicApiKey { secret: secret_ref },
        BTreeSet::from([ModelFeature::Streaming, ModelFeature::SystemRole]),
    )?;
    let task = ModelTaskRequest::new(
        vec![Message::new(
            MessageRole::User,
            vec![ContentPart::Text {
                text: "say hello".to_owned(),
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
    let events = execute(profile, task, Arc::new(MockData::default()), resolver)?;
    let captured = server.join().map_err(|_| "server panicked")??;
    assert!(captured.starts_with("POST /v1/messages HTTP/1.1"));
    assert!(
        captured
            .to_ascii_lowercase()
            .contains("anthropic-version: 2023-06-01")
    );
    assert!(
        captured
            .to_ascii_lowercase()
            .contains("x-api-key: test-secret-value")
    );
    assert!(events.iter().any(|event| {
        event
            .kind()
            .progress()
            .is_some_and(|(text, _, _)| text == "hello")
    }));
    assert!(matches!(
        events.last().map(InvocationEvent::kind),
        Some(InvocationEventKind::Terminal { .. })
    ));
    Ok(())
}

#[test]
fn anthropic_native_non_streaming_preserves_tool_calls_usage_and_finish() -> TestResult {
    let response = json!({
        "id":"message-1",
        "model":"mock-model",
        "content":[
            {"type":"text","text":"checking"},
            {"type":"tool_use","id":"tool-1","name":"lookup","input":{"id":"x"}}
        ],
        "stop_reason":"tool_use",
        "usage":{"input_tokens":8,"output_tokens":3,"cache_read_input_tokens":2}
    })
    .to_string();
    let (address, server) = serve(response, "application/json")?;
    let profile = profile(
        &address,
        "anthropic-local-tools",
        ProviderProtocol::Anthropic {
            version: "2023-06-01".to_owned(),
            path: "v1/messages".to_owned(),
        },
        AuthMode::NoAuth,
        BTreeSet::from([ModelFeature::SystemRole, ModelFeature::Tools]),
    )?;
    let task = ModelTaskRequest::new(
        vec![Message::new(
            MessageRole::User,
            vec![ContentPart::Text {
                text: "look up x".to_owned(),
            }],
            None,
        )?],
        vec![ToolDefinition::new(
            "lookup",
            "lookup",
            BoundedJson::new(json!({"type":"object"}))?,
        )?],
        None,
        SessionSelection::Fresh,
        None,
        64,
        false,
        BTreeMap::new(),
    )?;
    let data = Arc::new(MockData::default());
    let events = execute(
        profile,
        task,
        data.clone(),
        Arc::new(InMemorySecretResolver::new()),
    )?;
    let captured = server.join().map_err(|_| "server panicked")??;
    assert!(captured.contains("\"tools\""));
    assert!(events.iter().any(|event| {
        event
            .kind()
            .output()
            .is_some_and(|(name, _)| name == "tool_calls")
    }));
    assert_eq!(
        events
            .last()
            .and_then(|event| event.kind().terminal())
            .map(|terminal| terminal.status()),
        Some(TerminalStatus::Success)
    );
    assert_eq!(
        data.published.lock().map_err(|_| "published lock")?.len(),
        4
    );
    Ok(())
}
