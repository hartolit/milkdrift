use super::*;
use milkdrift_peer_protocol::{
    CancellationDisposition, PeerCancellationAcknowledgement, PeerCancellationRequest,
};

struct Fixture {
    _directory: tempfile::TempDir,
    store: RedbStore,
    owner: PeerId,
    target: PeerId,
    descriptor: CapabilityDescriptor,
    catalog: CatalogSnapshot,
}

impl Fixture {
    fn new(relationship_expiry: u64, catalog_expiry: u64) -> EvidenceResult<Self> {
        let directory = tempfile::tempdir()?;
        let store = RedbStore::open(directory.path())?;
        let owner = PeerId::new("peer-mutation-evidence")?;
        let target = PeerId::new("peer-mutation-target")?;
        let descriptor = descriptor()?;
        let catalog = CatalogSnapshot::new(1, BASE_TIME - 1, catalog_expiry, Vec::new())?;
        store.set_peer_admission_open(true)?;
        store.configure_peer_relationship(&PeerRelationshipState {
            peer: owner.clone(),
            generation: 1,
            enabled: true,
            expires_at_unix_ms: relationship_expiry,
            maximum_active: 4,
        })?;
        store.publish_peer_catalog(&PeerCatalogState {
            peer: owner.clone(),
            relationship_generation: 1,
            generation: 1,
            digest: catalog.digest.as_str().to_owned(),
            expires_at_unix_ms: catalog_expiry,
        })?;
        Ok(Self {
            _directory: directory,
            store,
            owner,
            target,
            descriptor,
            catalog,
        })
    }

    fn request(&self, index: u32, observations: u32) -> EvidenceResult<PeerInvocationRequest> {
        request_with_observation_limit(
            &self.owner,
            &self.target,
            &self.descriptor,
            self.catalog.digest.clone(),
            index,
            observations,
        )
    }
}

fn admission<'a>(
    fixture: &'a Fixture,
    request: &'a PeerInvocationRequest,
    authority: &'a AuthorityDecisionSnapshot,
    execution: &'a PeerExecutionId,
    accepted_at_unix_ms: u64,
    maximum_hot_terminal_records: u64,
) -> PeerAdmission<'a> {
    PeerAdmission {
        owner_peer: &fixture.owner,
        request,
        authority,
        execution,
        relationship_generation: 1,
        accepted_at_unix_ms,
        maximum_global_active: 4,
        maximum_dispatch_queue: 4,
        maximum_hot_terminal_records,
        archive_batch_size: 1,
        archive_terminal_before_or_at_unix_ms: BASE_TIME + 10_000,
    }
}

fn decision_from_request(
    request: AuthorityRequest,
    allowed: bool,
) -> EvidenceResult<AuthorityDecisionSnapshot> {
    Ok(AuthorityDecisionSnapshot::from_evaluation(
        PolicyId::new("policy:peer-mutation-evidence")?,
        1,
        request,
        vec![if allowed {
            DecisionReasonCode::Allowed
        } else {
            DecisionReasonCode::WrongActor
        }],
        AuthorityBudget::default(),
        SideEffectClass::ReadOnly,
    )?)
}

fn assert_admission_invalid(fixture: &Fixture, admission: &PeerAdmission<'_>) -> EvidenceResult {
    if fixture.store.admit_peer_execution(admission).is_ok() {
        return Err(std::io::Error::other("invalid peer admission was accepted").into());
    }
    Ok(())
}

fn claim(
    fixture: &Fixture,
    index: u32,
    claimed_at: u64,
    expires_at: u64,
) -> EvidenceResult<(WorkerId, u64)> {
    let worker = WorkerId::new(format!("peer-mutation-worker-{index}"))?;
    let PeerClaimOutcome::Claimed(record) =
        fixture
            .store
            .claim_peer_dispatch(&PeerDispatchClaimRequest {
                worker: &worker,
                claimed_at_unix_ms: claimed_at,
                lease_expires_at_unix_ms: expires_at,
            })?
    else {
        return Err(std::io::Error::other("peer mutation fixture was not claimed").into());
    };
    let generation = record
        .phase
        .claim()
        .ok_or_else(|| std::io::Error::other("claimed fixture lacks claim"))?
        .generation;
    Ok((worker, generation))
}

#[test]
fn admission_contract_rejects_each_independent_authority_and_bound_mismatch() -> EvidenceResult {
    let fixture = Fixture::new(BASE_TIME + 60_000, BASE_TIME + 60_000)?;
    let request = fixture.request(100, 4)?;
    let execution = PeerExecutionId::new("peer-admission-contract")?;
    let allowed = allowed_decision(&fixture.owner, 100)?;

    let mut denied_request = allowed.request().clone();
    denied_request.decision = DecisionId::new("decision:peer-denied")?;
    let denied = decision_from_request(denied_request, false)?;
    assert_admission_invalid(
        &fixture,
        &admission(&fixture, &request, &denied, &execution, BASE_TIME, 4),
    )?;

    for case in 0..11 {
        let mut authority_request = allowed.request().clone();
        authority_request.decision = DecisionId::new(format!("decision:peer-mismatch-{case}"))?;
        match case {
            0 => authority_request.operation = AuthorityOperation::InspectPeer,
            1 => authority_request.actor = ActorRef::new("peer:other-actor")?,
            2 => authority_request.resources.peer = Some(PeerId::new("peer-other")?),
            3 => {
                authority_request.resources.capability =
                    Some(CapabilityId::new("other-capability")?);
            }
            4 => {
                authority_request.resources.capability_operation =
                    Some(OperationId::new("other.operation")?);
            }
            5 => authority_request.provenance.revision = None,
            6 => authority_request.provenance.node = Some(NodeId::new("other-node")?),
            7 => {
                authority_request.provenance.execution = Some("other-execution".to_owned());
            }
            8 => authority_request.provenance.attempt = Some("other-attempt".to_owned()),
            9 => authority_request.provenance.descriptor_revision = Some(2),
            10 => authority_request.provenance.attempt = None,
            _ => return Err(std::io::Error::other("invalid authority test case").into()),
        }
        let mismatched = decision_from_request(authority_request, true)?;
        assert_admission_invalid(
            &fixture,
            &admission(&fixture, &request, &mismatched, &execution, BASE_TIME, 4),
        )?;
    }

    for case in 0..7 {
        let mut invalid = admission(&fixture, &request, &allowed, &execution, BASE_TIME, 4);
        match case {
            0 => invalid.relationship_generation = 0,
            1 => invalid.accepted_at_unix_ms = 0,
            2 => invalid.maximum_global_active = 0,
            3 => invalid.maximum_dispatch_queue = 0,
            4 => invalid.maximum_hot_terminal_records = 0,
            5 => invalid.archive_batch_size = 0,
            6 => invalid.archive_terminal_before_or_at_unix_ms = 0,
            _ => return Err(std::io::Error::other("invalid admission test case").into()),
        }
        assert_admission_invalid(&fixture, &invalid)?;
    }
    let invalid_hot = admission(&fixture, &request, &allowed, &execution, BASE_TIME, 3);
    assert_admission_invalid(&fixture, &invalid_hot)?;
    Ok(())
}

#[test]
fn admission_expiry_boundaries_and_retention_reclamation_are_exact() -> EvidenceResult {
    let exact = Fixture::new(BASE_TIME, BASE_TIME)?;
    let exact_request = exact.request(110, 2)?;
    let exact_decision = allowed_decision(&exact.owner, 110)?;
    let exact_execution = PeerExecutionId::new("peer-exact-expiry")?;
    if !matches!(
        exact.store.admit_peer_execution(&admission(
            &exact,
            &exact_request,
            &exact_decision,
            &exact_execution,
            BASE_TIME,
            4,
        ))?,
        PeerAdmissionOutcome::Accepted(_)
    ) {
        return Err(std::io::Error::other("exact peer expiry boundary was rejected").into());
    }

    let fixture = Fixture::new(BASE_TIME + 60_000, BASE_TIME + 60_000)?;
    let first_request = fixture.request(111, 1)?;
    let first_decision = allowed_decision(&fixture.owner, 111)?;
    let first_execution = PeerExecutionId::new("peer-retention-first")?;
    let mut first = admission(
        &fixture,
        &first_request,
        &first_decision,
        &first_execution,
        BASE_TIME,
        4,
    );
    first.maximum_global_active = 1;
    first.maximum_dispatch_queue = 1;
    first.maximum_hot_terminal_records = 1;
    fixture.store.admit_peer_execution(&first)?;
    let (worker, generation) = claim(&fixture, 111, BASE_TIME + 1, BASE_TIME + 100)?;
    fixture.store.mark_peer_entered(&PeerEntryRequest {
        owner: &fixture.owner,
        execution: &first_execution,
        worker: &worker,
        claim_generation: generation,
        relationship_generation: 1,
        entered_at_unix_ms: BASE_TIME + 2,
        authority: &first_decision,
    })?;
    fixture.store.append_peer_observation(
        &fixture.owner,
        &first_execution,
        &terminal_observation(&first_request, &first_execution, 1)?,
    )?;

    let second_request = fixture.request(112, 1)?;
    let second_decision = allowed_decision(&fixture.owner, 112)?;
    let second_execution = PeerExecutionId::new("peer-retention-second")?;
    let mut second = admission(
        &fixture,
        &second_request,
        &second_decision,
        &second_execution,
        BASE_TIME + 10,
        4,
    );
    second.maximum_global_active = 1;
    second.maximum_dispatch_queue = 1;
    second.maximum_hot_terminal_records = 1;
    second.archive_terminal_before_or_at_unix_ms = BASE_TIME + 10;
    if !matches!(
        fixture.store.admit_peer_execution(&second)?,
        PeerAdmissionOutcome::Accepted(_)
    ) {
        return Err(std::io::Error::other("retention reclamation did not admit new work").into());
    }
    let status = fixture.store.peer_execution_status()?;
    assert_eq!(status.active, 1);
    assert_eq!(status.hot_terminal, 0);
    assert_eq!(status.tombstones, 1);
    Ok(())
}

#[test]
fn claim_entry_release_and_uncertainty_boundaries_are_independent() -> EvidenceResult {
    let fixture = Fixture::new(BASE_TIME + 200, BASE_TIME + 60_000)?;
    let request = fixture.request(120, 4)?;
    let decision = allowed_decision(&fixture.owner, 120)?;
    let execution = PeerExecutionId::new("peer-claim-entry")?;
    fixture.store.admit_peer_execution(&admission(
        &fixture, &request, &decision, &execution, BASE_TIME, 4,
    ))?;
    let worker = WorkerId::new("peer-mutation-worker-120")?;
    assert!(
        fixture
            .store
            .claim_peer_dispatch(&PeerDispatchClaimRequest {
                worker: &worker,
                claimed_at_unix_ms: 0,
                lease_expires_at_unix_ms: 1,
            })
            .is_err()
    );
    assert!(
        fixture
            .store
            .claim_peer_dispatch(&PeerDispatchClaimRequest {
                worker: &worker,
                claimed_at_unix_ms: 1,
                lease_expires_at_unix_ms: 1,
            })
            .is_err()
    );
    let (worker, first_generation) = claim(&fixture, 120, BASE_TIME + 1, BASE_TIME + 100)?;
    fixture.store.release_peer_claim(
        &fixture.owner,
        &execution,
        &worker,
        first_generation,
        BASE_TIME + 2,
    )?;
    let (worker, second_generation) = claim(&fixture, 121, BASE_TIME + 3, BASE_TIME + 100)?;
    if !matches!(
        fixture.store.mark_peer_entered(&PeerEntryRequest {
            owner: &fixture.owner,
            execution: &execution,
            worker: &worker,
            claim_generation: second_generation,
            relationship_generation: 1,
            entered_at_unix_ms: BASE_TIME + 200,
            authority: &decision,
        })?,
        PeerEntryOutcome::Entered(_)
    ) {
        return Err(std::io::Error::other("exact entry expiry boundary was rejected").into());
    }
    assert!(
        fixture
            .store
            .release_peer_claim(
                &fixture.owner,
                &execution,
                &worker,
                second_generation,
                BASE_TIME + 4,
            )
            .is_err()
    );
    assert!(
        fixture
            .store
            .mark_peer_uncertain(
                &fixture.owner,
                &execution,
                &worker,
                second_generation,
                0,
                "uncertain",
            )
            .is_err()
    );
    assert!(
        fixture
            .store
            .mark_peer_uncertain(
                &fixture.owner,
                &execution,
                &worker,
                second_generation,
                BASE_TIME + 201,
                "",
            )
            .is_err()
    );
    assert!(
        fixture
            .store
            .mark_peer_uncertain(
                &fixture.owner,
                &execution,
                &worker,
                second_generation,
                BASE_TIME + 201,
                &"x".repeat(2_049),
            )
            .is_err()
    );
    fixture.store.mark_peer_uncertain(
        &fixture.owner,
        &execution,
        &worker,
        second_generation,
        BASE_TIME + 201,
        &"x".repeat(2_048),
    )?;
    Ok(())
}

#[test]
fn entry_relationship_checks_are_not_conflated() -> EvidenceResult {
    for (disabled, request_generation) in [(false, 1), (true, 2)] {
        let fixture = Fixture::new(BASE_TIME + 60_000, BASE_TIME + 60_000)?;
        let request = fixture.request(130 + u32::from(disabled), 2)?;
        let decision = allowed_decision(&fixture.owner, 130 + u32::from(disabled))?;
        let execution = PeerExecutionId::new(format!("peer-entry-relationship-{disabled}"))?;
        fixture.store.admit_peer_execution(&admission(
            &fixture, &request, &decision, &execution, BASE_TIME, 4,
        ))?;
        let (worker, generation) = claim(&fixture, 130, BASE_TIME + 1, BASE_TIME + 100)?;
        fixture
            .store
            .configure_peer_relationship(&PeerRelationshipState {
                peer: fixture.owner.clone(),
                generation: 2,
                enabled: !disabled,
                expires_at_unix_ms: BASE_TIME + 60_000,
                maximum_active: 4,
            })?;
        assert!(matches!(
            fixture.store.mark_peer_entered(&PeerEntryRequest {
                owner: &fixture.owner,
                execution: &execution,
                worker: &worker,
                claim_generation: generation,
                relationship_generation: request_generation,
                entered_at_unix_ms: BASE_TIME + 2,
                authority: &decision,
            })?,
            PeerEntryOutcome::RelationshipUnavailable
        ));
    }
    Ok(())
}

#[test]
fn observation_quota_cancellation_replay_and_recovery_are_exact() -> EvidenceResult {
    let fixture = Fixture::new(BASE_TIME + 60_000, BASE_TIME + 60_000)?;
    let request = fixture.request(140, 2)?;
    let decision = allowed_decision(&fixture.owner, 140)?;
    let execution = PeerExecutionId::new("peer-observation-bounds")?;
    fixture.store.admit_peer_execution(&admission(
        &fixture, &request, &decision, &execution, BASE_TIME, 4,
    ))?;
    let (worker, generation) = claim(&fixture, 140, BASE_TIME + 1, BASE_TIME + 100)?;
    fixture.store.mark_peer_entered(&PeerEntryRequest {
        owner: &fixture.owner,
        execution: &execution,
        worker: &worker,
        claim_generation: generation,
        relationship_generation: 1,
        entered_at_unix_ms: BASE_TIME + 2,
        authority: &decision,
    })?;
    fixture.store.append_peer_observation(
        &fixture.owner,
        &execution,
        &progress_observation(&request, &execution, 1)?,
    )?;
    assert!(
        fixture
            .store
            .append_peer_observation(
                &fixture.owner,
                &execution,
                &progress_observation(&request, &execution, 2)?,
            )
            .is_err()
    );
    fixture.store.append_peer_observation(
        &fixture.owner,
        &execution,
        &terminal_observation(&request, &execution, 2)?,
    )?;

    let cancellation_request = fixture.request(141, 2)?;
    let cancellation_decision = allowed_decision(&fixture.owner, 141)?;
    let cancellation_execution = PeerExecutionId::new("peer-cancellation-bounds")?;
    fixture.store.admit_peer_execution(&admission(
        &fixture,
        &cancellation_request,
        &cancellation_decision,
        &cancellation_execution,
        BASE_TIME + 10,
        4,
    ))?;
    let cancellation_request_id = PeerRequestId::new("peer-cancellation-request")?;
    let cancellation = |sequence, reason: String| PeerCancellationRequest {
        request_id: cancellation_request_id.clone(),
        execution: cancellation_execution.clone(),
        sequence,
        reason,
    };
    for (value, at) in [
        (cancellation(0, "cancel".to_owned()), BASE_TIME + 11),
        (cancellation(1, String::new()), BASE_TIME + 11),
        (cancellation(1, "x".repeat(513)), BASE_TIME + 11),
        (cancellation(1, "cancel".to_owned()), 0),
    ] {
        assert!(
            fixture
                .store
                .request_peer_cancellation(&fixture.owner, &value, at)
                .is_err()
        );
    }
    let exact = cancellation(1, "x".repeat(512));
    fixture
        .store
        .request_peer_cancellation(&fixture.owner, &exact, BASE_TIME + 11)?;
    fixture
        .store
        .request_peer_cancellation(&fixture.owner, &exact, BASE_TIME + 12)?;
    let conflict = cancellation(1, "different".to_owned());
    assert!(
        fixture
            .store
            .request_peer_cancellation(&fixture.owner, &conflict, BASE_TIME + 12)
            .is_err()
    );
    let cancellation_worker = WorkerId::new("peer-cancellation-worker")?;
    let claimed = fixture
        .store
        .claim_peer_dispatch(&PeerDispatchClaimRequest {
            worker: &cancellation_worker,
            claimed_at_unix_ms: BASE_TIME + 13,
            lease_expires_at_unix_ms: BASE_TIME + 100,
        })?;
    assert!(matches!(
        claimed,
        PeerClaimOutcome::CancellationRequested(_)
    ));
    let recovered = fixture
        .store
        .recover_peer_claims(BASE_TIME + 14, PageSize::new(1)?)?;
    assert_eq!(recovered.requeued, 1);
    assert!(!recovered.more);
    let empty = fixture
        .store
        .recover_peer_claims(BASE_TIME + 15, PageSize::new(1)?)?;
    assert_eq!(empty.requeued, 0);
    assert_eq!(empty.uncertain, 0);
    assert!(!empty.more);
    fixture.store.append_peer_observation(
        &fixture.owner,
        &cancellation_execution,
        &terminal_observation(&cancellation_request, &cancellation_execution, 1)?,
    )?;

    let acknowledgement = PeerCancellationAcknowledgement {
        request_id: exact.request_id.clone(),
        execution: cancellation_execution.clone(),
        disposition: CancellationDisposition::Accepted,
        terminal_boundary: false,
        terminal_evidence: None,
        detail: Some("accepted".to_owned()),
    };
    assert!(
        fixture
            .store
            .acknowledge_peer_cancellation(&fixture.owner, &acknowledgement, 0)
            .is_err()
    );
    let mut wrong_request = acknowledgement.clone();
    wrong_request.request_id = PeerRequestId::new("peer-cancellation-other")?;
    assert!(
        fixture
            .store
            .acknowledge_peer_cancellation(&fixture.owner, &wrong_request, BASE_TIME + 16,)
            .is_err()
    );
    fixture.store.acknowledge_peer_cancellation(
        &fixture.owner,
        &acknowledgement,
        BASE_TIME + 16,
    )?;
    fixture.store.acknowledge_peer_cancellation(
        &fixture.owner,
        &acknowledgement,
        BASE_TIME + 17,
    )?;
    let mut conflicting_acknowledgement = acknowledgement;
    conflicting_acknowledgement.disposition = CancellationDisposition::TooLate;
    assert!(
        fixture
            .store
            .acknowledge_peer_cancellation(
                &fixture.owner,
                &conflicting_acknowledgement,
                BASE_TIME + 17,
            )
            .is_err()
    );
    Ok(())
}

#[test]
fn entered_claim_recovery_is_uncertain_and_then_empty() -> EvidenceResult {
    let fixture = Fixture::new(BASE_TIME + 60_000, BASE_TIME + 60_000)?;
    let request = fixture.request(150, 2)?;
    let decision = allowed_decision(&fixture.owner, 150)?;
    let execution = PeerExecutionId::new("peer-entered-recovery")?;
    fixture.store.admit_peer_execution(&admission(
        &fixture, &request, &decision, &execution, BASE_TIME, 4,
    ))?;
    let (worker, generation) = claim(&fixture, 150, BASE_TIME + 1, BASE_TIME + 100)?;
    fixture.store.mark_peer_entered(&PeerEntryRequest {
        owner: &fixture.owner,
        execution: &execution,
        worker: &worker,
        claim_generation: generation,
        relationship_generation: 1,
        entered_at_unix_ms: BASE_TIME + 2,
        authority: &decision,
    })?;
    let recovered = fixture
        .store
        .recover_peer_claims(BASE_TIME + 3, PageSize::new(1)?)?;
    assert_eq!(recovered.requeued, 0);
    assert_eq!(recovered.uncertain, 1);
    assert!(!recovered.more);
    let empty = fixture
        .store
        .recover_peer_claims(BASE_TIME + 4, PageSize::new(1)?)?;
    assert!(!empty.more);
    Ok(())
}
