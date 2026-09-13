use super::*;
use serde_json::json;

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

fn profile(billing: Value) -> TestResult<EndpointProfile> {
    let mut value: Value = serde_json::from_str(include_str!(
        "../../../../examples/local-model/openai-compatible-loopback.example.json"
    ))?;
    value["billing"] = billing;
    value["token_limits"] = json!({"type":"byte_bpe", "template_tokens_per_message":32,
        "template_tokens_per_request":64, "maximum_input_tokens":10000, "maximum_output_tokens":128,
        "output_control":"max_tokens", "source":"controlled byte-BPE/template fixture v1; all generation capped"});
    Ok(serde_json::from_value(value)?)
}

fn wire() -> Value {
    json!({"model":"fixture", "messages":[{"role":"system","content":"context"},{"role":"user","content":"é🦀"}],
        "stream":false, "max_tokens":100})
}

fn tariff() -> Value {
    json!({"type":"text_tariff","currency":"EUR","input_micros_per_million":1,
        "cached_input_micros_per_million":2,"output_micros_per_million":3,"source":"fixture tariff v1; text tokens only"})
}

#[test]
fn whole_request_bound_includes_utf8_context_and_template_and_rejects_undermining_options()
-> TestResult {
    let profile = profile(json!({"type":"unbilled","source":"operator fixture declaration v1"}))?;
    let wire = wire();
    let bytes = serde_json::to_vec(&wire)?;
    let envelope = profile.prepared_envelope(&wire, &bytes)?;
    assert_eq!(
        envelope.input_units(),
        &AdmissionBound::Bounded(bytes.len() as u64 + 128)
    );
    assert_eq!(envelope.output_units(), &AdmissionBound::Bounded(100));
    assert_eq!(envelope.monetary_cost(), &AdmissionBound::NotApplicable);
    assert_eq!(profile.authority_cost_minor()?, None);
    for (key, value) in [
        ("n", json!(2)),
        ("max_completion_tokens", json!(1000)),
        ("tools", json!([])),
        ("reasoning_effort", json!("high")),
        ("max_tokens", json!(129)),
        ("response_format", json!({})),
    ] {
        let mut hostile = wire.clone();
        hostile[key] = value;
        assert!(
            profile
                .prepared_envelope(&hostile, &serde_json::to_vec(&hostile)?)
                .is_err(),
            "{key}"
        );
    }
    let mut expanded = wire.clone();
    expanded["messages"][0]["content"] = json!("evidence".repeat(2000));
    assert!(
        profile
            .prepared_envelope(&expanded, &serde_json::to_vec(&expanded)?)
            .is_err()
    );
    Ok(())
}

#[test]
fn tariff_uses_exact_currency_conservative_rounding_and_checked_arithmetic() -> TestResult {
    let profile = profile(tariff())?;
    let wire = wire();
    let envelope = profile.prepared_envelope(&wire, &serde_json::to_vec(&wire)?)?;
    let bound = envelope.monetary_cost().bounded().ok_or("cost missing")?;
    assert_eq!(bound.currency(), "EUR");
    assert_eq!(bound.maximum_micros(), 1);
    assert_eq!(profile.authority_cost_minor()?, Some(1));
    assert_eq!(charge(&[(1, 1), (1, 1)])?, 1);
    assert_eq!(charge(&[(1_000_000, 1)])?, 1);
    assert_eq!(charge(&[(1_000_001, 1)])?, 2);
    assert!(charge(&[(u64::MAX, u64::MAX)]).is_err());
    assert!(charge(&[(u64::MAX, u64::MAX), (u64::MAX, u64::MAX)]).is_err());
    Ok(())
}

#[test]
fn unbilled_and_calculated_usage_keep_provider_evidence_distinct_and_conflicts_unsettled()
-> TestResult {
    let local = profile(json!({"type":"unbilled","source":"operator fixture v1"}))?;
    let billed = profile(tariff())?;
    let response = |usage| {
        ModelResponse::new(
            "done".to_owned(),
            None,
            vec![],
            milkdrift_model::FinishReason::Stop,
            usage,
            std::collections::BTreeMap::new(),
        )
    };
    let usage = Usage {
        input_units: Some(100),
        output_units: Some(10),
        cached_input_units: Some(20),
        cost_micros: None,
        currency: None,
    };
    assert_eq!(local.accounted_usage(&response(usage.clone())?)?, usage);
    let observed = response(usage.clone())?;
    let accounted = billed.accounted_usage(&observed)?;
    assert_eq!(accounted.cost_micros, Some(1));
    assert_eq!(accounted.currency.as_deref(), Some("EUR"));
    assert_eq!(observed.usage().cost_micros, None);
    for (cost, currency) in [(Some(1), Some("USD")), (Some(2), Some("EUR"))] {
        let mut conflict = usage.clone();
        conflict.cost_micros = cost;
        conflict.currency = currency.map(str::to_owned);
        assert!(
            billed
                .accounted_usage(&response(conflict.clone())?)
                .is_err()
        );
        assert!(local.accounted_usage(&response(conflict)?).is_err());
    }
    let mut missing = usage;
    missing.cached_input_units = None;
    assert!(billed.accounted_usage(&response(missing)?).is_err());
    Ok(())
}

#[test]
fn old_missing_and_invalid_contracts_never_become_unbilled() -> TestResult {
    let original = serde_json::to_value(profile(json!({"type":"unknown"}))?)?;
    for (key, value) in [
        ("schema_version", json!(1)),
        ("billing", json!(null)),
        ("billing", json!({"type":"unbilled","source":""})),
        ("billing", json!({"type":"text_tariff","currency":"USD"})),
    ] {
        let mut invalid = original.clone();
        invalid[key] = value;
        assert!(EndpointProfile::from_json(&serde_json::to_vec(&invalid)?).is_err());
    }
    let unknown: EndpointProfile = serde_json::from_value(original)?;
    assert_eq!(unknown.authority_cost_minor()?, None);
    let wire = wire();
    assert!(
        unknown
            .prepared_envelope(&wire, &serde_json::to_vec(&wire)?)?
            .monetary_cost()
            .is_unknown()
    );
    Ok(())
}
