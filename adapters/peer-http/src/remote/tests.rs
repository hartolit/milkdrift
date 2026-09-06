use std::{
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    sync::atomic::{AtomicBool, AtomicU64},
    thread::JoinHandle,
    time::Duration,
};

use milkdrift_authority::SensitiveSecret;
use milkdrift_blueprint::{NodeId, RevisionId};
use milkdrift_capability::{
    CapabilityDescriptorDocument, InvocationEvent, InvocationEventKind, InvocationRequest,
    InvocationTerminal, OperationId, PeerId, SideEffectClass, TerminalStatus, TrustZone,
};
use milkdrift_capability_host::{
    AdapterExecutionContext, CapabilityHost, CapabilitySelectionPolicy, HostConfig,
    conformance::{
        AdapterConformanceCase, AdapterConformanceExpectations, ConformanceScenario,
        StartReplayExpectation, UnknownCancellationExpectation, run_adapter_conformance,
    },
};
use milkdrift_peer_protocol::{
    ArchivedExecutionSummary, CatalogSnapshot, DecodeLimits, DelegationRef, DrainState,
    ExecutionLimits, FeatureSet, HandshakeResponse, HardLimits, HeartbeatLease,
    InvocationAcceptance, ObservationCategory, PeerAuthority, PeerObservation, ProtocolEnvelope,
    ProtocolVersion, ProtocolVersionRange, RemoteExecutionStatus, SessionId, decode_envelope,
    encode_envelope,
};
use milkdrift_persistence::{AttemptId, NodeExecutionId};
use milkdrift_workspace::RunId;
use serde::Serialize;
use url::Url;

use super::*;
use crate::{InsecureLoopbackMode, PeerClientConfig, PeerClockError};

struct ControlledClock {
    now: AtomicU64,
    available: AtomicBool,
}

type ConformanceServer = (String, JoinHandle<Result<(), String>>);

impl ControlledClock {
    const fn new(now: u64) -> Self {
        Self {
            now: AtomicU64::new(now),
            available: AtomicBool::new(true),
        }
    }
}

impl PeerClock for ControlledClock {
    fn now_unix_ms(&self) -> Result<u64, PeerClockError> {
        if !self.available.load(Ordering::SeqCst) {
            return Err(PeerClockError::Unavailable);
        }
        Ok(self.now.load(Ordering::SeqCst))
    }
}

fn write_response<T: Serialize>(stream: &mut TcpStream, message: T) -> Result<(), String> {
    let body =
        encode_envelope(&ProtocolEnvelope::v1(message)).map_err(|error| error.to_string())?;
    let headers = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    stream
        .write_all(headers.as_bytes())
        .and_then(|()| stream.write_all(&body))
        .map_err(|error| error.to_string())
}

fn read_request_body(stream: &mut TcpStream) -> Result<Vec<u8>, String> {
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .map_err(|error| error.to_string())?;
    let mut bytes = Vec::new();
    let mut buffer = [0_u8; 4_096];
    loop {
        let read = stream
            .read(&mut buffer)
            .map_err(|error| error.to_string())?;
        if read == 0 {
            return Err("peer conformance request ended before its body".to_owned());
        }
        bytes.extend_from_slice(&buffer[..read]);
        let Some(header_end) = bytes.windows(4).position(|window| window == b"\r\n\r\n") else {
            continue;
        };
        let headers =
            std::str::from_utf8(&bytes[..header_end + 4]).map_err(|error| error.to_string())?;
        let content_length = headers
            .lines()
            .find_map(|line| {
                line.to_ascii_lowercase()
                    .strip_prefix("content-length: ")
                    .and_then(|value| value.parse::<usize>().ok())
            })
            .unwrap_or(0);
        let body_start = header_end + 4;
        if bytes.len() >= body_start.saturating_add(content_length) {
            return Ok(bytes[body_start..body_start + content_length].to_vec());
        }
    }
}

fn serve_archived_execution(
    remote_peer: PeerId,
) -> Result<ConformanceServer, Box<dyn std::error::Error>> {
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let address = listener.local_addr()?.to_string();
    let handle = std::thread::spawn(move || {
        let (mut handshake_stream, _) = listener.accept().map_err(|error| error.to_string())?;
        let _handshake = read_request_body(&mut handshake_stream)?;
        write_response(
            &mut handshake_stream,
            HandshakeResponse {
                peer: remote_peer,
                session: SessionId::new("session-remote-conformance-server")
                    .map_err(|error| error.to_string())?,
                selected_version: ProtocolVersion::V1_2,
                features: FeatureSet {
                    resumable_observations: true,
                    resumable_artifacts: true,
                    incremental_catalog: false,
                    archived_execution_replay: true,
                },
                limits: HardLimits::default(),
                lease: HeartbeatLease {
                    heartbeat_ms: 100,
                    idle_timeout_ms: 500,
                    execution_lease_ms: 1_000,
                },
                drain: DrainState::Ready,
            },
        )?;

        let (mut invocation_stream, _) = listener.accept().map_err(|error| error.to_string())?;
        let bytes = read_request_body(&mut invocation_stream)?;
        let envelope: ProtocolEnvelope<PeerInvocationRequest> =
            decode_envelope(&bytes, DecodeLimits::default()).map_err(|error| error.to_string())?;
        let request = envelope.message;
        let execution = PeerExecutionId::new("execution-remote-conformance")
            .map_err(|error| error.to_string())?;
        let terminal = InvocationTerminal::new(
            TerminalStatus::Success,
            Vec::new(),
            None,
            None,
            request.selection.operation_contract().side_effect(),
        )
        .map_err(|error| error.to_string())?;
        let event = InvocationEvent::new(
            request.request.invocation().clone(),
            1,
            InvocationEventKind::Terminal { terminal },
        )
        .map_err(|error| error.to_string())?;
        let observation = PeerObservation {
            execution: execution.clone(),
            sequence: 1,
            category: ObservationCategory::Terminal,
            event,
            observed_at_unix_ms: 100,
        };
        write_response(
            &mut invocation_stream,
            InvocationAcceptance::Archived {
                request_id: request.request_id,
                execution,
                request_digest: request.request_digest,
                accepted_at_unix_ms: 100,
                summary: Box::new(ArchivedExecutionSummary {
                    status: RemoteExecutionStatus::Terminal,
                    last_sequence: 1,
                    observation_digest: format!("b3_{}", "0".repeat(64)),
                    archived_at_unix_ms: 101,
                    final_observation: Some(observation),
                    uncertainty_reason: None,
                }),
            },
        )
    });
    Ok((address, handle))
}

struct RemoteCase {
    adapter: Arc<RemoteCapabilityAdapter>,
    descriptor: CapabilityDescriptor,
    request: InvocationRequest,
    context: AdapterExecutionContext,
    server: Option<JoinHandle<Result<(), String>>>,
}

fn remote_case(scenario: ConformanceScenario) -> Result<RemoteCase, Box<dyn std::error::Error>> {
    let origin = PeerId::new("peer-remote-conformance-origin")?;
    let remote = PeerId::new("peer-remote-conformance-target")?;
    let (address, server) = if scenario.executes() {
        let (address, server) = serve_archived_execution(remote.clone())?;
        (address, Some(server))
    } else {
        ("127.0.0.1:9".to_owned(), None)
    };
    let credential = Arc::new(SensitiveSecret::new(
        b"peer-remote-conformance-secret".to_vec(),
    ));
    let client = PeerHttpClient::new(PeerClientConfig {
        endpoint: Url::parse(&format!("http://{address}/"))?,
        local_peer: origin,
        expected_remote_peer: remote.clone(),
        session: SessionId::new("session-remote-conformance-client")?,
        versions: ProtocolVersionRange::default(),
        bearer_credential: credential.clone(),
        insecure_loopback: InsecureLoopbackMode::AllowInsecureLoopbackDevelopment,
        request_timeout: Duration::from_secs(2),
        observation_poll_interval: Duration::from_millis(1),
    })?;
    let relationship = PeerRelationship {
        remote_peer: remote.clone(),
        bearer_credential: credential,
        versions: ProtocolVersionRange::default(),
        authority: PeerAuthority::default(),
        capability_allow: BTreeSet::new(),
        capability_deny: BTreeSet::new(),
        operation_allow: BTreeSet::new(),
        maximum_side_effect: SideEffectClass::Unknown,
        execution_filesystem: Vec::new(),
        execution_network_profiles: BTreeSet::new(),
        execution_network_destinations: BTreeSet::new(),
        execution_secrets: BTreeSet::new(),
        execution_limits: ExecutionLimits {
            artifact_bytes: 1_024,
            duration_ms: 1_000,
            cost_micros: 1_000,
            observations: 8,
        },
        maximum_concurrent: 2,
        maximum_requests_per_minute: 10,
        maximum_artifact_bytes: 1_024,
        artifact_sensitivities: BTreeSet::new(),
        catalog_ttl_ms: 1_000,
        trust_zone: TrustZone::new("remote-conformance-zone")?,
        delegation: DelegationRef::new("remote-conformance-delegation")?,
        revocation_generation: 0,
        expires_at_unix_ms: 1_000,
        enabled: true,
    };
    let remote_descriptor = CapabilityDescriptorDocument::from_json(include_bytes!(
        "../../../../crates/capability/tests/fixtures/descriptor-v1.json"
    ))?
    .body()
    .clone();
    let local_capability = CapabilityId::new("peer-remote-conformance-capability")?;
    let mut trust_zones = remote_descriptor.trust_zones().clone();
    trust_zones.insert(relationship.trust_zone.clone());
    let local_descriptor = DescriptorBuilder::new(
        local_capability.clone(),
        1,
        remote_descriptor.category().clone(),
        remote_descriptor.admission().clone(),
        Locality::Peer,
    )
    .peer(Some(remote.clone()))
    .provider_profile(remote_descriptor.provider_profile().cloned())
    .operations(remote_descriptor.operations().clone())
    .trust_zones(trust_zones)
    .execution_trust(remote_descriptor.execution_trust())
    .resource_observations(remote_descriptor.resource_observations().cloned())
    .labels(remote_descriptor.labels().clone())
    .extensions(remote_descriptor.extensions().clone())
    .build()?;
    let operation = OperationId::new("model.generate")?;
    let request = InvocationRequest::new(
        InvocationId::new("invocation-remote-conformance")?,
        local_capability.clone(),
        operation,
        local_descriptor.provider_profile().cloned(),
        None,
        Vec::new(),
        BTreeMap::new(),
    )?;
    let adapter = Arc::new(RemoteCapabilityAdapter {
        authority_requirements: remote_authority_requirements(client.as_ref(), &relationship)?,
        client,
        relationship,
        catalog_generation: 1,
        catalog_digest: CatalogDigest::new(format!("b3_{}", "1".repeat(64)))?,
        catalog_expires_at_unix_ms: 1_000,
        remote_descriptor,
        local_capability,
        clock: Arc::new(ControlledClock::new(100)),
        active: Mutex::new(BTreeMap::new()),
        lifecycle: AtomicU8::new(Lifecycle::Created as u8),
    });
    let revision: RevisionId =
        serde_json::from_value(serde_json::json!(format!("rev_{}", "0".repeat(64))))?;
    Ok(RemoteCase {
        adapter,
        descriptor: local_descriptor,
        request,
        context: AdapterExecutionContext::new(
            RunId::new("run-remote-conformance")?,
            revision,
            NodeId::new("remote-conformance")?,
            NodeExecutionId::new("execution-remote-conformance")?,
            AttemptId::new("attempt-remote-conformance")?,
        ),
        server,
    })
}

fn remote_conformance_case(
    scenario: ConformanceScenario,
) -> Result<AdapterConformanceCase, Box<dyn std::error::Error>> {
    let fixture = remote_case(scenario)?;
    let case = AdapterConformanceCase::new(
        fixture.adapter,
        fixture.descriptor,
        fixture.request,
        fixture.context,
        AdapterConformanceExpectations {
            start_replay: StartReplayExpectation::Idempotent,
            available_while_draining: false,
            available_after_shutdown: false,
            unknown_cancellation: UnknownCancellationExpectation::Unavailable,
        },
    )?;
    Ok(match fixture.server {
        Some(server) => case.with_cleanup(move || {
            server
                .join()
                .map_err(|_| "remote conformance server panicked".to_owned())?
        }),
        None => case,
    })
}

#[test]
fn catalog_retirement_keeps_the_exact_active_permit_until_reporting_finishes()
-> Result<(), Box<dyn std::error::Error>> {
    use std::sync::mpsc;

    struct HeldTerminal {
        entered: mpsc::SyncSender<()>,
        release: Mutex<mpsc::Receiver<()>>,
    }
    impl AdapterReporter for HeldTerminal {
        fn invocation(&self, event: InvocationEvent) -> Result<(), AdapterError> {
            if matches!(event.kind(), InvocationEventKind::Terminal { .. }) {
                self.entered
                    .send(())
                    .map_err(|error| AdapterError::external_failure(error.to_string()))?;
                self.release
                    .lock()
                    .map_err(|error| AdapterError::external_failure(error.to_string()))?
                    .recv_timeout(Duration::from_secs(2))
                    .map_err(|error| AdapterError::external_failure(error.to_string()))?;
            }
            Ok(())
        }
        fn heartbeat(&self) -> Result<(), AdapterError> {
            Ok(())
        }
    }

    let fixture = remote_case(ConformanceScenario::HostDrain)?;
    let host = CapabilityHost::new(
        HostConfig {
            max_registrations: 1,
            max_generations_per_capability: 1,
            max_concurrent_per_generation: 1,
            observation_stale_after_ms: 1_000,
        },
        CapabilitySelectionPolicy::priorities(BTreeMap::new()),
    )?;
    host.register(fixture.descriptor.clone(), fixture.adapter, None)?;
    let key = (
        fixture.descriptor.identity().clone(),
        fixture.descriptor.descriptor_revision(),
    );
    let mut registrations = Registrations {
        active: BTreeMap::from([(
            key.clone(),
            Registration {
                local_capability: key.0.clone(),
                local_revision: key.1,
            },
        )]),
        draining: Vec::new(),
    };
    let snapshot = ResolvedCapabilitySnapshot::from_descriptor(
        &fixture.descriptor,
        fixture.request.operation(),
    )?;
    let (entered, entry) = mpsc::sync_channel(1);
    let (release, released) = mpsc::sync_channel(1);
    let executor = host.clone();
    let worker = std::thread::spawn(move || {
        executor.execute_exact_with_context(
            &snapshot,
            &fixture.request,
            &fixture.context,
            &HeldTerminal {
                entered,
                release: Mutex::new(released),
            },
        )
    });
    entry.recv_timeout(Duration::from_secs(2))?;
    registrations.retire(&key, &host)?;
    assert!(registrations.active.is_empty());
    assert_eq!(registrations.draining.len(), 1);
    registrations.reap(&host)?;
    assert_eq!(registrations.draining.len(), 1);
    release.send(())?;
    worker.join().map_err(|_| "remote worker panicked")??;
    registrations.reap(&host)?;
    assert!(registrations.draining.is_empty());
    let scope = milkdrift_authority::CapabilityAuthorityScope::allow_any(SideEffectClass::Unknown);
    assert!(host.catalog_generations(&scope)?.is_empty());
    fixture
        .server
        .ok_or("remote fixture server absent")?
        .join()
        .map_err(|_| "remote server panicked")??;
    Ok(())
}

#[test]
fn remote_capability_adapter_passes_shared_conformance() -> Result<(), Box<dyn std::error::Error>> {
    run_adapter_conformance(remote_conformance_case)?;
    Ok(())
}

#[test]
fn remote_catalog_registration_fails_closed_and_recovers_with_the_clock()
-> Result<(), Box<dyn std::error::Error>> {
    let local = PeerId::new("peer-remote-clock-local")?;
    let remote = PeerId::new("peer-remote-clock-target")?;
    let credential = Arc::new(SensitiveSecret::new(b"remote-clock-secret".to_vec()));
    let client = PeerHttpClient::new(PeerClientConfig {
        endpoint: Url::parse("http://127.0.0.1:1/")?,
        local_peer: local,
        expected_remote_peer: remote.clone(),
        session: SessionId::new("session-remote-clock")?,
        versions: ProtocolVersionRange::default(),
        bearer_credential: credential.clone(),
        insecure_loopback: InsecureLoopbackMode::AllowInsecureLoopbackDevelopment,
        request_timeout: Duration::from_millis(10),
        observation_poll_interval: Duration::from_millis(1),
    })?;
    let relationship = PeerRelationship {
        remote_peer: remote,
        bearer_credential: credential,
        versions: ProtocolVersionRange::default(),
        authority: PeerAuthority {
            actions: BTreeSet::new(),
        },
        capability_allow: BTreeSet::new(),
        capability_deny: BTreeSet::new(),
        operation_allow: BTreeSet::new(),
        maximum_side_effect: SideEffectClass::None,
        execution_filesystem: Vec::new(),
        execution_network_profiles: BTreeSet::new(),
        execution_network_destinations: BTreeSet::new(),
        execution_secrets: BTreeSet::new(),
        execution_limits: ExecutionLimits {
            artifact_bytes: 1,
            duration_ms: 1,
            cost_micros: 0,
            observations: 1,
        },
        maximum_concurrent: 1,
        maximum_requests_per_minute: 1,
        maximum_artifact_bytes: 1,
        artifact_sensitivities: BTreeSet::new(),
        catalog_ttl_ms: 10,
        trust_zone: TrustZone::new("remote-clock-zone")?,
        delegation: DelegationRef::new("remote-clock-delegation")?,
        revocation_generation: 0,
        expires_at_unix_ms: 1_000,
        enabled: true,
    };
    let host = CapabilityHost::new(
        HostConfig {
            max_registrations: 1,
            max_generations_per_capability: 1,
            max_concurrent_per_generation: 1,
            observation_stale_after_ms: 1_000,
        },
        CapabilitySelectionPolicy::priorities(BTreeMap::new()),
    )?;
    let clock = Arc::new(ControlledClock::new(100));
    let registry = PeerRegistry::new(host.clone(), client, relationship, clock.clone())?;
    let descriptor = CapabilityDescriptorDocument::from_json(include_bytes!(
        "../../../../crates/capability/tests/fixtures/descriptor-v1.json"
    ))?
    .body()
    .clone();
    let entry = milkdrift_peer_protocol::CatalogEntry {
        observation: CapabilityObservation::new(
            descriptor.identity().clone(),
            90,
            true,
            0,
            "live",
        )?,
        invocable_operations: BTreeSet::from([OperationId::new("model.generate")?]),
        descriptor,
        draining: false,
    };
    let catalog = CatalogSnapshot::new(1, 90, 110, vec![entry.clone()])?;

    clock.available.store(false, Ordering::SeqCst);
    assert!(matches!(
        registry.apply_catalog(catalog.clone()),
        Err(PeerHttpError::Unavailable(_))
    ));
    assert!(!registry.status().connected);

    clock.available.store(true, Ordering::SeqCst);
    assert!(registry.apply_catalog(catalog.clone()).is_ok());
    assert!(registry.status().connected);

    // A fresh catalog with the same remote descriptor must replace its expired local adapter.
    // The host's one-generation limit also proves that completed generations are reclaimed.
    for generation in 2..12 {
        let now = 100 + generation * 10;
        clock.now.store(now, Ordering::SeqCst);
        let replacement = CatalogSnapshot::new(generation, now, now + 10, vec![entry.clone()])?;
        let facts = registry.apply_catalog(replacement.clone())?;
        assert_eq!(facts.len(), 1);
        assert_eq!(facts[0].catalog_generation, generation);
        assert!(registry.apply_catalog(replacement)?.is_empty());
        assert_eq!(registry.registration_count(), 1);
        let registered = host.catalog_generations(
            &milkdrift_authority::CapabilityAuthorityScope::allow_any(SideEffectClass::Unknown),
        )?;
        assert_eq!(registered.len(), 1);
        let current = &registered[0];
        host.refresh_health(
            current.descriptor.identity(),
            current.descriptor.descriptor_revision(),
            now,
        )?;
        assert!(
            host.catalog_generations(&milkdrift_authority::CapabilityAuthorityScope::allow_any(
                SideEffectClass::Unknown
            ))?[0]
                .observation
                .as_ref()
                .is_some_and(CapabilityObservation::available)
        );
    }

    assert!(matches!(
        registry.apply_catalog(catalog),
        Err(PeerHttpError::Unavailable(_))
    ));
    assert!(!registry.status().connected);

    let relationship_expiry = registry.relationship.expires_at_unix_ms;
    clock
        .now
        .store(relationship_expiry.saturating_add(1), Ordering::SeqCst);
    let live_catalog = CatalogSnapshot::new(
        2,
        relationship_expiry,
        relationship_expiry.saturating_add(10),
        Vec::new(),
    )?;
    assert!(matches!(
        registry.apply_catalog(live_catalog),
        Err(PeerHttpError::Unavailable(_))
    ));
    assert!(!registry.status().connected);
    Ok(())
}
