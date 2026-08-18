//! Compatibility and invariant tests for provider-neutral contracts.

use std::collections::{BTreeMap, BTreeSet};

use milkdrift_capability::{
    AdmissionConstraints, BoundedJson, CancellationBehavior, CapabilityCategory,
    CapabilityDescriptor, CapabilityDescriptorDocument, CapabilityId, CapabilityObservation,
    CapabilityRequirement, ContractError, DescriptorBuilder, ExtensionKey, FeatureContract,
    FeatureId, IdempotencyBehavior, InputReference, InvocationId, InvocationRequest,
    InvocationRequestDocument, InvocationValueReference, Locality, OperationContract, OperationId,
    ProviderProfileRef, SchemaContract, SchemaId, SideEffectClass, StreamingMode, TrustZone,
    canonical_json_bytes,
};
use proptest::prelude::*;
use serde_json::json;

fn schema(name: &str) -> Result<SchemaContract, ContractError> {
    SchemaContract::new(
        SchemaId::new(name)?,
        1,
        BoundedJson::new(json!({"additionalProperties": false, "type": "object"}))?,
    )
}

fn descriptor() -> Result<CapabilityDescriptor, ContractError> {
    let feature_id = FeatureId::new("model.tools")?;
    let feature = FeatureContract::new(feature_id.clone(), None);
    let operation = OperationContract::new(
        schema("milkdrift.prompt")?,
        schema("milkdrift.response")?,
        BTreeSet::from([StreamingMode::None, StreamingMode::OutputFragments]),
        CancellationBehavior::BestEffort,
        IdempotencyBehavior::ProviderProfileScoped,
        SideEffectClass::None,
        BTreeMap::from([(feature_id, feature)]),
    )?;
    DescriptorBuilder::new(
        CapabilityId::new("anthropic-primary")?,
        3,
        CapabilityCategory::Model,
        AdmissionConstraints::new(4, 32)?,
        Locality::Remote,
    )
    .provider_profile(Some(ProviderProfileRef::new("anthropic-prod")?))
    .operations(BTreeMap::from([(
        OperationId::new("model.generate")?,
        operation,
    )]))
    .trust_zones(BTreeSet::from([TrustZone::new("external-approved")?]))
    .labels(BTreeSet::from(["hosted".to_owned()]))
    .extensions(BTreeMap::from([(
        ExtensionKey::new("org.milkdrift/example")?,
        BoundedJson::new(json!({"region": "eu"}))?,
    )]))
    .build()
}

#[test]
fn descriptor_round_trip_and_golden_encoding() -> Result<(), Box<dyn std::error::Error>> {
    let document = CapabilityDescriptorDocument::new(descriptor()?);
    let bytes = document.to_canonical_json()?;
    let fixture = include_bytes!("fixtures/descriptor-v1.json").trim_ascii_end();
    if fixture.is_empty() {
        eprintln!("{}", String::from_utf8(bytes.clone())?);
    }
    assert_eq!(bytes, fixture);
    let decoded = CapabilityDescriptorDocument::from_json(fixture)?;
    assert_eq!(decoded, document);
    assert_eq!(decoded.to_canonical_json()?, fixture);
    Ok(())
}

#[test]
fn requirement_matches_only_truthfully_advertised_features()
-> Result<(), Box<dyn std::error::Error>> {
    let descriptor = descriptor()?;
    let supported = CapabilityRequirement::new(OperationId::new("model.generate")?)
        .feature(FeatureId::new("model.tools")?)
        .streaming(StreamingMode::OutputFragments)
        .cancellation(true)
        .maximum_side_effect(SideEffectClass::None)
        .trust_zone(TrustZone::new("external-approved")?);
    assert!(descriptor.matches(&supported).is_match());

    let fabricated = CapabilityRequirement::new(OperationId::new("model.generate")?)
        .feature(FeatureId::new("model.exact_token_count")?);
    let mismatch = descriptor.matches(&fabricated);
    assert!(!mismatch.is_match());
    assert!(
        mismatch
            .mismatch_reasons()
            .iter()
            .any(|reason| reason == "features")
    );
    Ok(())
}

#[test]
fn immutable_description_and_mutable_observation_are_distinct()
-> Result<(), Box<dyn std::error::Error>> {
    let descriptor_json = canonical_json_bytes(&descriptor()?)?;
    let observation = CapabilityObservation::new(
        CapabilityId::new("anthropic-primary")?,
        1_776_000_000_000,
        true,
        2,
        "healthy",
    )?;
    let observation_json = canonical_json_bytes(&observation)?;
    let descriptor_text = String::from_utf8(descriptor_json)?;
    let observation_text = String::from_utf8(observation_json)?;
    assert!(!descriptor_text.contains("current_load"));
    assert!(observation_text.contains("current_load"));
    assert!(observation_text.contains("available"));
    Ok(())
}

#[test]
fn invocation_inputs_are_references_or_bounded_values() -> Result<(), Box<dyn std::error::Error>> {
    let input = InputReference::new(
        "prompt",
        InvocationValueReference::Inline {
            value: BoundedJson::new(json!({"text": "hello"}))?,
        },
    )?;
    let request = InvocationRequest::new(
        InvocationId::new("inv-001")?,
        CapabilityId::new("anthropic-primary")?,
        OperationId::new("model.generate")?,
        Some(ProviderProfileRef::new("anthropic-prod")?),
        None,
        vec![input],
        BTreeMap::new(),
    )?;
    let document = InvocationRequestDocument::new(request);
    let bytes = document.to_canonical_json()?;
    assert_eq!(InvocationRequestDocument::from_json(&bytes)?, document);
    Ok(())
}

#[test]
fn unsupported_version_and_hostile_bounds_fail_clearly() -> Result<(), Box<dyn std::error::Error>> {
    let future = br#"{"schema_version":2,"descriptor":{}}"#;
    assert!(matches!(
        CapabilityDescriptorDocument::from_json(future),
        Err(ContractError::UnsupportedVersion { found: 2, .. })
    ));
    let oversized = format!(
        "{{\"schema_version\":1,\"descriptor\":\"{}\"}}",
        "x".repeat(1_048_577)
    );
    assert!(matches!(
        CapabilityDescriptorDocument::from_json(oversized.as_bytes()),
        Err(ContractError::Bounds { .. })
    ));
    assert!(ExtensionKey::new("not-namespaced").is_err());
    assert!(CapabilityId::new("contains a space").is_err());
    Ok(())
}

proptest! {
    #[test]
    fn arbitrary_identity_text_never_bypasses_constructor(text in ".{0,300}") {
        if let Ok(identity) = CapabilityId::new(text) {
            prop_assert!(!identity.as_str().is_empty());
            prop_assert!(identity.as_str().len() <= 128);
            prop_assert!(identity.as_str().is_ascii());
        }
    }
}
