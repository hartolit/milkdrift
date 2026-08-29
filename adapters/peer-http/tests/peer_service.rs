//! Durable redb acceptance, fixed workers, recovery, cancellation and core artifacts.

use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{
        Arc, Barrier,
        atomic::{AtomicUsize, Ordering},
    },
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use milkdrift_authority::{
    ActorRef, AuthorityBudget, AuthorityDecisionSnapshot, AuthorityExecutionProvenance,
    AuthorityOperation, AuthorityRequest, BoundaryTimeMillis, DecisionId, DecisionReasonCode,
    GrantDigest, GrantId, PeerId, PolicyId, RequestedResourceFacts, SensitiveSecret,
};
use milkdrift_blueprint::{NodeId, RevisionId};
use milkdrift_capability::{
    AdmissionConstraints, BoundedJson, CancellationAcknowledgement, CancellationBehavior,
    CancellationRequest, CapabilityCategory, CapabilityDescriptor, CapabilityId,
    CapabilityObservation, DescriptorBuilder, IdempotencyBehavior, InvocationEvent,
    InvocationEventKind, InvocationId, InvocationRequest, InvocationTerminal, Locality,
    OperationContract, OperationId, ResolvedCapabilitySnapshot, SchemaContract, SchemaId,
    SideEffectClass, StreamingMode, TerminalStatus, TrustZone,
};
use milkdrift_capability_host::{
    AdapterError, AdapterInvocation, AdapterReporter, CapabilityAdapter, CapabilityHost,
    CapabilitySelectionPolicy, HostConfig,
};
use milkdrift_peer_http::{
    CorePeerArtifactStore, InsecureLoopbackMode, PeerArtifactStore, PeerClientConfig,
    PeerRelationship, PeerServerConfig, PeerService, PeerWorkerConfig, SystemPeerClock,
};
use milkdrift_peer_protocol::{
    ArtifactChunk, ArtifactMetadataOffer, ArtifactTransferDecision, ArtifactTransferDirection,
    CatalogDigest, DelegatedAuthorization, DelegationRef, ExecutionLimits, HardLimits,
    HeartbeatLease, InvocationAcceptance, ObservationCategory, PeerAction, PeerAuthority,
    PeerCancellationRequest, PeerExecutionId, PeerInvocationRequest, PeerObservation,
    PeerRequestId, ProtocolVersionRange, SessionId, TransferId,
};
use milkdrift_persistence::{
    ArtifactStore, PageSize, PeerAdmission, PeerAdmissionOutcome, PeerAdmissionRejection,
    PeerCatalogState, PeerClaimOutcome, PeerDispatchClaimRequest, PeerExecutionPhase,
    PeerExecutionStore, PeerRelationshipState, PeerRetentionRequest, TimestampMillis, WorkerId,
};
use milkdrift_redb_store::{
    FaultInjector, FaultPoint, RedbStore, RedbStoreConfig, injected_failure,
};
use milkdrift_workspace::{
    ArtifactId, ArtifactProvenance, ArtifactReference, ArtifactRetention, ArtifactSensitivity,
    CausalId, CausalReference, ContentDigest, MediaType,
};
use url::Url;

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

struct FailOnce {
    point: FaultPoint,
    remaining: AtomicUsize,
}

impl FailOnce {
    fn new(point: FaultPoint) -> Self {
        Self {
            point,
            remaining: AtomicUsize::new(1),
        }
    }
}

impl FaultInjector for FailOnce {
    fn check(&self, point: FaultPoint) -> Result<(), milkdrift_persistence::PersistenceError> {
        if point == self.point && self.remaining.swap(0, Ordering::SeqCst) == 1 {
            Err(injected_failure(point))
        } else {
            Ok(())
        }
    }
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(1, |duration| {
            u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
        })
}

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
                maximum_records: 10,
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
            maximum_records: 10,
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
            maximum_records: 10,
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
                maximum_records: 10,
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
            maximum_records: 10,
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
    claim_store.mark_peer_entered(&peer, &claim_execution, &worker, claim.generation, now())?;
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
    store.mark_peer_entered(&peer, &execution, &worker, second_claim.generation, now())?;
    let recovery = store.recover_peer_claims(now(), PageSize::new(8)?)?;
    assert_eq!(recovery.uncertain, 1);
    let uncertain = store
        .peer_execution(&peer, &execution)?
        .ok_or("execution missing")?;
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
            .mark_peer_entered(&peer, &execution, &worker, 2, now())
            .is_err()
    );
    let recovered = store.recover_peer_claims(now(), PageSize::new(8)?)?;
    assert_eq!(recovered.requeued, 1);
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
    store.mark_peer_entered(&peer, &entered_execution, &worker, generation, now())?;
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
            maximum_records: 1_000,
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
    store.mark_peer_entered(&peer, &execution, &worker, generation, now())?;
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
    assert_eq!(resumed.record.accounting.observations, 100);
    Ok(())
}

#[test]
fn fixed_worker_owner_bounds_execution_and_shutdown_joins() -> TestResult {
    let root = tempfile::tempdir()?;
    let store = Arc::new(RedbStore::open(root.path())?);
    let active = Arc::new(AtomicUsize::new(0));
    let maximum = Arc::new(AtomicUsize::new(0));
    let (host, descriptor) = host_with_adapter(Arc::new(TerminalAdapter {
        capability: CapabilityId::new("test-capability")?,
        delay: Duration::from_millis(20),
        active: active.clone(),
        maximum: maximum.clone(),
    }))?;
    let peer = PeerId::new("peer-a")?;
    let target = PeerId::new("peer-b")?;
    let service = PeerService::new(
        server_config(peer.clone(), target.clone(), 2, 16)?,
        host,
        store,
        Arc::new(SystemPeerClock),
    )?;
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
    let shutdown = service.shutdown_workers(Duration::from_secs(2));
    assert!(shutdown.clean);
    assert_eq!(shutdown.joined, 2);
    Ok(())
}

#[test]
fn core_artifact_transfer_preserves_metadata_provenance_resumes_and_reads_outbound() -> TestResult {
    let root = tempfile::tempdir()?;
    let peer = PeerId::new("peer-a")?;
    let serving = PeerId::new("peer-b")?;
    let execution = PeerExecutionId::new("execution-artifact")?;
    let bytes = b"verified ordinary core artifact".to_vec();
    let reference = ArtifactReference::new(
        ArtifactId::new("peer-imported-artifact")?,
        ContentDigest::for_bytes(&bytes),
        MediaType::new("application/octet-stream")?,
        u64::try_from(bytes.len())?,
    );
    let provenance = ArtifactProvenance::new(
        CausalReference::External {
            source: CausalId::new("remote-source")?,
        },
        Vec::new(),
    )?;
    let offer = ArtifactMetadataOffer {
        transfer: TransferId::new("transfer-core")?,
        direction: ArtifactTransferDirection::Upload,
        artifact: reference.clone(),
        sensitivity: ArtifactSensitivity::Internal,
        retention: ArtifactRetention::Indefinite,
        provenance: provenance.clone(),
        source_peer: peer.clone(),
        execution: execution.clone(),
        expires_at_unix_ms: now().saturating_add(60_000),
    };

    let core = Arc::new(RedbStore::open(root.path())?);
    let transfer = CorePeerArtifactStore::new(core.clone(), 1_048_576, 2_097_152)?;
    assert!(
        transfer
            .negotiate(&peer, &offer, u64::try_from(bytes.len())?.saturating_sub(1),)
            .is_err()
    );
    assert!(matches!(
        transfer.negotiate(&peer, &offer, 1_048_576)?,
        ArtifactTransferDecision::Transfer { next_offset: 0, .. }
    ));
    transfer.write_chunk(
        &peer,
        &ArtifactChunk {
            transfer: offer.transfer.clone(),
            offset: 0,
            bytes: bytes[..8].to_vec(),
            final_chunk: false,
        },
        1_048_576,
    )?;
    assert!(core.metadata(reference.artifact())?.is_none());
    drop(transfer);
    drop(core);

    let core = Arc::new(RedbStore::open(root.path())?);
    let transfer = CorePeerArtifactStore::new(core.clone(), 1_048_576, 2_097_152)?;
    assert!(matches!(
        transfer.negotiate(&peer, &offer, 1_048_576)?,
        ArtifactTransferDecision::Transfer { next_offset: 8, .. }
    ));
    assert_eq!(
        transfer.write_chunk(
            &peer,
            &ArtifactChunk {
                transfer: offer.transfer.clone(),
                offset: 8,
                bytes: bytes[8..].to_vec(),
                final_chunk: true,
            },
            1_048_576,
        )?,
        ArtifactTransferDecision::AlreadyPresent
    );
    let metadata = core
        .metadata(reference.artifact())?
        .ok_or("metadata missing")?;
    assert_eq!(metadata.sensitivity(), ArtifactSensitivity::Internal);
    assert_eq!(metadata.retention(), &ArtifactRetention::Indefinite);
    assert_eq!(metadata.provenance().producer(), provenance.producer());
    assert_eq!(metadata.provenance().causes().len(), 1);
    assert_eq!(
        transfer.negotiate(&peer, &offer, 1_048_576)?,
        ArtifactTransferDecision::AlreadyPresent
    );

    let download = ArtifactMetadataOffer {
        transfer: TransferId::new("transfer-download")?,
        direction: ArtifactTransferDirection::Download,
        artifact: reference,
        sensitivity: metadata.sensitivity(),
        retention: metadata.retention().clone(),
        provenance: metadata.provenance().clone(),
        source_peer: serving,
        execution,
        expires_at_unix_ms: now().saturating_add(60_000),
    };
    transfer.negotiate(&peer, &download, 1_048_576)?;
    let read = transfer.read_chunk(&peer, &download.transfer, 0, 1_048_576)?;
    assert_eq!(read.bytes, bytes);
    assert!(read.final_chunk);

    let corrupt_bytes = b"verified ordinary core artifacU".to_vec();
    assert_eq!(corrupt_bytes.len(), bytes.len());
    let corrupt_reference = ArtifactReference::new(
        ArtifactId::new("peer-corrupt-artifact")?,
        ContentDigest::for_bytes(&bytes),
        MediaType::new("application/octet-stream")?,
        u64::try_from(bytes.len())?,
    );
    let corrupt_offer = ArtifactMetadataOffer {
        transfer: TransferId::new("transfer-corrupt")?,
        direction: ArtifactTransferDirection::Upload,
        artifact: corrupt_reference.clone(),
        sensitivity: ArtifactSensitivity::Internal,
        retention: ArtifactRetention::Indefinite,
        provenance,
        source_peer: peer.clone(),
        execution: PeerExecutionId::new("execution-corrupt-artifact")?,
        expires_at_unix_ms: now().saturating_add(60_000),
    };
    transfer.negotiate(&peer, &corrupt_offer, 1_048_576)?;
    assert!(
        transfer
            .write_chunk(
                &peer,
                &ArtifactChunk {
                    transfer: corrupt_offer.transfer.clone(),
                    offset: 0,
                    bytes: corrupt_bytes,
                    final_chunk: true,
                },
                1_048_576,
            )
            .is_err()
    );
    assert!(core.metadata(corrupt_reference.artifact())?.is_none());
    transfer.abort(&peer, &corrupt_offer.transfer)?;
    assert!(!root.path().join("peer-artifacts-v1").exists());
    assert!(!root.path().join("peer-executions-v1").exists());
    Ok(())
}

fn descriptor() -> TestResult<CapabilityDescriptor> {
    let schema = || {
        SchemaContract::new(
            SchemaId::new("test.value")?,
            1,
            BoundedJson::new(serde_json::json!({"type": "object"}))?,
        )
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
        CapabilityId::new("test-capability")?,
        1,
        CapabilityCategory::Tool,
        AdmissionConstraints::new(16, 0)?,
        Locality::Local,
    )
    .operations(BTreeMap::from([(
        OperationId::new("test.execute")?,
        operation,
    )]))
    .build()?)
}

struct TerminalAdapter {
    capability: CapabilityId,
    delay: Duration,
    active: Arc<AtomicUsize>,
    maximum: Arc<AtomicUsize>,
}

impl CapabilityAdapter for TerminalAdapter {
    fn execute(
        &self,
        invocation: &AdapterInvocation<'_>,
        reporter: &dyn AdapterReporter,
    ) -> Result<(), AdapterError> {
        let active = self.active.fetch_add(1, Ordering::SeqCst) + 1;
        self.maximum.fetch_max(active, Ordering::SeqCst);
        thread::sleep(self.delay);
        let result = reporter.invocation(
            InvocationEvent::new(
                invocation.request().invocation().clone(),
                1,
                InvocationEventKind::Terminal {
                    terminal: InvocationTerminal::new(
                        TerminalStatus::Success,
                        Vec::new(),
                        None,
                        None,
                        SideEffectClass::ReadOnly,
                    )
                    .map_err(|error| AdapterError::external_failure(error.to_string()))?,
                },
            )
            .map_err(|error| AdapterError::external_failure(error.to_string()))?,
        );
        self.active.fetch_sub(1, Ordering::SeqCst);
        result
    }

    fn cancel(
        &self,
        request: &CancellationRequest,
    ) -> Result<CancellationAcknowledgement, AdapterError> {
        CancellationAcknowledgement::new(
            request.invocation().clone(),
            request.request_sequence(),
            true,
            true,
            None,
        )
        .map_err(|error| AdapterError::external_failure(error.to_string()))
    }

    fn health(&self, observed_at_unix_ms: u64) -> Result<CapabilityObservation, AdapterError> {
        CapabilityObservation::new(
            self.capability.clone(),
            observed_at_unix_ms,
            true,
            0,
            "healthy",
        )
        .map_err(|error| AdapterError::external_failure(error.to_string()))
    }
}

fn host_with_adapter(
    adapter: Arc<dyn CapabilityAdapter>,
) -> TestResult<(CapabilityHost, CapabilityDescriptor)> {
    let host = CapabilityHost::new(
        HostConfig {
            max_registrations: 32,
            max_generations_per_capability: 4,
            max_concurrent_per_generation: 16,
            observation_stale_after_ms: 60_000,
        },
        CapabilitySelectionPolicy::priorities(BTreeMap::new()),
    )?;
    let descriptor = descriptor()?;
    host.register(
        descriptor.clone(),
        adapter,
        Some(CapabilityObservation::new(
            descriptor.identity().clone(),
            now(),
            true,
            0,
            "healthy",
        )?),
    )?;
    Ok((host, descriptor))
}

fn relationship(remote_peer: PeerId, maximum_concurrent: u16) -> TestResult<PeerRelationship> {
    let limits = ExecutionLimits {
        artifact_bytes: 1_048_576,
        duration_ms: 30_000,
        cost_micros: 0,
        observations: 100,
    };
    Ok(PeerRelationship {
        remote_peer,
        bearer_credential: Arc::new(SensitiveSecret::new(b"peer-secret".to_vec())),
        versions: ProtocolVersionRange::default(),
        authority: PeerAuthority {
            actions: BTreeSet::from([
                PeerAction::ReadCatalog,
                PeerAction::Invoke,
                PeerAction::Cancel,
                PeerAction::ArtifactUpload,
                PeerAction::ArtifactDownload,
            ]),
        },
        capability_allow: BTreeSet::from([CapabilityId::new("test-capability")?]),
        capability_deny: BTreeSet::new(),
        operation_allow: BTreeSet::from([OperationId::new("test.execute")?]),
        maximum_side_effect: SideEffectClass::ReadOnly,
        execution_filesystem: Vec::new(),
        execution_network_profiles: BTreeSet::new(),
        execution_network_destinations: BTreeSet::new(),
        execution_secrets: BTreeSet::new(),
        execution_limits: limits,
        maximum_concurrent,
        maximum_requests_per_minute: 10_000,
        maximum_artifact_bytes: limits.artifact_bytes,
        catalog_ttl_ms: 60_000,
        trust_zone: TrustZone::new("trusted-peer")?,
        delegation: DelegationRef::new("delegation-configured")?,
        revocation_generation: 0,
        expires_at_unix_ms: now().saturating_add(600_000),
        enabled: true,
    })
}

fn server_config(
    remote: PeerId,
    local: PeerId,
    threads: u16,
    maximum: u32,
) -> TestResult<PeerServerConfig> {
    Ok(PeerServerConfig {
        local_peer: local,
        session: SessionId::new("session-test")?,
        versions: ProtocolVersionRange::default(),
        limits: HardLimits::default(),
        lease: HeartbeatLease {
            heartbeat_ms: 100,
            idle_timeout_ms: 1_000,
            execution_lease_ms: 5_000,
        },
        relationships: vec![relationship(remote, u16::try_from(maximum)?)?],
        workers: PeerWorkerConfig {
            threads,
            maximum_global_active: maximum,
            maximum_dispatch_queue: maximum,
            maximum_records: 1_000,
            recovery_page: 32,
            poll_interval: Duration::from_millis(5),
        },
    })
}

fn configure_store(
    store: &RedbStore,
    peer: &PeerId,
    digest: &CatalogDigest,
    maximum: u32,
) -> TestResult {
    store.configure_peer_relationship(&PeerRelationshipState {
        peer: peer.clone(),
        generation: 1,
        enabled: true,
        expires_at_unix_ms: now().saturating_add(600_000),
        maximum_active: maximum,
    })?;
    store.publish_peer_catalog(&PeerCatalogState {
        peer: peer.clone(),
        relationship_generation: 1,
        generation: 1,
        digest: digest.as_str().to_owned(),
        expires_at_unix_ms: now().saturating_add(60_000),
    })?;
    Ok(())
}

fn admit(
    store: &RedbStore,
    peer: &PeerId,
    request: &PeerInvocationRequest,
    execution: &PeerExecutionId,
    maximum: u32,
) -> TestResult {
    assert!(matches!(
        store.admit_peer_execution(&PeerAdmission {
            owner_peer: peer,
            request,
            authority: &allowed_decision(peer)?,
            execution,
            relationship_generation: 1,
            accepted_at_unix_ms: now(),
            maximum_global_active: maximum,
            maximum_dispatch_queue: maximum,
            maximum_records: 1_000,
        })?,
        PeerAdmissionOutcome::Accepted(_)
    ));
    Ok(())
}

fn claim(
    store: &RedbStore,
    worker: &WorkerId,
) -> TestResult<milkdrift_persistence::PeerExecutionRecord> {
    match store.claim_peer_dispatch(&PeerDispatchClaimRequest {
        worker,
        claimed_at_unix_ms: now(),
        lease_expires_at_unix_ms: now().saturating_add(30_000),
    })? {
        PeerClaimOutcome::Claimed(record) => Ok(record),
        other => Err(format!("expected claimed execution, got {other:?}").into()),
    }
}

fn allowed_decision(peer: &PeerId) -> TestResult<AuthorityDecisionSnapshot> {
    let mut resources = RequestedResourceFacts::empty();
    resources.peer = Some(peer.clone());
    resources.capability = Some(CapabilityId::new("test-capability")?);
    resources.capability_operation = Some(OperationId::new("test.execute")?);
    let request = AuthorityRequest {
        decision: DecisionId::new(format!("decision:{}", peer.as_str()))?,
        actor: ActorRef::new(format!("peer:{}", peer.as_str()))?,
        grant: GrantId::new(format!("grant:{}", peer.as_str()))?,
        grant_revision: 1,
        grant_digest: GrantDigest::new(format!("b3_{}", "0".repeat(64)))?,
        revocation_generation: 0,
        operation: AuthorityOperation::InvokePeerCapability,
        resources,
        budget: AuthorityBudget::default(),
        evaluated_at: BoundaryTimeMillis::new(now()),
        provenance: AuthorityExecutionProvenance {
            revision: Some(serde_json::from_value::<RevisionId>(serde_json::json!(
                format!("rev_{}", "1".repeat(64))
            ))?),
            node: Some(NodeId::new("node-peer-test")?),
            execution: Some("execution-peer-test".to_owned()),
            attempt: Some("attempt-peer-test".to_owned()),
            descriptor_revision: Some(1),
            peer: Some(peer.clone()),
            idempotency: Some(IdempotencyBehavior::CapabilityScoped),
        },
    };
    Ok(AuthorityDecisionSnapshot::from_evaluation(
        PolicyId::new("policy:peer-test")?,
        1,
        request,
        vec![DecisionReasonCode::Allowed],
        AuthorityBudget::default(),
        SideEffectClass::ReadOnly,
    )?)
}

#[allow(clippy::too_many_arguments)]
fn request(
    issuer: &PeerId,
    target: &PeerId,
    descriptor: &CapabilityDescriptor,
    catalog_generation: u64,
    catalog_digest: CatalogDigest,
    request_identity: &str,
    invocation_identity: &str,
) -> TestResult<PeerInvocationRequest> {
    let operation = OperationId::new("test.execute")?;
    let selection = ResolvedCapabilitySnapshot::from_descriptor(descriptor, &operation)?;
    let invocation = InvocationRequest::new(
        InvocationId::new(invocation_identity)?,
        descriptor.identity().clone(),
        operation.clone(),
        None,
        None,
        Vec::new(),
        BTreeMap::new(),
    )?;
    let request_id = PeerRequestId::new(request_identity)?;
    let deadline = now().saturating_add(120_000);
    let limits = ExecutionLimits {
        artifact_bytes: 1_048_576,
        duration_ms: 30_000,
        cost_micros: 0,
        observations: 100,
    };
    Ok(PeerInvocationRequest::new(
        request_id.clone(),
        catalog_generation,
        catalog_digest,
        selection,
        invocation,
        limits,
        deadline,
        DelegatedAuthorization {
            reference: DelegationRef::new("delegation-configured")?,
            issuer_peer: issuer.clone(),
            actor: ActorRef::new(format!("peer:{}", issuer.as_str()))?,
            target_peer: target.clone(),
            capability: descriptor.identity().clone(),
            operation,
            request: request_id,
            limits,
            expires_at_unix_ms: deadline,
            nonce: invocation_identity.to_owned(),
            provenance: milkdrift_peer_protocol::PeerExecutionProvenance {
                run: "run-peer-test".to_owned(),
                revision: format!("rev_{}", "1".repeat(64)),
                node: "node-peer-test".to_owned(),
                execution: "execution-peer-test".to_owned(),
                attempt: "attempt-peer-test".to_owned(),
            },
        },
    )?)
}

fn terminal_observation(
    request: &PeerInvocationRequest,
    execution: &PeerExecutionId,
    sequence: u64,
    status: TerminalStatus,
) -> TestResult<PeerObservation> {
    Ok(PeerObservation {
        execution: execution.clone(),
        sequence,
        category: ObservationCategory::Terminal,
        event: InvocationEvent::new(
            request.request.invocation().clone(),
            sequence,
            InvocationEventKind::Terminal {
                terminal: InvocationTerminal::new(
                    status,
                    Vec::new(),
                    None,
                    None,
                    SideEffectClass::ReadOnly,
                )?,
            },
        )?,
        observed_at_unix_ms: now(),
    })
}

fn progress_observation(
    request: &PeerInvocationRequest,
    execution: &PeerExecutionId,
    sequence: u64,
) -> TestResult<PeerObservation> {
    Ok(PeerObservation {
        execution: execution.clone(),
        sequence,
        category: ObservationCategory::Progress,
        event: InvocationEvent::new(
            request.request.invocation().clone(),
            sequence,
            InvocationEventKind::Progress {
                message: format!("bounded progress {sequence}"),
                completed_units: Some(sequence),
                total_units: Some(100),
            },
        )?,
        observed_at_unix_ms: now(),
    })
}
