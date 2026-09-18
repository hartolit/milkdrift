use super::*;
use milkdrift_capability_host::DirectInputSelection;

#[test]
fn fresh_direct_model_uses_explicit_selection_without_workflow_manifest() -> TestResult {
    let response = json!({
        "id": "direct-response", "model": "mock-model",
        "choices": [{"message": {"content": "direct result"}, "finish_reason": "stop"}],
        "usage": {"prompt_tokens": 1, "completion_tokens": 2}
    })
    .to_string();
    let (address, server) = serve(response, "application/json")?;
    let profile = profile(
        &address,
        "direct-model",
        ProviderProtocol::OpenAiCompatible {
            path: "v1/chat/completions".to_owned(),
        },
        AuthMode::NoAuth,
        BTreeSet::from([ModelFeature::SystemRole]),
    )?;
    let capability = CapabilityId::new("model-direct")?;
    let descriptor = descriptor_for_profile(capability.clone(), &profile)?;
    let snapshot = ResolvedCapabilitySnapshot::from_descriptor(
        &descriptor,
        &OperationId::new("model.generate")?,
    )?;
    let task = ModelTaskRequest::new(
        vec![Message::new(
            MessageRole::User,
            vec![ContentPart::Text {
                text: "explicit direct prompt".to_owned(),
            }],
            None,
        )?],
        Vec::new(),
        None,
        SessionSelection::Fresh,
        None,
        8,
        false,
        BTreeMap::new(),
    )?;
    let input = InputReference::new(
        MODEL_TASK_INPUT_NAME,
        InvocationValueReference::Inline {
            value: BoundedJson::new(serde_json::to_value(ModelTaskRequestDocument::new(task))?)?,
        },
    )?;
    let request = InvocationRequest::new(
        InvocationId::new("direct-invocation")?,
        capability.clone(),
        OperationId::new("model.generate")?,
        Some(profile.identity().clone()),
        None,
        vec![input],
        BTreeMap::new(),
    )?;
    let context = AdapterExecutionContext::direct(DirectInputSelection::new(&request, 16_384)?);
    let data = Arc::new(MockData::default());
    let adapter = ModelEndpointAdapter::new(
        capability,
        profile,
        Arc::new(InMemorySecretResolver::default()),
        data.clone(),
    )?;
    adapter.start()?;
    let reporter = Reporter::default();
    let invocation = AdapterInvocation::with_context(&snapshot, &request, &context);
    let _envelope = adapter.admission_envelope(&invocation)?;
    assert!(
        data.artifacts
            .lock()
            .map_err(|_| "artifact lock")?
            .is_empty()
    );
    adapter.execute(&invocation, &reporter)?;
    let wire = server.join().map_err(|_| "server panic")??;
    let body: Value = serde_json::from_str(
        wire.split("\r\n\r\n")
            .nth(1)
            .ok_or("request body missing")?,
    )?;
    let selection = body["messages"][0]["content"][0]["text"]
        .as_str()
        .ok_or("selection header missing")?;
    assert!(selection.contains("explicit_inputs_only"));
    assert!(selection.contains("\"origin\":\"direct\""));
    assert!(!selection.contains("\"run\""));
    assert_eq!(body["messages"][1]["content"], "explicit direct prompt");
    assert!(
        data.published
            .lock()
            .map_err(|_| "publication lock")?
            .iter()
            .any(|(_, _, bytes)| String::from_utf8_lossy(bytes).contains("direct result"))
    );
    assert!(!reporter.events()?.is_empty());
    Ok(())
}
