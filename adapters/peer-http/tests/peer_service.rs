//! Durable acceptance, authenticated execution, and verified artifact tests.

use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use milkdrift_authority::{
    ActorRef, AuthorityBudget, CapabilityAuthorityScope, PeerId, SensitiveSecret,
};
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
    FilePeerArtifactStore, FilePeerExecutionStore, InsecureLoopbackMode, PeerArtifactError,
    PeerArtifactFaultInjector, PeerArtifactFaultPoint, PeerArtifactStore, PeerClientConfig,
    PeerExecutionStore, PeerHttpClient, PeerHttpError, PeerRegistry, PeerRelationship,
    PeerServerConfig, PeerService, PeerStoreError, PeerStoreFaultInjector, PeerStoreFaultPoint,
    StoreAcceptance, SystemPeerClock, peer_router,
};
use milkdrift_peer_protocol::{
    ArtifactChunk, ArtifactMetadataOffer, ArtifactTransferDecision, ArtifactTransferDirection,
    CancellationDisposition, DelegatedAuthorization, DelegationRef, ExecutionLimits,
    HandshakeRequest, HardLimits, HeartbeatLease, InvocationAcceptance, ObservationCategory,
    PeerAction, PeerAuthority, PeerCancellationRequest, PeerExecutionId, PeerInvocationRequest,
    PeerRequestId, ProtocolVersionRange, SessionId, TransferId,
};
use url::Url;

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

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
    assert!(
        config(
            "https://user:password@peer.example/",
            InsecureLoopbackMode::Disabled,
        )?
        .validate()
        .is_err()
    );
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
        AdmissionConstraints::new(4, 0)?,
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
}

impl CapabilityAdapter for TerminalAdapter {
    fn execute(
        &self,
        invocation: &AdapterInvocation<'_>,
        reporter: &dyn AdapterReporter,
    ) -> Result<(), AdapterError> {
        reporter.invocation(
            InvocationEvent::new(
                invocation.request().invocation().clone(),
                1,
                InvocationEventKind::Progress {
                    message: "remote progress".to_owned(),
                    completed_units: Some(1),
                    total_units: Some(1),
                },
            )
            .map_err(|error| AdapterError::external_failure(error.to_string()))?,
        )?;
        reporter.invocation(
            InvocationEvent::new(
                invocation.request().invocation().clone(),
                2,
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
        )
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

fn host_with_adapter() -> TestResult<(CapabilityHost, CapabilityDescriptor)> {
    let host = empty_host()?;
    let descriptor = descriptor()?;
    let capability = descriptor.identity().clone();
    host.register(
        descriptor.clone(),
        Arc::new(TerminalAdapter {
            capability: capability.clone(),
        }),
        Some(CapabilityObservation::new(
            capability,
            now(),
            true,
            0,
            "healthy",
        )?),
    )?;
    Ok((host, descriptor))
}

fn empty_host() -> TestResult<CapabilityHost> {
    Ok(CapabilityHost::new(
        HostConfig {
            max_registrations: 32,
            max_generations_per_capability: 4,
            max_concurrent_per_generation: 4,
            observation_stale_after_ms: 60_000,
        },
        CapabilitySelectionPolicy::new(
            CapabilityAuthorityScope::any(SideEffectClass::Unknown),
            AuthorityBudget {
                cost_minor: Some(u64::MAX),
                duration_ms: Some(u64::MAX),
                invocations: Some(u64::MAX),
                artifact_bytes: Some(u64::MAX),
                concurrency: Some(32),
            },
            BTreeMap::new(),
        ),
    )?)
}

#[derive(Default)]
struct CollectReporter {
    events: Mutex<Vec<InvocationEvent>>,
}

impl AdapterReporter for CollectReporter {
    fn invocation(&self, event: InvocationEvent) -> Result<(), AdapterError> {
        self.events
            .lock()
            .map_err(|_| AdapterError::external_failure("test reporter unavailable"))?
            .push(event);
        Ok(())
    }

    fn heartbeat(&self) -> Result<(), AdapterError> {
        Ok(())
    }
}

fn relationship(remote_peer: PeerId, expiry: u64) -> TestResult<PeerRelationship> {
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
        execution_limits: limits,
        maximum_concurrent: 4,
        maximum_requests_per_minute: 600,
        maximum_artifact_bytes: limits.artifact_bytes,
        catalog_ttl_ms: 10_000,
        trust_zone: TrustZone::new("trusted-peer")?,
        delegation: DelegationRef::new("delegation-configured")?,
        revocation_generation: 0,
        expires_at_unix_ms: expiry,
        enabled: true,
    })
}

#[test]
fn durable_acceptance_replays_and_conflicts_across_reopen() -> TestResult {
    let root = tempfile::tempdir()?;
    let store = FilePeerExecutionStore::open(root.path())?;
    let peer = PeerId::new("peer-a")?;
    let accepted_request = request(&peer, &PeerId::new("peer-b")?, descriptor()?, 1, None, None)?;
    let execution = PeerExecutionId::new("execution-1")?;
    assert!(matches!(
        store.accept(&peer, &accepted_request, &execution, 10, 20)?,
        StoreAcceptance::New(_)
    ));
    assert!(matches!(
        store.accept(&peer, &accepted_request, &execution, 11, 21)?,
        StoreAcceptance::Replay(_)
    ));
    drop(store);
    let reopened = FilePeerExecutionStore::open(root.path())?;
    assert!(
        reopened
            .by_request(&peer, &accepted_request.request_id)?
            .is_some()
    );
    let different = request(
        &peer,
        &PeerId::new("peer-b")?,
        descriptor()?,
        1,
        None,
        Some(now().saturating_add(90_000)),
    )?;
    assert!(matches!(
        reopened.accept(&peer, &different, &execution, 12, 22)?,
        StoreAcceptance::Conflict(_)
    ));
    Ok(())
}

struct FailAfterAcceptance(AtomicBool);

impl PeerStoreFaultInjector for FailAfterAcceptance {
    fn check(&self, point: PeerStoreFaultPoint) -> Result<(), PeerStoreError> {
        if point == PeerStoreFaultPoint::AcceptanceAfterCommit
            && self.0.swap(false, Ordering::SeqCst)
        {
            Err(PeerStoreError::Io("injected lost response".to_owned()))
        } else {
            Ok(())
        }
    }
}

#[test]
fn lost_acceptance_response_keeps_one_durable_execution() -> TestResult {
    let root = tempfile::tempdir()?;
    let store = FilePeerExecutionStore::open_with_faults(
        root.path(),
        Arc::new(FailAfterAcceptance(AtomicBool::new(true))),
    )?;
    let peer_a = PeerId::new("peer-a")?;
    let request = request(
        &peer_a,
        &PeerId::new("peer-b")?,
        descriptor()?,
        1,
        None,
        None,
    )?;
    let execution = PeerExecutionId::new("execution-one")?;
    assert!(store.accept(&peer_a, &request, &execution, 10, 20).is_err());
    let known = store
        .by_request(&peer_a, &request.request_id)?
        .ok_or("acceptance was not durable")?;
    assert_eq!(known.execution, execution);
    assert!(matches!(
        store.accept(&peer_a, &request, &execution, 11, 21)?,
        StoreAcceptance::Replay(_)
    ));
    Ok(())
}

struct FailObservationAppend(AtomicBool);

impl PeerStoreFaultInjector for FailObservationAppend {
    fn check(&self, point: PeerStoreFaultPoint) -> Result<(), PeerStoreError> {
        if point == PeerStoreFaultPoint::ObservationBeforeCommit
            && self.0.swap(false, Ordering::SeqCst)
        {
            Err(PeerStoreError::Io(
                "injected observation append fault".to_owned(),
            ))
        } else {
            Ok(())
        }
    }
}

#[test]
fn failed_observation_append_never_exposes_a_sequence_gap() -> TestResult {
    let root = tempfile::tempdir()?;
    let store = FilePeerExecutionStore::open_with_faults(
        root.path(),
        Arc::new(FailObservationAppend(AtomicBool::new(true))),
    )?;
    let peer_a = PeerId::new("peer-a")?;
    let peer_b = PeerId::new("peer-b")?;
    let request = request(&peer_a, &peer_b, descriptor()?, 1, None, None)?;
    let execution = PeerExecutionId::new("execution-observation-fault")?;
    store.accept(&peer_a, &request, &execution, 10, 20)?;
    let event = InvocationEvent::new(
        request.request.invocation().clone(),
        1,
        InvocationEventKind::Progress {
            message: "one".to_owned(),
            completed_units: None,
            total_units: None,
        },
    )?;
    let observation = milkdrift_peer_protocol::PeerObservation {
        execution: execution.clone(),
        sequence: 1,
        category: ObservationCategory::Progress,
        event,
        observed_at_unix_ms: 11,
    };
    assert!(
        store
            .append_observation(&peer_a, &execution, observation.clone())
            .is_err()
    );
    assert!(
        store
            .by_execution(&peer_a, &execution)?
            .ok_or("record disappeared")?
            .observations
            .is_empty()
    );
    assert_eq!(
        store
            .append_observation(&peer_a, &execution, observation)?
            .observations
            .len(),
        1
    );
    Ok(())
}

#[test]
fn observation_quota_reserves_the_final_sequence_for_terminal_evidence() -> TestResult {
    let root = tempfile::tempdir()?;
    let store = FilePeerExecutionStore::open(root.path())?;
    let peer_a = PeerId::new("peer-a")?;
    let request = request_named_with_limits(
        &peer_a,
        &PeerId::new("peer-b")?,
        descriptor()?,
        1,
        None,
        None,
        "request-observation-limit",
        "invocation-observation-limit",
        ExecutionLimits {
            artifact_bytes: 1_048_576,
            duration_ms: 30_000,
            cost_micros: 0,
            observations: 2,
        },
    )?;
    let execution = PeerExecutionId::new("execution-observation-limit")?;
    store.accept(&peer_a, &request, &execution, 10, 20)?;
    let progress = |sequence| -> TestResult<milkdrift_peer_protocol::PeerObservation> {
        Ok(milkdrift_peer_protocol::PeerObservation {
            execution: execution.clone(),
            sequence,
            category: ObservationCategory::Progress,
            event: InvocationEvent::new(
                request.request.invocation().clone(),
                sequence,
                InvocationEventKind::Progress {
                    message: "progress".to_owned(),
                    completed_units: None,
                    total_units: None,
                },
            )?,
            observed_at_unix_ms: 11,
        })
    };
    store.append_observation(&peer_a, &execution, progress(1)?)?;
    assert!(
        store
            .append_observation(&peer_a, &execution, progress(2)?)
            .is_err()
    );
    let terminal = InvocationEvent::new(
        request.request.invocation().clone(),
        2,
        InvocationEventKind::Terminal {
            terminal: InvocationTerminal::new(
                TerminalStatus::Success,
                Vec::new(),
                None,
                None,
                SideEffectClass::ReadOnly,
            )?,
        },
    )?;
    assert_eq!(
        store
            .append_observation(
                &peer_a,
                &execution,
                milkdrift_peer_protocol::PeerObservation {
                    execution: execution.clone(),
                    sequence: 2,
                    category: ObservationCategory::Terminal,
                    event: terminal,
                    observed_at_unix_ms: 12,
                },
            )?
            .observations
            .len(),
        2
    );
    Ok(())
}

#[test]
fn restart_after_entry_intent_records_uncertainty_without_reexecution() -> TestResult {
    let root = tempfile::tempdir()?;
    let (host, descriptor) = host_with_adapter()?;
    let peer_a = PeerId::new("peer-a")?;
    let peer_b = PeerId::new("peer-b")?;
    let store = Arc::new(FilePeerExecutionStore::open(root.path())?);
    let request = request(&peer_a, &peer_b, descriptor, 1, None, None)?;
    let execution = PeerExecutionId::new("execution-restart")?;
    store.accept(
        &peer_a,
        &request,
        &execution,
        now(),
        now().saturating_add(30_000),
    )?;
    store.mark_running(&peer_a, &execution)?;
    let service = PeerService::new(
        PeerServerConfig {
            local_peer: peer_b,
            session: SessionId::new("session-restarted")?,
            versions: ProtocolVersionRange::default(),
            limits: HardLimits::default(),
            lease: HeartbeatLease {
                heartbeat_ms: 100,
                idle_timeout_ms: 1_000,
                execution_lease_ms: 10_000,
            },
            relationships: vec![relationship(peer_a.clone(), now().saturating_add(60_000))?],
        },
        host,
        store,
        Arc::new(SystemPeerClock),
    )?;
    service.recover(16)?;
    let page = service.observations(&peer_a, &execution, 0, 16)?;
    assert!(page.closed);
    assert_eq!(page.observations.len(), 1);
    assert_eq!(
        page.observations[0].category,
        ObservationCategory::Uncertainty
    );
    Ok(())
}

#[test]
fn authenticated_identity_filters_catalog_and_executes_once() -> TestResult {
    let root = tempfile::tempdir()?;
    let (host, descriptor) = host_with_adapter()?;
    let peer_a = PeerId::new("peer-a")?;
    let peer_b = PeerId::new("peer-b")?;
    let expiry = now().saturating_add(60_000);
    let relationship = relationship(peer_a.clone(), expiry)?;
    let service = PeerService::new(
        PeerServerConfig {
            local_peer: peer_b.clone(),
            session: SessionId::new("session-b")?,
            versions: ProtocolVersionRange::default(),
            limits: HardLimits::default(),
            lease: HeartbeatLease {
                heartbeat_ms: 100,
                idle_timeout_ms: 1_000,
                execution_lease_ms: 10_000,
            },
            relationships: vec![relationship],
        },
        host,
        Arc::new(FilePeerExecutionStore::open(
            root.path().join("executions"),
        )?),
        Arc::new(SystemPeerClock),
    )?;
    assert_eq!(service.authenticate_bearer(b"peer-secret")?, peer_a);
    assert!(service.authenticate_bearer(b"wrong").is_err());
    let claimed_wrong = HandshakeRequest {
        claimed_peer: PeerId::new("attacker")?,
        session: SessionId::new("session-a")?,
        versions: ProtocolVersionRange::default(),
        features: Default::default(),
        limits: HardLimits::default(),
    };
    assert!(service.handshake(&peer_a, &claimed_wrong).is_err());
    let catalog = service.catalog(&peer_a)?;
    assert_eq!(catalog.entries.len(), 1);
    assert_eq!(
        catalog.entries[0].descriptor.identity(),
        descriptor.identity()
    );
    let invocation = request(
        &peer_a,
        &peer_b,
        descriptor,
        catalog.generation,
        Some(catalog.digest.clone()),
        None,
    )?;
    let execution = match service.invoke(&peer_a, invocation.clone())? {
        InvocationAcceptance::Accepted {
            execution,
            replayed: false,
            ..
        } => execution,
        other => return Err(format!("unexpected acceptance: {other:?}").into()),
    };
    assert!(matches!(
        service.invoke(&peer_a, invocation)?,
        InvocationAcceptance::Accepted { replayed: true, .. }
    ));
    let page = loop {
        let page = service.observations(&peer_a, &execution, 0, 16)?;
        if page.closed {
            break page;
        }
        thread::sleep(Duration::from_millis(5));
    };
    assert_eq!(page.observations.len(), 2);
    assert_eq!(page.observations[0].category, ObservationCategory::Progress);
    assert_eq!(page.observations[1].category, ObservationCategory::Terminal);
    let resumed = service.observations(&peer_a, &execution, 1, 16)?;
    assert_eq!(resumed.observations.len(), 1);
    assert_eq!(resumed.observations[0].sequence, 2);
    let cancellation = service.cancel(
        &peer_a,
        &PeerCancellationRequest {
            request_id: PeerRequestId::new("cancel-after-terminal")?,
            execution,
            sequence: 1,
            reason: "late cancellation test".to_owned(),
        },
    )?;
    assert_eq!(cancellation.disposition, CancellationDisposition::TooLate);
    assert!(cancellation.terminal_boundary);
    assert!(cancellation.terminal_evidence.is_some());
    service.revoke_peer(&peer_a)?;
    assert!(service.authenticate_bearer(b"peer-secret").is_err());
    assert!(service.catalog(&peer_a).is_err());
    Ok(())
}

#[test]
fn authenticated_request_rate_is_bounded_per_action() -> TestResult {
    let root = tempfile::tempdir()?;
    let (host, _) = host_with_adapter()?;
    let peer_a = PeerId::new("peer-a")?;
    let peer_b = PeerId::new("peer-b")?;
    let mut relationship = relationship(peer_a.clone(), now().saturating_add(60_000))?;
    relationship.maximum_requests_per_minute = 1;
    let service = PeerService::new(
        PeerServerConfig {
            local_peer: peer_b,
            session: SessionId::new("session-rate")?,
            versions: ProtocolVersionRange::default(),
            limits: HardLimits::default(),
            lease: HeartbeatLease {
                heartbeat_ms: 100,
                idle_timeout_ms: 1_000,
                execution_lease_ms: 10_000,
            },
            relationships: vec![relationship],
        },
        host,
        Arc::new(FilePeerExecutionStore::open(root.path())?),
        Arc::new(SystemPeerClock),
    )?;
    assert_eq!(service.catalog(&peer_a)?.entries.len(), 1);
    assert!(matches!(
        service.catalog(&peer_a),
        Err(PeerHttpError::Overloaded(_))
    ));
    Ok(())
}

#[test]
fn catalog_generation_changes_for_health_and_drain() -> TestResult {
    let root = tempfile::tempdir()?;
    let (host, descriptor) = host_with_adapter()?;
    let peer_a = PeerId::new("peer-a")?;
    let peer_b = PeerId::new("peer-b")?;
    let service = PeerService::new(
        PeerServerConfig {
            local_peer: peer_b,
            session: SessionId::new("session-catalog")?,
            versions: ProtocolVersionRange::default(),
            limits: HardLimits::default(),
            lease: HeartbeatLease {
                heartbeat_ms: 100,
                idle_timeout_ms: 1_000,
                execution_lease_ms: 10_000,
            },
            relationships: vec![relationship(peer_a.clone(), now().saturating_add(60_000))?],
        },
        host.clone(),
        Arc::new(FilePeerExecutionStore::open(root.path())?),
        Arc::new(SystemPeerClock),
    )?;
    let initial = service.catalog(&peer_a)?;
    host.refresh_health(
        descriptor.identity(),
        descriptor.descriptor_revision(),
        now().saturating_add(1_000),
    )?;
    let refreshed = service.catalog(&peer_a)?;
    assert!(refreshed.generation > initial.generation);
    assert_ne!(refreshed.digest, initial.digest);
    service.begin_drain();
    let drained = service.catalog(&peer_a)?;
    assert!(drained.generation > refreshed.generation);
    assert!(drained.entries.is_empty());
    Ok(())
}

#[test]
fn concurrency_quota_rejects_before_second_acceptance() -> TestResult {
    let root = tempfile::tempdir()?;
    let (host, descriptor) = host_with_adapter()?;
    let peer_a = PeerId::new("peer-a")?;
    let peer_b = PeerId::new("peer-b")?;
    let mut peer_relationship = relationship(peer_a.clone(), now().saturating_add(60_000))?;
    peer_relationship.maximum_concurrent = 1;
    let store = Arc::new(FilePeerExecutionStore::open(root.path())?);
    let service = PeerService::new(
        PeerServerConfig {
            local_peer: peer_b.clone(),
            session: SessionId::new("session-overload")?,
            versions: ProtocolVersionRange::default(),
            limits: HardLimits::default(),
            lease: HeartbeatLease {
                heartbeat_ms: 100,
                idle_timeout_ms: 1_000,
                execution_lease_ms: 10_000,
            },
            relationships: vec![peer_relationship],
        },
        host,
        store.clone(),
        Arc::new(SystemPeerClock),
    )?;
    let catalog = service.catalog(&peer_a)?;
    let first = request_named(
        &peer_a,
        &peer_b,
        descriptor.clone(),
        catalog.generation,
        Some(catalog.digest.clone()),
        None,
        "request-overload-one",
        "invocation-overload-one",
    )?;
    store.accept(
        &peer_a,
        &first,
        &PeerExecutionId::new("execution-overload-one")?,
        now(),
        now().saturating_add(30_000),
    )?;
    let second = request_named(
        &peer_a,
        &peer_b,
        descriptor,
        catalog.generation,
        Some(catalog.digest),
        None,
        "request-overload-two",
        "invocation-overload-two",
    )?;
    assert!(matches!(
        service.invoke(&peer_a, second)?,
        InvocationAcceptance::Rejected { code, .. } if code == "overload"
    ));
    assert!(
        store
            .by_request(&peer_a, &PeerRequestId::new("request-overload-two")?)?
            .is_none()
    );
    Ok(())
}

#[test]
fn relationship_filters_and_artifact_authority_default_deny() -> TestResult {
    let root = tempfile::tempdir()?;
    let (host, _) = host_with_adapter()?;
    let peer_a = PeerId::new("peer-a")?;
    let peer_b = PeerId::new("peer-b")?;
    let mut peer_relationship = relationship(peer_a.clone(), now().saturating_add(60_000))?;
    peer_relationship.operation_allow.clear();
    peer_relationship
        .authority
        .actions
        .remove(&PeerAction::ArtifactUpload);
    let service = PeerService::new_with_artifacts(
        PeerServerConfig {
            local_peer: peer_b,
            session: SessionId::new("session-deny")?,
            versions: ProtocolVersionRange::default(),
            limits: HardLimits::default(),
            lease: HeartbeatLease {
                heartbeat_ms: 100,
                idle_timeout_ms: 1_000,
                execution_lease_ms: 10_000,
            },
            relationships: vec![peer_relationship],
        },
        host,
        Arc::new(FilePeerExecutionStore::open(
            root.path().join("executions"),
        )?),
        Arc::new(FilePeerArtifactStore::open(root.path().join("artifacts"))?),
        Arc::new(SystemPeerClock),
    )?;
    assert!(service.catalog(&peer_a)?.entries.is_empty());
    let bytes = b"denied";
    let offer = ArtifactMetadataOffer {
        transfer: TransferId::new("transfer-denied")?,
        direction: ArtifactTransferDirection::Upload,
        artifact: milkdrift_capability::ArtifactReference::new(
            "denied-artifact",
            blake3::hash(bytes).to_hex().to_string(),
            Some("application/octet-stream".to_owned()),
            Some(u64::try_from(bytes.len())?),
        )?,
        source_peer: peer_a.clone(),
        execution: PeerExecutionId::new("execution-denied")?,
        expires_at_unix_ms: now().saturating_add(60_000),
    };
    assert!(matches!(
        service.negotiate_artifact(&peer_a, &offer),
        Err(PeerHttpError::Unauthorized(_))
    ));
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn remote_catalog_registration_executes_through_peer_http_once() -> TestResult {
    let root = tempfile::tempdir()?;
    let (server_host, _) = host_with_adapter()?;
    let peer_a = PeerId::new("peer-a")?;
    let peer_b = PeerId::new("peer-b")?;
    let expiry = now().saturating_add(60_000);
    let server_service = PeerService::new(
        PeerServerConfig {
            local_peer: peer_b.clone(),
            session: SessionId::new("session-server")?,
            versions: ProtocolVersionRange::default(),
            limits: HardLimits::default(),
            lease: HeartbeatLease {
                heartbeat_ms: 100,
                idle_timeout_ms: 1_000,
                execution_lease_ms: 10_000,
            },
            relationships: vec![relationship(peer_a.clone(), expiry)?],
        },
        server_host,
        Arc::new(FilePeerExecutionStore::open(
            root.path().join("executions"),
        )?),
        Arc::new(SystemPeerClock),
    )?;
    let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).await?;
    let endpoint = Url::parse(&format!("http://{}/", listener.local_addr()?))?;
    let router = peer_router(server_service.clone());
    let server = tokio::spawn(async move { axum::serve(listener, router).await });
    let local_host = empty_host()?;
    let local_relationship = relationship(peer_b.clone(), expiry)?;
    let result = tokio::task::spawn_blocking(move || -> Result<_, String> {
        let client = PeerHttpClient::new(PeerClientConfig {
            endpoint,
            local_peer: peer_a,
            expected_remote_peer: peer_b,
            session: SessionId::new("session-client").map_err(|error| error.to_string())?,
            versions: ProtocolVersionRange::default(),
            bearer_credential: Arc::new(SensitiveSecret::new(b"peer-secret".to_vec())),
            insecure_loopback: InsecureLoopbackMode::AllowInsecureLoopbackDevelopment,
            request_timeout: Duration::from_secs(5),
            observation_poll_interval: Duration::from_millis(5),
        })
        .map_err(|error| error.to_string())?;
        let registry = PeerRegistry::new(local_host.clone(), client, local_relationship)
            .map_err(|error| error.to_string())?;
        let provenance = registry.connect().map_err(|error| error.to_string())?;
        if provenance.len() != 1 {
            return Err("remote catalog did not register exactly one capability".to_owned());
        }
        let scope = CapabilityAuthorityScope::any(SideEffectClass::Unknown);
        let generation = local_host
            .catalog_generations(&scope)
            .map_err(|error| error.to_string())?
            .into_iter()
            .find(|generation| generation.descriptor.locality() == Locality::Remote)
            .ok_or_else(|| "remote generation was not registered".to_owned())?;
        let operation = OperationId::new("test.execute").map_err(|error| error.to_string())?;
        let snapshot =
            ResolvedCapabilitySnapshot::from_descriptor(&generation.descriptor, &operation)
                .map_err(|error| error.to_string())?;
        let request = InvocationRequest::new(
            InvocationId::new("remote-invocation").map_err(|error| error.to_string())?,
            generation.descriptor.identity().clone(),
            operation,
            None,
            None,
            Vec::new(),
            BTreeMap::new(),
        )
        .map_err(|error| error.to_string())?;
        let first = CollectReporter::default();
        local_host
            .execute_exact(&snapshot, &request, &first)
            .map_err(|error| error.to_string())?;
        let second = CollectReporter::default();
        local_host
            .execute_exact(&snapshot, &request, &second)
            .map_err(|error| error.to_string())?;
        let first = first
            .events
            .lock()
            .map_err(|_| "first reporter unavailable".to_owned())?
            .clone();
        let second = second
            .events
            .lock()
            .map_err(|_| "second reporter unavailable".to_owned())?
            .clone();
        Ok((first, second))
    })
    .await??;
    for events in [&result.0, &result.1] {
        assert_eq!(events.len(), 2);
        assert!(matches!(
            events[0].kind(),
            InvocationEventKind::Progress { .. }
        ));
        assert!(matches!(
            events[1].kind(),
            InvocationEventKind::Terminal { terminal }
                if terminal.status() == TerminalStatus::Success
        ));
    }
    let lookup = server_service.lookup(
        &PeerId::new("peer-a")?,
        &PeerRequestId::new("request:remote-invocation")?,
    )?;
    assert!(matches!(
        lookup,
        milkdrift_peer_protocol::InvocationLookup::Known {
            status: milkdrift_peer_protocol::RemoteExecutionStatus::Terminal,
            ..
        }
    ));
    server.abort();
    let _ = server.await;
    Ok(())
}

#[test]
fn artifact_upload_verifies_deduplicates_and_abort_hides_partial_content() -> TestResult {
    let root = tempfile::tempdir()?;
    let store = FilePeerArtifactStore::open(root.path())?;
    let peer = PeerId::new("peer-a")?;
    let execution = PeerExecutionId::new("execution-a")?;
    let bytes = b"verified peer artifact".to_vec();
    let digest = blake3::hash(&bytes).to_hex().to_string();
    let offer = ArtifactMetadataOffer {
        transfer: TransferId::new("transfer-a")?,
        direction: ArtifactTransferDirection::Upload,
        artifact: milkdrift_capability::ArtifactReference::new(
            "remote-artifact",
            digest,
            Some("application/octet-stream".to_owned()),
            Some(u64::try_from(bytes.len())?),
        )?,
        source_peer: peer.clone(),
        execution,
        expires_at_unix_ms: now().saturating_add(60_000),
    };
    let mut over_quota = offer.clone();
    over_quota.transfer = TransferId::new("transfer-over-quota")?;
    assert!(store.negotiate(&peer, &over_quota, 1).is_err());
    let mut secret = offer.clone();
    secret.transfer = TransferId::new("transfer-secret")?;
    secret.artifact = milkdrift_capability::ArtifactReference::new(
        "secret-artifact",
        secret.artifact.digest(),
        Some("application/x-secret".to_owned()),
        secret.artifact.size_bytes(),
    )?;
    assert!(store.negotiate(&peer, &secret, 1_048_576).is_err());
    assert!(TransferId::new("../escape").is_err());
    assert!(matches!(
        store.negotiate(&peer, &offer, 1_048_576)?,
        ArtifactTransferDecision::Transfer { next_offset: 0, .. }
    ));
    assert_eq!(
        store.write_chunk(
            &peer,
            &ArtifactChunk {
                transfer: offer.transfer.clone(),
                offset: 0,
                bytes: bytes.clone(),
                final_chunk: true,
            },
            1_048_576,
        )?,
        ArtifactTransferDecision::AlreadyPresent
    );
    let mut replay = offer.clone();
    replay.transfer = TransferId::new("transfer-b")?;
    assert_eq!(
        store.negotiate(&peer, &replay, 1_048_576)?,
        ArtifactTransferDecision::AlreadyPresent
    );
    let mut mismatch = replay.clone();
    mismatch.transfer = TransferId::new("transfer-mismatch")?;
    mismatch.artifact = milkdrift_capability::ArtifactReference::new(
        "mismatch",
        "0".repeat(64),
        Some("application/octet-stream".to_owned()),
        Some(u64::try_from(bytes.len())?),
    )?;
    store.negotiate(&peer, &mismatch, 1_048_576)?;
    assert!(
        store
            .write_chunk(
                &peer,
                &ArtifactChunk {
                    transfer: mismatch.transfer.clone(),
                    offset: 0,
                    bytes: bytes.clone(),
                    final_chunk: true,
                },
                1_048_576,
            )
            .is_err()
    );
    assert!(
        store
            .write_chunk(
                &peer,
                &ArtifactChunk {
                    transfer: mismatch.transfer,
                    offset: 0,
                    bytes: vec![1],
                    final_chunk: false,
                },
                1_048_576,
            )
            .is_err()
    );
    let mut interrupted = offer;
    interrupted.transfer = TransferId::new("transfer-interrupted")?;
    interrupted.artifact = milkdrift_capability::ArtifactReference::new(
        "partial",
        "0".repeat(64),
        Some("application/octet-stream".to_owned()),
        Some(10),
    )?;
    store.negotiate(&peer, &interrupted, 1_048_576)?;
    store.write_chunk(
        &peer,
        &ArtifactChunk {
            transfer: interrupted.transfer.clone(),
            offset: 0,
            bytes: vec![1, 2, 3],
            final_chunk: false,
        },
        1_048_576,
    )?;
    store.abort(&peer, &interrupted.transfer)?;
    assert!(
        store
            .write_chunk(
                &peer,
                &ArtifactChunk {
                    transfer: interrupted.transfer,
                    offset: 3,
                    bytes: vec![4],
                    final_chunk: false,
                },
                1_048_576,
            )
            .is_err()
    );
    Ok(())
}

struct FailMetadataPublication(AtomicBool);

impl PeerArtifactFaultInjector for FailMetadataPublication {
    fn check(&self, point: PeerArtifactFaultPoint) -> Result<(), PeerArtifactError> {
        if point == PeerArtifactFaultPoint::MetadataBeforePublication
            && self.0.swap(false, Ordering::SeqCst)
        {
            Err(PeerArtifactError::Io(
                "injected metadata publication fault".to_owned(),
            ))
        } else {
            Ok(())
        }
    }
}

#[test]
fn artifact_publication_fault_is_recovered_only_after_reverification() -> TestResult {
    let root = tempfile::tempdir()?;
    let peer = PeerId::new("peer-a")?;
    let bytes = b"publication boundary".to_vec();
    let offer = ArtifactMetadataOffer {
        transfer: TransferId::new("transfer-publication-fault")?,
        direction: ArtifactTransferDirection::Upload,
        artifact: milkdrift_capability::ArtifactReference::new(
            "remote-artifact",
            blake3::hash(&bytes).to_hex().to_string(),
            Some("application/octet-stream".to_owned()),
            Some(u64::try_from(bytes.len())?),
        )?,
        source_peer: peer.clone(),
        execution: PeerExecutionId::new("execution-publication-fault")?,
        expires_at_unix_ms: now().saturating_add(60_000),
    };
    let store = FilePeerArtifactStore::open_with_faults(
        root.path(),
        Arc::new(FailMetadataPublication(AtomicBool::new(true))),
    )?;
    store.negotiate(&peer, &offer, 1_048_576)?;
    assert!(
        store
            .write_chunk(
                &peer,
                &ArtifactChunk {
                    transfer: offer.transfer.clone(),
                    offset: 0,
                    bytes,
                    final_chunk: true,
                },
                1_048_576,
            )
            .is_err()
    );
    drop(store);
    let reopened = FilePeerArtifactStore::open(root.path())?;
    assert_eq!(
        reopened.negotiate(&peer, &offer, 1_048_576)?,
        ArtifactTransferDecision::AlreadyPresent
    );
    Ok(())
}

fn request(
    issuer: &PeerId,
    target: &PeerId,
    descriptor: CapabilityDescriptor,
    catalog_generation: u64,
    catalog_digest: Option<milkdrift_peer_protocol::CatalogDigest>,
    deadline: Option<u64>,
) -> TestResult<PeerInvocationRequest> {
    request_named(
        issuer,
        target,
        descriptor,
        catalog_generation,
        catalog_digest,
        deadline,
        "request-a",
        "invocation-a",
    )
}

#[allow(clippy::too_many_arguments)]
fn request_named(
    issuer: &PeerId,
    target: &PeerId,
    descriptor: CapabilityDescriptor,
    catalog_generation: u64,
    catalog_digest: Option<milkdrift_peer_protocol::CatalogDigest>,
    deadline: Option<u64>,
    request_identity: &str,
    invocation_identity: &str,
) -> TestResult<PeerInvocationRequest> {
    request_named_with_limits(
        issuer,
        target,
        descriptor,
        catalog_generation,
        catalog_digest,
        deadline,
        request_identity,
        invocation_identity,
        ExecutionLimits {
            artifact_bytes: 1_048_576,
            duration_ms: 30_000,
            cost_micros: 0,
            observations: 100,
        },
    )
}

#[allow(clippy::too_many_arguments)]
fn request_named_with_limits(
    issuer: &PeerId,
    target: &PeerId,
    descriptor: CapabilityDescriptor,
    catalog_generation: u64,
    catalog_digest: Option<milkdrift_peer_protocol::CatalogDigest>,
    deadline: Option<u64>,
    request_identity: &str,
    invocation_identity: &str,
    limits: ExecutionLimits,
) -> TestResult<PeerInvocationRequest> {
    let operation = OperationId::new("test.execute")?;
    let selection = ResolvedCapabilitySnapshot::from_descriptor(&descriptor, &operation)?;
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
    let deadline = deadline.unwrap_or_else(|| now().saturating_add(30_000));
    let catalog_digest = match catalog_digest {
        Some(digest) => digest,
        None => {
            milkdrift_peer_protocol::CatalogSnapshot::new(
                catalog_generation,
                now().saturating_sub(1),
                deadline,
                Vec::new(),
            )?
            .digest
        }
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
            nonce: request_identity.to_owned(),
        },
    )?)
}
