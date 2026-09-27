//! Individually valid publication data must still belong to the enclosing serving record.
use milkdrift_authority::{
    ActorRef, AuthorityBudget, AuthorityDecisionSnapshot, AuthorityExecutionProvenance,
    AuthorityOperation, AuthorityRequest, BoundaryTimeMillis, DecisionId, DecisionReasonCode,
    GrantDigest, GrantId, PolicyId, RequestedResourceFacts,
};
use milkdrift_capability::{
    AdmissionConstraints, BoundedJson, CancellationBehavior, CapabilityCategory, CapabilityId,
    DescriptorBuilder, IdempotencyBehavior, InputReference, InvocationId, InvocationRequest,
    InvocationValueReference, Locality, OperationContract, OperationId, PeerId,
    ResolvedCapabilitySnapshot, SchemaContract, SchemaId, SideEffectClass, StreamingMode,
};
use milkdrift_peer_protocol::{
    CatalogSnapshot, DelegatedAuthorization, DelegationRef, ExecutionLimits, InvocationOrigin,
    PeerExecutionId, PeerRequestId, ServingInvocationRequest,
};
use milkdrift_persistence::{
    CommandId, ControllerAccountDeclaration, ControllerResourceBudget, PeerEntryEvidence,
    PeerExecutionAccounting, PeerExecutionPhase, PeerExecutionRecord,
    SERVING_EXECUTION_RECORD_SCHEMA_VERSION, WorkerId,
    published::{PublishedInvocationPlan, PublishedInvocationSource, PublishedServiceIdentity},
};
use milkdrift_workspace::RunId;
use std::collections::{BTreeMap, BTreeSet};

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

fn record() -> TestResult<PeerExecutionRecord> {
    let capability = CapabilityId::new("method:record-test")?;
    let operation = OperationId::new("method.invoke")?;
    let schema = SchemaContract::new(
        SchemaId::new("record-test.value")?,
        1,
        BoundedJson::new(serde_json::json!({"type":"object"}))?,
    )?;
    let descriptor = DescriptorBuilder::new(
        capability.clone(),
        1,
        CapabilityCategory::Tool,
        AdmissionConstraints::new(1, 0)?,
        Locality::Local,
    )
    .operations(BTreeMap::from([(
        operation.clone(),
        OperationContract::new(
            schema.clone(),
            schema,
            BTreeSet::from([StreamingMode::Progress]),
            CancellationBehavior::Acknowledged,
            IdempotencyBehavior::CapabilityScoped,
            SideEffectClass::ReadOnly,
            BTreeMap::new(),
        )?,
    )]))
    .build()?;
    let actor = ActorRef::new("peer:caller")?;
    let mut resources = RequestedResourceFacts::empty();
    resources.peer = Some(PeerId::new("caller")?);
    resources.capability = Some(capability.clone());
    resources.capability_operation = Some(operation.clone());
    let authority = AuthorityDecisionSnapshot::from_evaluation(
        PolicyId::new("policy:record-test")?,
        1,
        AuthorityRequest {
            decision: DecisionId::new("decision:record-test")?,
            actor: actor.clone(),
            grant: GrantId::new("grant:record-test")?,
            grant_revision: 1,
            grant_digest: GrantDigest::new(format!("b3_{}", "0".repeat(64)))?,
            revocation_generation: 0,
            operation: AuthorityOperation::InvokePeerCapability,
            resources,
            budget: AuthorityBudget::default(),
            evaluated_at: BoundaryTimeMillis::new(100),
            provenance: AuthorityExecutionProvenance {
                descriptor_revision: Some(1),
                ..Default::default()
            },
        },
        vec![DecisionReasonCode::Allowed],
        AuthorityBudget::default(),
        SideEffectClass::ReadOnly,
    )?;
    let limits = ExecutionLimits {
        nested_invocations: None,
        artifact_bytes: 1024,
        duration_ms: 1000,
        cost_micros: 0,
        cost_currency: None,
        input_units: None,
        output_units: None,
        observations: 10,
    };
    let request_id = PeerRequestId::new("request:record-test")?;
    let request = ServingInvocationRequest::new(
        request_id.clone(),
        1,
        CatalogSnapshot::new(1, 1, 20000, Vec::new())?.digest,
        ResolvedCapabilitySnapshot::from_descriptor(&descriptor, &operation)?,
        InvocationRequest::new(
            InvocationId::new("invocation:origin")?,
            capability.clone(),
            operation.clone(),
            None,
            None,
            Vec::new(),
            BTreeMap::new(),
        )?,
        limits.clone(),
        10000,
        DelegatedAuthorization {
            publication_ancestry: Vec::new(),
            controller_reservation: None,
            reference: DelegationRef::new("delegation:record-test")?,
            issuer_peer: PeerId::new("caller")?,
            actor,
            target_peer: PeerId::new("serving")?,
            capability: capability.clone(),
            operation: operation.clone(),
            request: request_id,
            limits,
            expires_at_unix_ms: 10000,
            nonce: "record-test-nonce".into(),
            origin: InvocationOrigin::Direct,
        },
    )?;
    let mut record = PeerExecutionRecord {
        published_invocation: None,
        schema_version: SERVING_EXECUTION_RECORD_SCHEMA_VERSION,
        caller: request.authorization.caller(),
        relationship_generation: 1,
        request,
        authority: authority.clone(),
        execution: PeerExecutionId::new("execution:record-test")?,
        acceptance_sequence: 1,
        accepted_at_unix_ms: 100,
        phase: PeerExecutionPhase::AwaitingWorkflow {
            evidence: PeerEntryEvidence {
                worker: WorkerId::new("worker:record-test")?,
                claim_generation: 1,
                entered_at_unix_ms: 101,
                authority: authority.clone(),
            },
        },
        cancellation: None,
        last_observation_sequence: 0,
        accounting: PeerExecutionAccounting::default(),
        observation_digest: super::super::observation_genesis_digest(),
        revision: 1,
    };
    let invocation = record.managed_invocation()?;
    let child = RunId::new("run:published-child")?;
    let method_digest = format!("b3_{}", "1".repeat(64));
    record.published_invocation = Some(PublishedInvocationPlan {
        source: PublishedInvocationSource::Serving {
            caller: record.caller.clone(),
            execution: record.execution.clone(),
        },
        schema_version: 1,
        invocation: invocation.clone(),
        request: InvocationRequest::new(
            invocation.clone(),
            capability.clone(),
            operation,
            None,
            None,
            Vec::new(),
            BTreeMap::new(),
        )?,
        capability,
        generation: 1,
        method_digest: method_digest.clone(),
        allowance: ControllerAccountDeclaration::for_published_invocation(
            child.clone(),
            invocation,
            method_digest,
            ControllerResourceBudget::new(0, None, 0, 0, 1024, 1, 0)?,
        )?,
        child_run: child,
        // These are opaque to the storage link reader; runtime owns command decoding.
        create_command: "{}".into(),
        start_command: "{}".into(),
        cancel_command: CommandId::new("cancel:record-test")?,
        service: PublishedServiceIdentity {
            actor: authority.request().actor.clone(),
            grant: authority.request().grant.clone(),
            grant_revision: 1,
            grant_digest: authority.request().grant_digest.clone(),
            revocation_generation: 0,
        },
        caller: authority,
        maximum_depth: 1,
        ancestry: Vec::new(),
        deadline_unix_ms: 10000,
    });
    Ok(record)
}

#[test]
fn published_record_corruption_refuses_both_transaction_readers() -> TestResult {
    let original = record()?;
    for case in [
        "valid",
        "source",
        "capability",
        "inputs",
        "outputs",
        "observations",
    ] {
        let mut record = original.clone();
        let plan = record
            .published_invocation
            .as_mut()
            .ok_or("association absent")?;
        if case == "source" {
            plan.source = PublishedInvocationSource::Serving {
                caller: record.caller.clone(),
                execution: PeerExecutionId::new("execution:unrelated")?,
            };
        }
        if case == "capability" {
            plan.capability = CapabilityId::new("method:unrelated")?;
            let mut request = plan.caller.request().clone();
            request.resources.capability = Some(plan.capability.clone());
            plan.caller = AuthorityDecisionSnapshot::from_evaluation(
                PolicyId::new("policy:record-test")?,
                1,
                request,
                vec![DecisionReasonCode::Allowed],
                AuthorityBudget::default(),
                SideEffectClass::ReadOnly,
            )?;
        }
        if matches!(case, "capability" | "inputs") {
            let inputs = if case == "inputs" {
                vec![InputReference::new(
                    "unaccepted",
                    InvocationValueReference::Inline {
                        value: BoundedJson::new(serde_json::json!({"changed":true}))?,
                    },
                )?]
            } else {
                Vec::new()
            };
            plan.request = InvocationRequest::new(
                plan.invocation.clone(),
                plan.capability.clone(),
                OperationId::new("method.invoke")?,
                None,
                None,
                inputs,
                BTreeMap::new(),
            )?;
        }
        plan.validate()?;
        if case == "outputs" {
            record.accounting.outputs = 1;
        }
        if case == "observations" {
            record.last_observation_sequence = u64::from(record.request.limits.observations) + 1;
            record.accounting.observations = record.request.limits.observations + 1;
        }
        let root = tempfile::tempdir()?;
        let db = redb::Database::create(root.path().join("peer.redb"))?;
        let write = db.begin_write()?;
        crate::schema::initialize_tables(&write)?;
        write
            .open_table(crate::schema::PEER_EXECUTION_LOCATIONS)?
            .insert(record.execution.as_str(), 1)?;
        // Encode a valid document envelope but bypass the validating writer to model disk corruption.
        let bytes = crate::json::encode(&record, "peer execution")?;
        write
            .open_table(crate::schema::PEER_EXECUTIONS)?
            .insert(record.execution.as_str(), bytes.as_slice())?;
        let assert_result = |result| {
            if case != "valid" {
                let expected = if matches!(case, "outputs" | "observations") {
                    "primary facts are invalid"
                } else {
                    "published serving link differs"
                };
                assert!(
                    matches!(result, Err(milkdrift_persistence::PersistenceError::Storage {
                    class: milkdrift_persistence::StorageFailureClass::Corruption, message,
                }) if message.contains(expected))
                );
            } else {
                assert!(
                    matches!(result, Ok(Some(milkdrift_persistence::PeerExecutionSnapshot::Hot(actual))) if *actual == record)
                );
            }
        };
        assert_result(super::super::snapshot_optional_in_transaction(
            &write,
            &record.execution,
        ));
        write.commit()?;
        assert_result(super::super::snapshot_optional_in_read_transaction(
            &db.begin_read()?,
            &record.execution,
        ));
    }
    Ok(())
}
