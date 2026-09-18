//! Remote imports consume the origin's existing reservation in the publication transaction.
use super::*;

#[test]
fn remote_output_charge_replays_after_reopen_with_its_original_reservation() -> TestResult {
    let directory = TempDir::new()?;
    let store = RedbStore::open(directory.path())?;
    let owner = RunId::new("remote-controller-owner")?;
    let child = RunId::new("remote-controller-child")?;
    let declaration = establish(&store, &owner, "remote-output-owner")?;
    bind_child(&store, &child, &declaration, "remote-output-child")?;
    let state = store
        .controller_account(declaration.account())?
        .ok_or("account absent")?;
    let attempt = AttemptId::new("remote-output-attempt")?;
    let reservation = ControllerReservationId::for_attempt(declaration.account(), &attempt)?;
    let envelope = InvocationAdmissionEnvelope::new(
        milkdrift_capability::AdmissionUnit::Unknown,
        AdmissionBound::NotApplicable,
        AdmissionBound::NotApplicable,
        AdmissionBound::Bounded(16),
        AdmissionBound::NotApplicable,
    );
    let mut candidate = state.clone();
    let outcome = candidate.admit(
        reservation.clone(),
        attempt.clone(),
        CapabilityCategory::Process,
        &envelope,
    )?;
    let entry = request(
        &child,
        "remote-output-entry",
        "remote-output-entry-event",
        RunSequence::ZERO,
        RunEventKind::CapabilityAdapterEntryDecisionRecorded {
            attempt: attempt.clone(),
            authorization: decision(true, "remote-output-entry")?,
            controller_admission: outcome.clone(),
        },
    )?
    .with_controller_account_transaction(transaction(
        "remote-output-entry-transition",
        Some((&state, &declaration)),
        vec![ControllerAccountAction::AdmitEntry {
            account: declaration.account().clone(),
            reservation: reservation.clone(),
            attempt,
            category: CapabilityCategory::Process,
            envelope,
            expected_outcome: outcome,
        }],
    )?)?;
    store.commit_command(&entry)?;
    let bytes = b"remote result";
    let host = milkdrift_capability::PeerId::new("remote-serving-host")?;
    let metadata = ArtifactMetadata::new(
        milkdrift_workspace::ArtifactReference::new(
            ArtifactId::new("remote-output-artifact")?,
            ContentDigest::for_bytes(bytes),
            MediaType::new("text/plain")?,
            bytes.len() as u64,
        ),
        ArtifactSensitivity::Restricted,
        ArtifactRetention::WhileReferenced,
        ArtifactProvenance::new(
            CausalReference::External {
                source: CausalId::new(
                    "peer:remote-serving-host/execution:remote-output-invocation",
                )?,
            },
            vec![CausalReference::HostInvocation {
                host: host.clone(),
                invocation: milkdrift_capability::InvocationId::new("remote-output-invocation")?,
            }],
        )?,
    )?;
    let publication = BeginArtifactPublication::for_transfer(
        ArtifactPublicationId::new("remote-output-publication")?,
        host,
        CausalId::new("remote-output-transfer")?,
        metadata,
        workspace_budget()?,
        WorkspaceUsage::EMPTY,
        Some(
            milkdrift_persistence::ControllerArtifactOwner::RemoteInvocationReservation {
                run: child,
                reservation,
            },
        ),
    )?;
    store.begin_publication(&publication)?;
    store.write_chunk(publication.publication(), 0, bytes)?;
    let before = store
        .controller_account(declaration.account())?
        .ok_or("account absent")?;
    assert_eq!(before.settled().artifact_bytes(), 0);
    assert_eq!(before.outstanding().artifact_bytes(), 16);
    store.commit_publication(publication.publication())?;
    let charged = store
        .controller_account(declaration.account())?
        .ok_or("account absent")?;
    assert_eq!(charged.settled().artifact_bytes(), bytes.len() as u64);
    assert_eq!(
        charged.outstanding().artifact_bytes(),
        16 - bytes.len() as u64
    );
    assert_eq!(charged.settled().process_admissions(), 1);
    assert!(!has_integrity_failure(&store)?);
    drop(store);
    let store = RedbStore::open(directory.path())?;
    assert!(matches!(
        store.begin_publication(&publication)?,
        milkdrift_persistence::BeginArtifactOutcome::AlreadyCommitted(_)
    ));
    store.commit_publication(publication.publication())?;
    assert_eq!(
        store.controller_account(declaration.account())?,
        Some(charged)
    );
    assert!(!has_integrity_failure(&store)?);
    Ok(())
}
