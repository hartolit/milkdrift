//! Real serving acceptance and publication fixtures shared by contract and physical tests.
use milkdrift_authority::*;
use milkdrift_capability::*;
use milkdrift_capability_host::{AdapterExecutionContext, DirectInputSelection};
use milkdrift_peer_protocol::*;
use milkdrift_persistence::*;
use milkdrift_redb_store::RedbStore;
use std::collections::BTreeMap;
/// Fallible fixture result, including bounded mechanism errors.
pub type Result<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

/// Allowed decision retaining the complete fixture request.
pub fn decision(request: AuthorityRequest) -> Result<AuthorityDecisionSnapshot> {
    Ok(AuthorityDecisionSnapshot::from_evaluation(
        PolicyId::new("managed-test")?,
        1,
        request,
        vec![DecisionReasonCode::Allowed],
        AuthorityBudget::default(),
        SideEffectClass::Unknown,
    )?)
}
/// Fixture actor and finite originating resource scope.
pub fn caller() -> Result<AuthorityRequest> {
    Ok(AuthorityRequest {
        decision: DecisionId::new("managed-test")?,
        actor: ActorRef::new("human:managed")?,
        grant: GrantId::new("grant:managed")?,
        grant_revision: 1,
        grant_digest: GrantDigest::new(format!("b3_{}", "1".repeat(64)))?,
        revocation_generation: 0,
        operation: AuthorityOperation::InvokeCapability,
        resources: RequestedResourceFacts::empty(),
        budget: AuthorityBudget::default(),
        evaluated_at: BoundaryTimeMillis::new(100),
        provenance: Default::default(),
    })
}

/// Accept and enter through the production serving store; the resource reservation joins that transaction.
pub fn entered(
    store: &RedbStore,
    descriptor: &CapabilityDescriptor,
    key: &str,
    input: &str,
    value: serde_json::Value,
) -> Result<(
    InvocationRequest,
    AdapterExecutionContext,
    PeerExecutionRecord,
)> {
    let operation = descriptor
        .operations()
        .keys()
        .find(|operation| input != "request" || operation.as_str() == "resource.manage")
        .ok_or("operation missing")?
        .clone();
    let key_value =
        if descriptor.operations()[&operation].side_effect() == SideEffectClass::IdempotentWrite {
            Some(IdempotencyKey::new(key)?)
        } else {
            None
        };
    let request = InvocationRequest::new(
        InvocationId::new(key)?,
        descriptor.identity().clone(),
        operation.clone(),
        descriptor.provider_profile().cloned(),
        key_value.clone(),
        vec![InputReference::new(
            input,
            InvocationValueReference::Inline {
                value: BoundedJson::new(value)?,
            },
        )?],
        BTreeMap::new(),
    )?;
    let mut authority = caller()?;
    authority.provenance.descriptor_revision = Some(descriptor.descriptor_revision());
    authority.resources.capability = Some(descriptor.identity().clone());
    authority.resources.capability_operation = Some(operation.clone());
    authority.resources.side_effect = descriptor.operations()[&operation].side_effect();
    let authorization = ClientInvocationAuthorization {
        host: PeerId::new("host-managed-test")?,
        actor: authority.actor.clone(),
        grant: authority.grant.clone(),
        grant_revision: 1,
        grant_digest: authority.grant_digest.clone(),
        revocation_generation: 0,
    };
    let catalog = CatalogSnapshot::new(1, 1, 1_000_000, Vec::new())?;
    let serving = ServingInvocationRequest::new(
        PeerRequestId::new(key)?,
        1,
        catalog.digest.clone(),
        ResolvedCapabilitySnapshot::from_descriptor(descriptor, &operation)?,
        request.clone(),
        ExecutionLimits {
            artifact_bytes: 16_777_216,
            duration_ms: 300_000,
            cost_micros: 0,
            cost_currency: None,
            input_units: (*descriptor.category()
                == milkdrift_capability::CapabilityCategory::Model)
                .then_some(8192),
            output_units: (*descriptor.category()
                == milkdrift_capability::CapabilityCategory::Model)
                .then_some(4096),
            observations: 128,
        },
        1_000_000,
        authorization,
    )?;
    let caller = serving.authorization.caller();
    store.set_peer_admission_open(true)?;
    store.configure_peer_relationship(&ServingCallerState {
        caller: caller.clone(),
        generation: 1,
        enabled: true,
        expires_at_unix_ms: 1_000_000,
        maximum_active: 32,
    })?;
    store.publish_peer_catalog(&ServingCatalogState {
        caller: caller.clone(),
        relationship_generation: 1,
        generation: 1,
        digest: catalog.digest.as_str().to_owned(),
        expires_at_unix_ms: 1_000_000,
    })?;
    let authority = decision(authority)?;
    let execution = PeerExecutionId::new(format!("exec-{key}"))?;
    let admission = store.admit_peer_execution(&PeerAdmission {
        caller: &caller,
        request: &serving,
        authority: &authority,
        execution: &execution,
        relationship_generation: 1,
        accepted_at_unix_ms: 100,
        maximum_global_active: 32,
        maximum_dispatch_queue: 32,
        maximum_hot_terminal_records: 128,
        archive_batch_size: 16,
        archive_terminal_before_or_at_unix_ms: 1,
    })?;
    if !matches!(admission, PeerAdmissionOutcome::Accepted(_)) {
        return Err(format!("acceptance: {admission:?}").into());
    }
    let worker = WorkerId::new(format!("worker-{key}"))?;
    let PeerClaimOutcome::Claimed(claimed) =
        store.claim_peer_dispatch(&PeerDispatchClaimRequest {
            worker: &worker,
            claimed_at_unix_ms: 101,
            lease_expires_at_unix_ms: 900_000,
        })?
    else {
        return Err("claim missing".into());
    };
    let PeerEntryOutcome::Entered(record) = store.mark_peer_entered(&PeerEntryRequest {
        owner: &caller,
        execution: &execution,
        worker: &worker,
        claim_generation: claimed.phase.claim().ok_or("claim absent")?.generation,
        relationship_generation: 1,
        entered_at_unix_ms: 102,
        authority: &authority,
    })?
    else {
        return Err("entry refused".into());
    };
    let context = milkdrift_capability_host::conformance::entered_serving_context(
        AdapterExecutionContext::direct(DirectInputSelection::new(&request, 16_777_216)?),
        &record,
    )?;
    let rewritten = InvocationRequest::new(
        record.managed_invocation()?,
        descriptor.identity().clone(),
        operation,
        descriptor.provider_profile().cloned(),
        key_value.clone(),
        request.inputs().to_vec(),
        BTreeMap::new(),
    )?;
    Ok((rewritten, context, *record))
}
