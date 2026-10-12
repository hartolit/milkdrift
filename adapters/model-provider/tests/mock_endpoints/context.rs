use super::*;

#[test]
fn injected_manifest_system_role_is_negotiated_before_network_entry() -> TestResult {
    let profile = profile(
        "127.0.0.1:9",
        "missing-system-role",
        ProviderProtocol::OpenAiCompatible {
            path: "v1/chat/completions".to_owned(),
        },
        AuthMode::NoAuth,
        BTreeSet::new(),
    )?;
    let task = ModelTaskRequest::new(
        vec![Message::new(
            MessageRole::User,
            vec![ContentPart::Text {
                text: "must not reach the network".to_owned(),
            }],
            None,
        )?],
        Vec::new(),
        None,
        SessionSelection::Fresh,
        None,
        32,
        false,
        BTreeMap::new(),
    )?;
    let error = match execute(
        profile,
        task,
        Arc::new(MockData::default()),
        Arc::new(InMemorySecretResolver::new()),
    ) {
        Err(error) => error,
        Ok(_) => return Err("adapter-injected system context was not negotiated".into()),
    };
    assert!(error.to_string().contains("system role"));
    Ok(())
}

#[test]
fn explicit_artifact_bytes_are_sent_with_identity_and_provider_result_is_published() -> TestResult {
    let response = json!({"choices":[{"message":{"content":"grounded report"},"finish_reason":"stop"}],"usage":{"prompt_tokens":4,"completion_tokens":2}}).to_string();
    let (address, server) = serve(response, "application/json")?;
    let profile = profile(
        &address,
        "explicit-evidence",
        ProviderProtocol::OpenAiCompatible {
            path: "v1/chat/completions".into(),
        },
        AuthMode::NoAuth,
        BTreeSet::from([ModelFeature::SystemRole]),
    )?;
    let data = Arc::new(MockData::default());
    let evidence = data.install(
        "immutable-evidence",
        "text/plain",
        b"package = meaningful-project; dependency = serde".to_vec(),
    )?;
    let events = execute_bound(
        profile,
        super::uncertainty::ordinary_task(false)?,
        data.clone(),
        Arc::new(InMemorySecretResolver::new()),
        vec![InputReference::new(
            "project",
            InvocationValueReference::Artifact {
                reference: evidence.clone(),
            },
        )?],
    )?;
    let wire = server.join().map_err(|_| "server panicked")??;
    assert!(wire.contains("meaningful-project"));
    assert!(wire.contains(evidence.digest()));
    assert_eq!(
        events
            .last()
            .and_then(|event| event.kind().terminal())
            .map(|terminal| terminal.status()),
        Some(TerminalStatus::Success)
    );
    assert!(
        data.published
            .lock()
            .map_err(|_| "publication lock")?
            .iter()
            .any(|(_, _, bytes)| String::from_utf8_lossy(bytes).contains("grounded report"))
    );
    Ok(())
}
