//! Deterministic protocol negotiation, bounds, digest, and selection tests.

use std::collections::{BTreeMap, BTreeSet};

use milkdrift_authority::{ActorRef, PeerId};
use milkdrift_capability::{
    AdmissionConstraints, BoundedJson, CancellationBehavior, CapabilityCategory, CapabilityId,
    CapabilityObservation, DescriptorBuilder, IdempotencyBehavior, InvocationId, InvocationRequest,
    Locality, OperationContract, OperationId, ResolvedCapabilitySnapshot, SchemaContract, SchemaId,
    SideEffectClass, StreamingMode,
};
use milkdrift_peer_protocol::{
    CatalogEntry, CatalogSnapshot, DecodeLimits, DelegatedAuthorization, DelegationRef,
    ExecutionLimits, PeerInvocationRequest, PeerRequestId, ProtocolEnvelope, ProtocolVersion,
    ProtocolVersionRange, decode_envelope, encode_envelope,
};

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

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

#[test]
fn version_negotiation_fails_closed_on_unknown_major() -> TestResult {
    let local = ProtocolVersionRange::default();
    assert_eq!(local.negotiate(local)?, ProtocolVersion::V1_0);
    let unknown = ProtocolVersionRange::new(
        ProtocolVersion { major: 2, minor: 0 },
        ProtocolVersion { major: 2, minor: 1 },
    )?;
    assert!(local.negotiate(unknown).is_err());
    Ok(())
}

#[test]
fn decoder_rejects_bytes_depth_duplicates_and_unknown_major() -> TestResult {
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
        observations: 100,
    };
    let peer_a = PeerId::new("peer-a")?;
    let peer_b = PeerId::new("peer-b")?;
    let delegation = DelegatedAuthorization {
        reference: DelegationRef::new("delegation-1")?,
        issuer_peer: peer_a.clone(),
        actor: ActorRef::new("peer:peer-a")?,
        target_peer: peer_b,
        capability: descriptor.identity().clone(),
        operation,
        request: request_id.clone(),
        limits,
        expires_at_unix_ms: 20_000,
        nonce: "nonce-1".to_owned(),
        provenance: milkdrift_peer_protocol::PeerExecutionProvenance {
            run: "run-1".to_owned(),
            revision: "revision-1".to_owned(),
            node: "node-1".to_owned(),
            execution: "execution-1".to_owned(),
            attempt: "attempt-1".to_owned(),
        },
    };
    let catalog = CatalogSnapshot::new(1, 1_000, 20_000, Vec::new())?;
    let request = PeerInvocationRequest::new(
        request_id,
        1,
        catalog.digest,
        selection,
        invocation,
        limits,
        15_000,
        delegation,
    )?;
    let mut value = serde_json::to_value(&request)?;
    value["deadline_unix_ms"] = serde_json::json!(15_001);
    assert!(serde_json::from_value::<PeerInvocationRequest>(value).is_err());
    Ok(())
}
