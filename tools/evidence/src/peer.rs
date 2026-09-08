//! Drive peer records through acceptance, entry, observation, and archival in a local store.
//!
//! Replaying hot records and tombstones measures the persistence contract without claiming
//! connectivity or interoperability with an independently deployed peer.

use std::collections::{BTreeMap, BTreeSet};

use milkdrift_authority::{
    ActorRef, AuthorityBudget, AuthorityDecisionSnapshot, AuthorityExecutionProvenance,
    AuthorityOperation, AuthorityRequest, BoundaryTimeMillis, DecisionId, DecisionReasonCode,
    GrantDigest, GrantId, PolicyId, RequestedResourceFacts,
};
use milkdrift_blueprint::{NodeId, RevisionId};
use milkdrift_capability::{
    AdmissionConstraints, BoundedJson, CancellationBehavior, CapabilityCategory,
    CapabilityDescriptor, CapabilityId, DescriptorBuilder, IdempotencyBehavior, InvocationEvent,
    InvocationEventKind, InvocationId, InvocationRequest, InvocationTerminal, Locality,
    OperationContract, OperationId, PeerId, ResolvedCapabilitySnapshot, SchemaContract, SchemaId,
    SideEffectClass, StreamingMode, TerminalStatus,
};
use milkdrift_peer_protocol::{
    CatalogDigest, CatalogSnapshot, DelegatedAuthorization, DelegationRef, ExecutionLimits,
    ObservationCategory, PeerExecutionId, PeerInvocationRequest, PeerObservation, PeerRequestId,
};
use milkdrift_persistence::{
    PageSize, PeerAdmission, PeerAdmissionOutcome, PeerAdmissionRejection, PeerCatalogState,
    PeerClaimOutcome, PeerDispatchClaimRequest, PeerEntryOutcome, PeerEntryRequest,
    PeerExecutionSnapshot, PeerExecutionStore, PeerRelationshipState, PeerRetentionRequest,
    TimestampMillis, WorkerId,
};
use milkdrift_redb_store::RedbStore;

use crate::EvidenceResult;

const BASE_TIME: u64 = 1_000_000;
const PROGRESS_PER_EXECUTION: u64 = 16;

#[derive(serde::Serialize)]
pub(crate) struct PeerTurnoverEvidence {
    pub(crate) executions: u64,
    pub(crate) observations: u64,
    pub(crate) final_active: u32,
    pub(crate) final_dispatch_queued: u32,
    pub(crate) final_hot: u64,
    pub(crate) final_tombstones: u64,
    pub(crate) peak_active: u32,
    pub(crate) peak_hot: u64,
    pub(crate) active_snapshot_logical_bytes: u64,
    pub(crate) hot_snapshot_logical_bytes: u64,
    pub(crate) tombstone_snapshot_logical_bytes: u64,
    pub(crate) observation_logical_bytes: u64,
}

pub(crate) fn peer_storage_turnover(executions: u32) -> EvidenceResult<PeerTurnoverEvidence> {
    if executions == 0 {
        return Err(std::io::Error::other("peer turnover requires an execution").into());
    }
    verify_admission_rejection_dimensions()?;
    verify_admission_capacity_dimensions()?;
    let directory = tempfile::tempdir()?;
    let store = RedbStore::open(directory.path())?;
    let owner = PeerId::new("peer-operational-evidence")?;
    let target = PeerId::new("peer-operational-target")?;
    let descriptor = descriptor()?;
    let catalog = CatalogSnapshot::new(1, BASE_TIME - 1, BASE_TIME + 60_000, Vec::new())?;
    configure_store(&store, &owner, &catalog.digest)?;
    let mut observation_count = 0_u64;
    let mut peak_active = 0_u32;
    let mut peak_hot = 0_u64;
    let mut active_snapshot_logical_bytes = 0_u64;
    let mut hot_snapshot_logical_bytes = 0_u64;
    let mut tombstone_snapshot_logical_bytes = 0_u64;
    let mut observation_logical_bytes = 0_u64;

    for index in 0..executions {
        let request = request(&owner, &target, &descriptor, catalog.digest.clone(), index)?;
        let execution = PeerExecutionId::new(format!("peer-execution-{index:04}"))?;
        let decision = allowed_decision(&owner, index)?;
        let admission = PeerAdmission {
            owner_peer: &owner,
            request: &request,
            authority: &decision,
            execution: &execution,
            relationship_generation: 1,
            accepted_at_unix_ms: BASE_TIME + u64::from(index),
            maximum_global_active: 1,
            maximum_dispatch_queue: 1,
            maximum_hot_terminal_records: 4,
            archive_batch_size: 1,
            archive_terminal_before_or_at_unix_ms: 1,
        };
        if !matches!(
            store.admit_peer_execution(&admission)?,
            PeerAdmissionOutcome::Accepted(_)
        ) {
            return Err(std::io::Error::other("fresh peer execution was not accepted").into());
        }
        let active = store
            .peer_execution_by_request(&owner, &request.request_id)?
            .ok_or_else(|| std::io::Error::other("active peer lookup was absent"))?;
        if !matches!(active, PeerExecutionSnapshot::Hot(_)) {
            return Err(std::io::Error::other("active peer lookup was not exact").into());
        }
        active_snapshot_logical_bytes =
            active_snapshot_logical_bytes.saturating_add(snapshot_logical_bytes(&active)?);
        let active_status = store.peer_execution_status()?;
        peak_active = peak_active.max(active_status.active);
        let worker = WorkerId::new(format!("peer-worker-{index:04}"))?;
        let claimed = store.claim_peer_dispatch(&PeerDispatchClaimRequest {
            worker: &worker,
            claimed_at_unix_ms: BASE_TIME + 100 + u64::from(index),
            lease_expires_at_unix_ms: BASE_TIME + 30_000,
        })?;
        let PeerClaimOutcome::Claimed(claimed) = claimed else {
            return Err(std::io::Error::other("peer dispatch was not claimed").into());
        };
        let claim_generation = claimed
            .phase
            .claim()
            .ok_or_else(|| std::io::Error::other("claimed peer record lacks its claim"))?
            .generation;
        if !matches!(
            store.mark_peer_entered(&PeerEntryRequest {
                owner: &owner,
                execution: &execution,
                worker: &worker,
                claim_generation,
                relationship_generation: 1,
                entered_at_unix_ms: BASE_TIME + 200 + u64::from(index),
                authority: &decision,
            })?,
            PeerEntryOutcome::Entered(_)
        ) {
            return Err(std::io::Error::other("peer entry was not committed").into());
        }
        for sequence in 1..=PROGRESS_PER_EXECUTION {
            let observation = progress_observation(&request, &execution, sequence)?;
            observation_logical_bytes = observation_logical_bytes
                .saturating_add(u64::try_from(serde_json::to_vec(&observation)?.len())?);
            store.append_peer_observation(&owner, &execution, &observation)?;
            observation_count = observation_count.saturating_add(1);
        }
        let terminal_sequence = PROGRESS_PER_EXECUTION + 1;
        let terminal = terminal_observation(&request, &execution, terminal_sequence)?;
        observation_logical_bytes = observation_logical_bytes
            .saturating_add(u64::try_from(serde_json::to_vec(&terminal)?.len())?);
        store.append_peer_observation(&owner, &execution, &terminal)?;
        observation_count = observation_count.saturating_add(1);

        let hot = store
            .peer_execution(&owner, &execution)?
            .ok_or_else(|| std::io::Error::other("terminal peer lookup was absent"))?;
        if !matches!(hot, PeerExecutionSnapshot::Hot(_)) {
            return Err(std::io::Error::other("terminal peer lookup was not hot").into());
        }
        hot_snapshot_logical_bytes =
            hot_snapshot_logical_bytes.saturating_add(snapshot_logical_bytes(&hot)?);
        let hot_status = store.peer_execution_status()?;
        peak_hot = peak_hot.max(hot_status.hot_terminal);

        let first = store.peer_observations(&owner, &execution, 0, PageSize::new(8)?)?;
        let second = store.peer_observations(&owner, &execution, 8, PageSize::new(8)?)?;
        let third = store.peer_observations(&owner, &execution, 16, PageSize::new(8)?)?;
        if first.observations.len() != 8
            || second.observations.len() != 8
            || third.observations.len() != 1
            || third.observations[0].sequence != terminal_sequence
        {
            return Err(std::io::Error::other("peer observation paging/resume changed").into());
        }

        let archived = store.archive_peer_executions(&PeerRetentionRequest {
            terminal_before_or_at: TimestampMillis::new(BASE_TIME + 10_000),
            archived_at: TimestampMillis::new(BASE_TIME + 20_000 + u64::from(index)),
            limit: PageSize::new(1)?,
        })?;
        if archived.archived != 1
            || !matches!(
                store.admit_peer_execution(&admission)?,
                PeerAdmissionOutcome::Replayed(_)
            )
        {
            return Err(std::io::Error::other("peer tombstone replay changed").into());
        }
        let tombstone = store
            .peer_execution(&owner, &execution)?
            .ok_or_else(|| std::io::Error::other("peer tombstone lookup was absent"))?;
        if !matches!(tombstone, PeerExecutionSnapshot::Archived(_)) {
            return Err(std::io::Error::other("peer tombstone replay changed").into());
        }
        tombstone_snapshot_logical_bytes =
            tombstone_snapshot_logical_bytes.saturating_add(snapshot_logical_bytes(&tombstone)?);
        let archived_page = store.peer_observations(&owner, &execution, 0, PageSize::new(8)?)?;
        if !matches!(archived_page.execution, PeerExecutionSnapshot::Archived(_))
            || !archived_page.observations.is_empty()
        {
            return Err(
                std::io::Error::other("archived observation history was not compact").into(),
            );
        }
    }
    store.verify_peer_execution_integrity()?;
    let status = store.peer_execution_status()?;
    if status.active != 0 || status.hot_terminal != 0 || status.tombstones != u64::from(executions)
    {
        return Err(std::io::Error::other("peer retention accounting changed").into());
    }
    Ok(PeerTurnoverEvidence {
        executions: u64::from(executions),
        observations: observation_count,
        final_active: status.active,
        final_dispatch_queued: status.dispatch_queued,
        final_hot: status.hot_terminal,
        final_tombstones: status.tombstones,
        peak_active,
        peak_hot,
        active_snapshot_logical_bytes,
        hot_snapshot_logical_bytes,
        tombstone_snapshot_logical_bytes,
        observation_logical_bytes,
    })
}

fn snapshot_logical_bytes(snapshot: &PeerExecutionSnapshot) -> EvidenceResult<u64> {
    let length = match snapshot {
        PeerExecutionSnapshot::Hot(record) => serde_json::to_vec(record.as_ref())?.len(),
        PeerExecutionSnapshot::Archived(tombstone) => serde_json::to_vec(tombstone.as_ref())?.len(),
    };
    Ok(u64::try_from(length)?)
}

fn verify_admission_rejection_dimensions() -> EvidenceResult {
    let future = BASE_TIME + 60_000;
    type AlterAdmission = fn(&mut PeerRelationshipState, &mut PeerCatalogState);
    let cases: [(AlterAdmission, PeerAdmissionRejection); 6] = [
        (
            |relationship, _| relationship.enabled = false,
            PeerAdmissionRejection::RelationshipUnavailable,
        ),
        (
            |relationship, catalog| {
                relationship.generation = 2;
                catalog.relationship_generation = 2;
            },
            PeerAdmissionRejection::RelationshipUnavailable,
        ),
        (
            |relationship, _| relationship.expires_at_unix_ms = BASE_TIME - 1,
            PeerAdmissionRejection::RelationshipUnavailable,
        ),
        (
            |_, catalog| catalog.generation = 2,
            PeerAdmissionRejection::CatalogUnavailable,
        ),
        (
            |_, catalog| catalog.digest = format!("b3_{}", "f".repeat(64)),
            PeerAdmissionRejection::CatalogUnavailable,
        ),
        (
            |_, catalog| catalog.expires_at_unix_ms = BASE_TIME - 1,
            PeerAdmissionRejection::CatalogUnavailable,
        ),
    ];
    for (alter, expected) in cases {
        let directory = tempfile::tempdir()?;
        let store = RedbStore::open(directory.path())?;
        let owner = PeerId::new("peer-admission-matrix")?;
        let target = PeerId::new("peer-admission-target")?;
        let descriptor = descriptor()?;
        let catalog = CatalogSnapshot::new(1, BASE_TIME - 1, future, Vec::new())?;
        let request = request(&owner, &target, &descriptor, catalog.digest.clone(), 0)?;
        let decision = allowed_decision(&owner, 0)?;
        store.set_peer_admission_open(true)?;
        let mut relationship = PeerRelationshipState {
            peer: owner.clone(),
            generation: 1,
            enabled: true,
            expires_at_unix_ms: future,
            maximum_active: 4,
        };
        let mut catalog_state = PeerCatalogState {
            peer: owner.clone(),
            relationship_generation: 1,
            generation: 1,
            digest: catalog.digest.as_str().to_owned(),
            expires_at_unix_ms: future,
        };
        alter(&mut relationship, &mut catalog_state);
        store.configure_peer_relationship(&relationship)?;
        if relationship.enabled {
            store.publish_peer_catalog(&catalog_state)?;
        }
        let execution = PeerExecutionId::new("peer-admission-matrix-execution")?;
        let outcome = store.admit_peer_execution(&PeerAdmission {
            owner_peer: &owner,
            request: &request,
            authority: &decision,
            execution: &execution,
            relationship_generation: 1,
            accepted_at_unix_ms: BASE_TIME,
            maximum_global_active: 4,
            maximum_dispatch_queue: 4,
            maximum_hot_terminal_records: 4,
            archive_batch_size: 1,
            archive_terminal_before_or_at_unix_ms: 1,
        })?;
        if !matches!(outcome, PeerAdmissionOutcome::Rejected(actual) if actual == expected) {
            return Err(std::io::Error::other("peer admission rejection dimension widened").into());
        }
    }
    Ok(())
}

fn verify_admission_capacity_dimensions() -> EvidenceResult {
    for (relationship_maximum, global_maximum, dispatch_maximum, expected) in [
        (1, 4, 4, PeerAdmissionRejection::PeerCapacity),
        (2, 1, 4, PeerAdmissionRejection::GlobalCapacity),
        (2, 2, 1, PeerAdmissionRejection::DispatchCapacity),
    ] {
        let directory = tempfile::tempdir()?;
        let store = RedbStore::open(directory.path())?;
        let owner = PeerId::new("peer-capacity-matrix")?;
        let target = PeerId::new("peer-capacity-target")?;
        let descriptor = descriptor()?;
        let catalog = CatalogSnapshot::new(1, BASE_TIME - 1, BASE_TIME + 60_000, Vec::new())?;
        store.set_peer_admission_open(true)?;
        store.configure_peer_relationship(&PeerRelationshipState {
            peer: owner.clone(),
            generation: 1,
            enabled: true,
            expires_at_unix_ms: BASE_TIME + 60_000,
            maximum_active: relationship_maximum,
        })?;
        store.publish_peer_catalog(&PeerCatalogState {
            peer: owner.clone(),
            relationship_generation: 1,
            generation: 1,
            digest: catalog.digest.as_str().to_owned(),
            expires_at_unix_ms: BASE_TIME + 60_000,
        })?;
        for index in 0..2 {
            let request = request(&owner, &target, &descriptor, catalog.digest.clone(), index)?;
            let decision = allowed_decision(&owner, index)?;
            let execution = PeerExecutionId::new(format!("peer-capacity-execution-{index}"))?;
            let outcome = store.admit_peer_execution(&PeerAdmission {
                owner_peer: &owner,
                request: &request,
                authority: &decision,
                execution: &execution,
                relationship_generation: 1,
                accepted_at_unix_ms: BASE_TIME + u64::from(index),
                maximum_global_active: global_maximum,
                maximum_dispatch_queue: dispatch_maximum,
                maximum_hot_terminal_records: 4,
                archive_batch_size: 1,
                archive_terminal_before_or_at_unix_ms: 1,
            })?;
            if index == 0 && !matches!(outcome, PeerAdmissionOutcome::Accepted(_)) {
                return Err(
                    std::io::Error::other("capacity fixture did not accept first slot").into(),
                );
            }
            if index == 1
                && !matches!(outcome, PeerAdmissionOutcome::Rejected(actual) if actual == expected)
            {
                return Err(
                    std::io::Error::other("peer admission capacity dimension widened").into(),
                );
            }
        }
    }
    Ok(())
}

fn descriptor() -> EvidenceResult<CapabilityDescriptor> {
    let schema = || {
        Ok::<_, crate::EvidenceError>(SchemaContract::new(
            SchemaId::new("evidence.value")?,
            1,
            BoundedJson::new(serde_json::json!({"type": "object"}))?,
        )?)
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
        CapabilityId::new("evidence-peer-capability")?,
        1,
        CapabilityCategory::Tool,
        AdmissionConstraints::new(4, 0)?,
        Locality::Local,
    )
    .operations(BTreeMap::from([(
        OperationId::new("evidence.execute")?,
        operation,
    )]))
    .build()?)
}

fn configure_store(store: &RedbStore, peer: &PeerId, digest: &CatalogDigest) -> EvidenceResult {
    store.set_peer_admission_open(true)?;
    store.configure_peer_relationship(&PeerRelationshipState {
        peer: peer.clone(),
        generation: 1,
        enabled: true,
        expires_at_unix_ms: BASE_TIME + 60_000,
        maximum_active: 1,
    })?;
    store.publish_peer_catalog(&PeerCatalogState {
        peer: peer.clone(),
        relationship_generation: 1,
        generation: 1,
        digest: digest.as_str().to_owned(),
        expires_at_unix_ms: BASE_TIME + 60_000,
    })?;
    Ok(())
}

fn allowed_decision(peer: &PeerId, index: u32) -> EvidenceResult<AuthorityDecisionSnapshot> {
    let mut resources = RequestedResourceFacts::empty();
    resources.peer = Some(peer.clone());
    resources.capability = Some(CapabilityId::new("evidence-peer-capability")?);
    resources.capability_operation = Some(OperationId::new("evidence.execute")?);
    let request = AuthorityRequest {
        decision: DecisionId::new(format!("decision:peer-evidence-{index}"))?,
        actor: ActorRef::new(format!("peer:{}", peer.as_str()))?,
        grant: GrantId::new(format!("grant:{}", peer.as_str()))?,
        grant_revision: 1,
        grant_digest: GrantDigest::new(format!("b3_{}", "0".repeat(64)))?,
        revocation_generation: 0,
        operation: AuthorityOperation::InvokePeerCapability,
        resources,
        budget: AuthorityBudget::default(),
        evaluated_at: BoundaryTimeMillis::new(BASE_TIME + u64::from(index)),
        provenance: AuthorityExecutionProvenance {
            revision: Some(serde_json::from_value::<RevisionId>(serde_json::json!(
                format!("rev_{}", "1".repeat(64))
            ))?),
            node: Some(NodeId::new("node-peer-evidence")?),
            execution: Some(format!("execution-peer-evidence-{index}")),
            attempt: Some(format!("attempt-peer-evidence-{index}")),
            descriptor_revision: Some(1),
            peer: Some(peer.clone()),
            idempotency: Some(IdempotencyBehavior::CapabilityScoped),
        },
    };
    Ok(AuthorityDecisionSnapshot::from_evaluation(
        PolicyId::new("policy:peer-operational-evidence")?,
        1,
        request,
        vec![DecisionReasonCode::Allowed],
        AuthorityBudget::default(),
        SideEffectClass::ReadOnly,
    )?)
}

fn request(
    issuer: &PeerId,
    target: &PeerId,
    descriptor: &CapabilityDescriptor,
    catalog_digest: CatalogDigest,
    index: u32,
) -> EvidenceResult<PeerInvocationRequest> {
    request_with_observation_limit(issuer, target, descriptor, catalog_digest, index, 100)
}

fn request_with_observation_limit(
    issuer: &PeerId,
    target: &PeerId,
    descriptor: &CapabilityDescriptor,
    catalog_digest: CatalogDigest,
    index: u32,
    observation_limit: u32,
) -> EvidenceResult<PeerInvocationRequest> {
    let operation = OperationId::new("evidence.execute")?;
    let selection = ResolvedCapabilitySnapshot::from_descriptor(descriptor, &operation)?;
    let invocation = InvocationRequest::new(
        InvocationId::new(format!("peer-invocation-{index:04}"))?,
        descriptor.identity().clone(),
        operation.clone(),
        None,
        None,
        Vec::new(),
        BTreeMap::new(),
    )?;
    let request_id = PeerRequestId::new(format!("peer-request-{index:04}"))?;
    let deadline = BASE_TIME + 120_000;
    let limits = ExecutionLimits {
        artifact_bytes: 1_048_576,
        duration_ms: 30_000,
        cost_micros: 0,
        observations: observation_limit,
    };
    Ok(PeerInvocationRequest::new(
        request_id.clone(),
        1,
        catalog_digest,
        selection,
        invocation,
        limits,
        deadline,
        DelegatedAuthorization {
            reference: DelegationRef::new("delegation-operational-evidence")?,
            issuer_peer: issuer.clone(),
            actor: ActorRef::new(format!("peer:{}", issuer.as_str()))?,
            target_peer: target.clone(),
            capability: descriptor.identity().clone(),
            operation,
            request: request_id,
            limits,
            expires_at_unix_ms: deadline,
            nonce: format!("peer-nonce-{index:04}"),
            provenance: milkdrift_peer_protocol::PeerExecutionProvenance {
                run: "run-peer-operational-evidence".to_owned(),
                revision: format!("rev_{}", "1".repeat(64)),
                node: "node-peer-evidence".to_owned(),
                execution: format!("execution-peer-evidence-{index}"),
                attempt: format!("attempt-peer-evidence-{index}"),
            },
        },
    )?)
}

fn progress_observation(
    request: &PeerInvocationRequest,
    execution: &PeerExecutionId,
    sequence: u64,
) -> EvidenceResult<PeerObservation> {
    Ok(PeerObservation {
        execution: execution.clone(),
        sequence,
        category: ObservationCategory::Progress,
        event: InvocationEvent::new(
            request.request.invocation().clone(),
            sequence,
            InvocationEventKind::Progress {
                message: format!("bounded evidence progress {sequence}"),
                completed_units: Some(sequence),
                total_units: Some(PROGRESS_PER_EXECUTION),
            },
        )?,
        observed_at_unix_ms: BASE_TIME + sequence,
    })
}

fn terminal_observation(
    request: &PeerInvocationRequest,
    execution: &PeerExecutionId,
    sequence: u64,
) -> EvidenceResult<PeerObservation> {
    Ok(PeerObservation {
        execution: execution.clone(),
        sequence,
        category: ObservationCategory::Terminal,
        event: InvocationEvent::new(
            request.request.invocation().clone(),
            sequence,
            InvocationEventKind::Terminal {
                terminal: InvocationTerminal::new(
                    TerminalStatus::Success,
                    Vec::new(),
                    None,
                    None,
                    SideEffectClass::ReadOnly,
                )?,
            },
        )?,
        observed_at_unix_ms: BASE_TIME + sequence,
    })
}

#[cfg(test)]
mod tests;
