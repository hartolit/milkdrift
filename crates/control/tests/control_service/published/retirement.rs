//! Retired definitions outlive their registry adapters and still replay after complete reopen.
use super::*;
use milkdrift_peer_protocol::{
    CatalogSnapshot, ClientInvocationAuthorization, DirectInvocationRequest, ExecutionLimits,
    ObservationCategory, PeerExecutionId, PeerObservation, PeerRequestId,
};
use milkdrift_persistence::{
    PeerAdmission, PeerAdmissionOutcome, PeerAdmissionRejection, PeerClaimOutcome,
    PeerDispatchClaimRequest, PeerExecutionStore, ServingCallerState, ServingCatalogState,
};

fn retire(
    fixture: &Fixture,
    generation: u64,
) -> TestResult<milkdrift_persistence::published::PublishedMethodRecord> {
    let evaluator = GrantSetEvaluator::new(
        PolicyId::new("test.publication")?,
        1,
        [publication_grant("human:caller", "grant:caller")?],
        BTreeMap::new(),
    )?;
    let mut request = fixture.decision.request().clone();
    request.resources.capability_operation = Some(OperationId::new("method.retire")?);
    Ok(fixture.published.retire(
        fixture.method.descriptor.identity(),
        generation,
        1,
        &evaluator.evaluate(&request)?,
        &milkdrift_persistence::IntegrityDigest::hash(format!("retire-{generation}").as_bytes()),
    )?)
}

fn generations(fixture: &Fixture) -> TestResult<Vec<milkdrift_capability_host::GenerationView>> {
    Ok(fixture
        .host
        .generations(
            &CapabilityAuthorityScope::allow_any(SideEffectClass::Unknown),
            NOW,
        )?
        .into_iter()
        .filter(|g| g.capability == *fixture.method.descriptor.identity())
        .collect())
}

#[test]
fn retired_generations_turn_over_beyond_registry_capacity_and_reopen() -> TestResult {
    let directory = TempDir::new()?;
    let mut accepted = Vec::new();
    for number in 1..=6 {
        let fixture = fixture(directory.path(), &format!("turnover-{number}"))?;
        let mut method = fixture.method.clone();
        let mut descriptor = serde_json::to_value(&method.descriptor)?;
        descriptor
            .as_object_mut()
            .ok_or("fixture must be an object")?
            .insert("descriptor_revision".to_owned(), serde_json::json!(number));
        method.descriptor = serde_json::from_value(descriptor)?;
        let request = milkdrift_persistence::IntegrityDigest::hash(&serde_json::to_vec(&method)?);
        let expected = (number > 1).then_some(2);
        let published =
            fixture
                .published
                .publish(method.clone(), expected, &fixture.decision, &request)?;
        let run = start_outer(&fixture, &format!("run:turnover-{number}"))?;
        runtime_tick(&fixture.runtime)?;
        let plan = fixture
            .store
            .published_local_page(None, PageSize::new(8)?)?
            .0
            .pop()
            .ok_or("pending plan absent")?;
        assert_eq!(plan.generation, number);
        let retired = retire(&fixture, number)?;
        fixture.published.maintain_retirement()?;
        assert_eq!(generations(&fixture)?.len(), 1);
        assert_eq!(
            generations(&fixture)?
                .first()
                .ok_or("retiring generation missing")?
                .pending_workflows,
            1
        );
        drop(fixture);

        let fixture = super::fixture(directory.path(), &format!("settle-{number}"))?;
        assert_eq!(generations(&fixture)?.len(), 1);
        assert!(
            generations(&fixture)?
                .first()
                .ok_or("retiring generation missing")?
                .draining
        );
        for _ in 0..64 {
            fixture.clock.advance(1)?;
            runtime_tick(&fixture.runtime)?;
            fixture.published.maintain_retirement()?;
            if fixture.runtime.projection(&run)?.lifecycle().is_completed() {
                break;
            }
        }
        assert_eq!(
            fixture.runtime.projection(&run)?.lifecycle(),
            RunLifecycle::Terminal(RunOutcome::Succeeded)
        );
        assert!(generations(&fixture)?.is_empty());
        assert_eq!(
            fixture.store.published_invocation(&plan.source)?.as_ref(),
            Some(&plan)
        );
        assert_eq!(retire(&fixture, number)?, retired);
        accepted.push((method, expected, request, published, retired, plan));
    }
    let fixture = fixture(directory.path(), "turnover-history")?;
    assert!(generations(&fixture)?.is_empty());
    for (method, expected, request, published, retired, plan) in accepted {
        assert_eq!(
            fixture
                .published
                .publish(method, expected, &fixture.decision, &request)?,
            published
        );
        assert_eq!(
            fixture
                .store
                .published_method(&plan.capability, plan.generation)?,
            Some(retired)
        );
        assert_eq!(
            fixture.store.published_invocation(&plan.source)?,
            Some(plan)
        );
    }
    assert!(generations(&fixture)?.is_empty());
    assert_eq!(fixture.process.entries(), 0);
    Ok(())
}

#[test]
fn queued_serving_acceptance_survives_retirement_and_reopen_before_entry() -> TestResult {
    let directory = TempDir::new()?;
    let first = fixture(directory.path(), "queued")?;
    let grant = publication_grant("human:caller", "grant:caller")?;
    let evaluator = GrantSetEvaluator::new(
        PolicyId::new("test.publication")?,
        1,
        [grant.clone()],
        BTreeMap::new(),
    )?;
    let mut authority_request = first.decision.request().clone();
    authority_request.operation = AuthorityOperation::InvokeCapability;
    authority_request.resources.capability_operation = Some(OperationId::new("method.invoke")?);
    authority_request.provenance.descriptor_revision = Some(1);
    let authority = evaluator.evaluate(&authority_request)?;
    assert!(authority.is_allowed());
    let host = milkdrift_capability::PeerId::new("host:queued")?;
    let catalog = CatalogSnapshot::new(1, NOW, NOW + 60_000, Vec::new())?;
    let mut submission = DirectInvocationRequest {
        host: host.clone(),
        request_id: PeerRequestId::new("request:queued")?,
        catalog_generation: 1,
        catalog_digest: catalog.digest.clone(),
        selection: milkdrift_capability::ResolvedCapabilitySnapshot::from_descriptor(
            &first.method.capability_descriptor()?,
            &OperationId::new("method.invoke")?,
        )?,
        request: milkdrift_capability::InvocationRequest::new(
            milkdrift_capability::InvocationId::new("invocation:queued")?,
            first.method.descriptor.identity().clone(),
            OperationId::new("method.invoke")?,
            None,
            None,
            Vec::new(),
            BTreeMap::new(),
        )?,
        limits: ExecutionLimits {
            nested_invocations: None,
            artifact_bytes: 1024,
            duration_ms: 1000,
            cost_micros: 0,
            cost_currency: None,
            input_units: None,
            output_units: None,
            observations: 8,
        },
        deadline_unix_ms: NOW + 60_000,
    };
    let basis = ClientInvocationAuthorization {
        host,
        actor: grant.actor().clone(),
        grant: grant.identity().clone(),
        grant_revision: 1,
        grant_digest: grant.digest()?,
        revocation_generation: 0,
    };
    let request = submission.bind(basis.clone())?;
    let caller = request.authorization.caller();
    first
        .store
        .configure_peer_relationship(&ServingCallerState {
            caller: caller.clone(),
            generation: 1,
            enabled: true,
            expires_at_unix_ms: NOW + 60_000,
            maximum_active: 4,
        })?;
    first.store.publish_peer_catalog(&ServingCatalogState {
        caller: caller.clone(),
        relationship_generation: 1,
        generation: 1,
        digest: catalog.digest.as_str().to_owned(),
        expires_at_unix_ms: NOW + 60_000,
    })?;
    first.store.set_peer_admission_open(true)?;
    let execution = PeerExecutionId::new("execution:queued")?;
    let admission = PeerAdmission {
        caller: &caller,
        request: &request,
        authority: &authority,
        execution: &execution,
        relationship_generation: 1,
        accepted_at_unix_ms: NOW,
        maximum_global_active: 4,
        maximum_dispatch_queue: 4,
        maximum_hot_terminal_records: 8,
        archive_batch_size: 4,
        archive_terminal_before_or_at_unix_ms: 1,
    };
    assert!(matches!(
        first.store.admit_peer_execution(&admission)?,
        PeerAdmissionOutcome::Accepted(_)
    ));
    let queued = first.store.peer_execution(&caller, &execution)?;
    retire(&first, 1)?;
    assert_eq!(generations(&first)?.len(), 1);
    assert_eq!(
        generations(&first)?
            .first()
            .ok_or("queued generation missing")?
            .pending_workflows,
        0
    );
    assert!(matches!(
        first.store.admit_peer_execution(&admission)?,
        PeerAdmissionOutcome::Replayed(_)
    ));
    // Simulate a submission that read the old catalog before retirement committed.
    submission.request_id = PeerRequestId::new("request:too-late")?;
    let too_late = submission.bind(basis)?;
    assert!(matches!(
        first.store.admit_peer_execution(&PeerAdmission {
            request: &too_late,
            ..admission
        })?,
        PeerAdmissionOutcome::Rejected(PeerAdmissionRejection::CatalogUnavailable)
    ));
    drop(first);

    let reopened = fixture(directory.path(), "queued-reopened")?;
    assert_eq!(reopened.store.peer_execution(&caller, &execution)?, queued);
    reopened.published.maintain_retirement()?;
    assert_eq!(generations(&reopened)?.len(), 1);
    assert!(
        generations(&reopened)?
            .first()
            .ok_or("restored generation missing")?
            .draining
    );
    let worker = WorkerId::new("worker:queued")?;
    assert!(matches!(
        reopened
            .store
            .claim_peer_dispatch(&PeerDispatchClaimRequest {
                worker: &worker,
                claimed_at_unix_ms: NOW,
                lease_expires_at_unix_ms: NOW + 1000,
            })?,
        PeerClaimOutcome::Claimed(_)
    ));
    reopened.store.append_peer_observation(
        &caller,
        &execution,
        &PeerObservation {
            execution: execution.clone(),
            sequence: 1,
            category: ObservationCategory::Terminal,
            event: milkdrift_capability::InvocationEvent::new(
                request.request.invocation().clone(),
                1,
                milkdrift_capability::InvocationEventKind::Terminal {
                    terminal: milkdrift_capability::InvocationTerminal::new(
                        milkdrift_capability::TerminalStatus::Cancelled,
                        Vec::new(),
                        None,
                        None,
                        SideEffectClass::ReadOnly,
                    )?,
                },
            )?,
            observed_at_unix_ms: NOW,
        },
    )?;
    reopened.published.maintain_retirement()?;
    assert!(generations(&reopened)?.is_empty());
    Ok(())
}
