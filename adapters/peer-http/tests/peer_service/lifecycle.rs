//! Admission races, cancellation, workers, and service lifecycle behavior.

use super::support::*;

#[test]
fn durable_drain_and_relationship_generation_close_the_adapter_entry_race() -> TestResult {
    let root = tempfile::tempdir()?;
    let store = RedbStore::open(root.path())?;
    let peer = PeerId::new("peer-entry-race")?;
    let target = PeerId::new("peer-entry-target")?;
    let descriptor = descriptor()?;
    let catalog = milkdrift_peer_protocol::CatalogSnapshot::new(
        1,
        now().saturating_sub(1),
        now().saturating_add(60_000),
        Vec::new(),
    )?;
    configure_store(&store, &peer, &catalog.digest, 1)?;
    let request = request(
        &peer,
        &target,
        &descriptor,
        1,
        catalog.digest,
        "request-entry-race",
        "invocation-entry-race",
    )?;
    let execution = PeerExecutionId::new("execution-entry-race")?;
    assert!(matches!(
        store.admit_peer_execution(&PeerAdmission {
            owner_peer: &peer,
            request: &request,
            authority: &allowed_decision(&peer)?,
            execution: &execution,
            relationship_generation: 1,
            accepted_at_unix_ms: now(),
            maximum_global_active: 1,
            maximum_dispatch_queue: 1,
            maximum_hot_terminal_records: 8,
            archive_batch_size: 2,
            archive_terminal_before_or_at_unix_ms: 1,
        })?,
        PeerAdmissionOutcome::Accepted(_)
    ));
    let worker = WorkerId::new("worker-entry-race")?;
    let claimed = claim(&store, &worker)?;
    let generation = claimed.phase.claim().ok_or("claim missing")?.generation;
    store.set_peer_admission_open(false)?;
    let authority = allowed_decision(&peer)?;
    let entry_request = PeerEntryRequest {
        owner: &peer,
        execution: &execution,
        worker: &worker,
        claim_generation: generation,
        relationship_generation: 1,
        entered_at_unix_ms: now(),
        authority: &authority,
    };
    assert_eq!(
        store.mark_peer_entered(&entry_request)?,
        PeerEntryOutcome::AdmissionClosed
    );
    store.set_peer_admission_open(true)?;
    store.configure_peer_relationship(&PeerRelationshipState {
        peer: peer.clone(),
        generation: 2,
        enabled: false,
        expires_at_unix_ms: now().saturating_add(60_000),
        maximum_active: 1,
    })?;
    assert_eq!(
        store.mark_peer_entered(&entry_request)?,
        PeerEntryOutcome::RelationshipUnavailable
    );
    Ok(())
}

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
fn recovery_reports_a_remaining_claim_frontier_after_a_bounded_page() -> TestResult {
    let root = tempfile::tempdir()?;
    let store = RedbStore::open(root.path())?;
    let peer = PeerId::new("peer-recovery-page")?;
    let target = PeerId::new("peer-recovery-page-target")?;
    let descriptor = descriptor()?;
    let catalog = milkdrift_peer_protocol::CatalogSnapshot::new(
        1,
        1,
        now().saturating_add(60_000),
        Vec::new(),
    )?;
    configure_store(&store, &peer, &catalog.digest, 2)?;
    for ordinal in 0..2 {
        let request = request(
            &peer,
            &target,
            &descriptor,
            1,
            catalog.digest.clone(),
            &format!("request-recovery-page-{ordinal}"),
            &format!("invocation-recovery-page-{ordinal}"),
        )?;
        let execution = PeerExecutionId::new(format!("execution-recovery-page-{ordinal}"))?;
        admit(&store, &peer, &request, &execution, 2)?;
        let _ = claim(
            &store,
            &WorkerId::new(format!("worker-recovery-page-{ordinal}"))?,
        )?;
    }
    let first = store.recover_peer_claims(now(), PageSize::new(1)?)?;
    assert_eq!(first.requeued, 1);
    assert!(first.more);
    let second = store.recover_peer_claims(now(), PageSize::new(1)?)?;
    assert_eq!(second.requeued, 1);
    assert!(!second.more);
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

#[test]
fn observation_history_is_append_only_and_pages_bound_long_stream_memory() -> TestResult {
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
    configure_store(&store, &peer, &catalog.digest, 1)?;
    let request = request(
        &peer,
        &target,
        &descriptor,
        1,
        catalog.digest,
        "request-long-observations",
        "invocation-long-observations",
    )?;
    let execution = PeerExecutionId::new("execution-long-observations")?;
    admit(&store, &peer, &request, &execution, 1)?;
    let worker = WorkerId::new("observation-worker")?;
    let claimed = claim(&store, &worker)?;
    let generation = claimed.phase.claim().ok_or("claim missing")?.generation;
    enter(&store, &peer, &execution, &worker, generation)?;
    for sequence in 1..100 {
        store.append_peer_observation(
            &peer,
            &execution,
            &progress_observation(&request, &execution, sequence)?,
        )?;
    }
    store.append_peer_observation(
        &peer,
        &execution,
        &terminal_observation(&request, &execution, 100, TerminalStatus::Success)?,
    )?;
    let first = store.peer_observations(&peer, &execution, 93, PageSize::new(4)?)?;
    assert_eq!(
        first
            .observations
            .iter()
            .map(|observation| observation.sequence)
            .collect::<Vec<_>>(),
        vec![94, 95, 96, 97]
    );
    let resumed = store.peer_observations(&peer, &execution, 97, PageSize::new(4)?)?;
    assert_eq!(
        resumed
            .observations
            .iter()
            .map(|observation| observation.sequence)
            .collect::<Vec<_>>(),
        vec![98, 99, 100]
    );
    let PeerExecutionSnapshot::Hot(record) = resumed.execution else {
        return Err("hot observation record archived unexpectedly".into());
    };
    assert_eq!(record.accounting.observations, 100);
    Ok(())
}

#[test]
fn inbound_peer_authority_includes_adapter_declared_secret_requirements() -> TestResult {
    let root = tempfile::tempdir()?;
    let store = Arc::new(RedbStore::open(root.path())?);
    let active = Arc::new(AtomicUsize::new(0));
    let maximum = Arc::new(AtomicUsize::new(0));
    let calls = Arc::new(AtomicUsize::new(0));
    let (host, descriptor) = host_with_adapter(Arc::new(TerminalAdapter {
        capability: CapabilityId::new("test-capability")?,
        delay: Duration::ZERO,
        active,
        maximum,
        calls,
        requirements: CapabilityExecutionRequirements {
            secrets: BTreeSet::from([SecretRef::new("secret:peer-denied")?]),
            ..CapabilityExecutionRequirements::default()
        },
    }))?;
    let peer = PeerId::new("peer-secret-denied")?;
    let target = PeerId::new("peer-secret-target")?;
    let service = PeerService::new(
        server_config(peer.clone(), target.clone(), 1, 4)?,
        host,
        store,
        Arc::new(SystemPeerClock),
    )?;
    service.recover(1_024)?;
    let catalog = service.catalog(&peer)?;
    let denied = service.invoke(
        &peer,
        request(
            &peer,
            &target,
            &descriptor,
            catalog.generation,
            catalog.digest,
            "request-secret-denied",
            "invocation-secret-denied",
        )?,
    );
    assert!(
        denied.is_err(),
        "undelegated adapter secret reached admission"
    );
    assert!(service.shutdown_workers(Duration::from_secs(2)).clean);
    Ok(())
}

#[test]
fn fixed_worker_owner_bounds_execution_and_shutdown_joins() -> TestResult {
    let root = tempfile::tempdir()?;
    let store = Arc::new(RedbStore::open(root.path())?);
    let active = Arc::new(AtomicUsize::new(0));
    let maximum = Arc::new(AtomicUsize::new(0));
    let calls = Arc::new(AtomicUsize::new(0));
    let (host, descriptor) = host_with_adapter(Arc::new(TerminalAdapter {
        capability: CapabilityId::new("test-capability")?,
        delay: Duration::from_millis(20),
        active: active.clone(),
        maximum: maximum.clone(),
        calls,
        requirements: CapabilityExecutionRequirements::default(),
    }))?;
    let peer = PeerId::new("peer-a")?;
    let target = PeerId::new("peer-b")?;
    let service = PeerService::new(
        server_config(peer.clone(), target.clone(), 2, 16)?,
        host,
        store,
        Arc::new(SystemPeerClock),
    )?;
    let closed_catalog = service.catalog(&peer)?;
    let closed_request = request(
        &peer,
        &target,
        &descriptor,
        closed_catalog.generation,
        closed_catalog.digest,
        "request-before-recovery",
        "invocation-before-recovery",
    )?;
    assert!(matches!(
        service.invoke(&peer, closed_request)?,
        InvocationAcceptance::Rejected { code, .. } if code == "draining"
    ));
    service.recover(1_024)?;
    let catalog = service.catalog(&peer)?;
    let mut accepted = Vec::new();
    for index in 0..8 {
        let request = request(
            &peer,
            &target,
            &descriptor,
            catalog.generation,
            catalog.digest.clone(),
            &format!("request-worker-{index}"),
            &format!("invocation-worker-{index}"),
        )?;
        if let InvocationAcceptance::Accepted { execution, .. } = service.invoke(&peer, request)? {
            accepted.push(execution);
        }
    }
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while std::time::Instant::now() < deadline {
        if accepted.iter().all(|execution| {
            service
                .observations(&peer, execution, 0, 8)
                .is_ok_and(|page| page.terminal)
        }) {
            break;
        }
        thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(accepted.len(), 8);
    assert!(maximum.load(Ordering::SeqCst) <= 2);
    let forged_reference = ArtifactReference::new(
        ArtifactId::new("forged-peer-artifact")?,
        ContentDigest::for_bytes(b"forged"),
        MediaType::new("application/octet-stream")?,
        6,
    );
    let forged_offer = ArtifactMetadataOffer {
        transfer: TransferId::new("forged-peer-transfer")?,
        direction: ArtifactTransferDirection::Upload,
        artifact: forged_reference,
        sensitivity: ArtifactSensitivity::Internal,
        retention: ArtifactRetention::WhileReferenced,
        provenance: ArtifactProvenance::new(
            CausalReference::External {
                source: CausalId::new("caller-asserted-producer")?,
            },
            Vec::new(),
        )?,
        source_peer: peer.clone(),
        execution: PeerExecutionId::new("nonexistent-peer-execution")?,
        expires_at_unix_ms: now().saturating_add(60_000),
    };
    assert!(
        service.negotiate_artifact(&peer, &forged_offer).is_err(),
        "artifact upload accepted caller-asserted execution provenance"
    );
    let shutdown = service.shutdown_workers(Duration::from_secs(2));
    assert!(shutdown.clean);
    assert_eq!(shutdown.joined, 2);
    Ok(())
}

#[test]
fn service_archived_replay_returns_summary_without_second_adapter_entry() -> TestResult {
    let root = tempfile::tempdir()?;
    let store = Arc::new(RedbStore::open(root.path())?);
    let active = Arc::new(AtomicUsize::new(0));
    let maximum = Arc::new(AtomicUsize::new(0));
    let calls = Arc::new(AtomicUsize::new(0));
    let (host, descriptor) = host_with_adapter(Arc::new(TerminalAdapter {
        capability: CapabilityId::new("test-capability")?,
        delay: Duration::ZERO,
        active,
        maximum,
        calls: calls.clone(),
        requirements: CapabilityExecutionRequirements::default(),
    }))?;
    let peer = PeerId::new("peer-service-archive")?;
    let target = PeerId::new("peer-service-archive-target")?;
    let mut config = server_config(peer.clone(), target.clone(), 1, 4)?;
    config.workers.maximum_hot_terminal_records = 4;
    config.workers.archive_batch_size = 1;
    config.workers.observation_hot_retention = Duration::from_millis(1);
    let service = PeerService::new(config, host, store.clone(), Arc::new(SystemPeerClock))?;
    service.recover(1_024)?;
    let catalog = service.catalog(&peer)?;
    let invocation_request = request(
        &peer,
        &target,
        &descriptor,
        catalog.generation,
        catalog.digest,
        "request-service-archive",
        "invocation-service-archive",
    )?;
    let accepted = service.invoke(&peer, invocation_request.clone())?;
    let InvocationAcceptance::Accepted { execution, .. } = accepted else {
        return Err("first service invocation was not accepted".into());
    };
    let deadline = std::time::Instant::now() + Duration::from_secs(2);
    loop {
        if matches!(
            store.peer_execution(&peer, &execution)?,
            Some(PeerExecutionSnapshot::Hot(ref record))
                if matches!(record.phase, PeerExecutionPhase::Terminal { .. })
        ) {
            break;
        }
        if std::time::Instant::now() >= deadline {
            return Err("service invocation did not terminalize".into());
        }
        thread::sleep(Duration::from_millis(2));
    }
    thread::sleep(Duration::from_millis(2));
    service.maintain_retention()?;
    assert!(matches!(
        store.peer_execution(&peer, &execution)?,
        Some(PeerExecutionSnapshot::Archived(_))
    ));
    assert!(matches!(
        service.invoke(&peer, invocation_request.clone())?,
        InvocationAcceptance::Archived { execution: replayed, .. } if replayed == execution
    ));
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    let page = service.observations(&peer, &execution, 0, 8)?;
    assert!(page.observations.is_empty());
    assert!(page.closed);
    assert!(matches!(page.history, ObservationHistory::Archived { .. }));

    let conflicting = request(
        &peer,
        &target,
        &descriptor,
        invocation_request.catalog_generation,
        invocation_request.catalog_digest.clone(),
        invocation_request.request_id.as_str(),
        "invocation-service-archive-conflict",
    )?;
    assert!(matches!(
        service.invoke(&peer, conflicting)?,
        InvocationAcceptance::Rejected { code, retryable: false, .. }
            if code == "idempotency_conflict"
    ));
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert!(service.shutdown_workers(Duration::from_secs(2)).clean);
    Ok(())
}
