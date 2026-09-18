//! Deterministic protocol negotiation, bounds, digest, and selection tests.

use std::collections::{BTreeMap, BTreeSet};

use milkdrift_authority::ActorRef;
use milkdrift_capability::{
    AdmissionConstraints, ArtifactReference, BoundedJson, CancellationBehavior, CapabilityCategory,
    CapabilityId, CapabilityObservation, DescriptorBuilder, IdempotencyBehavior, InputReference,
    InvocationId, InvocationRequest, InvocationValueReference, Locality, OperationContract,
    OperationId, PeerId, ResolvedCapabilitySnapshot, SchemaContract, SchemaId, SideEffectClass,
    StreamingMode,
};
use milkdrift_peer_protocol::{
    ArchivedExecutionSummary, CatalogEntry, CatalogSnapshot, DecodeLimits, DelegatedAuthorization,
    DelegationRef, ExecutionLimits, InvocationLookup, ObservationHistory, ObservationPage,
    PeerExecutionId, PeerRequestId, ProtocolEnvelope, ProtocolVersion, ProtocolVersionRange,
    RemoteExecutionStatus, ServingInvocationRequest, decode_envelope, encode_envelope,
};

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

#[test]
fn serving_allowance_requires_enforced_units_currency_and_input_plus_output_bytes() -> TestResult {
    use milkdrift_capability::{
        AdmissionBound, AdmissionMonetaryBound, AdmissionUnit, InvocationAdmissionEnvelope,
    };
    let limits = ExecutionLimits {
        artifact_bytes: 13,
        duration_ms: 100,
        cost_micros: 3,
        cost_currency: Some("USD".to_owned()),
        input_units: Some(1),
        output_units: Some(2),
        observations: 8,
    };
    let prepared = InvocationAdmissionEnvelope::new(
        AdmissionUnit::ModelTokens,
        AdmissionBound::Bounded(1),
        AdmissionBound::Bounded(2),
        AdmissionBound::Bounded(10),
        AdmissionBound::Bounded(AdmissionMonetaryBound::new(3, "USD")?),
    );
    assert!(limits.permits_prepared(&prepared, 3));
    assert!(!limits.permits_prepared(&prepared, 4));
    assert!(!limits.permits_prepared(&InvocationAdmissionEnvelope::unknown(), 0));
    for changed in [
        ExecutionLimits {
            cost_currency: Some("EUR".to_owned()),
            ..limits.clone()
        },
        ExecutionLimits {
            input_units: Some(0),
            ..limits.clone()
        },
        ExecutionLimits {
            output_units: None,
            ..limits.clone()
        },
        ExecutionLimits {
            cost_micros: 2,
            ..limits.clone()
        },
    ] {
        assert!(!changed.permits_prepared(&prepared, 3));
    }
    let mut invalid = limits.clone();
    invalid.cost_currency = None;
    assert!(invalid.validate().is_err());
    let mut changed_request = limits.clone();
    changed_request.cost_currency = Some("EUR".to_owned());
    assert!(!limits.contains(&changed_request));
    assert_eq!(
        limits.admission_envelope()?.input_units(),
        prepared.input_units()
    );
    Ok(())
}

fn descriptor() -> TestResult<milkdrift_capability::CapabilityDescriptor> {
    let schema = || {
        SchemaContract::new(
            SchemaId::new("test.value")?,
            1,
            BoundedJson::new(serde_json::json!({"type": "object"}))?,
        )
    };
    let operation = OperationContract::new(
        schema()?,
        schema()?,
        BTreeSet::from([StreamingMode::Progress]),
        CancellationBehavior::Acknowledged,
        IdempotencyBehavior::CapabilityScoped,
        SideEffectClass::ReadOnly,
        BTreeMap::new(),
    )?;
    Ok(DescriptorBuilder::new(
        CapabilityId::new("test-capability")?,
        7,
        CapabilityCategory::Tool,
        AdmissionConstraints::new(2, 0)?,
        Locality::Local,
    )
    .operations(BTreeMap::from([(
        OperationId::new("test.execute")?,
        operation,
    )]))
    .build()?)
}

fn request_with_artifact_limit(
    suffix: &str,
    size_bytes: Option<u64>,
    artifact_limit: u64,
) -> TestResult<ServingInvocationRequest> {
    let descriptor = descriptor()?;
    let operation = OperationId::new("test.execute")?;
    let request_id = PeerRequestId::new(format!("request-{suffix}"))?;
    let limits = ExecutionLimits {
        artifact_bytes: artifact_limit,
        duration_ms: 10_000,
        cost_micros: 0,
        cost_currency: None,
        input_units: None,
        output_units: None,
        observations: 10,
    };
    ServingInvocationRequest::new(
        request_id.clone(),
        1,
        CatalogSnapshot::new(1, 1, 20_000, Vec::new())?.digest,
        ResolvedCapabilitySnapshot::from_descriptor(&descriptor, &operation)?,
        InvocationRequest::new(
            InvocationId::new(format!("invocation-{suffix}"))?,
            descriptor.identity().clone(),
            operation.clone(),
            None,
            None,
            vec![InputReference::new(
                "payload",
                InvocationValueReference::Artifact {
                    reference: ArtifactReference::new(
                        format!("artifact-{suffix}"),
                        "a".repeat(64),
                        Some("application/octet-stream".to_owned()),
                        size_bytes,
                    )?,
                },
            )?],
            BTreeMap::new(),
        )?,
        limits.clone(),
        15_000,
        DelegatedAuthorization {
            controller_reservation: None,
            reference: DelegationRef::new(format!("delegation-{suffix}"))?,
            issuer_peer: PeerId::new("peer-a")?,
            actor: ActorRef::new("peer:peer-a")?,
            target_peer: PeerId::new("peer-b")?,
            capability: descriptor.identity().clone(),
            operation,
            request: request_id,
            limits: limits.clone(),
            expires_at_unix_ms: 20_000,
            nonce: format!("nonce-{suffix}"),
            origin: milkdrift_peer_protocol::InvocationOrigin::Workflow {
                provenance: milkdrift_peer_protocol::PeerExecutionProvenance {
                    run: "run-1".to_owned(),
                    revision: "revision-1".to_owned(),
                    node: "node-1".to_owned(),
                    execution: "execution-1".to_owned(),
                    attempt: "attempt-1".to_owned(),
                },
            },
        },
    )
    .map_err(Into::into)
}

#[test]
fn version_negotiation_fails_closed_on_unknown_major() -> TestResult {
    let local = ProtocolVersionRange::default();
    assert_eq!(local.negotiate(local)?, ProtocolVersion::V1_4);
    let unknown = ProtocolVersionRange::new(
        ProtocolVersion { major: 2, minor: 0 },
        ProtocolVersion { major: 2, minor: 1 },
    )?;
    assert!(local.negotiate(unknown).is_err());
    let legacy_minor = ProtocolVersionRange::new(
        ProtocolVersion { major: 1, minor: 1 },
        ProtocolVersion { major: 1, minor: 1 },
    )?;
    assert!(local.negotiate(legacy_minor).is_err());
    Ok(())
}

#[test]
fn artifact_input_quota_requires_exact_sizes_and_accepts_the_exact_boundary() -> TestResult {
    assert!(request_with_artifact_limit("missing", None, 8).is_err());
    assert!(request_with_artifact_limit("overflow", Some(9), 8).is_err());
    assert_eq!(
        request_with_artifact_limit("boundary", Some(8), 8)?.input_artifact_bytes()?,
        8
    );
    Ok(())
}

#[test]
fn invocation_lookup_is_bound_to_the_exact_queried_request() -> TestResult {
    let requested = PeerRequestId::new("request-expected")?;
    let swapped = InvocationLookup::NotAccepted {
        request_id: PeerRequestId::new("request-swapped")?,
    };
    assert!(swapped.validate_for(&requested).is_err());
    InvocationLookup::NotAccepted {
        request_id: requested.clone(),
    }
    .validate_for(&requested)?;
    Ok(())
}

#[test]
fn decoder_rejects_bounds_duplicates_and_every_non_current_version() -> TestResult {
    let envelope = ProtocolEnvelope::v1(serde_json::json!({"ok": true}));
    let bytes = encode_envelope(&envelope)?;
    assert!(
        decode_envelope::<serde_json::Value>(
            &bytes,
            DecodeLimits {
                bytes: bytes.len().saturating_sub(1),
                ..DecodeLimits::default()
            }
        )
        .is_err()
    );
    assert!(
        decode_envelope::<serde_json::Value>(
            br#"{"protocol":{"major":1,"minor":0},"message":{"a":1,"a":2},"extensions":{}}"#,
            DecodeLimits::default(),
        )
        .is_err()
    );
    assert!(
        decode_envelope::<serde_json::Value>(
            br#"{"protocol":{"major":2,"minor":0},"message":null,"extensions":{}}"#,
            DecodeLimits::default(),
        )
        .is_err()
    );
    for minor in [0_u16, 1, 2, 3, 5, u16::MAX] {
        let bytes = format!(
            "{{\"protocol\":{{\"major\":1,\"minor\":{minor}}},\"message\":null,\"extensions\":{{}}}}"
        );
        assert!(matches!(
            decode_envelope::<serde_json::Value>(bytes.as_bytes(), DecodeLimits::default()),
            Err(milkdrift_peer_protocol::PeerProtocolError::IncompatibleVersion)
        ));
    }
    let nested = format!(
        "{{\"protocol\":{{\"major\":1,\"minor\":0}},\"message\":{} ,\"extensions\":{{}}}}",
        "[".repeat(40) + &"]".repeat(40)
    );
    assert!(
        decode_envelope::<serde_json::Value>(nested.as_bytes(), DecodeLimits::default(),).is_err()
    );
    Ok(())
}

#[test]
fn catalog_digest_and_expiry_are_exact() -> TestResult {
    let descriptor = descriptor()?;
    let observation =
        CapabilityObservation::new(descriptor.identity().clone(), 1_000, true, 0, "healthy")?;
    let snapshot = CatalogSnapshot::new(
        1,
        1_000,
        2_000,
        vec![CatalogEntry {
            descriptor,
            invocable_operations: BTreeSet::from([OperationId::new("test.execute")?]),
            observation,
            draining: false,
        }],
    )?;
    assert!(snapshot.is_live_at(2_000));
    assert!(!snapshot.is_live_at(2_001));
    let bytes = serde_json::to_vec(&snapshot)?;
    let mut value: serde_json::Value = serde_json::from_slice(&bytes)?;
    value["generation"] = serde_json::json!(2);
    assert!(serde_json::from_value::<CatalogSnapshot>(value).is_err());
    Ok(())
}

#[test]
fn invocation_digest_binds_catalog_selection_delegation_and_request() -> TestResult {
    let descriptor = descriptor()?;
    let operation = OperationId::new("test.execute")?;
    let selection = ResolvedCapabilitySnapshot::from_descriptor(&descriptor, &operation)?;
    let invocation = InvocationRequest::new(
        InvocationId::new("invocation-1")?,
        descriptor.identity().clone(),
        operation.clone(),
        None,
        None,
        Vec::new(),
        BTreeMap::new(),
    )?;
    let request_id = PeerRequestId::new("request-1")?;
    let limits = ExecutionLimits {
        artifact_bytes: 1024,
        duration_ms: 10_000,
        cost_micros: 0,
        cost_currency: None,
        input_units: None,
        output_units: None,
        observations: 100,
    };
    let peer_a = PeerId::new("peer-a")?;
    let peer_b = PeerId::new("peer-b")?;
    let delegation = DelegatedAuthorization {
        controller_reservation: None,
        reference: DelegationRef::new("delegation-1")?,
        issuer_peer: peer_a.clone(),
        actor: ActorRef::new("peer:peer-a")?,
        target_peer: peer_b,
        capability: descriptor.identity().clone(),
        operation,
        request: request_id.clone(),
        limits: limits.clone(),
        expires_at_unix_ms: 20_000,
        nonce: "nonce-1".to_owned(),
        origin: milkdrift_peer_protocol::InvocationOrigin::Workflow {
            provenance: milkdrift_peer_protocol::PeerExecutionProvenance {
                run: "run-1".to_owned(),
                revision: "revision-1".to_owned(),
                node: "node-1".to_owned(),
                execution: "execution-1".to_owned(),
                attempt: "attempt-1".to_owned(),
            },
        },
    };
    let catalog = CatalogSnapshot::new(1, 1_000, 20_000, Vec::new())?;
    let request = ServingInvocationRequest::new(
        request_id,
        1,
        catalog.digest,
        selection,
        invocation,
        limits.clone(),
        15_000,
        delegation,
    )?;
    let mut value = serde_json::to_value(&request)?;
    value["deadline_unix_ms"] = serde_json::json!(15_001);
    assert!(serde_json::from_value::<ServingInvocationRequest>(value).is_err());
    Ok(())
}

#[test]
fn archived_observation_history_is_typed_closed_and_truthfully_uncertain() -> TestResult {
    let execution = PeerExecutionId::new("execution-archived")?;
    let summary = ArchivedExecutionSummary {
        status: RemoteExecutionStatus::OutcomeUnknown,
        last_sequence: 0,
        observation_digest: format!("b3_{}", "0".repeat(64)),
        archived_at_unix_ms: 10,
        final_observation: None,
        uncertainty_reason: Some(
            "adapter entry is known but terminal evidence was lost".to_owned(),
        ),
    };
    let page = ObservationPage {
        status: RemoteExecutionStatus::OutcomeUnknown,
        execution: execution.clone(),
        after_sequence: 0,
        observations: Vec::new(),
        next_sequence: 0,
        terminal: false,
        closed: true,
        history: ObservationHistory::Archived {
            summary: Box::new(summary.clone()),
        },
    };
    page.validate(8)?;
    let mut invalid = page;
    invalid.closed = false;
    assert!(invalid.validate(8).is_err());
    summary.validate(&execution)?;
    Ok(())
}
