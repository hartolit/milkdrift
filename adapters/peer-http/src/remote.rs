use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread,
};

use milkdrift_authority::{AuthorityBudget, CapabilityExecutionRequirements, NetworkProfileRef};
use milkdrift_capability::{
    BoundedJson, CancellationAcknowledgement, CancellationRequest, CapabilityDescriptor,
    CapabilityId, CapabilityObservation, DescriptorBuilder, ErrorClass, ExtensionKey,
    InvocationEvent, InvocationEventKind, InvocationFailure, InvocationId, InvocationRequest,
    InvocationTerminal, Locality, ResolvedCapabilitySnapshot, TerminalStatus,
};
use milkdrift_capability_host::{
    AdapterError, AdapterInvocation, AdapterReporter, CapabilityAdapter, CapabilityHost,
};
use milkdrift_peer_protocol::{
    ArchivedExecutionSummary, CancellationDisposition, CatalogDigest, CatalogSnapshot,
    DelegatedAuthorization, InvocationAcceptance, ObservationHistory, PeerCancellationRequest,
    PeerExecutionId, PeerExecutionProvenance, PeerInvocationRequest, PeerRequestId, SessionId,
};
use serde::{Deserialize, Serialize};

use crate::{PeerClock, PeerHttpClient, PeerHttpError, PeerRelationship};

/// Exact remote facts retained next to each ordinary local adapter registration.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RemoteCapabilityProvenance {
    /// Authenticated remote peer.
    pub peer: milkdrift_authority::PeerId,
    /// Exact remote catalog generation.
    pub catalog_generation: u64,
    /// Exact remote catalog digest.
    pub catalog_digest: CatalogDigest,
    /// Original remote capability identity.
    pub remote_capability: CapabilityId,
    /// Original remote descriptor generation.
    pub remote_descriptor_revision: u64,
    /// Hard catalog expiry.
    pub expires_at_unix_ms: u64,
    /// Process-local registration generation, advanced after an irreversible local drain.
    pub registration_generation: u64,
}

#[derive(Clone)]
struct Registration {
    local_capability: CapabilityId,
    local_revision: u64,
}

/// Safe live peer/session/catalog diagnostics without credentials or endpoint internals.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct PeerRegistryStatus {
    /// True after an authenticated handshake and valid catalog, even when filtering yields zero entries.
    pub connected: bool,
    /// Authenticated remote daemon session identity.
    pub remote_session: Option<SessionId>,
    /// Current verified catalog generation.
    pub catalog_generation: Option<u64>,
    /// Current verified catalog digest.
    pub catalog_digest: Option<CatalogDigest>,
    /// Current catalog expiry.
    pub catalog_expires_at_unix_ms: Option<u64>,
    /// Stable redacted health label.
    pub health: String,
}

/// Catalog consumer that maps each remote generation into the existing capability host.
pub struct PeerRegistry {
    host: CapabilityHost,
    client: Arc<PeerHttpClient>,
    relationship: PeerRelationship,
    clock: Arc<dyn PeerClock>,
    registrations: Mutex<BTreeMap<(CapabilityId, u64), Registration>>,
    registration_generation: Mutex<u64>,
    status: Mutex<PeerRegistryStatus>,
}

impl std::fmt::Debug for PeerRegistry {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PeerRegistry")
            .field("remote_peer", self.client.remote_peer())
            .field("credential", &"[redacted]")
            .finish_non_exhaustive()
    }
}

impl PeerRegistry {
    /// Creates a disconnected registry manager.
    pub fn new(
        host: CapabilityHost,
        client: Arc<PeerHttpClient>,
        relationship: PeerRelationship,
        clock: Arc<dyn PeerClock>,
    ) -> Result<Self, PeerHttpError> {
        relationship.validate()?;
        if &relationship.remote_peer != client.remote_peer() {
            return Err(PeerHttpError::Configuration(
                "relationship and client remote peer identities differ".to_owned(),
            ));
        }
        Ok(Self {
            host,
            client,
            relationship,
            clock,
            registrations: Mutex::new(BTreeMap::new()),
            registration_generation: Mutex::new(0),
            status: Mutex::new(PeerRegistryStatus {
                health: "disconnected".to_owned(),
                ..PeerRegistryStatus::default()
            }),
        })
    }

    fn now(&self) -> Result<u64, PeerHttpError> {
        self.clock
            .now_unix_ms()
            .map_err(|error| PeerHttpError::Unavailable(error.to_string()))
    }

    fn require_relationship_live(&self, now: u64) -> Result<(), PeerHttpError> {
        if now > self.relationship.expires_at_unix_ms {
            return Err(PeerHttpError::Unavailable(
                "remote peer relationship expired".to_owned(),
            ));
        }
        Ok(())
    }

    /// Authenticates, fetches, validates, and generation-safely replaces the remote catalog.
    pub fn connect(&self) -> Result<Vec<RemoteCapabilityProvenance>, PeerHttpError> {
        let result = self
            .now()
            .and_then(|now| self.require_relationship_live(now))
            .and_then(|()| self.client.handshake())
            .and_then(|handshake| {
                self.client.catalog().and_then(|catalog| {
                    let provenance = self.apply_catalog(catalog.clone())?;
                    let mut status = self.status.lock().map_err(|_| {
                        PeerHttpError::Unavailable("peer registry status unavailable".to_owned())
                    })?;
                    status.connected = true;
                    status.remote_session = Some(handshake.session);
                    status.catalog_generation = Some(catalog.generation);
                    status.catalog_digest = Some(catalog.digest);
                    status.catalog_expires_at_unix_ms = Some(catalog.expires_at_unix_ms);
                    status.health = "authenticated_catalog_live".to_owned();
                    Ok(provenance)
                })
            });
        if let Err(error) = &result {
            self.disconnect()?;
            if let Ok(mut status) = self.status.lock() {
                status.health = format!("unavailable:{}", error_class(error));
            }
        }
        result
    }

    /// Authenticated remote peer managed by this registry.
    #[must_use]
    pub const fn remote_peer(&self) -> &milkdrift_authority::PeerId {
        &self.relationship.remote_peer
    }

    /// Current live registration count for operator diagnostics.
    #[must_use]
    pub fn registration_count(&self) -> usize {
        self.registrations
            .lock()
            .map_or(0, |registrations| registrations.len())
    }

    /// Current safe peer/session/catalog diagnostic snapshot.
    #[must_use]
    pub fn status(&self) -> PeerRegistryStatus {
        self.status.lock().map_or_else(
            |_| PeerRegistryStatus {
                health: "unavailable".to_owned(),
                ..PeerRegistryStatus::default()
            },
            |status| status.clone(),
        )
    }

    /// Applies one exact complete snapshot, draining generations absent from the replacement.
    fn apply_catalog(
        &self,
        catalog: CatalogSnapshot,
    ) -> Result<Vec<RemoteCapabilityProvenance>, PeerHttpError> {
        catalog
            .validate()
            .map_err(|error| PeerHttpError::Protocol(error.to_string()))?;
        let now = self.now()?;
        if let Err(error) = self.require_relationship_live(now) {
            self.disconnect()?;
            return Err(error);
        }
        if !catalog.is_live_at(now) {
            self.disconnect()?;
            return Err(PeerHttpError::Unavailable(
                "remote catalog expired before registration".to_owned(),
            ));
        }
        let mut accepted = BTreeSet::new();
        let mut provenance = Vec::new();
        let mut registrations = self.registrations.lock().map_err(|_| {
            PeerHttpError::Unavailable("peer registry state unavailable".to_owned())
        })?;
        let registration_generation = {
            let mut generation = self.registration_generation.lock().map_err(|_| {
                PeerHttpError::Unavailable("peer registry generation unavailable".to_owned())
            })?;
            if registrations.is_empty() {
                *generation = generation.saturating_add(1).max(1);
            }
            *generation
        };
        for entry in &catalog.entries {
            if entry.draining || !entry.observation.available() {
                continue;
            }
            let key = (
                entry.descriptor.identity().clone(),
                entry.descriptor.descriptor_revision(),
            );
            accepted.insert(key.clone());
            if registrations.contains_key(&key) {
                continue;
            }
            let facts = RemoteCapabilityProvenance {
                peer: self.relationship.remote_peer.clone(),
                catalog_generation: catalog.generation,
                catalog_digest: catalog.digest.clone(),
                remote_capability: entry.descriptor.identity().clone(),
                remote_descriptor_revision: entry.descriptor.descriptor_revision(),
                expires_at_unix_ms: catalog.expires_at_unix_ms,
                registration_generation,
            };
            let descriptor = local_descriptor(
                &entry.descriptor,
                &self.relationship,
                &facts,
                &entry.invocable_operations,
            )?;
            let local_capability = descriptor.identity().clone();
            let local_revision = descriptor.descriptor_revision();
            let authority_requirements =
                remote_authority_requirements(self.client.as_ref(), &self.relationship)?;
            let adapter = Arc::new(RemoteCapabilityAdapter {
                client: self.client.clone(),
                relationship: self.relationship.clone(),
                catalog_generation: catalog.generation,
                catalog_digest: catalog.digest.clone(),
                catalog_expires_at_unix_ms: catalog.expires_at_unix_ms,
                remote_descriptor: entry.descriptor.clone(),
                local_capability: local_capability.clone(),
                authority_requirements,
                clock: self.clock.clone(),
                active: Mutex::new(BTreeMap::new()),
                draining: AtomicBool::new(false),
            });
            let observation = CapabilityObservation::new(
                local_capability.clone(),
                entry.observation.observed_at_unix_ms(),
                true,
                entry.observation.current_load(),
                format!("remote peer {} available", self.relationship.remote_peer),
            )
            .map_err(|error| PeerHttpError::Protocol(error.to_string()))?;
            self.host
                .register(descriptor, adapter, Some(observation))
                .map_err(|error| PeerHttpError::Unavailable(error.to_string()))?;
            registrations.insert(
                key,
                Registration {
                    local_capability,
                    local_revision,
                },
            );
            provenance.push(facts);
        }
        let stale = registrations
            .iter()
            .filter(|(key, _registration)| !accepted.contains(*key))
            .map(|(key, registration)| (key.clone(), registration.clone()))
            .collect::<Vec<_>>();
        for (key, registration) in stale {
            let _ = self
                .host
                .begin_drain(&registration.local_capability, registration.local_revision);
            registrations.remove(&key);
        }
        if let Ok(mut status) = self.status.lock() {
            status.connected = true;
            status.catalog_generation = Some(catalog.generation);
            status.catalog_digest = Some(catalog.digest.clone());
            status.catalog_expires_at_unix_ms = Some(catalog.expires_at_unix_ms);
            status.health = "catalog_live".to_owned();
        }
        Ok(provenance)
    }

    /// Drains all registrations immediately on explicit disconnect or authentication loss.
    pub fn disconnect(&self) -> Result<(), PeerHttpError> {
        let mut registrations = self.registrations.lock().map_err(|_| {
            PeerHttpError::Unavailable("peer registry state unavailable".to_owned())
        })?;
        for registration in registrations.values() {
            let _ = self
                .host
                .begin_drain(&registration.local_capability, registration.local_revision);
        }
        registrations.clear();
        let mut status = self.status.lock().map_err(|_| {
            PeerHttpError::Unavailable("peer registry status unavailable".to_owned())
        })?;
        *status = PeerRegistryStatus {
            health: "disconnected".to_owned(),
            ..PeerRegistryStatus::default()
        };
        Ok(())
    }
}

struct RemoteCapabilityAdapter {
    client: Arc<PeerHttpClient>,
    relationship: PeerRelationship,
    catalog_generation: u64,
    catalog_digest: CatalogDigest,
    catalog_expires_at_unix_ms: u64,
    remote_descriptor: CapabilityDescriptor,
    local_capability: CapabilityId,
    authority_requirements: CapabilityExecutionRequirements,
    clock: Arc<dyn PeerClock>,
    active: Mutex<BTreeMap<InvocationId, PeerExecutionId>>,
    draining: AtomicBool,
}

impl CapabilityAdapter for RemoteCapabilityAdapter {
    fn authority_requirements(&self) -> CapabilityExecutionRequirements {
        self.authority_requirements.clone()
    }

    fn execute(
        &self,
        invocation: &AdapterInvocation<'_>,
        reporter: &dyn AdapterReporter,
    ) -> Result<(), AdapterError> {
        if self.draining.load(Ordering::SeqCst) {
            return Err(AdapterError::unavailable(
                "remote peer catalog is unavailable or expired",
            ));
        }
        let now = self
            .clock
            .now_unix_ms()
            .map_err(|error| AdapterError::unavailable(error.to_string()))?;
        if now > self.catalog_expires_at_unix_ms || now > self.relationship.expires_at_unix_ms {
            return Err(AdapterError::unavailable(
                "remote peer catalog is unavailable or expired",
            ));
        }
        let remote_selection = ResolvedCapabilitySnapshot::from_descriptor(
            &self.remote_descriptor,
            invocation.request().operation(),
        )
        .map_err(|error| AdapterError::rejected(error.to_string()))?;
        let remote_request = remap_request(invocation.request(), self.remote_descriptor.identity())
            .map_err(|error| AdapterError::rejected(error.to_string()))?;
        let request_id = PeerRequestId::new(format!(
            "request:{}",
            invocation.request().invocation().as_str()
        ))
        .map_err(|error| AdapterError::rejected(error.to_string()))?;
        let deadline = now
            .saturating_add(self.relationship.execution_limits.duration_ms)
            .min(self.catalog_expires_at_unix_ms)
            .min(self.relationship.expires_at_unix_ms);
        let actor = milkdrift_authority::ActorRef::new(format!(
            "peer:{}",
            self.client.local_peer().as_str()
        ))
        .map_err(|error| AdapterError::rejected(error.to_string()))?;
        let context = invocation.context().ok_or_else(|| {
            AdapterError::rejected("remote execution requires exact durable run provenance")
        })?;
        let delegation = DelegatedAuthorization {
            reference: self.relationship.delegation.clone(),
            issuer_peer: self.client.local_peer().clone(),
            actor,
            target_peer: self.client.remote_peer().clone(),
            capability: remote_selection.capability().clone(),
            operation: remote_selection.operation().clone(),
            request: request_id.clone(),
            limits: self.relationship.execution_limits,
            expires_at_unix_ms: deadline,
            nonce: request_id.as_str().to_owned(),
            provenance: PeerExecutionProvenance {
                run: context.run().to_string(),
                revision: context.revision().to_string(),
                node: context.node().to_string(),
                execution: context.execution().to_string(),
                attempt: context.attempt().to_string(),
            },
        };
        let request = PeerInvocationRequest::new(
            request_id,
            self.catalog_generation,
            self.catalog_digest.clone(),
            remote_selection,
            remote_request,
            self.relationship.execution_limits,
            deadline,
            delegation,
        )
        .map_err(|error| AdapterError::rejected(error.to_string()))?;
        let execution = match self.client.submit(&request) {
            Ok(InvocationAcceptance::Accepted { execution, .. }) => execution,
            Ok(InvocationAcceptance::Archived { summary, .. }) => {
                return report_archived_summary(
                    invocation.request().invocation(),
                    1,
                    invocation.resolution().operation_contract().side_effect(),
                    &summary,
                    reporter,
                );
            }
            Ok(InvocationAcceptance::Rejected {
                code,
                detail,
                retryable,
                ..
            }) => {
                let failure = InvocationFailure::new(
                    if retryable {
                        ErrorClass::RateLimit
                    } else {
                        ErrorClass::InvalidRequest
                    },
                    retryable,
                    code,
                    detail,
                    None,
                )
                .map_err(|error| AdapterError::rejected(error.to_string()))?;
                let terminal = InvocationTerminal::new(
                    TerminalStatus::Rejected,
                    Vec::new(),
                    Some(failure),
                    None,
                    milkdrift_capability::SideEffectClass::None,
                )
                .map_err(|error| AdapterError::rejected(error.to_string()))?;
                return reporter.invocation(
                    InvocationEvent::new(
                        invocation.request().invocation().clone(),
                        1,
                        InvocationEventKind::Terminal { terminal },
                    )
                    .map_err(|error| AdapterError::rejected(error.to_string()))?,
                );
            }
            Err(error) => return Err(AdapterError::unavailable(error.to_string())),
        };
        self.active
            .lock()
            .map_err(|_| AdapterError::external_failure("remote execution map unavailable"))?
            .insert(invocation.request().invocation().clone(), execution.clone());
        let mut after: u64 = 0;
        let result = 'observing: loop {
            let now = match self.clock.now_unix_ms() {
                Ok(now) => now,
                Err(_) => {
                    break report_uncertainty(
                        invocation.request().invocation(),
                        after.saturating_add(1),
                        invocation.resolution().operation_contract().side_effect(),
                        "local clock became unavailable after remote execution acceptance",
                        reporter,
                    );
                }
            };
            if now > deadline {
                break report_uncertainty(
                    invocation.request().invocation(),
                    after.saturating_add(1),
                    invocation.resolution().operation_contract().side_effect(),
                    "remote execution deadline elapsed without terminal evidence",
                    reporter,
                );
            }
            match self.client.observations(&execution, after, 128) {
                Ok(page) => {
                    let archived_summary = match &page.history {
                        ObservationHistory::Archived { summary } => Some(summary.clone()),
                        ObservationHistory::Hot => None,
                    };
                    let empty = page.observations.is_empty();
                    for observation in page.observations {
                        if observation.event.invocation() != invocation.request().invocation()
                            || observation.sequence != after.saturating_add(1)
                        {
                            break 'observing Err(AdapterError::external_failure(
                                "remote observation stream was not contiguous",
                            ));
                        }
                        after = observation.sequence;
                        if let Err(error) = reporter.invocation(observation.event) {
                            break 'observing Err(error);
                        }
                    }
                    if let Some(summary) = archived_summary {
                        break report_archived_summary(
                            invocation.request().invocation(),
                            after.saturating_add(1),
                            invocation.resolution().operation_contract().side_effect(),
                            &summary,
                            reporter,
                        );
                    }
                    if page.closed {
                        break Ok(());
                    }
                    if empty {
                        if let Err(error) = reporter.heartbeat() {
                            break Err(error);
                        }
                        thread::sleep(self.client.observation_poll_interval());
                    }
                }
                Err(PeerHttpError::NotFound(_)) => {
                    break report_uncertainty(
                        invocation.request().invocation(),
                        after.saturating_add(1),
                        invocation.resolution().operation_contract().side_effect(),
                        "accepted remote execution record became irrecoverably unavailable",
                        reporter,
                    );
                }
                Err(_) => {
                    let _ = reporter.heartbeat();
                    thread::sleep(self.client.observation_poll_interval());
                }
            }
        };
        if let Ok(mut active) = self.active.lock() {
            active.remove(invocation.request().invocation());
        }
        result
    }

    fn cancel(
        &self,
        request: &CancellationRequest,
    ) -> Result<CancellationAcknowledgement, AdapterError> {
        let execution = self
            .active
            .lock()
            .map_err(|_| AdapterError::external_failure("remote execution map unavailable"))?
            .get(request.invocation())
            .cloned()
            .ok_or_else(|| AdapterError::unavailable("remote execution identity is unknown"))?;
        let peer_request = PeerCancellationRequest {
            request_id: PeerRequestId::new(format!(
                "cancel:{}:{}",
                request.invocation().as_str(),
                request.request_sequence()
            ))
            .map_err(|error| AdapterError::rejected(error.to_string()))?,
            execution,
            sequence: request.request_sequence(),
            reason: request.reason().to_owned(),
        };
        match self.client.cancel(&peer_request) {
            Ok(acknowledgement) => CancellationAcknowledgement::new(
                request.invocation().clone(),
                request.request_sequence(),
                acknowledgement.disposition == CancellationDisposition::Accepted,
                acknowledgement.terminal_boundary,
                acknowledgement.detail,
            )
            .map_err(|error| AdapterError::external_failure(error.to_string())),
            Err(error) => CancellationAcknowledgement::new(
                request.invocation().clone(),
                request.request_sequence(),
                false,
                false,
                Some(format!("cancellation acknowledgement unknown: {error}")),
            )
            .map_err(|contract| AdapterError::external_failure(contract.to_string())),
        }
    }

    fn health(&self, observed_at_unix_ms: u64) -> Result<CapabilityObservation, AdapterError> {
        let available = !self.draining.load(Ordering::SeqCst)
            && observed_at_unix_ms <= self.catalog_expires_at_unix_ms
            && observed_at_unix_ms <= self.relationship.expires_at_unix_ms;
        CapabilityObservation::new(
            self.local_capability.clone(),
            observed_at_unix_ms,
            available,
            u32::try_from(self.active.lock().map_or(0, |active| active.len())).unwrap_or(u32::MAX),
            if available {
                "authenticated remote peer catalog is live"
            } else {
                "authenticated remote peer catalog expired or is draining"
            },
        )
        .map_err(|error| AdapterError::external_failure(error.to_string()))
    }

    fn begin_drain(&self) -> Result<(), AdapterError> {
        self.draining.store(true, Ordering::SeqCst);
        Ok(())
    }
}

fn remote_authority_requirements(
    client: &PeerHttpClient,
    relationship: &PeerRelationship,
) -> Result<CapabilityExecutionRequirements, PeerHttpError> {
    let network_profile =
        NetworkProfileRef::new(format!("peer:{}", relationship.remote_peer.as_str()))
            .map_err(|error| PeerHttpError::Configuration(error.to_string()))?;
    Ok(CapabilityExecutionRequirements {
        network_profiles: BTreeSet::from([network_profile]),
        network_destinations: BTreeSet::from([client.endpoint_destination()]),
        budget: AuthorityBudget {
            cost_minor: Some(
                relationship
                    .execution_limits
                    .cost_micros
                    .saturating_add(9_999)
                    / 10_000,
            ),
            duration_ms: Some(relationship.execution_limits.duration_ms),
            invocations: Some(1),
            artifact_bytes: Some(
                relationship
                    .execution_limits
                    .artifact_bytes
                    .max(relationship.maximum_artifact_bytes),
            ),
            concurrency: Some(1),
            ..AuthorityBudget::default()
        },
        ..CapabilityExecutionRequirements::default()
    })
}

fn remap_request(
    request: &InvocationRequest,
    remote_capability: &CapabilityId,
) -> Result<InvocationRequest, milkdrift_capability::ContractError> {
    let mut mapped = InvocationRequest::new(
        request.invocation().clone(),
        remote_capability.clone(),
        request.operation().clone(),
        request.provider_profile().cloned(),
        request.idempotency_key().cloned(),
        request.inputs().to_vec(),
        request.extensions().clone(),
    )?;
    if let Some(manifest) = request.context_manifest() {
        mapped = mapped.with_context_manifest(manifest.clone())?;
    }
    Ok(mapped)
}

fn local_descriptor(
    remote: &CapabilityDescriptor,
    relationship: &PeerRelationship,
    provenance: &RemoteCapabilityProvenance,
    allowed_operations: &BTreeSet<milkdrift_capability::OperationId>,
) -> Result<CapabilityDescriptor, PeerHttpError> {
    let identity = mapped_capability_id(&relationship.remote_peer, remote.identity())?;
    let revision = mapped_revision(provenance);
    let operations = remote
        .operations()
        .iter()
        .filter(|(identity, _contract)| allowed_operations.contains(*identity))
        .map(|(identity, contract)| (identity.clone(), contract.clone()))
        .collect();
    let mut trust_zones = remote.trust_zones().clone();
    trust_zones.insert(relationship.trust_zone.clone());
    let mut extensions = remote.extensions().clone();
    let extension = ExtensionKey::new("dev.milkdrift.peer/provenance")
        .map_err(|error| PeerHttpError::Protocol(error.to_string()))?;
    let value = serde_json::to_value(provenance)
        .map_err(|error| PeerHttpError::Protocol(error.to_string()))?;
    extensions.insert(
        extension,
        BoundedJson::new(value).map_err(|error| PeerHttpError::Protocol(error.to_string()))?,
    );
    DescriptorBuilder::new(
        identity,
        revision,
        remote.category().clone(),
        remote.admission().clone(),
        Locality::Peer,
    )
    .peer(Some(relationship.remote_peer.clone()))
    .provider_profile(remote.provider_profile().cloned())
    .operations(operations)
    .trust_zones(trust_zones)
    .execution_trust(remote.execution_trust())
    .resource_observations(remote.resource_observations().cloned())
    .labels(remote.labels().clone())
    .extensions(extensions)
    .build()
    .map_err(|error| PeerHttpError::Protocol(error.to_string()))
}

fn mapped_capability_id(
    peer: &milkdrift_authority::PeerId,
    remote: &CapabilityId,
) -> Result<CapabilityId, PeerHttpError> {
    let peer_hash = &blake3::hash(peer.as_str().as_bytes()).to_hex()[..12];
    let candidate = format!("peer:{peer_hash}:{}", remote.as_str());
    let value = if candidate.len() <= 128 {
        candidate
    } else {
        let mut hasher = blake3::Hasher::new();
        hasher.update(peer.as_str().as_bytes());
        hasher.update(remote.as_str().as_bytes());
        format!("peer:{}", &hasher.finalize().to_hex()[..40])
    };
    CapabilityId::new(value).map_err(|error| PeerHttpError::Protocol(error.to_string()))
}

fn mapped_revision(provenance: &RemoteCapabilityProvenance) -> u64 {
    let bytes = serde_json::to_vec(provenance).unwrap_or_default();
    let digest = blake3::hash(&bytes);
    let mut array = [0_u8; 8];
    array.copy_from_slice(&digest.as_bytes()[..8]);
    u64::from_le_bytes(array).max(1)
}

fn report_uncertainty(
    invocation: &InvocationId,
    sequence: u64,
    side_effect: milkdrift_capability::SideEffectClass,
    message: &str,
    reporter: &dyn AdapterReporter,
) -> Result<(), AdapterError> {
    let failure = InvocationFailure::new(
        ErrorClass::Unknown,
        false,
        "remote_outcome_uncertain",
        message,
        None,
    )
    .map_err(|error| AdapterError::external_failure(error.to_string()))?;
    let terminal = InvocationTerminal::new(
        TerminalStatus::Uncertain,
        Vec::new(),
        Some(failure),
        None,
        side_effect,
    )
    .map_err(|error| AdapterError::external_failure(error.to_string()))?;
    reporter.invocation(
        InvocationEvent::new(
            invocation.clone(),
            sequence,
            InvocationEventKind::Terminal { terminal },
        )
        .map_err(|error| AdapterError::external_failure(error.to_string()))?,
    )
}

fn report_archived_summary(
    invocation: &InvocationId,
    sequence: u64,
    side_effect: milkdrift_capability::SideEffectClass,
    summary: &ArchivedExecutionSummary,
    reporter: &dyn AdapterReporter,
) -> Result<(), AdapterError> {
    if let Some(observation) = &summary.final_observation
        && let Some(terminal) = observation.event.kind().terminal()
    {
        let event = InvocationEvent::new(
            invocation.clone(),
            sequence,
            InvocationEventKind::Terminal {
                terminal: terminal.clone(),
            },
        )
        .map_err(|error| AdapterError::external_failure(error.to_string()))?;
        return reporter.invocation(event);
    }
    report_uncertainty(
        invocation,
        sequence,
        side_effect,
        summary
            .uncertainty_reason
            .as_deref()
            .unwrap_or("remote execution history was archived without terminal evidence"),
        reporter,
    )
}

fn error_class(error: &PeerHttpError) -> &'static str {
    match error {
        PeerHttpError::Configuration(_) => "configuration",
        PeerHttpError::Unauthenticated => "unauthenticated",
        PeerHttpError::Unauthorized(_) => "unauthorized",
        PeerHttpError::Protocol(_) => "protocol",
        PeerHttpError::Transport(_) => "transport",
        PeerHttpError::NotFound(_) => "not_found",
        PeerHttpError::Overloaded(_) => "overloaded",
        PeerHttpError::Persistence(_) => "persistence",
        PeerHttpError::Unavailable(_) => "unavailable",
    }
}

#[cfg(test)]
mod tests {
    use std::{
        sync::atomic::{AtomicBool, AtomicU64},
        time::Duration,
    };

    use milkdrift_authority::{PeerId, SensitiveSecret};
    use milkdrift_capability::{SideEffectClass, TrustZone};
    use milkdrift_capability_host::{CapabilityHost, CapabilitySelectionPolicy, HostConfig};
    use milkdrift_peer_protocol::{
        CatalogSnapshot, DelegationRef, ExecutionLimits, PeerAuthority, ProtocolVersionRange,
        SessionId,
    };
    use url::Url;

    use super::*;
    use crate::{InsecureLoopbackMode, PeerClientConfig, PeerClockError};

    struct ControlledClock {
        now: AtomicU64,
        available: AtomicBool,
    }

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
        let registry = PeerRegistry::new(host, client, relationship, clock.clone())?;
        let catalog = CatalogSnapshot::new(1, 90, 110, Vec::new())?;

        clock.available.store(false, Ordering::SeqCst);
        assert!(matches!(
            registry.apply_catalog(catalog.clone()),
            Err(PeerHttpError::Unavailable(_))
        ));
        assert!(!registry.status().connected);

        clock.available.store(true, Ordering::SeqCst);
        assert!(registry.apply_catalog(catalog.clone()).is_ok());
        assert!(registry.status().connected);

        clock.now.store(111, Ordering::SeqCst);
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
}
