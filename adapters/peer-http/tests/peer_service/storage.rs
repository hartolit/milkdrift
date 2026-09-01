//! Durable acceptance, recovery, archival, and integrity behavior.

use super::support::*;

#[test]
fn transport_configuration_requires_https_or_explicit_loopback_development() -> TestResult {
    let config = |endpoint: &str, insecure_loopback| -> TestResult<PeerClientConfig> {
        Ok(PeerClientConfig {
            endpoint: Url::parse(endpoint)?,
            local_peer: PeerId::new("peer-a")?,
            expected_remote_peer: PeerId::new("peer-b")?,
            session: SessionId::new("session-a")?,
            versions: ProtocolVersionRange::default(),
            bearer_credential: Arc::new(SensitiveSecret::new(b"peer-secret".to_vec())),
            insecure_loopback,
            request_timeout: Duration::from_secs(1),
            observation_poll_interval: Duration::from_millis(10),
        })
    };
    assert!(
        config("https://peer.example/", InsecureLoopbackMode::Disabled)?
            .validate()
            .is_ok()
    );
    assert!(
        config("http://127.0.0.1:8080/", InsecureLoopbackMode::Disabled)?
            .validate()
            .is_err()
    );
    assert!(
        config(
            "http://127.0.0.1:8080/",
            InsecureLoopbackMode::AllowInsecureLoopbackDevelopment,
        )?
        .validate()
        .is_ok()
    );
    assert!(
        config(
            "http://192.0.2.10/",
            InsecureLoopbackMode::AllowInsecureLoopbackDevelopment,
        )?
        .validate()
        .is_err()
    );
    Ok(())
}

#[test]
fn atomic_final_slot_and_idempotency_survive_reopen() -> TestResult {
    let root = tempfile::tempdir()?;
    let store = Arc::new(RedbStore::open(root.path())?);
    let peer = PeerId::new("peer-a")?;
    let target = PeerId::new("peer-b")?;
    let descriptor = descriptor()?;
    let catalog = milkdrift_peer_protocol::CatalogSnapshot::new(
        1,
        now().saturating_sub(1),
        now().saturating_add(60_000),
        Vec::new(),
    )?;
    configure_store(store.as_ref(), &peer, &catalog.digest, 1)?;
    let request_a = request(
        &peer,
        &target,
        &descriptor,
        1,
        catalog.digest.clone(),
        "request-a",
        "invocation-a",
    )?;
    let request_b = request(
        &peer,
        &target,
        &descriptor,
        1,
        catalog.digest,
        "request-b",
        "invocation-b",
    )?;
    let barrier = Arc::new(Barrier::new(3));
    let mut handles = Vec::new();
    for (request, execution) in [
        (request_a.clone(), PeerExecutionId::new("execution-a")?),
        (request_b, PeerExecutionId::new("execution-b")?),
    ] {
        let store = store.clone();
        let peer = peer.clone();
        let barrier = barrier.clone();
        let authority = allowed_decision(&peer)?;
        handles.push(thread::spawn(move || {
            barrier.wait();
            store.admit_peer_execution(&PeerAdmission {
                owner_peer: &peer,
                request: &request,
                authority: &authority,
                execution: &execution,
                relationship_generation: 1,
                accepted_at_unix_ms: now(),
                maximum_global_active: 1,
                maximum_dispatch_queue: 1,
                maximum_hot_terminal_records: 10,
                archive_batch_size: 2,
                archive_terminal_before_or_at_unix_ms: 1,
            })
        }));
    }
    barrier.wait();
    let mut outcomes = Vec::new();
    for handle in handles {
        outcomes.push(
            handle
                .join()
                .map_err(|_| "peer admission test thread panicked")??,
        );
    }
    assert_eq!(
        outcomes
            .iter()
            .filter(|outcome| matches!(outcome, PeerAdmissionOutcome::Accepted(_)))
            .count(),
        1
    );
    assert_eq!(
        outcomes
            .iter()
            .filter(|outcome| matches!(outcome, PeerAdmissionOutcome::Rejected(_)))
            .count(),
        1
    );
    let accepted = outcomes
        .into_iter()
        .find_map(|outcome| match outcome {
            PeerAdmissionOutcome::Accepted(record) => Some(record),
            _ => None,
        })
        .ok_or("one request should be accepted")?;
    let accepted_request = accepted.request.clone();
    let accepted_execution = accepted.execution.clone();
    drop(store);

    let reopened = RedbStore::open(root.path())?;
    assert!(matches!(
        reopened.admit_peer_execution(&PeerAdmission {
            owner_peer: &peer,
            request: &accepted_request,
            authority: &allowed_decision(&peer)?,
            execution: &accepted_execution,
            relationship_generation: 1,
            accepted_at_unix_ms: now(),
            maximum_global_active: 1,
            maximum_dispatch_queue: 1,
            maximum_hot_terminal_records: 10,
            archive_batch_size: 2,
            archive_terminal_before_or_at_unix_ms: 1,
        })?,
        PeerAdmissionOutcome::Replayed(_)
    ));
    let different = request(
        &peer,
        &target,
        &descriptor,
        1,
        accepted_request.catalog_digest.clone(),
        accepted_request.request_id.as_str(),
        "different-invocation",
    )?;
    assert!(matches!(
        reopened.admit_peer_execution(&PeerAdmission {
            owner_peer: &peer,
            request: &different,
            authority: &allowed_decision(&peer)?,
            execution: &accepted_execution,
            relationship_generation: 1,
            accepted_at_unix_ms: now(),
            maximum_global_active: 1,
            maximum_dispatch_queue: 1,
            maximum_hot_terminal_records: 10,
            archive_batch_size: 2,
            archive_terminal_before_or_at_unix_ms: 1,
        })?,
        PeerAdmissionOutcome::Conflict(_)
    ));
    Ok(())
}

#[test]
fn commit_boundary_faults_preserve_acceptance_claim_and_observation_truth() -> TestResult {
    let descriptor = descriptor()?;
    let peer = PeerId::new("peer-a")?;
    let target = PeerId::new("peer-b")?;

    let admission_root = tempfile::tempdir()?;
    let admission_store = RedbStore::open_with_config(
        RedbStoreConfig::new(admission_root.path()).with_fault_injector(Arc::new(FailOnce::new(
            FaultPoint::AfterPeerAdmissionCommit,
        ))),
    )?;
    let admission_catalog = milkdrift_peer_protocol::CatalogSnapshot::new(
        1,
        1,
        now().saturating_add(60_000),
        Vec::new(),
    )?;
    configure_store(&admission_store, &peer, &admission_catalog.digest, 2)?;
    let admission_request = request(
        &peer,
        &target,
        &descriptor,
        1,
        admission_catalog.digest,
        "request-lost-response",
        "invocation-lost-response",
    )?;
    let admission_execution = PeerExecutionId::new("execution-lost-response")?;
    assert!(
        admission_store
            .admit_peer_execution(&PeerAdmission {
                owner_peer: &peer,
                request: &admission_request,
                authority: &allowed_decision(&peer)?,
                execution: &admission_execution,
                relationship_generation: 1,
                accepted_at_unix_ms: now(),
                maximum_global_active: 2,
                maximum_dispatch_queue: 2,
                maximum_hot_terminal_records: 10,
                archive_batch_size: 2,
                archive_terminal_before_or_at_unix_ms: 1,
            })
            .is_err()
    );
    drop(admission_store);
    let admission_store = RedbStore::open(admission_root.path())?;
    assert!(matches!(
        admission_store.admit_peer_execution(&PeerAdmission {
            owner_peer: &peer,
            request: &admission_request,
            authority: &allowed_decision(&peer)?,
            execution: &admission_execution,
            relationship_generation: 1,
            accepted_at_unix_ms: now(),
            maximum_global_active: 2,
            maximum_dispatch_queue: 2,
            maximum_hot_terminal_records: 10,
            archive_batch_size: 2,
            archive_terminal_before_or_at_unix_ms: 1,
        })?,
        PeerAdmissionOutcome::Replayed(_)
    ));

    let claim_root = tempfile::tempdir()?;
    let claim_store = RedbStore::open_with_config(
        RedbStoreConfig::new(claim_root.path())
            .with_fault_injector(Arc::new(FailOnce::new(FaultPoint::BeforePeerClaimCommit))),
    )?;
    configure_store(&claim_store, &peer, &admission_request.catalog_digest, 2)?;
    let claim_request = request(
        &peer,
        &target,
        &descriptor,
        1,
        admission_request.catalog_digest.clone(),
        "request-claim-fault",
        "invocation-claim-fault",
    )?;
    let claim_execution = PeerExecutionId::new("execution-claim-fault")?;
    admit(&claim_store, &peer, &claim_request, &claim_execution, 2)?;
    let worker = WorkerId::new("fault-worker")?;
    assert!(
        claim_store
            .claim_peer_dispatch(&PeerDispatchClaimRequest {
                worker: &worker,
                claimed_at_unix_ms: now(),
                lease_expires_at_unix_ms: now().saturating_add(30_000),
            })
            .is_err()
    );
    drop(claim_store);
    let claim_store = RedbStore::open(claim_root.path())?;
    let claimed = claim(&claim_store, &worker)?;
    let claim = claimed.phase.claim().ok_or("claim missing")?.clone();
    enter(
        &claim_store,
        &peer,
        &claim_execution,
        &worker,
        claim.generation,
    )?;
    drop(claim_store);

    let observation_store =
        RedbStore::open_with_config(RedbStoreConfig::new(claim_root.path()).with_fault_injector(
            Arc::new(FailOnce::new(FaultPoint::AfterPeerObservationCommit)),
        ))?;
    let terminal =
        terminal_observation(&claim_request, &claim_execution, 1, TerminalStatus::Success)?;
    assert!(
        observation_store
            .append_peer_observation(&peer, &claim_execution, &terminal)
            .is_err()
    );
    drop(observation_store);
    let observation_store = RedbStore::open(claim_root.path())?;
    assert!(matches!(
        observation_store.append_peer_observation(&peer, &claim_execution, &terminal)?,
        milkdrift_persistence::PeerObservationAppend::Replayed(_)
    ));
    let observations =
        observation_store.peer_observations(&peer, &claim_execution, 0, PageSize::new(1)?)?;
    assert_eq!(observations.observations, vec![terminal]);
    Ok(())
}

#[test]
fn claims_recover_at_truthful_entry_boundary_and_late_terminal_is_idempotent() -> TestResult {
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
    configure_store(&store, &peer, &catalog.digest, 4)?;
    let request = request(
        &peer,
        &target,
        &descriptor,
        1,
        catalog.digest,
        "request-recovery",
        "invocation-recovery",
    )?;
    let execution = PeerExecutionId::new("execution-recovery")?;
    admit(&store, &peer, &request, &execution, 4)?;
    let worker = WorkerId::new("test-worker")?;
    let first = claim(&store, &worker)?;
    let first_generation = first.phase.claim().ok_or("claim absent")?.generation;
    let recovery = store.recover_peer_claims(now(), PageSize::new(8)?)?;
    assert_eq!(recovery.requeued, 1);
    let second = claim(&store, &worker)?;
    let second_claim = second.phase.claim().ok_or("second claim absent")?.clone();
    assert_ne!(first_generation, second_claim.generation);
    enter(&store, &peer, &execution, &worker, second_claim.generation)?;
    let recovery = store.recover_peer_claims(now(), PageSize::new(8)?)?;
    assert_eq!(recovery.uncertain, 1);
    let uncertain = store
        .peer_execution(&peer, &execution)?
        .ok_or("execution missing")?;
    let PeerExecutionSnapshot::Hot(uncertain) = uncertain else {
        return Err("uncertain execution archived unexpectedly".into());
    };
    assert!(matches!(
        uncertain.phase,
        PeerExecutionPhase::Uncertain { .. }
    ));
    assert_eq!(uncertain.last_observation_sequence, 0);

    let terminal = terminal_observation(&request, &execution, 1, TerminalStatus::Success)?;
    assert!(matches!(
        store.append_peer_observation(&peer, &execution, &terminal)?,
        milkdrift_persistence::PeerObservationAppend::Appended(_)
    ));
    assert!(matches!(
        store.append_peer_observation(&peer, &execution, &terminal)?,
        milkdrift_persistence::PeerObservationAppend::Replayed(_)
    ));
    let page = store.peer_observations(&peer, &execution, 0, PageSize::new(1)?)?;
    assert_eq!(page.observations, vec![terminal]);
    let archived = store.archive_peer_executions(&PeerRetentionRequest {
        terminal_before_or_at: TimestampMillis::new(now().saturating_add(1)),
        archived_at: TimestampMillis::new(now().saturating_add(2)),
        limit: PageSize::new(8)?,
    })?;
    assert_eq!(archived.archived, 1);
    assert!(
        store
            .peer_execution_by_request(&peer, &request.request_id)?
            .is_some()
    );
    Ok(())
}

#[test]
fn cumulative_output_artifacts_cannot_exceed_the_accepted_total_quota() -> TestResult {
    let root = tempfile::tempdir()?;
    let store = RedbStore::open(root.path())?;
    let peer = PeerId::new("peer-artifact-quota")?;
    let target = PeerId::new("peer-artifact-quota-target")?;
    let descriptor = descriptor()?;
    let catalog = milkdrift_peer_protocol::CatalogSnapshot::new(
        1,
        1,
        now().saturating_add(60_000),
        Vec::new(),
    )?;
    configure_store(&store, &peer, &catalog.digest, 1)?;
    let request = request_with_input_artifact(
        &peer,
        &target,
        &descriptor,
        1,
        catalog.digest,
        "request-artifact-quota",
        "invocation-artifact-quota",
        448_576,
    )?;
    let execution = PeerExecutionId::new("execution-artifact-quota")?;
    admit(&store, &peer, &request, &execution, 1)?;
    let admitted = store
        .peer_execution(&peer, &execution)?
        .ok_or("admitted execution missing")?;
    let PeerExecutionSnapshot::Hot(admitted) = admitted else {
        return Err("admitted execution archived unexpectedly".into());
    };
    assert_eq!(admitted.accounting.artifact_bytes, 448_576);
    let worker = WorkerId::new("artifact-quota-worker")?;
    let claimed = claim(&store, &worker)?;
    let generation = claimed.phase.claim().ok_or("claim missing")?.generation;
    enter(&store, &peer, &execution, &worker, generation)?;

    let first = output_observation(&request, &execution, 1, "first", 600_000, 'a')?;
    store.append_peer_observation(&peer, &execution, &first)?;
    let second = output_observation(&request, &execution, 2, "second", 1, 'b')?;
    let Err(error) = store.append_peer_observation(&peer, &execution, &second) else {
        return Err("cumulative output bytes exceeded the accepted quota".into());
    };
    assert!(matches!(
        error,
        milkdrift_persistence::PersistenceError::Bounds {
            location: "peer_execution_artifact_bytes",
            ..
        }
    ));
    let snapshot = store
        .peer_execution(&peer, &execution)?
        .ok_or("execution missing after bounded refusal")?;
    let PeerExecutionSnapshot::Hot(record) = snapshot else {
        return Err("active execution archived unexpectedly".into());
    };
    assert_eq!(record.last_observation_sequence, 1);
    assert_eq!(record.accounting.artifact_bytes, 1_048_576);
    Ok(())
}

#[test]
fn archived_tombstones_reclaim_hot_capacity_and_preserve_replay_conflict_and_history_truth()
-> TestResult {
    let root = tempfile::tempdir()?;
    let store = Arc::new(RedbStore::open(root.path())?);
    let peer = PeerId::new("peer-retention")?;
    let target = PeerId::new("peer-retention-target")?;
    let descriptor = descriptor()?;
    let catalog = milkdrift_peer_protocol::CatalogSnapshot::new(
        1,
        1,
        now().saturating_add(60_000),
        Vec::new(),
    )?;
    configure_store(&store, &peer, &catalog.digest, 1)?;
    let first = request(
        &peer,
        &target,
        &descriptor,
        1,
        catalog.digest.clone(),
        "request-retention-first",
        "invocation-retention-first",
    )?;
    let first_execution = PeerExecutionId::new("execution-retention-first")?;
    admit(&store, &peer, &first, &first_execution, 1)?;
    let artifact_bytes = b"shared core bytes outlive peer observation history".to_vec();
    let artifact_digest = ContentDigest::for_bytes(&artifact_bytes);
    let first_artifact = ArtifactReference::new(
        ArtifactId::new("retention-artifact-first")?,
        artifact_digest,
        MediaType::new("application/octet-stream")?,
        u64::try_from(artifact_bytes.len())?,
    );
    let second_artifact = ArtifactReference::new(
        ArtifactId::new("retention-artifact-deduplicated")?,
        artifact_digest,
        MediaType::new("application/octet-stream")?,
        u64::try_from(artifact_bytes.len())?,
    );
    let artifact_store = CorePeerArtifactStore::new(store.clone(), 1_048_576, 2_097_152)?;
    for (ordinal, artifact, execution) in [
        (1_u8, first_artifact.clone(), first_execution.clone()),
        (
            2_u8,
            second_artifact.clone(),
            PeerExecutionId::new("execution-retention-artifact-consumer")?,
        ),
    ] {
        let offer = ArtifactMetadataOffer {
            transfer: TransferId::new(format!("transfer-retention-{ordinal}"))?,
            direction: ArtifactTransferDirection::Upload,
            artifact,
            sensitivity: ArtifactSensitivity::Internal,
            retention: ArtifactRetention::Indefinite,
            provenance: ArtifactProvenance::new(
                CausalReference::External {
                    source: CausalId::new(format!("retention-source-{ordinal}"))?,
                },
                Vec::new(),
            )?,
            source_peer: peer.clone(),
            execution,
            expires_at_unix_ms: now().saturating_add(60_000),
        };
        assert!(matches!(
            artifact_store.negotiate(&peer, &offer, 1_048_576)?,
            ArtifactTransferDecision::Transfer { .. }
        ));
        assert_eq!(
            artifact_store.write_chunk(
                &peer,
                &ArtifactChunk {
                    transfer: offer.transfer,
                    offset: 0,
                    bytes: artifact_bytes.clone(),
                    final_chunk: true,
                },
                1_048_576,
            )?,
            ArtifactTransferDecision::AlreadyPresent
        );
    }
    let worker = WorkerId::new("retention-worker")?;
    claim(&store, &worker)?;
    store.append_peer_observation(
        &peer,
        &first_execution,
        &terminal_observation(&first, &first_execution, 1, TerminalStatus::Success)?,
    )?;
    assert_eq!(
        store.peer_execution_status()?,
        milkdrift_persistence::PeerExecutionStatus {
            hot_terminal: 1,
            ..Default::default()
        }
    );

    let archived_at = now().saturating_add(100);
    let page = store.archive_peer_executions(&PeerRetentionRequest {
        terminal_before_or_at: TimestampMillis::new(archived_at),
        archived_at: TimestampMillis::new(archived_at),
        limit: PageSize::new(1)?,
    })?;
    assert_eq!(page.archived, 1);
    assert!(!page.more);
    let status = store.peer_execution_status()?;
    assert_eq!(status.active, 0);
    assert_eq!(status.hot_terminal, 0);
    assert_eq!(status.tombstones, 1);
    assert_eq!(status.archive_generation, 1);
    assert_eq!(status.last_archived_at_unix_ms, Some(archived_at));
    store.verify_peer_execution_integrity()?;

    assert!(matches!(
        store.admit_peer_execution(&PeerAdmission {
            owner_peer: &peer,
            request: &first,
            authority: &allowed_decision(&peer)?,
            execution: &first_execution,
            relationship_generation: 1,
            accepted_at_unix_ms: now(),
            maximum_global_active: 1,
            maximum_dispatch_queue: 1,
            maximum_hot_terminal_records: 1,
            archive_batch_size: 1,
            archive_terminal_before_or_at_unix_ms: archived_at,
        })?,
        PeerAdmissionOutcome::Replayed(PeerExecutionSnapshot::Archived(_))
    ));
    let conflict = request(
        &peer,
        &target,
        &descriptor,
        1,
        catalog.digest.clone(),
        first.request_id.as_str(),
        "invocation-retention-conflict",
    )?;
    assert!(matches!(
        store.admit_peer_execution(&PeerAdmission {
            owner_peer: &peer,
            request: &conflict,
            authority: &allowed_decision(&peer)?,
            execution: &PeerExecutionId::new("execution-retention-conflict")?,
            relationship_generation: 1,
            accepted_at_unix_ms: now(),
            maximum_global_active: 1,
            maximum_dispatch_queue: 1,
            maximum_hot_terminal_records: 1,
            archive_batch_size: 1,
            archive_terminal_before_or_at_unix_ms: archived_at,
        })?,
        PeerAdmissionOutcome::Conflict(PeerExecutionSnapshot::Archived(_))
    ));
    let observations = store.peer_observations(&peer, &first_execution, 0, PageSize::new(8)?)?;
    assert!(observations.observations.is_empty());
    assert!(matches!(
        observations.execution,
        PeerExecutionSnapshot::Archived(_)
    ));
    assert!(store.metadata(first_artifact.artifact())?.is_some());
    assert!(store.metadata(second_artifact.artifact())?.is_some());
    let retained_download = ArtifactMetadataOffer {
        transfer: TransferId::new("transfer-retention-download")?,
        direction: ArtifactTransferDirection::Download,
        artifact: first_artifact.clone(),
        sensitivity: ArtifactSensitivity::Internal,
        retention: ArtifactRetention::Indefinite,
        provenance: store
            .metadata(first_artifact.artifact())?
            .ok_or("retained core artifact metadata missing")?
            .provenance()
            .clone(),
        source_peer: target.clone(),
        execution: first_execution.clone(),
        expires_at_unix_ms: now().saturating_add(60_000),
    };
    artifact_store.negotiate(&peer, &retained_download, 1_048_576)?;
    assert_eq!(
        artifact_store
            .read_chunk(&peer, &retained_download.transfer, 0, 1_048_576)?
            .bytes,
        artifact_bytes
    );

    let second = request(
        &peer,
        &target,
        &descriptor,
        1,
        catalog.digest,
        "request-retention-second",
        "invocation-retention-second",
    )?;
    assert!(matches!(
        store.admit_peer_execution(&PeerAdmission {
            owner_peer: &peer,
            request: &second,
            authority: &allowed_decision(&peer)?,
            execution: &PeerExecutionId::new("execution-retention-second")?,
            relationship_generation: 1,
            accepted_at_unix_ms: now(),
            maximum_global_active: 1,
            maximum_dispatch_queue: 1,
            maximum_hot_terminal_records: 1,
            archive_batch_size: 1,
            archive_terminal_before_or_at_unix_ms: archived_at,
        })?,
        PeerAdmissionOutcome::Accepted(_)
    ));
    assert_eq!(store.peer_execution_status()?.active, 1);
    Ok(())
}

#[test]
fn archival_fault_boundaries_are_atomic_restart_safe_and_idempotent() -> TestResult {
    for (point, committed) in [
        (FaultPoint::AfterPeerTombstoneInsert, false),
        (FaultPoint::AfterPeerObservationCleanup, false),
        (FaultPoint::AfterPeerHotRemove, false),
        (FaultPoint::AfterPeerArchiveAccounting, false),
        (FaultPoint::BeforePeerArchiveCommit, false),
        (FaultPoint::AfterPeerArchiveCommit, true),
    ] {
        let root = tempfile::tempdir()?;
        let peer = PeerId::new(format!("peer-archive-fault-{point:?}"))?;
        let target = PeerId::new("peer-archive-fault-target")?;
        let descriptor = descriptor()?;
        let catalog = milkdrift_peer_protocol::CatalogSnapshot::new(
            1,
            1,
            now().saturating_add(60_000),
            Vec::new(),
        )?;
        let execution = PeerExecutionId::new("execution-archive-fault")?;
        let request = request(
            &peer,
            &target,
            &descriptor,
            1,
            catalog.digest.clone(),
            "request-archive-fault",
            "invocation-archive-fault",
        )?;
        {
            let store = RedbStore::open(root.path())?;
            configure_store(&store, &peer, &catalog.digest, 1)?;
            admit(&store, &peer, &request, &execution, 1)?;
            claim(&store, &WorkerId::new("archive-fault-worker")?)?;
            store.append_peer_observation(
                &peer,
                &execution,
                &terminal_observation(&request, &execution, 1, TerminalStatus::Success)?,
            )?;
        }
        let boundary = now().saturating_add(100);
        let store = RedbStore::open_with_config(
            RedbStoreConfig::new(root.path()).with_fault_injector(Arc::new(FailOnce::new(point))),
        )?;
        assert!(
            store
                .archive_peer_executions(&PeerRetentionRequest {
                    terminal_before_or_at: TimestampMillis::new(boundary),
                    archived_at: TimestampMillis::new(boundary),
                    limit: PageSize::new(1)?,
                })
                .is_err()
        );
        drop(store);
        let store = RedbStore::open(root.path())?;
        assert_eq!(
            store.peer_execution_status()?.hot_terminal,
            u64::from(!committed)
        );
        assert_eq!(
            store.peer_execution_status()?.tombstones,
            u64::from(committed)
        );
        assert_eq!(
            matches!(
                store.peer_execution(&peer, &execution)?,
                Some(PeerExecutionSnapshot::Archived(_))
            ),
            committed
        );
        store.verify_peer_execution_integrity()?;
        assert_eq!(
            store
                .archive_peer_executions(&PeerRetentionRequest {
                    terminal_before_or_at: TimestampMillis::new(boundary),
                    archived_at: TimestampMillis::new(boundary),
                    limit: PageSize::new(1)?,
                })?
                .archived,
            u32::from(!committed)
        );
        assert_eq!(
            store
                .archive_peer_executions(&PeerRetentionRequest {
                    terminal_before_or_at: TimestampMillis::new(boundary),
                    archived_at: TimestampMillis::new(boundary),
                    limit: PageSize::new(1)?,
                })?
                .archived,
            0
        );
        store.verify_peer_execution_integrity()?;
    }
    Ok(())
}

#[test]
fn uncertain_tombstone_replays_without_becoming_retryable_terminal_evidence() -> TestResult {
    let root = tempfile::tempdir()?;
    let store = RedbStore::open(root.path())?;
    let peer = PeerId::new("peer-uncertain-archive")?;
    let target = PeerId::new("peer-uncertain-target")?;
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
        "request-uncertain-archive",
        "invocation-uncertain-archive",
    )?;
    let execution = PeerExecutionId::new("execution-uncertain-archive")?;
    admit(&store, &peer, &request, &execution, 1)?;
    let worker = WorkerId::new("uncertain-archive-worker")?;
    let claimed = claim(&store, &worker)?;
    let generation = claimed.phase.claim().ok_or("claim absent")?.generation;
    enter(&store, &peer, &execution, &worker, generation)?;
    store.mark_peer_uncertain(
        &peer,
        &execution,
        &worker,
        generation,
        now(),
        "adapter result was lost after entry",
    )?;
    let boundary = now().saturating_add(100);
    store.archive_peer_executions(&PeerRetentionRequest {
        terminal_before_or_at: TimestampMillis::new(boundary),
        archived_at: TimestampMillis::new(boundary),
        limit: PageSize::new(1)?,
    })?;
    let replay = store.admit_peer_execution(&PeerAdmission {
        owner_peer: &peer,
        request: &request,
        authority: &allowed_decision(&peer)?,
        execution: &execution,
        relationship_generation: 1,
        accepted_at_unix_ms: now(),
        maximum_global_active: 1,
        maximum_dispatch_queue: 1,
        maximum_hot_terminal_records: 1,
        archive_batch_size: 1,
        archive_terminal_before_or_at_unix_ms: boundary,
    })?;
    let PeerAdmissionOutcome::Replayed(PeerExecutionSnapshot::Archived(tombstone)) = replay else {
        return Err("uncertain archived request did not resolve its tombstone".into());
    };
    assert!(matches!(
        tombstone.disposition,
        milkdrift_persistence::PeerArchivedDisposition::Uncertain { .. }
    ));
    assert_eq!(tombstone.last_observation_sequence, 0);
    assert_eq!(store.peer_execution_status()?.tombstones, 1);
    store.verify_peer_execution_integrity()?;
    Ok(())
}

#[test]
fn peer_integrity_verification_detects_tombstone_index_and_counter_corruption() -> TestResult {
    enum Corruption {
        Tombstone,
        Location,
        Accounting,
    }

    for (ordinal, corruption) in [
        Corruption::Tombstone,
        Corruption::Location,
        Corruption::Accounting,
    ]
    .into_iter()
    .enumerate()
    {
        let root = tempfile::tempdir()?;
        let peer = PeerId::new(format!("peer-integrity-{ordinal}"))?;
        let target = PeerId::new("peer-integrity-target")?;
        let descriptor = descriptor()?;
        let catalog = milkdrift_peer_protocol::CatalogSnapshot::new(
            1,
            1,
            now().saturating_add(60_000),
            Vec::new(),
        )?;
        let execution = PeerExecutionId::new(format!("execution-integrity-{ordinal}"))?;
        let invocation = request(
            &peer,
            &target,
            &descriptor,
            1,
            catalog.digest.clone(),
            &format!("request-integrity-{ordinal}"),
            &format!("invocation-integrity-{ordinal}"),
        )?;
        {
            let store = RedbStore::open(root.path())?;
            configure_store(&store, &peer, &catalog.digest, 1)?;
            admit(&store, &peer, &invocation, &execution, 1)?;
            claim(
                &store,
                &WorkerId::new(format!("integrity-worker-{ordinal}"))?,
            )?;
            store.append_peer_observation(
                &peer,
                &execution,
                &terminal_observation(&invocation, &execution, 1, TerminalStatus::Success)?,
            )?;
            let boundary = now().saturating_add(100);
            store.archive_peer_executions(&PeerRetentionRequest {
                terminal_before_or_at: TimestampMillis::new(boundary),
                archived_at: TimestampMillis::new(boundary),
                limit: PageSize::new(1)?,
            })?;
            store.verify_peer_execution_integrity()?;
        }

        let database = Database::open(root.path().join("milkdrift.redb"))?;
        let write = database.begin_write()?;
        match corruption {
            Corruption::Tombstone => {
                let mut table = write.open_table(PEER_EXECUTION_TOMBSTONES)?;
                let mut bytes = table
                    .get(execution.as_str())?
                    .ok_or("tombstone row missing")?
                    .value()
                    .to_vec();
                let last = bytes.last_mut().ok_or("tombstone row empty")?;
                *last ^= 1;
                table.insert(execution.as_str(), bytes.as_slice())?;
            }
            Corruption::Location => {
                write
                    .open_table(PEER_EXECUTION_LOCATIONS)?
                    .insert(execution.as_str(), 1)?;
            }
            Corruption::Accounting => {
                let mut table = write.open_table(PEER_EXECUTION_ACCOUNTING)?;
                let mut bytes = table
                    .get("global")?
                    .ok_or("global peer accounting row missing")?
                    .value()
                    .to_vec();
                let last = bytes.last_mut().ok_or("global peer accounting row empty")?;
                *last ^= 1;
                table.insert("global", bytes.as_slice())?;
            }
        }
        write.commit()?;
        drop(database);

        if let Ok(store) = RedbStore::open(root.path()) {
            assert!(
                store.verify_peer_execution_integrity().is_err(),
                "peer integrity accepted corruption variant {ordinal}"
            );
        }
    }
    Ok(())
}

#[test]
fn peer_integrity_requires_accounting_for_every_active_peer() -> TestResult {
    let root = tempfile::tempdir()?;
    let peer = PeerId::new("peer-integrity-missing-accounting")?;
    let target = PeerId::new("peer-integrity-missing-accounting-target")?;
    let descriptor = descriptor()?;
    let catalog = milkdrift_peer_protocol::CatalogSnapshot::new(
        1,
        1,
        now().saturating_add(60_000),
        Vec::new(),
    )?;
    let execution = PeerExecutionId::new("execution-integrity-missing-accounting")?;
    let invocation = request(
        &peer,
        &target,
        &descriptor,
        1,
        catalog.digest.clone(),
        "request-integrity-missing-accounting",
        "invocation-integrity-missing-accounting",
    )?;
    {
        let store = RedbStore::open(root.path())?;
        configure_store(&store, &peer, &catalog.digest, 1)?;
        admit(&store, &peer, &invocation, &execution, 1)?;
        store.verify_peer_execution_integrity()?;
    }

    let database = Database::open(root.path().join("milkdrift.redb"))?;
    let write = database.begin_write()?;
    let removed = {
        let mut accounting = write.open_table(PEER_EXECUTION_ACCOUNTING)?;
        accounting.remove(peer.as_str())?.is_some()
    };
    assert!(removed, "per-peer accounting row was absent");
    write.commit()?;
    drop(database);

    let store = RedbStore::open(root.path())?;
    assert!(
        store.verify_peer_execution_integrity().is_err(),
        "peer integrity accepted an active execution without per-peer accounting"
    );
    Ok(())
}

#[test]
fn checksum_valid_peer_primary_and_tombstone_fact_corruption_is_rejected() -> TestResult {
    let root = tempfile::tempdir()?;
    let peer = PeerId::new("peer-fact-corruption")?;
    let target = PeerId::new("peer-fact-corruption-target")?;
    let descriptor = descriptor()?;
    let catalog = milkdrift_peer_protocol::CatalogSnapshot::new(
        1,
        1,
        now().saturating_add(60_000),
        Vec::new(),
    )?;
    let request = request_with_input_artifact(
        &peer,
        &target,
        &descriptor,
        1,
        catalog.digest.clone(),
        "request-fact-corruption",
        "invocation-fact-corruption",
        10,
    )?;
    let execution = PeerExecutionId::new("execution-fact-corruption")?;
    let record = {
        let store = RedbStore::open(root.path())?;
        configure_store(&store, &peer, &catalog.digest, 1)?;
        admit(&store, &peer, &request, &execution, 1)?;
        let snapshot = store
            .peer_execution(&peer, &execution)?
            .ok_or("fact-corruption record missing")?;
        let PeerExecutionSnapshot::Hot(record) = snapshot else {
            return Err("fresh fact-corruption record was archived".into());
        };
        *record
    };

    let mut invalid_records = Vec::new();
    let mut invalid = record.clone();
    invalid.schema_version = 0;
    invalid_records.push(invalid);
    let mut invalid = record.clone();
    invalid.relationship_generation = 0;
    invalid_records.push(invalid);
    let mut invalid = record.clone();
    invalid.acceptance_sequence = 0;
    invalid_records.push(invalid);
    let mut invalid = record.clone();
    invalid.accepted_at_unix_ms = 0;
    invalid_records.push(invalid);
    let mut invalid = record.clone();
    invalid.revision = 0;
    invalid_records.push(invalid);
    let mut invalid = record.clone();
    invalid.accounting.observations = 1;
    invalid_records.push(invalid);
    let mut invalid = record.clone();
    invalid.last_observation_sequence = 101;
    invalid.accounting.observations = 101;
    invalid_records.push(invalid);
    let mut invalid = record.clone();
    invalid.accounting.artifact_bytes = invalid.request.limits.artifact_bytes + 1;
    invalid_records.push(invalid);
    let mut invalid = record.clone();
    invalid.accounting.artifact_bytes = 0;
    invalid_records.push(invalid);
    let mut invalid = record.clone();
    invalid.observation_digest = "not-a-digest".to_owned();
    invalid_records.push(invalid);

    for invalid in invalid_records {
        overwrite_peer_document(
            root.path(),
            PEER_EXECUTIONS,
            execution.as_str(),
            "peer execution",
            &invalid,
        )?;
        assert_peer_integrity_refuses(root.path(), "invalid hot execution facts")?;
        overwrite_peer_document(
            root.path(),
            PEER_EXECUTIONS,
            execution.as_str(),
            "peer execution",
            &record,
        )?;
    }

    let mut legacy = record.clone();
    legacy.schema_version = 2;
    legacy.accounting.artifact_bytes = 0;
    overwrite_peer_document(
        root.path(),
        PEER_EXECUTIONS,
        execution.as_str(),
        "peer execution",
        &legacy,
    )?;
    let legacy_store = RedbStore::open(root.path())?;
    legacy_store.verify_peer_execution_integrity()?;
    drop(legacy_store);
    overwrite_peer_document(
        root.path(),
        PEER_EXECUTIONS,
        execution.as_str(),
        "peer execution",
        &record,
    )?;

    let tombstone = {
        let store = RedbStore::open(root.path())?;
        let worker = WorkerId::new("fact-corruption-worker")?;
        let _ = claim(&store, &worker)?;
        let boundary = now().saturating_add(100);
        let mut terminal = terminal_observation(&request, &execution, 1, TerminalStatus::Success)?;
        terminal.observed_at_unix_ms = boundary;
        store.append_peer_observation(&peer, &execution, &terminal)?;
        store.archive_peer_executions(&PeerRetentionRequest {
            terminal_before_or_at: TimestampMillis::new(boundary),
            archived_at: TimestampMillis::new(boundary),
            limit: PageSize::new(1)?,
        })?;
        let snapshot = store
            .peer_execution(&peer, &execution)?
            .ok_or("fact-corruption tombstone missing")?;
        let PeerExecutionSnapshot::Archived(tombstone) = snapshot else {
            return Err("fact-corruption execution did not archive".into());
        };
        *tombstone
    };

    for invalid in invalid_tombstones(&tombstone, &request)? {
        overwrite_peer_document(
            root.path(),
            PEER_EXECUTION_TOMBSTONES,
            execution.as_str(),
            "peer execution tombstone",
            &invalid,
        )?;
        assert_peer_integrity_refuses(root.path(), "invalid archived execution facts")?;
        overwrite_peer_document(
            root.path(),
            PEER_EXECUTION_TOMBSTONES,
            execution.as_str(),
            "peer execution tombstone",
            &tombstone,
        )?;
    }
    let mut exact_uncertainty_bound = tombstone.clone();
    exact_uncertainty_bound.disposition = PeerArchivedDisposition::Uncertain {
        uncertain_at_unix_ms: tombstone.archived_at_unix_ms,
        reason: "x".repeat(2_048),
    };
    overwrite_peer_document(
        root.path(),
        PEER_EXECUTION_TOMBSTONES,
        execution.as_str(),
        "peer execution tombstone",
        &exact_uncertainty_bound,
    )?;
    let store = RedbStore::open(root.path())?;
    store.verify_peer_execution_integrity()?;
    drop(store);

    let mut exact_cancellation = tombstone.clone();
    exact_cancellation.cancellation = Some(valid_cancellation(&execution)?);
    overwrite_peer_document(
        root.path(),
        PEER_EXECUTION_TOMBSTONES,
        execution.as_str(),
        "peer execution tombstone",
        &exact_cancellation,
    )?;
    let store = RedbStore::open(root.path())?;
    store.verify_peer_execution_integrity()?;
    Ok(())
}
