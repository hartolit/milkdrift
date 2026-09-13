use super::*;

fn bounded_profile(address: &str, billing: Value) -> TestResult<EndpointProfile> {
    let mut value = serde_json::to_value(profile(
        address,
        "accounted",
        ProviderProtocol::OpenAiCompatible {
            path: "v1/chat/completions".to_owned(),
        },
        AuthMode::NoAuth,
        BTreeSet::from([ModelFeature::SystemRole, ModelFeature::Streaming]),
    )?)?;
    value["billing"] = billing;
    value["token_limits"] = json!({"type":"byte_bpe","template_tokens_per_message":64,
        "template_tokens_per_request":64,"maximum_input_tokens":32768,"maximum_output_tokens":64,
        "output_control":"max_completion_tokens","source":"bounded mock endpoint contract v1"});
    Ok(serde_json::from_value(value)?)
}

#[test]
fn prepared_mapping_and_reported_usage_settle_the_supported_tariff_without_relabelling_provider_cost()
-> TestResult {
    use milkdrift_capability::{
        AdmissionMonetaryBound, CapabilityCategory, InvocationAdmissionEnvelope,
    };
    use milkdrift_persistence::{
        AttemptUsage, ControllerAccountDeclaration, ControllerAccountState,
        ControllerAdmissionOutcome, ControllerReservationId, ControllerResourceBudget,
        CurrencyCode, MonetaryUsage,
    };
    let (address,server)=serve(json!({"choices":[{"message":{"content":"complete"},"finish_reason":"stop"}],
        "usage":{"prompt_tokens":3,"completion_tokens":2,"total_tokens":5,"prompt_tokens_details":{"cached_tokens":1}}}).to_string(),"application/json")?;
    let profile = bounded_profile(
        &address,
        json!({"type":"text_tariff","currency":"EUR",
        "input_micros_per_million":1,"cached_input_micros_per_million":1,"output_micros_per_million":2,"source":"fixture tariff v1"}),
    )?;
    let data = Arc::new(MockData::default());
    let events = execute(
        profile,
        super::uncertainty::ordinary_task(false)?,
        data.clone(),
        Arc::new(InMemorySecretResolver::new()),
    )?;
    let request = server.join().map_err(|_| "fixture panicked")??;
    let wire: Value = serde_json::from_str(request.split_once("\r\n\r\n").ok_or("body absent")?.1)?;
    assert_eq!(wire["max_completion_tokens"], 64);
    assert!(wire.get("max_tokens").is_none());
    let terminal = events
        .last()
        .and_then(|event| event.kind().terminal())
        .ok_or("terminal absent")?;
    assert_eq!(terminal.status(), TerminalStatus::Success);
    let usage = terminal.usage().ok_or("usage absent")?;
    assert_eq!(usage.cost_micros(), Some(1));
    assert_eq!(usage.currency(), Some("EUR"));
    let published = data.published.lock().map_err(|_| "outputs lock")?;
    let response: Value = serde_json::from_slice(
        &published
            .iter()
            .find(|v| v.0 == "model_response")
            .ok_or("response absent")?
            .2,
    )?;
    assert!(response["response"]["usage"]["cost_micros"].is_null());
    let facts = &response["response"]["provider_metadata"]["org.milkdrift/model-accounting"];
    assert_eq!(facts["cost_basis"], "tariff_calculated");
    let envelope: InvocationAdmissionEnvelope =
        serde_json::from_value(facts["reservation_envelope"].clone())?;
    let mut account = ControllerAccountState::establish(ControllerAccountDeclaration::new(
        RunId::new("run-billed")?,
        NodeExecutionId::new("execution-billed")?,
        "policy:billed",
        ControllerResourceBudget::new(
            2,
            Some(CurrencyCode::new("EUR")?),
            65536,
            128,
            8_388_608,
            1,
            2,
        )?,
    )?)?;
    let attempt = AttemptId::new("attempt-billed")?;
    let reservation =
        ControllerReservationId::for_attempt(account.declaration().account(), &attempt)?;
    assert!(matches!(
        account.admit(
            reservation.clone(),
            attempt,
            CapabilityCategory::Model,
            &envelope
        )?,
        ControllerAdmissionOutcome::Reserved { .. }
    ));
    account.settle_terminal(
        &reservation,
        Some(&AttemptUsage {
            input_units: usage.input_units(),
            output_units: usage.output_units(),
            duration_ms: usage.duration_ms(),
            cost: Some(MonetaryUsage {
                micros: 1,
                currency: CurrencyCode::new("EUR")?,
            }),
        }),
    )?;
    assert_eq!(account.settled().cost_micros(), 1);
    assert!(account.blocked().is_none());
    assert!(account.reservations().is_empty());
    assert_eq!(
        envelope.monetary_cost(),
        &AdmissionBound::Bounded(AdmissionMonetaryBound::new(1, "EUR")?)
    );
    Ok(())
}

#[test]
fn complete_but_missing_or_conflicting_model_usage_stays_visible_and_conservative() -> TestResult {
    for (usage, status) in [
        (json!({"prompt_tokens":2}), TerminalStatus::Success),
        (
            json!({"prompt_tokens":2,"completion_tokens":65}),
            TerminalStatus::Uncertain,
        ),
        (
            json!({"prompt_tokens":2,"completion_tokens":1,"cost_micros":1,"currency":"USD"}),
            TerminalStatus::Uncertain,
        ),
        (
            json!({"prompt_tokens":2,"completion_tokens":1,"total_tokens":8}),
            TerminalStatus::Uncertain,
        ),
        (json!(7), TerminalStatus::Uncertain),
        (
            json!({"prompt_tokens":2,"completion_tokens":1,"prompt_tokens_details":7}),
            TerminalStatus::Uncertain,
        ),
        (
            json!({"prompt_tokens":2,"completion_tokens":1,"completion_tokens_details":{"reasoning_tokens":"1"}}),
            TerminalStatus::Uncertain,
        ),
    ] {
        let (address,server)=serve(json!({"choices":[{"message":{"content":"complete"},"finish_reason":"stop"}],"usage":usage}).to_string(),"application/json")?;
        let profile = bounded_profile(
            &address,
            json!({"type":"unbilled","source":"operator fixture v1"}),
        )?;
        let data = Arc::new(MockData::default());
        let events = execute(
            profile,
            super::uncertainty::ordinary_task(false)?,
            data.clone(),
            Arc::new(InMemorySecretResolver::new()),
        )?;
        server.join().map_err(|_| "fixture panicked")??;
        let terminal = events
            .last()
            .and_then(|event| event.kind().terminal())
            .ok_or("terminal absent")?;
        assert_eq!(terminal.status(), status);
        if status == TerminalStatus::Success {
            assert!(
                terminal
                    .usage()
                    .ok_or("usage absent")?
                    .output_units()
                    .is_none()
            );
        } else if let Some(response) = data
            .published
            .lock()
            .map_err(|_| "outputs lock")?
            .iter()
            .find(|v| v.0 == "model_response")
        {
            let response: Value = serde_json::from_slice(&response.2)?;
            let facts =
                &response["response"]["provider_metadata"]["org.milkdrift/model-accounting"];
            assert_eq!(facts["cost_basis"], "unresolved");
            assert!(facts["accounted_cost_micros"].is_null());
            if response["response"]["usage"]["cost_micros"].is_u64() {
                assert_eq!(response["response"]["usage"]["cost_micros"], 1);
                assert!(
                    terminal
                        .usage()
                        .ok_or("usage absent")?
                        .cost_micros()
                        .is_none()
                );
            }
        }
    }
    Ok(())
}

#[test]
fn nullable_usage_details_preserve_raw_evidence_and_settle_in_both_response_modes() -> TestResult {
    let raw_usage = json!({"prompt_tokens":3,"completion_tokens":2,"total_tokens":5,
        "prompt_tokens_details":{"cached_tokens":null,"audio_tokens":null},
        "completion_tokens_details":{"reasoning_tokens":1,"audio_tokens":null,
            "accepted_prediction_tokens":null,"rejected_prediction_tokens":null}});
    for streaming in [false, true] {
        let body = if streaming {
            format!(
                "data: {}\n\ndata: {}\n\ndata: [DONE]\n\n",
                json!({"choices":[{"index":0,"delta":{"content":"complete"},"finish_reason":"stop"}],"usage":null}),
                json!({"choices":[],"usage":raw_usage})
            )
        } else {
            json!({"choices":[{"message":{"content":"complete"},"finish_reason":"stop"}],
                "usage":raw_usage})
            .to_string()
        };
        let (address, server) = serve(
            body,
            if streaming {
                "text/event-stream"
            } else {
                "application/json"
            },
        )?;
        let profile = bounded_profile(
            &address,
            json!({"type":"text_tariff","currency":"EUR",
            "input_micros_per_million":1,"cached_input_micros_per_million":1,
            "output_micros_per_million":2,"source":"fixture tariff v1"}),
        )?;
        let data = Arc::new(MockData::default());
        let events = execute(
            profile,
            super::uncertainty::ordinary_task(streaming)?,
            data.clone(),
            Arc::new(InMemorySecretResolver::new()),
        )?;
        server.join().map_err(|_| "fixture panicked")??;
        let terminal = events
            .last()
            .and_then(|event| event.kind().terminal())
            .ok_or("terminal absent")?;
        assert_eq!(
            terminal.status(),
            TerminalStatus::Success,
            "streaming={streaming}"
        );
        assert_eq!(
            terminal.usage().ok_or("usage absent")?.cost_micros(),
            Some(1)
        );
        let published = data.published.lock().map_err(|_| "outputs lock")?;
        let response: Value = serde_json::from_slice(
            &published
                .iter()
                .find(|v| v.0 == "model_response")
                .ok_or("response absent")?
                .2,
        )?;
        assert_eq!(
            response["response"]["provider_metadata"]["org.milkdrift.openai/response"]["usage"],
            raw_usage
        );
        assert!(response["response"]["usage"]["cached_input_units"].is_null());
        assert!(response["response"]["usage"]["cost_micros"].is_null());
    }
    Ok(())
}

#[test]
fn streamed_partial_usage_cannot_erase_a_prior_charge_or_output_count() -> TestResult {
    for earlier in [
        json!({"cost_micros":1,"currency":"USD"}),
        json!({"completion_tokens":65}),
    ] {
        let body = format!(
            "data: {}\n\ndata: {}\n\ndata: [DONE]\n\n",
            json!({"choices":[{"delta":{"content":"complete"},"finish_reason":"stop"}],"usage":earlier}),
            json!({"choices":[],"usage":{"prompt_tokens":2,"completion_tokens":1}})
        );
        let (address, server) = serve(body, "text/event-stream")?;
        let profile = bounded_profile(
            &address,
            json!({"type":"unbilled","source":"operator fixture v1"}),
        )?;
        let data = Arc::new(MockData::default());
        let events = execute(
            profile,
            super::uncertainty::ordinary_task(true)?,
            data.clone(),
            Arc::new(InMemorySecretResolver::new()),
        )?;
        server.join().map_err(|_| "fixture panicked")??;
        let terminal = events
            .last()
            .and_then(|event| event.kind().terminal())
            .ok_or("terminal absent")?;
        assert_eq!(
            terminal.status(),
            TerminalStatus::Uncertain,
            "earlier={earlier}"
        );
        assert!(
            terminal
                .usage()
                .ok_or("usage absent")?
                .input_units()
                .is_none()
        );
        assert!(
            data.published
                .lock()
                .map_err(|_| "outputs lock")?
                .is_empty()
        );
    }
    Ok(())
}
