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
fn admission_requires_a_live_exact_relationship_and_catalog() -> TestResult {
    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    enum Case {
        ExactExpiry,
        MissingRelationship,
        DisabledRelationship,
        RelationshipGeneration,
        ExpiredRelationship,
        MissingCatalog,
        StaleCatalogRelationship,
        CatalogGeneration,
        CatalogDigest,
        ExpiredCatalog,
    }

    for (case, rejection) in [
        (Case::ExactExpiry, None),
        (
            Case::MissingRelationship,
            Some(PeerAdmissionRejection::RelationshipUnavailable),
        ),
        (
            Case::DisabledRelationship,
            Some(PeerAdmissionRejection::RelationshipUnavailable),
        ),
        (
            Case::ExpiredRelationship,
            Some(PeerAdmissionRejection::RelationshipUnavailable),
        ),
        (
            Case::RelationshipGeneration,
            Some(PeerAdmissionRejection::RelationshipUnavailable),
        ),
        (
            Case::MissingCatalog,
            Some(PeerAdmissionRejection::CatalogUnavailable),
        ),
        (
            Case::StaleCatalogRelationship,
            Some(PeerAdmissionRejection::CatalogUnavailable),
        ),
        (
            Case::CatalogGeneration,
            Some(PeerAdmissionRejection::CatalogUnavailable),
        ),
        (
            Case::CatalogDigest,
            Some(PeerAdmissionRejection::CatalogUnavailable),
        ),
        (
            Case::ExpiredCatalog,
            Some(PeerAdmissionRejection::CatalogUnavailable),
        ),
    ] {
        let root = tempfile::tempdir()?;
        let store = RedbStore::open(root.path())?;
        let peer = PeerId::new("peer-admission-current")?;
        let target = PeerId::new("peer-admission-target")?;
        let catalog = milkdrift_peer_protocol::CatalogSnapshot::new(
            1,
            1,
            now().saturating_add(60_000),
            Vec::new(),
        )?;
        let requested_digest = if case == Case::CatalogDigest {
            milkdrift_peer_protocol::CatalogSnapshot::new(
                1,
                2,
                catalog.expires_at_unix_ms,
                Vec::new(),
            )?
            .digest
        } else {
            catalog.digest.clone()
        };
        let request = request(
            &peer,
            &target,
            &descriptor()?,
            if case == Case::CatalogGeneration {
                2
            } else {
                1
            },
            requested_digest,
            "request-admission-current",
            "invocation-admission-current",
        )?;
        let authority = allowed_decision(&peer)?;
        let boundary = now();
        let mut relationship = PeerRelationshipState {
            peer: peer.clone(),
            generation: 1,
            enabled: true,
            expires_at_unix_ms: boundary - u64::from(case == Case::ExpiredRelationship),
            maximum_active: 1,
        };
        store.set_peer_admission_open(true)?;
        if case != Case::MissingRelationship {
            store.configure_peer_relationship(&relationship)?;
        }
        if !matches!(case, Case::MissingRelationship | Case::MissingCatalog) {
            store.publish_peer_catalog(&PeerCatalogState {
                peer: peer.clone(),
                relationship_generation: 1,
                generation: 1,
                digest: catalog.digest.as_str().to_owned(),
                expires_at_unix_ms: boundary - u64::from(case == Case::ExpiredCatalog),
            })?;
        }
        let relationship_generation = match case {
            Case::DisabledRelationship | Case::StaleCatalogRelationship => {
                relationship.generation = 2;
                relationship.enabled = case != Case::DisabledRelationship;
                store.configure_peer_relationship(&relationship)?;
                2
            }
            Case::RelationshipGeneration => 2,
            _ => 1,
        };
        let execution = PeerExecutionId::new("execution-admission-current")?;
        let outcome = store.admit_peer_execution(&PeerAdmission {
            owner_peer: &peer,
            request: &request,
            authority: &authority,
            execution: &execution,
            relationship_generation,
            accepted_at_unix_ms: boundary,
            maximum_global_active: 1,
            maximum_dispatch_queue: 1,
            maximum_hot_terminal_records: 1,
            archive_batch_size: 1,
            archive_terminal_before_or_at_unix_ms: 1,
        })?;
        if let Some(expected) = rejection {
            assert!(
                matches!(outcome, PeerAdmissionOutcome::Rejected(actual) if actual == expected),
                "unexpected admission result for {case:?}: {outcome:?}"
            );
            assert!(store.peer_execution(&peer, &execution)?.is_none());
            let status = store.peer_execution_status()?;
            assert_eq!(status.active, 0);
            assert_eq!(status.dispatch_queued, 0);
        } else {
            assert!(matches!(outcome, PeerAdmissionOutcome::Accepted(_)));
            assert!(store.peer_execution(&peer, &execution)?.is_some());
            assert_eq!(store.peer_execution_status()?.active, 1);
        }
    }
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
fn request_index_with_one_mismatched_identity_is_rejected_as_corruption() -> TestResult {
    let root = tempfile::tempdir()?;
    let peer = PeerId::new("peer-request-index-corruption")?;
    let target = PeerId::new("peer-request-index-target")?;
    let descriptor = descriptor()?;
    let catalog = milkdrift_peer_protocol::CatalogSnapshot::new(
        1,
        now().saturating_sub(1),
        now().saturating_add(60_000),
        Vec::new(),
    )?;
    let first = request(
        &peer,
        &target,
        &descriptor,
        1,
        catalog.digest.clone(),
        "request-index-first",
        "invocation-index-first",
    )?;
    let second = request(
        &peer,
        &target,
        &descriptor,
        1,
        catalog.digest.clone(),
        "request-index-second",
        "invocation-index-second",
    )?;
    let first_execution = PeerExecutionId::new("execution-index-first")?;
    let second_execution = PeerExecutionId::new("execution-index-second")?;
    {
        let store = RedbStore::open(root.path())?;
        configure_store(&store, &peer, &catalog.digest, 2)?;
        admit(&store, &peer, &first, &first_execution, 2)?;
        admit(&store, &peer, &second, &second_execution, 2)?;
    }

    let database = Database::open(root.path().join("milkdrift.redb"))?;
    let write = database.begin_write()?;
    let first_key = {
        let by_request = write.open_table(PEER_EXECUTIONS_BY_REQUEST)?;
        by_request
            .iter()?
            .find_map(|row| {
                let (key, value) = row.ok()?;
                (value.value() == first_execution.as_str()).then(|| key.value().to_vec())
            })
            .ok_or("first peer request index row is absent")?
    };
    write
        .open_table(PEER_EXECUTIONS_BY_REQUEST)?
        .insert(first_key.as_slice(), second_execution.as_str())?;
    write.commit()?;
    drop(database);

    let reopened = RedbStore::open(root.path())?;
    let result = reopened.admit_peer_execution(&PeerAdmission {
        owner_peer: &peer,
        request: &first,
        authority: &allowed_decision(&peer)?,
        execution: &first_execution,
        relationship_generation: 1,
        accepted_at_unix_ms: now(),
        maximum_global_active: 2,
        maximum_dispatch_queue: 2,
        maximum_hot_terminal_records: 10,
        archive_batch_size: 2,
        archive_terminal_before_or_at_unix_ms: 1,
    });
    assert!(matches!(
        result,
        Err(PersistenceError::Storage {
            class: StorageFailureClass::Corruption,
            ..
        } | PersistenceError::Corruption(_))
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

#[path = "storage/retention.rs"]
mod retention;

use super::faults::FailOnce;
