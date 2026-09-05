use super::*;

#[test]
fn cancellation_before_entry_prevents_adapter_invocation_and_survives_claim_recovery() -> TestResult
{
    let root = tempfile::tempdir()?;
    let store = RedbStore::open(root.path())?;
    let peer = PeerId::new("peer-a")?;
    let target = PeerId::new("peer-b")?;
    let descriptor = descriptor()?;
    let catalog = milkdrift_peer_protocol::CatalogSnapshot::new(
        1,
        1,
        now().saturating_add(60_000),
        Vec::new(),
    )?;
    configure_store(&store, &peer, &catalog.digest, 2)?;
    let request = request(
        &peer,
        &target,
        &descriptor,
        1,
        catalog.digest,
        "request-cancel",
        "invocation-cancel",
    )?;
    let execution = PeerExecutionId::new("execution-cancel")?;
    admit(&store, &peer, &request, &execution, 2)?;
    let cancellation = PeerCancellationRequest {
        request_id: PeerRequestId::new("cancel-request")?,
        execution: execution.clone(),
        sequence: 1,
        reason: "operator cancellation".to_owned(),
    };
    store.request_peer_cancellation(&peer, &cancellation, now())?;
    let worker = WorkerId::new("cancel-worker")?;
    let claimed = store.claim_peer_dispatch(&PeerDispatchClaimRequest {
        worker: &worker,
        claimed_at_unix_ms: now(),
        lease_expires_at_unix_ms: now().saturating_add(30_000),
    })?;
    assert!(matches!(
        claimed,
        PeerClaimOutcome::CancellationRequested(_)
    ));
    assert!(
        store
            .mark_peer_entered(&PeerEntryRequest {
                owner: &peer,
                execution: &execution,
                worker: &worker,
                claim_generation: 2,
                relationship_generation: 1,
                entered_at_unix_ms: now(),
                authority: &allowed_decision(&peer)?,
            })
            .is_err()
    );
    let recovered = store.recover_peer_claims(now(), PageSize::new(8)?)?;
    assert_eq!(recovered.requeued, 1);
    Ok(())
}

#[test]
fn cancellation_terminal_commit_fault_recovers_and_replays_the_exact_acknowledgement() -> TestResult
{
    let root = tempfile::tempdir()?;
    let fault = Arc::new(FailOnce::new(FaultPoint::AfterPeerObservationCommit));
    let store = Arc::new(RedbStore::open_with_config(
        RedbStoreConfig::new(root.path()).with_fault_injector(fault.clone()),
    )?);
    let calls = Arc::new(AtomicUsize::new(0));
    let (host, descriptor) = host_with_adapter(Arc::new(TerminalAdapter {
        capability: CapabilityId::new("test-capability")?,
        delay: Duration::ZERO,
        active: Arc::new(AtomicUsize::new(0)),
        maximum: Arc::new(AtomicUsize::new(0)),
        calls: calls.clone(),
        requirements: CapabilityExecutionRequirements::default(),
    }))?;
    let peer = PeerId::new("peer-cancellation-commit-fault")?;
    let target = PeerId::new("peer-cancellation-commit-target")?;
    let service = PeerService::new(
        server_config(peer.clone(), target.clone(), 1, 1)?,
        host,
        store.clone(),
        system_peer_clock(),
    )?;
    let catalog_expiry = now().saturating_add(60_000);
    let catalog = milkdrift_peer_protocol::CatalogSnapshot::new(1, 1, catalog_expiry, Vec::new())?;
    store.set_peer_admission_open(true)?;
    store.publish_peer_catalog(&PeerCatalogState {
        peer: peer.clone(),
        relationship_generation: 1,
        generation: catalog.generation,
        digest: catalog.digest.as_str().to_owned(),
        expires_at_unix_ms: catalog_expiry,
    })?;
    let invocation = request(
        &peer,
        &target,
        &descriptor,
        catalog.generation,
        catalog.digest,
        "request-cancellation-commit-fault",
        "invocation-cancellation-commit-fault",
    )?;
    let execution = PeerExecutionId::new("execution-cancellation-commit-fault")?;
    admit(&store, &peer, &invocation, &execution, 1)?;
    let cancellation = PeerCancellationRequest {
        request_id: PeerRequestId::new("cancel-cancellation-commit-fault")?,
        execution: execution.clone(),
        sequence: 1,
        reason: "operator cancellation".to_owned(),
    };
    store.request_peer_cancellation(&peer, &cancellation, now())?;

    service.recover(1_024)?;
    let deadline = std::time::Instant::now() + Duration::from_secs(2);
    let acknowledgement = loop {
        let snapshot = store
            .peer_execution(&peer, &execution)?
            .ok_or("cancelled execution disappeared")?;
        let PeerExecutionSnapshot::Hot(record) = snapshot else {
            return Err("cancelled execution archived before acknowledgement recovery".into());
        };
        if let Some(acknowledgement) = record.cancellation.and_then(|value| value.acknowledgement) {
            break acknowledgement;
        }
        if std::time::Instant::now() >= deadline {
            return Err("cancellation acknowledgement recovery timed out".into());
        }
        thread::sleep(Duration::from_millis(10));
    };
    assert_eq!(
        acknowledgement.disposition,
        CancellationDisposition::Accepted
    );
    assert!(acknowledgement.terminal_boundary);
    assert!(
        acknowledgement
            .terminal_evidence
            .as_ref()
            .and_then(|observation| observation.event.kind().terminal())
            .is_some_and(|terminal| terminal.status() == TerminalStatus::Cancelled)
    );
    assert_eq!(service.cancel(&peer, &cancellation)?, acknowledgement);
    assert!(fault.triggered());
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    assert!(service.shutdown_workers(Duration::from_secs(2)).clean);
    Ok(())
}

#[test]
fn post_entry_and_post_terminal_cancellation_and_revocation_preserve_truth() -> TestResult {
    let root = tempfile::tempdir()?;
    let store = RedbStore::open(root.path())?;
    let peer = PeerId::new("peer-a")?;
    let target = PeerId::new("peer-b")?;
    let descriptor = descriptor()?;
    let catalog = milkdrift_peer_protocol::CatalogSnapshot::new(
        1,
        1,
        now().saturating_add(60_000),
        Vec::new(),
    )?;
    configure_store(&store, &peer, &catalog.digest, 3)?;

    let entered_request = request(
        &peer,
        &target,
        &descriptor,
        1,
        catalog.digest.clone(),
        "request-entered-cancel",
        "invocation-entered-cancel",
    )?;
    let entered_execution = PeerExecutionId::new("execution-entered-cancel")?;
    admit(&store, &peer, &entered_request, &entered_execution, 3)?;
    let worker = WorkerId::new("cancel-entered-worker")?;
    let claimed = claim(&store, &worker)?;
    let generation = claimed.phase.claim().ok_or("claim missing")?.generation;
    enter(&store, &peer, &entered_execution, &worker, generation)?;
    let entered_cancellation = PeerCancellationRequest {
        request_id: PeerRequestId::new("cancel-entered")?,
        execution: entered_execution.clone(),
        sequence: 1,
        reason: "disconnect after request".to_owned(),
    };
    let requested = store.request_peer_cancellation(&peer, &entered_cancellation, now())?;
    assert!(matches!(
        requested.phase,
        PeerExecutionPhase::CancellationRequested {
            evidence: Some(_),
            ..
        }
    ));
    assert!(
        requested
            .cancellation
            .as_ref()
            .is_some_and(|cancellation| cancellation.acknowledgement.is_none())
    );
    let recovery = store.recover_peer_claims(now(), PageSize::new(8)?)?;
    assert_eq!(recovery.uncertain, 1);

    let terminal_request = request(
        &peer,
        &target,
        &descriptor,
        1,
        catalog.digest.clone(),
        "request-terminal-cancel",
        "invocation-terminal-cancel",
    )?;
    let terminal_execution = PeerExecutionId::new("execution-terminal-cancel")?;
    admit(&store, &peer, &terminal_request, &terminal_execution, 3)?;
    let terminal_worker = WorkerId::new("terminal-worker")?;
    let _terminal_claim = claim(&store, &terminal_worker)?;
    let terminal = terminal_observation(
        &terminal_request,
        &terminal_execution,
        1,
        TerminalStatus::Success,
    )?;
    store.append_peer_observation(&peer, &terminal_execution, &terminal)?;
    let terminal_cancellation = PeerCancellationRequest {
        request_id: PeerRequestId::new("cancel-terminal")?,
        execution: terminal_execution.clone(),
        sequence: 1,
        reason: "too late cancellation".to_owned(),
    };
    let terminal_record = store.request_peer_cancellation(&peer, &terminal_cancellation, now())?;
    assert!(matches!(
        terminal_record.phase,
        PeerExecutionPhase::Terminal { .. }
    ));
    assert_eq!(
        &terminal_record
            .cancellation
            .as_ref()
            .ok_or("terminal cancellation facts missing")?
            .request,
        &terminal_cancellation
    );

    store.configure_peer_relationship(&PeerRelationshipState {
        peer: peer.clone(),
        generation: 2,
        enabled: false,
        expires_at_unix_ms: now().saturating_add(600_000),
        maximum_active: 3,
    })?;
    let blocked_request = request(
        &peer,
        &target,
        &descriptor,
        1,
        catalog.digest,
        "request-after-revocation",
        "invocation-after-revocation",
    )?;
    assert!(matches!(
        store.admit_peer_execution(&PeerAdmission {
            owner_peer: &peer,
            request: &blocked_request,
            authority: &allowed_decision(&peer)?,
            execution: &PeerExecutionId::new("execution-after-revocation")?,
            relationship_generation: 2,
            accepted_at_unix_ms: now(),
            maximum_global_active: 3,
            maximum_dispatch_queue: 3,
            maximum_hot_terminal_records: 1_000,
            archive_batch_size: 16,
            archive_terminal_before_or_at_unix_ms: 1,
        })?,
        PeerAdmissionOutcome::Rejected(PeerAdmissionRejection::RelationshipUnavailable)
    ));
    assert!(
        store
            .peer_execution_by_request(&peer, &terminal_request.request_id)?
            .is_some()
    );
    let first_archive = store.archive_peer_executions(&PeerRetentionRequest {
        terminal_before_or_at: TimestampMillis::new(now().saturating_add(1)),
        archived_at: TimestampMillis::new(now().saturating_add(2)),
        limit: PageSize::new(1)?,
    })?;
    assert_eq!(first_archive.archived, 1);
    assert!(first_archive.more);
    let second_archive = store.archive_peer_executions(&PeerRetentionRequest {
        terminal_before_or_at: TimestampMillis::new(now().saturating_add(3)),
        archived_at: TimestampMillis::new(now().saturating_add(4)),
        limit: PageSize::new(1)?,
    })?;
    assert_eq!(second_archive.archived, 1);
    assert!(!second_archive.more);
    Ok(())
}
