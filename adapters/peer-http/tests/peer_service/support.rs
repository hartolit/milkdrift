//! Shared peer-service integration fixtures and durable-store mutation helpers.

pub(super) use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
    sync::{
        Arc, Barrier, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

pub(super) use milkdrift_authority::{
    ActorRef, AuthorityBudget, AuthorityDecisionSnapshot, AuthorityExecutionProvenance,
    AuthorityOperation, AuthorityRequest, BoundaryTimeMillis, CapabilityExecutionRequirements,
    DecisionId, DecisionReasonCode, GrantDigest, GrantId, PeerId, PolicyId, RequestedResourceFacts,
    SecretRef, SensitiveSecret,
};
pub(super) use milkdrift_blueprint::{NodeId, RevisionId};
pub(super) use milkdrift_capability::{
    AdmissionConstraints, ArtifactReference as InvocationArtifactReference, BoundedJson,
    CancellationAcknowledgement, CancellationBehavior, CancellationRequest, CapabilityCategory,
    CapabilityDescriptor, CapabilityId, CapabilityObservation, DescriptorBuilder,
    IdempotencyBehavior, InputReference, InvocationAdmissionEnvelope, InvocationEvent,
    InvocationEventKind, InvocationId, InvocationRequest, InvocationTerminal,
    InvocationValueReference, Locality, OperationContract, OperationId, ResolvedCapabilitySnapshot,
    SchemaContract, SchemaId, SideEffectClass, StreamingMode, TerminalStatus, TrustZone,
};
pub(super) use milkdrift_capability_host::{
    AdapterError, AdapterInvocation, AdapterReporter, CapabilityAdapter, CapabilityHost,
    CapabilitySelectionPolicy, HostConfig,
};
pub(super) use milkdrift_peer_http::{
    CorePeerArtifactStore, InsecureLoopbackMode, PeerArtifactError, PeerArtifactStore,
    PeerClientConfig, PeerClock, PeerClockError, PeerHttpError, PeerRelationship, PeerServerConfig,
    PeerService, PeerWorkerConfig, SystemPeerClock,
};
pub(super) use milkdrift_peer_protocol::{
    ArtifactChunk, ArtifactMetadataOffer, ArtifactTransferDecision, ArtifactTransferDirection,
    CancellationDisposition, CatalogDigest, DelegatedAuthorization, DelegationRef, ExecutionLimits,
    HardLimits, HeartbeatLease, InvocationAcceptance, ObservationCategory, ObservationHistory,
    PeerAction, PeerAuthority, PeerCancellationAcknowledgement, PeerCancellationRequest,
    PeerExecutionId, PeerInvocationRequest, PeerObservation, PeerRequestId, ProtocolVersionRange,
    SessionId, TransferId,
};
pub(super) use milkdrift_persistence::{
    ArtifactStore, PageSize, PeerAdmission, PeerAdmissionOutcome, PeerAdmissionRejection,
    PeerArchivedDisposition, PeerCancellationRecord, PeerCatalogState, PeerClaimOutcome,
    PeerDispatchClaimRequest, PeerEntryOutcome, PeerEntryRequest, PeerExecutionPhase,
    PeerExecutionSnapshot, PeerExecutionStore, PeerExecutionTombstone, PeerRelationshipState,
    PeerRetentionRequest, PersistenceError, StorageFailureClass, TimestampMillis, WorkerId,
};
pub(super) use milkdrift_redb_store::{
    FaultInjector, FaultPoint, RedbStore, RedbStoreConfig, injected_failure,
};
pub(super) use milkdrift_workspace::{
    ArtifactId, ArtifactProvenance, ArtifactReference, ArtifactRetention, ArtifactSensitivity,
    CausalId, CausalReference, ContentDigest, MediaType,
};
pub(super) use redb::{Database, ReadableTable, TableDefinition};
pub(super) use serde::Serialize;
pub(super) use url::Url;

pub(super) type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

pub(super) const PEER_EXECUTION_TOMBSTONES: TableDefinition<'static, &'static str, &'static [u8]> =
    TableDefinition::new("milkdrift.v2.peers.executions.tombstones");
pub(super) const PEER_EXECUTION_LOCATIONS: TableDefinition<'static, &'static str, u8> =
    TableDefinition::new("milkdrift.v2.peers.executions.locations");
pub(super) const PEER_EXECUTION_ACCOUNTING: TableDefinition<'static, &'static str, &'static [u8]> =
    TableDefinition::new("milkdrift.v2.peers.accounting");
pub(super) const PEER_EXECUTIONS: TableDefinition<'static, &'static str, &'static [u8]> =
    TableDefinition::new("milkdrift.v2.peers.executions.hot");
pub(super) const PEER_EXECUTIONS_BY_REQUEST: TableDefinition<'static, &'static [u8], &'static str> =
    TableDefinition::new("milkdrift.v2.peers.executions_by_request");

#[derive(Debug)]
struct ControlledClockState {
    observed_at_unix_ms: u64,
    last_unix_ms: u64,
    available: bool,
    unavailable_observations: u64,
}

#[derive(Debug)]
pub(super) struct ControlledPeerClock {
    state: Mutex<ControlledClockState>,
}

impl ControlledPeerClock {
    pub(super) fn new(observed_at_unix_ms: u64) -> Self {
        Self {
            state: Mutex::new(ControlledClockState {
                observed_at_unix_ms,
                last_unix_ms: 0,
                available: true,
                unavailable_observations: 0,
            }),
        }
    }

    pub(super) fn set(&self, observed_at_unix_ms: u64) -> Result<(), PeerClockError> {
        self.state
            .lock()
            .map_err(|_| PeerClockError::Unavailable)?
            .observed_at_unix_ms = observed_at_unix_ms;
        Ok(())
    }

    pub(super) fn set_available(&self, available: bool) -> Result<(), PeerClockError> {
        self.state
            .lock()
            .map_err(|_| PeerClockError::Unavailable)?
            .available = available;
        Ok(())
    }

    pub(super) fn unavailable_observations(&self) -> Result<u64, PeerClockError> {
        Ok(self
            .state
            .lock()
            .map_err(|_| PeerClockError::Unavailable)?
            .unavailable_observations)
    }
}

impl PeerClock for ControlledPeerClock {
    fn now_unix_ms(&self) -> Result<u64, PeerClockError> {
        let mut state = self.state.lock().map_err(|_| PeerClockError::Unavailable)?;
        if !state.available {
            state.unavailable_observations = state.unavailable_observations.saturating_add(1);
            return Err(PeerClockError::Unavailable);
        }
        if state.observed_at_unix_ms < state.last_unix_ms {
            return Err(PeerClockError::MovedBackwards);
        }
        state.last_unix_ms = state.observed_at_unix_ms;
        Ok(state.observed_at_unix_ms)
    }
}

pub(super) fn system_peer_clock() -> Arc<dyn PeerClock> {
    Arc::new(SystemPeerClock::default())
}

pub(super) struct FailOnce {
    point: FaultPoint,
    remaining: AtomicUsize,
}

impl FailOnce {
    pub(super) fn new(point: FaultPoint) -> Self {
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

pub(super) fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(1, |duration| {
            u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
        })
}
pub(super) fn descriptor() -> TestResult<CapabilityDescriptor> {
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

pub(super) struct TerminalAdapter {
    pub(super) capability: CapabilityId,
    pub(super) delay: Duration,
    pub(super) active: Arc<AtomicUsize>,
    pub(super) maximum: Arc<AtomicUsize>,
    pub(super) calls: Arc<AtomicUsize>,
    pub(super) requirements: CapabilityExecutionRequirements,
}

pub(super) struct ClockFailingAdapter {
    pub(super) capability: CapabilityId,
    pub(super) clock: Arc<ControlledPeerClock>,
    pub(super) calls: Arc<AtomicUsize>,
}

impl CapabilityAdapter for ClockFailingAdapter {
    fn admission_envelope(
        &self,
        _invocation: &AdapterInvocation<'_>,
    ) -> Result<InvocationAdmissionEnvelope, AdapterError> {
        Ok(InvocationAdmissionEnvelope::not_applicable())
    }

    fn execute(
        &self,
        _invocation: &AdapterInvocation<'_>,
        _reporter: &dyn AdapterReporter,
    ) -> Result<(), AdapterError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.clock
            .set_available(false)
            .map_err(|error| AdapterError::external_failure(error.to_string()))?;
        Err(AdapterError::external_failure(
            "deterministic adapter failure after entry",
        ))
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

impl CapabilityAdapter for TerminalAdapter {
    fn admission_envelope(
        &self,
        _invocation: &AdapterInvocation<'_>,
    ) -> Result<InvocationAdmissionEnvelope, AdapterError> {
        Ok(InvocationAdmissionEnvelope::not_applicable())
    }

    fn authority_requirements(&self) -> CapabilityExecutionRequirements {
        self.requirements.clone()
    }

    fn execute(
        &self,
        invocation: &AdapterInvocation<'_>,
        reporter: &dyn AdapterReporter,
    ) -> Result<(), AdapterError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
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

pub(super) fn host_with_adapter(
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

pub(super) fn relationship(
    remote_peer: PeerId,
    maximum_concurrent: u16,
) -> TestResult<PeerRelationship> {
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
        artifact_sensitivities: BTreeSet::from([
            ArtifactSensitivity::Public,
            ArtifactSensitivity::Internal,
            ArtifactSensitivity::Restricted,
        ]),
        catalog_ttl_ms: 60_000,
        trust_zone: TrustZone::new("trusted-peer")?,
        delegation: DelegationRef::new("delegation-configured")?,
        revocation_generation: 0,
        expires_at_unix_ms: now().saturating_add(600_000),
        enabled: true,
    })
}

pub(super) fn server_config(
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
            maximum_hot_terminal_records: 1_000,
            archive_batch_size: 16,
            observation_hot_retention: Duration::from_secs(60),
            recovery_page: 32,
            poll_interval: Duration::from_millis(5),
        },
    })
}

pub(super) fn configure_store(
    store: &RedbStore,
    peer: &PeerId,
    digest: &CatalogDigest,
    maximum: u32,
) -> TestResult {
    store.set_peer_admission_open(true)?;
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

pub(super) fn admit(
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
            maximum_hot_terminal_records: 1_000,
            archive_batch_size: 16,
            archive_terminal_before_or_at_unix_ms: 1,
        })?,
        PeerAdmissionOutcome::Accepted(_)
    ));
    Ok(())
}

pub(super) fn claim(
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

pub(super) fn enter(
    store: &RedbStore,
    peer: &PeerId,
    execution: &PeerExecutionId,
    worker: &WorkerId,
    claim_generation: u64,
) -> TestResult {
    assert!(matches!(
        store.mark_peer_entered(&PeerEntryRequest {
            owner: peer,
            execution,
            worker,
            claim_generation,
            relationship_generation: 1,
            entered_at_unix_ms: now(),
            authority: &allowed_decision(peer)?,
        })?,
        PeerEntryOutcome::Entered(_)
    ));
    Ok(())
}

pub(super) fn allowed_decision(peer: &PeerId) -> TestResult<AuthorityDecisionSnapshot> {
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

#[allow(clippy::too_many_arguments)] // This reviewed boundary keeps its complete invariant-bearing fact set explicit.
pub(super) fn request(
    issuer: &PeerId,
    target: &PeerId,
    descriptor: &CapabilityDescriptor,
    catalog_generation: u64,
    catalog_digest: CatalogDigest,
    request_identity: &str,
    invocation_identity: &str,
) -> TestResult<PeerInvocationRequest> {
    request_with_optional_input_artifact(
        issuer,
        target,
        descriptor,
        catalog_generation,
        catalog_digest,
        request_identity,
        invocation_identity,
        None,
    )
}

#[allow(clippy::too_many_arguments)] // Exact peer fixture facts stay visible at the contract boundary.
pub(super) fn request_with_input_artifact(
    issuer: &PeerId,
    target: &PeerId,
    descriptor: &CapabilityDescriptor,
    catalog_generation: u64,
    catalog_digest: CatalogDigest,
    request_identity: &str,
    invocation_identity: &str,
    input_artifact_bytes: u64,
) -> TestResult<PeerInvocationRequest> {
    request_with_optional_input_artifact(
        issuer,
        target,
        descriptor,
        catalog_generation,
        catalog_digest,
        request_identity,
        invocation_identity,
        Some(input_artifact_bytes),
    )
}

#[allow(clippy::too_many_arguments)] // Exact peer fixture facts stay visible at the contract boundary.
pub(super) fn request_with_optional_input_artifact(
    issuer: &PeerId,
    target: &PeerId,
    descriptor: &CapabilityDescriptor,
    catalog_generation: u64,
    catalog_digest: CatalogDigest,
    request_identity: &str,
    invocation_identity: &str,
    input_artifact_bytes: Option<u64>,
) -> TestResult<PeerInvocationRequest> {
    let operation = OperationId::new("test.execute")?;
    let selection = ResolvedCapabilitySnapshot::from_descriptor(descriptor, &operation)?;
    let inputs = input_artifact_bytes
        .map(|size_bytes| {
            Ok(InputReference::new(
                "input-artifact",
                InvocationValueReference::Artifact {
                    reference: InvocationArtifactReference::new(
                        "artifact-input-quota",
                        "c".repeat(64),
                        Some("application/octet-stream".to_owned()),
                        Some(size_bytes),
                    )?,
                },
            )?)
        })
        .into_iter()
        .collect::<TestResult<Vec<_>>>()?;
    let invocation = InvocationRequest::new(
        InvocationId::new(invocation_identity)?,
        descriptor.identity().clone(),
        operation.clone(),
        None,
        None,
        inputs,
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

pub(super) fn terminal_observation(
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

pub(super) fn output_observation(
    request: &PeerInvocationRequest,
    execution: &PeerExecutionId,
    sequence: u64,
    name: &str,
    size_bytes: u64,
    digest_character: char,
) -> TestResult<PeerObservation> {
    Ok(PeerObservation {
        execution: execution.clone(),
        sequence,
        category: ObservationCategory::Artifact,
        event: InvocationEvent::new(
            request.request.invocation().clone(),
            sequence,
            InvocationEventKind::Output {
                name: name.to_owned(),
                reference: InvocationArtifactReference::new(
                    format!("artifact-{name}"),
                    digest_character.to_string().repeat(64),
                    Some("application/octet-stream".to_owned()),
                    Some(size_bytes),
                )?,
            },
        )?,
        observed_at_unix_ms: now(),
    })
}

pub(super) fn progress_observation(
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

pub(super) fn invalid_tombstones(
    valid: &PeerExecutionTombstone,
    request: &PeerInvocationRequest,
) -> TestResult<Vec<PeerExecutionTombstone>> {
    let mut invalid_tombstones = Vec::new();
    macro_rules! invalid_with {
        ($change:expr) => {{
            let mut invalid = valid.clone();
            $change(&mut invalid);
            invalid_tombstones.push(invalid);
        }};
    }
    invalid_with!(|value: &mut PeerExecutionTombstone| value.schema_version = 0);
    invalid_with!(|value: &mut PeerExecutionTombstone| value.relationship_generation = 0);
    invalid_with!(|value: &mut PeerExecutionTombstone| value.acceptance_sequence = 0);
    invalid_with!(|value: &mut PeerExecutionTombstone| value.accepted_at_unix_ms = 0);
    invalid_with!(|value: &mut PeerExecutionTombstone| value.catalog_generation = 0);
    invalid_with!(|value: &mut PeerExecutionTombstone| value.capability_generation = 0);
    invalid_with!(|value: &mut PeerExecutionTombstone| value.authority.grant_revision = 0);
    invalid_with!(|value: &mut PeerExecutionTombstone| value.authority.policy_version = 0);
    invalid_with!(|value: &mut PeerExecutionTombstone| value.archived_at_unix_ms = 0);
    invalid_with!(|value: &mut PeerExecutionTombstone| {
        value.compacted_through_sequence = value.last_observation_sequence.saturating_add(1);
    });
    invalid_with!(|value: &mut PeerExecutionTombstone| {
        value.accounting.observations = value.accounting.observations.saturating_add(1);
    });
    invalid_with!(|value: &mut PeerExecutionTombstone| value.request_digest = "invalid".to_owned());
    invalid_with!(|value: &mut PeerExecutionTombstone| value.catalog_digest = "invalid".to_owned());
    invalid_with!(
        |value: &mut PeerExecutionTombstone| value.capability_digest = "invalid".to_owned()
    );
    invalid_with!(
        |value: &mut PeerExecutionTombstone| value.authority.decision_digest = "invalid".to_owned()
    );
    invalid_with!(
        |value: &mut PeerExecutionTombstone| value.observation_digest = "invalid".to_owned()
    );

    let other_execution = PeerExecutionId::new("execution-fact-corruption-other")?;
    let wrong_execution =
        terminal_observation(request, &other_execution, 1, TerminalStatus::Success)?;
    invalid_with!(|value: &mut PeerExecutionTombstone| {
        value.disposition = PeerArchivedDisposition::Terminal {
            observation: Box::new(wrong_execution.clone()),
        };
    });
    let wrong_sequence = terminal_observation(
        request,
        &valid.execution,
        valid.last_observation_sequence.saturating_add(1),
        TerminalStatus::Success,
    )?;
    invalid_with!(|value: &mut PeerExecutionTombstone| {
        value.disposition = PeerArchivedDisposition::Terminal {
            observation: Box::new(wrong_sequence.clone()),
        };
    });
    let progress =
        progress_observation(request, &valid.execution, valid.last_observation_sequence)?;
    invalid_with!(|value: &mut PeerExecutionTombstone| {
        value.disposition = PeerArchivedDisposition::Terminal {
            observation: Box::new(progress.clone()),
        };
    });
    let mut late = terminal_observation(
        request,
        &valid.execution,
        valid.last_observation_sequence,
        TerminalStatus::Success,
    )?;
    late.observed_at_unix_ms = valid.archived_at_unix_ms.saturating_add(1);
    invalid_with!(|value: &mut PeerExecutionTombstone| {
        value.disposition = PeerArchivedDisposition::Terminal {
            observation: Box::new(late.clone()),
        };
    });
    invalid_with!(|value: &mut PeerExecutionTombstone| {
        value.disposition = PeerArchivedDisposition::Uncertain {
            uncertain_at_unix_ms: 0,
            reason: "reason".to_owned(),
        };
    });
    invalid_with!(|value: &mut PeerExecutionTombstone| {
        value.disposition = PeerArchivedDisposition::Uncertain {
            uncertain_at_unix_ms: value.archived_at_unix_ms.saturating_add(1),
            reason: "reason".to_owned(),
        };
    });
    invalid_with!(|value: &mut PeerExecutionTombstone| {
        value.disposition = PeerArchivedDisposition::Uncertain {
            uncertain_at_unix_ms: value.archived_at_unix_ms,
            reason: String::new(),
        };
    });
    invalid_with!(|value: &mut PeerExecutionTombstone| {
        value.disposition = PeerArchivedDisposition::Uncertain {
            uncertain_at_unix_ms: value.archived_at_unix_ms,
            reason: "x".repeat(2_049),
        };
    });
    let mut wrong_cancellation = valid_cancellation(&valid.execution)?;
    wrong_cancellation.request.execution = other_execution;
    invalid_with!(|value: &mut PeerExecutionTombstone| {
        value.cancellation = Some(wrong_cancellation.clone());
    });
    Ok(invalid_tombstones)
}

pub(super) fn valid_cancellation(
    execution: &PeerExecutionId,
) -> TestResult<PeerCancellationRecord> {
    let request_id = PeerRequestId::new("cancel-fact-corruption")?;
    Ok(PeerCancellationRecord {
        request: PeerCancellationRequest {
            request_id: request_id.clone(),
            execution: execution.clone(),
            sequence: 1,
            reason: "fact corruption cancellation".to_owned(),
        },
        requested_at_unix_ms: 1,
        acknowledgement: Some(PeerCancellationAcknowledgement {
            request_id,
            execution: execution.clone(),
            disposition: CancellationDisposition::Accepted,
            terminal_boundary: false,
            terminal_evidence: None,
            detail: None,
        }),
        acknowledged_at_unix_ms: Some(2),
    })
}

pub(super) fn overwrite_peer_document<T: Serialize>(
    root: &Path,
    table: TableDefinition<'static, &'static str, &'static [u8]>,
    key: &str,
    family: &'static str,
    payload: &T,
) -> TestResult {
    #[derive(Serialize)]
    struct Envelope<'a, T> {
        schema_version: u32,
        family: &'static str,
        checksum: String,
        payload: &'a T,
    }

    let payload_bytes = serde_json::to_vec(payload)?;
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"milkdrift.redb.internal-document.v1\0");
    hasher.update(&u64::try_from(family.len())?.to_be_bytes());
    hasher.update(family.as_bytes());
    hasher.update(&u64::try_from(payload_bytes.len())?.to_be_bytes());
    hasher.update(&payload_bytes);
    let bytes = serde_json::to_vec(&Envelope {
        schema_version: 1,
        family,
        checksum: hasher.finalize().to_hex().to_string(),
        payload,
    })?;
    let database = Database::open(root.join("milkdrift.redb"))?;
    let write = database.begin_write()?;
    write.open_table(table)?.insert(key, bytes.as_slice())?;
    write.commit()?;
    Ok(())
}

pub(super) fn assert_peer_integrity_refuses(root: &Path, case: &str) -> TestResult {
    if let Ok(store) = RedbStore::open(root)
        && store.verify_peer_execution_integrity().is_ok()
    {
        return Err(format!("peer integrity accepted {case}").into());
    }
    Ok(())
}
