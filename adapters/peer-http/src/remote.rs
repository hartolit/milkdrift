use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{
        Arc, Mutex,
        atomic::{AtomicU8, Ordering},
    },
    thread,
};

use milkdrift_authority::{AuthorityBudget, CapabilityExecutionRequirements, NetworkProfileRef};
use milkdrift_capability::{
    AdmissionBound, BoundedJson, CancellationAcknowledgement, CancellationRequest,
    CapabilityDescriptor, CapabilityId, CapabilityObservation, DescriptorBuilder, ErrorClass,
    ExtensionKey, InvocationAdmissionEnvelope, InvocationEvent, InvocationEventKind,
    InvocationFailure, InvocationId, InvocationRequest, InvocationTerminal, Locality, PeerId,
    ResolvedCapabilitySnapshot, TerminalStatus,
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
    pub peer: PeerId,
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

#[derive(Default)]
struct Registrations {
    active: BTreeMap<(CapabilityId, u64), Registration>,
    // Retained only while exact host permits remain; the host bounds total generations.
    draining: Vec<Registration>,
}

impl Registrations {
    fn reap(&mut self, host: &CapabilityHost) -> Result<(), PeerHttpError> {
        let mut index = 0;
        while let Some(registration) = self.draining.get(index) {
            match host.finish_drain(&registration.local_capability, registration.local_revision) {
                Ok(()) => {
                    self.draining.swap_remove(index);
                }
                Err(milkdrift_capability_host::HostError::InFlight(_)) => index += 1,
                Err(error) => return Err(PeerHttpError::Unavailable(error.to_string())),
            }
        }
        Ok(())
    }

    fn retire(
        &mut self,
        key: &(CapabilityId, u64),
        host: &CapabilityHost,
    ) -> Result<(), PeerHttpError> {
        if let Some(registration) = self.active.get(key) {
            host.begin_drain(&registration.local_capability, registration.local_revision)
                .map_err(|error| PeerHttpError::Unavailable(error.to_string()))?;
            if let Some(registration) = self.active.remove(key) {
                self.draining.push(registration);
            }
        }
        self.reap(host)
    }
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

/// Make a configured peer's advertised operations selectable by the local runtime.
///
/// [`Self::connect`] verifies a complete catalog and registers ordinary adapters with exact
/// remote provenance. Renewal replaces catalog-bound local generations; old held permits drain.
/// [`Self::disconnect`] stops new resolution without cancelling remote executions already accepted.
pub struct PeerRegistry {
    host: CapabilityHost,
    client: Arc<PeerHttpClient>,
    relationship: PeerRelationship,
    clock: Arc<dyn PeerClock>,
    registrations: Mutex<Registrations>,
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
            registrations: Mutex::new(Registrations::default()),
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
    pub const fn remote_peer(&self) -> &PeerId {
        &self.relationship.remote_peer
    }

    /// Current live registration count for operator diagnostics.
    #[must_use]
    pub fn registration_count(&self) -> usize {
        self.registrations
            .lock()
            .map_or(0, |registrations| registrations.active.len())
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
        registrations.reap(&self.host)?;
        let registration_generation = {
            let mut generation = self.registration_generation.lock().map_err(|_| {
                PeerHttpError::Unavailable("peer registry generation unavailable".to_owned())
            })?;
            if registrations.active.is_empty() {
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
            if registrations
                .active
                .get(&key)
                .is_some_and(|registered| registered.local_revision == local_revision)
            {
                continue;
            }
            registrations.retire(&key, &self.host)?;
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
                lifecycle: AtomicU8::new(Lifecycle::Created as u8),
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
            registrations.active.insert(
                key,
                Registration {
                    local_capability,
                    local_revision,
                },
            );
            provenance.push(facts);
        }
        let stale = registrations
            .active
            .iter()
            .filter(|(key, _registration)| !accepted.contains(*key))
            .map(|(key, _registration)| key.clone())
            .collect::<Vec<_>>();
        for key in stale {
            registrations.retire(&key, &self.host)?;
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
        for key in registrations.active.keys().cloned().collect::<Vec<_>>() {
            registrations.retire(&key, &self.host)?;
        }
        registrations.reap(&self.host)?;
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
    lifecycle: AtomicU8,
}

impl CapabilityAdapter for RemoteCapabilityAdapter {
    fn authority_requirements(&self) -> CapabilityExecutionRequirements {
        self.authority_requirements.clone()
    }

    fn admission_envelope(
        &self,
        _invocation: &AdapterInvocation<'_>,
    ) -> Result<InvocationAdmissionEnvelope, AdapterError> {
        Ok(remote_admission_envelope())
    }

    fn start(&self) -> Result<(), AdapterError> {
        loop {
            let prior = self.lifecycle.load(Ordering::SeqCst);
            if prior == Lifecycle::Started as u8 {
                return Ok(());
            }
            if prior != Lifecycle::Created as u8 {
                return Err(AdapterError::rejected(
                    "remote capability adapter cannot restart after drain or shutdown",
                ));
            }
            if self
                .lifecycle
                .compare_exchange(
                    prior,
                    Lifecycle::Started as u8,
                    Ordering::SeqCst,
                    Ordering::SeqCst,
                )
                .is_ok()
            {
                return Ok(());
            }
        }
    }

    fn execute(
        &self,
        invocation: &AdapterInvocation<'_>,
        reporter: &dyn AdapterReporter,
    ) -> Result<(), AdapterError> {
        let lifecycle = self.lifecycle.load(Ordering::SeqCst);
        if lifecycle != Lifecycle::Started as u8 && lifecycle != Lifecycle::Draining as u8 {
            return Err(AdapterError::unavailable(
                "remote capability adapter is not accepting exact work",
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
            if self.lifecycle.load(Ordering::SeqCst) == Lifecycle::Stopped as u8 {
                break report_uncertainty(
                    invocation.request().invocation(),
                    after.saturating_add(1),
                    invocation.resolution().operation_contract().side_effect(),
                    "remote adapter shutdown interrupted observation before terminal evidence",
                    reporter,
                );
            }
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
        let available = self.lifecycle.load(Ordering::SeqCst) == Lifecycle::Started as u8
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
        loop {
            let prior = self.lifecycle.load(Ordering::SeqCst);
            if prior == Lifecycle::Draining as u8 {
                return Ok(());
            }
            if prior != Lifecycle::Started as u8 {
                return Err(AdapterError::rejected(
                    "remote capability adapter must be started before drain",
                ));
            }
            if self
                .lifecycle
                .compare_exchange(
                    prior,
                    Lifecycle::Draining as u8,
                    Ordering::SeqCst,
                    Ordering::SeqCst,
                )
                .is_ok()
            {
                return Ok(());
            }
        }
    }

    fn shutdown(&self) -> Result<(), AdapterError> {
        self.lifecycle
            .store(Lifecycle::Stopped as u8, Ordering::SeqCst);
        Ok(())
    }
}

#[repr(u8)]
enum Lifecycle {
    Created = 0,
    Started = 1,
    Draining = 2,
    Stopped = 3,
}

// Relationship quotas bound what the origin asks for. They do not establish a locally enforceable
// resource reservation for work running on another host.
fn remote_admission_envelope() -> InvocationAdmissionEnvelope {
    InvocationAdmissionEnvelope::new(
        AdmissionBound::Unknown,
        AdmissionBound::Unknown,
        AdmissionBound::Unknown,
        AdmissionBound::Unknown,
    )
}

#[cfg(test)]
mod admission_tests {
    use super::*;

    #[test]
    fn remote_generation_exposes_no_local_enforceable_resource_claim() {
        let first = remote_admission_envelope();
        let second = remote_admission_envelope();
        assert_eq!(first, second);
        assert!(first.input_units().is_unknown());
        assert!(first.output_units().is_unknown());
        assert!(first.artifact_bytes().is_unknown());
        assert!(first.monetary_cost().is_unknown());
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
    peer: &PeerId,
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
mod tests;
