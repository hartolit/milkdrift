//! Durable executor-facing capability contract compatibility and invariant tests.

use std::collections::{BTreeMap, BTreeSet};

use milkdrift_capability::{
    AdmissionConstraints, ArtifactReference, BoundedJson, CancellationAcknowledgement,
    CancellationAcknowledgementDocument, CancellationBehavior, CancellationRequest,
    CancellationRequestDocument, CapabilityCategory, CapabilityDescriptor, CapabilityId,
    CapabilityRequirement, ContractError, DescriptorBuilder, ErrorClass, ExtensionKey,
    FeatureContract, FeatureId, IdempotencyBehavior, IdempotencyKey, InputReference,
    InvocationEvent, InvocationEventDocument, InvocationEventKind, InvocationFailure, InvocationId,
    InvocationRequest, InvocationRequestDocument, InvocationTerminal, InvocationValueReference,
    Locality, OperationContract, OperationId, ProviderProfileRef, ResolvedCapabilitySnapshot,
    ResolvedCapabilitySnapshotDocument, ResourceObservations, SchemaContract, SchemaId,
    SideEffectClass, StreamingMode, TerminalStatus, TrustZone, UsageObservation,
};
use serde::Serialize;
use serde_json::{Value, json};

const ARTIFACT_DIGEST: &str = "abababababababababababababababababababababababababababababababab";

fn schema(identity: &str) -> Result<SchemaContract, ContractError> {
    SchemaContract::new(
        SchemaId::new(identity)?,
        1,
        BoundedJson::new(json!({"additionalProperties": false, "type": "object"}))?,
    )
}

fn operation() -> Result<OperationContract, ContractError> {
    let feature_id = FeatureId::new("tool.batch")?;
    let feature = FeatureContract::new(feature_id.clone(), Some(schema("tool.batch.settings")?));
    OperationContract::new(
        schema("tool.publish.input")?,
        schema("tool.publish.output")?,
        BTreeSet::from([StreamingMode::Progress]),
        CancellationBehavior::Acknowledged,
        IdempotencyBehavior::ProviderProfileScoped,
        SideEffectClass::IdempotentWrite,
        BTreeMap::from([(feature_id, feature)]),
    )
}

fn descriptor_at(revision: u64) -> Result<CapabilityDescriptor, ContractError> {
    DescriptorBuilder::new(
        CapabilityId::new("publisher-primary")?,
        revision,
        CapabilityCategory::Tool,
        AdmissionConstraints::new(3, 9)?,
        Locality::Remote,
    )
    .provider_profile(Some(ProviderProfileRef::new("publisher-prod")?))
    .operations(BTreeMap::from([(
        OperationId::new("tool.publish")?,
        operation()?,
    )]))
    .trust_zones(BTreeSet::from([TrustZone::new("external-write")?]))
    .resource_observations(Some(ResourceObservations::new(
        Some(125),
        Some(800),
        Some("DKK".to_owned()),
    )?))
    .labels(BTreeSet::from(["durable".to_owned()]))
    .extensions(BTreeMap::from([(
        ExtensionKey::new("org.milkdrift/region")?,
        BoundedJson::new(json!("eu"))?,
    )]))
    .build()
}

fn descriptor() -> Result<CapabilityDescriptor, ContractError> {
    descriptor_at(7)
}

fn artifact() -> Result<ArtifactReference, ContractError> {
    ArtifactReference::new(
        "artifact-output",
        ARTIFACT_DIGEST,
        Some("application/json".to_owned()),
        Some(17),
    )
}

fn invocation_request() -> Result<InvocationRequest, ContractError> {
    InvocationRequest::new(
        InvocationId::new("invocation-007")?,
        CapabilityId::new("publisher-primary")?,
        OperationId::new("tool.publish")?,
        Some(ProviderProfileRef::new("publisher-prod")?),
        Some(IdempotencyKey::new("run-1-node-2-attempt-1")?),
        vec![InputReference::new(
            "payload",
            InvocationValueReference::Inline {
                value: BoundedJson::new(json!({"message": "hello"}))?,
            },
        )?],
        BTreeMap::from([(
            ExtensionKey::new("org.milkdrift/trace")?,
            BoundedJson::new(json!("trace-7"))?,
        )]),
    )
}

fn terminal() -> Result<InvocationTerminal, ContractError> {
    InvocationTerminal::new(
        TerminalStatus::Uncertain,
        vec![artifact()?],
        Some(InvocationFailure::new(
            ErrorClass::Transport,
            true,
            "remote_timeout",
            "provider connection closed after dispatch",
            Some(250),
        )?),
        Some(UsageObservation::new(
            Some(10),
            Some(20),
            Some(75),
            Some(42),
            Some("USD".to_owned()),
            BTreeMap::new(),
        )?),
        SideEffectClass::IdempotentWrite,
    )
}

fn invocation_event() -> Result<InvocationEvent, ContractError> {
    InvocationEvent::new(
        InvocationId::new("invocation-007")?,
        3,
        InvocationEventKind::Terminal {
            terminal: terminal()?,
        },
    )
}

fn assert_golden<T: Serialize>(
    value: &T,
    fixture: &[u8],
    label: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let actual = milkdrift_capability::canonical_json_bytes(value)?;
    let expected = fixture.trim_ascii_end();
    assert_eq!(actual, expected, "{label} v1 wire format changed");
    Ok(())
}

#[test]
fn executor_documents_round_trip_with_golden_v1_bytes() -> Result<(), Box<dyn std::error::Error>> {
    let request = InvocationRequestDocument::new(invocation_request()?);
    assert_golden(
        &request,
        include_bytes!("fixtures/invocation-request-v1.json"),
        "invocation request",
    )?;
    assert_eq!(
        InvocationRequestDocument::from_json(&request.to_canonical_json()?)?,
        request
    );

    let event = InvocationEventDocument::new(invocation_event()?);
    assert_golden(
        &event,
        include_bytes!("fixtures/invocation-event-v1.json"),
        "invocation event",
    )?;
    assert_eq!(
        InvocationEventDocument::from_json(&event.to_canonical_json()?)?,
        event
    );

    let cancellation = CancellationRequestDocument::new(CancellationRequest::new(
        InvocationId::new("invocation-007")?,
        2,
        "run cancellation requested",
    )?);
    assert_golden(
        &cancellation,
        include_bytes!("fixtures/cancellation-request-v1.json"),
        "cancellation request",
    )?;
    assert_eq!(
        CancellationRequestDocument::from_json(&cancellation.to_canonical_json()?)?,
        cancellation
    );

    let acknowledgement =
        CancellationAcknowledgementDocument::new(CancellationAcknowledgement::new(
            InvocationId::new("invocation-007")?,
            2,
            true,
            true,
            Some("executor reached its cancellation boundary".to_owned()),
        )?);
    assert_golden(
        &acknowledgement,
        include_bytes!("fixtures/cancellation-acknowledgement-v1.json"),
        "cancellation acknowledgement",
    )?;
    assert_eq!(
        CancellationAcknowledgementDocument::from_json(&acknowledgement.to_canonical_json()?)?,
        acknowledgement
    );
    Ok(())
}

#[test]
fn resolved_snapshot_is_exact_digest_bound_and_golden() -> Result<(), Box<dyn std::error::Error>> {
    let descriptor = descriptor()?;
    let operation_id = OperationId::new("tool.publish")?;
    let snapshot = ResolvedCapabilitySnapshot::from_descriptor(&descriptor, &operation_id)?;
    snapshot.validate_against(&descriptor)?;
    assert_eq!(snapshot.capability(), descriptor.identity());
    assert_eq!(snapshot.descriptor_revision(), 7);
    assert_eq!(
        snapshot.provider_profile().map(ProviderProfileRef::as_str),
        Some("publisher-prod")
    );
    assert_eq!(snapshot.operation(), &operation_id);
    assert_eq!(
        snapshot.operation_contract().idempotency(),
        IdempotencyBehavior::ProviderProfileScoped
    );
    assert_eq!(
        snapshot.operation_contract().side_effect(),
        SideEffectClass::IdempotentWrite
    );
    assert_eq!(snapshot.digest().len(), 64);

    let document = ResolvedCapabilitySnapshotDocument::new(snapshot.clone());
    assert_golden(
        &document,
        include_bytes!("fixtures/resolved-capability-snapshot-v1.json"),
        "resolved capability snapshot",
    )?;
    let encoded = document.to_canonical_json()?;
    assert_eq!(
        ResolvedCapabilitySnapshotDocument::from_json(&encoded)?,
        document
    );

    assert!(snapshot.validate_against(&descriptor_at(8)?).is_err());
    let mut tampered = serde_json::to_value(&snapshot)?;
    tampered["descriptor_revision"] = json!(8);
    assert!(serde_json::from_value::<ResolvedCapabilitySnapshot>(tampered).is_err());
    Ok(())
}

#[test]
fn immutable_accessors_expose_every_executor_fact() -> Result<(), Box<dyn std::error::Error>> {
    let descriptor = descriptor()?;
    assert_eq!(descriptor.identity().as_str(), "publisher-primary");
    assert_eq!(descriptor.descriptor_revision(), 7);
    assert_eq!(
        descriptor
            .provider_profile()
            .map(ProviderProfileRef::as_str),
        Some("publisher-prod")
    );
    assert_eq!(descriptor.category(), &CapabilityCategory::Tool);
    assert_eq!(descriptor.admission().max_concurrent(), 3);
    assert_eq!(descriptor.admission().max_queued(), 9);
    assert_eq!(descriptor.locality(), Locality::Remote);
    assert!(
        descriptor
            .trust_zones()
            .contains(&TrustZone::new("external-write")?)
    );
    let resources = descriptor
        .resource_observations()
        .ok_or("missing resources")?;
    assert_eq!(resources.estimated_cost_micros(), Some(125));
    assert_eq!(resources.estimated_duration_ms(), Some(800));
    assert_eq!(resources.currency(), Some("DKK"));
    assert!(descriptor.labels().contains("durable"));
    assert_eq!(descriptor.extensions().len(), 1);

    let operation_id = OperationId::new("tool.publish")?;
    let operation = descriptor
        .operation(&operation_id)
        .ok_or("missing operation")?;
    assert_eq!(operation.input().id().as_str(), "tool.publish.input");
    assert_eq!(operation.output().id().as_str(), "tool.publish.output");
    assert!(operation.streaming().contains(&StreamingMode::Progress));
    assert_eq!(operation.cancellation(), CancellationBehavior::Acknowledged);
    assert_eq!(
        operation.idempotency(),
        IdempotencyBehavior::ProviderProfileScoped
    );
    assert_eq!(operation.side_effect(), SideEffectClass::IdempotentWrite);
    let feature = operation
        .features()
        .get(&FeatureId::new("tool.batch")?)
        .ok_or("missing feature")?;
    assert_eq!(
        feature.settings_schema().map(SchemaContract::version),
        Some(1)
    );

    let requirement = CapabilityRequirement::new(operation_id.clone())
        .exact(CapabilityId::new("publisher-primary")?)
        .provider_profile(ProviderProfileRef::new("publisher-prod")?)
        .category(CapabilityCategory::Tool)
        .feature(FeatureId::new("tool.batch")?)
        .streaming(StreamingMode::Progress)
        .cancellation(true)
        .maximum_side_effect(SideEffectClass::IdempotentWrite)
        .trust_zone(TrustZone::new("external-write")?);
    assert_eq!(
        requirement.exact_capability().map(CapabilityId::as_str),
        Some("publisher-primary")
    );
    assert_eq!(
        requirement
            .provider_profile_ref()
            .map(ProviderProfileRef::as_str),
        Some("publisher-prod")
    );
    assert!(requirement.categories().contains(&CapabilityCategory::Tool));
    assert_eq!(requirement.operation(), &operation_id);
    assert_eq!(requirement.required_features().len(), 1);
    assert_eq!(requirement.streaming_mode(), Some(StreamingMode::Progress));
    assert!(requirement.cancellation_required());
    assert_eq!(
        requirement.maximum_side_effect_class(),
        SideEffectClass::IdempotentWrite
    );
    assert_eq!(requirement.trust_zones().len(), 1);

    let request = invocation_request()?;
    assert_eq!(request.invocation().as_str(), "invocation-007");
    assert_eq!(request.capability(), descriptor.identity());
    assert_eq!(request.operation(), &operation_id);
    assert_eq!(
        request.provider_profile().map(ProviderProfileRef::as_str),
        Some("publisher-prod")
    );
    assert_eq!(
        request.idempotency_key().map(IdempotencyKey::as_str),
        Some("run-1-node-2-attempt-1")
    );
    assert_eq!(request.inputs()[0].name(), "payload");
    assert!(request.inputs()[0].value().inline().is_some());
    assert_eq!(request.extensions().len(), 1);

    let event = invocation_event()?;
    assert_eq!(event.invocation(), request.invocation());
    assert_eq!(event.sequence(), 3);
    let terminal = event.kind().terminal().ok_or("missing terminal")?;
    assert_eq!(terminal.status(), TerminalStatus::Uncertain);
    assert_eq!(terminal.side_effect(), SideEffectClass::IdempotentWrite);
    let output = &terminal.outputs()[0];
    assert_eq!(output.identity(), "artifact-output");
    assert_eq!(output.digest(), ARTIFACT_DIGEST);
    assert_eq!(output.media_type(), Some("application/json"));
    assert_eq!(output.size_bytes(), Some(17));
    let failure = terminal.failure().ok_or("missing failure")?;
    assert_eq!(failure.class(), ErrorClass::Transport);
    assert!(failure.retryable());
    assert_eq!(failure.code(), "remote_timeout");
    assert!(!failure.message().is_empty());
    assert_eq!(failure.retry_after_ms(), Some(250));
    let usage = terminal.usage().ok_or("missing usage")?;
    assert_eq!(usage.input_units(), Some(10));
    assert_eq!(usage.output_units(), Some(20));
    assert_eq!(usage.duration_ms(), Some(75));
    assert_eq!(usage.cost_micros(), Some(42));
    assert_eq!(usage.currency(), Some("USD"));
    assert!(usage.extensions().is_empty());

    let cancellation = CancellationRequest::new(InvocationId::new("invocation-007")?, 2, "cancel")?;
    assert_eq!(cancellation.invocation(), request.invocation());
    assert_eq!(cancellation.request_sequence(), 2);
    assert_eq!(cancellation.reason(), "cancel");
    let acknowledgement = CancellationAcknowledgement::new(
        InvocationId::new("invocation-007")?,
        2,
        true,
        false,
        Some("pending".to_owned()),
    )?;
    assert_eq!(acknowledgement.invocation(), request.invocation());
    assert_eq!(acknowledgement.request_sequence(), 2);
    assert!(acknowledgement.accepted());
    assert!(!acknowledgement.terminal_boundary());
    assert_eq!(acknowledgement.detail(), Some("pending"));
    Ok(())
}

#[test]
fn malformed_direct_serde_input_cannot_bypass_constructors()
-> Result<(), Box<dyn std::error::Error>> {
    let mut operation_value = serde_json::to_value(operation()?)?;
    operation_value["streaming"] = json!([]);
    assert!(serde_json::from_value::<OperationContract>(operation_value).is_err());

    let mut feature_schema_value = serde_json::to_value(operation()?)?;
    feature_schema_value["features"]["tool.batch"]["settings_schema"]["version"] = json!(0);
    assert!(serde_json::from_value::<OperationContract>(feature_schema_value).is_err());

    assert!(
        serde_json::from_value::<AdmissionConstraints>(json!({
            "max_concurrent": 0,
            "max_queued": 1
        }))
        .is_err()
    );
    assert!(
        serde_json::from_value::<InvocationValueReference>(json!({
            "type": "workspace_value",
            "identity": "",
            "version": "v1"
        }))
        .is_err()
    );
    assert!(
        serde_json::from_value::<InputReference>(json!({
            "name": "contains a space",
            "value": {"type": "inline", "value": null}
        }))
        .is_err()
    );
    assert!(
        serde_json::from_value::<InvocationFailure>(json!({
            "class": "transport",
            "retryable": true,
            "code": "",
            "message": "bad",
            "retry_after_ms": null
        }))
        .is_err()
    );
    assert!(
        serde_json::from_value::<UsageObservation>(json!({
            "input_units": null,
            "output_units": null,
            "duration_ms": null,
            "cost_micros": 1,
            "currency": null,
            "extensions": {}
        }))
        .is_err()
    );

    let mut terminal_value = serde_json::to_value(terminal()?)?;
    terminal_value["failure"] = Value::Null;
    assert!(serde_json::from_value::<InvocationTerminal>(terminal_value).is_err());

    let oversized_features: Vec<_> = (0..=256).map(|index| format!("tool.f{index}")).collect();
    let mut requirement_value = serde_json::to_value(CapabilityRequirement::new(
        OperationId::new("tool.publish")?,
    ))?;
    requirement_value["required_features"] = json!(oversized_features);
    assert!(serde_json::from_value::<CapabilityRequirement>(requirement_value).is_err());

    let mut direct_document =
        serde_json::to_value(InvocationEventDocument::new(invocation_event()?))?;
    direct_document["schema_version"] = json!(2);
    assert!(serde_json::from_value::<InvocationEventDocument>(direct_document).is_err());
    Ok(())
}
