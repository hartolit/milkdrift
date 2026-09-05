//! Configured peer composition, rotating credentials and live registration lifecycle.
use super::{
    DaemonHost, HostError, Owner, PublicFailure, clock::DurableClock,
    peer_store::OwnerPeerArtifactStore, peer_store::OwnerPeerExecutionStore, queue::OwnerQueue,
    read_model::bounded, read_model::invalid, read_model::not_found, read_model::unauthorized,
};
use crate::{config::PeerHostConfig, config::PeerSideEffectConfig};
use milkdrift_authority::SecretRef;
use milkdrift_capability::{CapabilityId, PeerId, SideEffectClass};
use milkdrift_capability_host::CapabilityHost;
use milkdrift_control_protocol::{ErrorCode, PeerRead};
use milkdrift_local_secret::LocalSecretResolver;
use milkdrift_peer_http::{
    CorePeerArtifactStore, InsecureLoopbackMode, PeerAuthenticator, PeerClientConfig,
    PeerCredentialSource, PeerHttpClient, PeerHttpError, PeerRegistry, PeerRelationship,
    PeerServerConfig, PeerService, PeerWorkerConfig,
};
use milkdrift_peer_protocol::{
    DelegationRef, ExecutionLimits, HardLimits, HeartbeatLease, PROTOCOL_MAJOR_V1, PeerAuthority,
    ProtocolVersion, ProtocolVersionRange, SessionId,
};
use milkdrift_persistence::TimestampMillis;
use milkdrift_redb_store::RedbStore;
use std::{collections::BTreeMap, collections::BTreeSet, sync::Arc, sync::Weak, time::Duration};
use subtle::ConstantTimeEq as _;

impl DaemonHost {
    /// Returns the optional distinct peer route service for router composition.
    #[must_use]
    pub(crate) fn peer_service(&self) -> Option<Arc<PeerService>> {
        self.peer_service.clone()
    }

    /// Returns stable sorted peer health/catalog observations without secret values.
    #[must_use]
    pub(crate) fn peers(&self) -> Vec<PeerRead> {
        let revoked = self.revoked_peers.lock().ok();
        self.peer_registries
            .values()
            .map(|registry| {
                let status = registry.status();
                let registered_capabilities = registry.registration_count();
                let is_revoked = revoked
                    .as_ref()
                    .is_some_and(|peers| peers.contains(registry.remote_peer()));
                PeerRead {
                    peer_id: registry.remote_peer().as_str().to_owned(),
                    connected: status.connected && !is_revoked,
                    health: if is_revoked {
                        "revoked".to_owned()
                    } else {
                        status.health
                    },
                    session_id: status
                        .remote_session
                        .map(|session| session.as_str().to_owned()),
                    catalog_generation: status.catalog_generation,
                    catalog_digest: status
                        .catalog_digest
                        .map(|digest| digest.as_str().to_owned()),
                    registered_capabilities,
                    catalog_expires_at_unix_ms: status.catalog_expires_at_unix_ms,
                    revoked: is_revoked,
                }
            })
            .collect()
    }

    /// Manually authenticates and refreshes one configured remote peer catalog.
    pub(crate) async fn connect_peer(&self, peer: &PeerId) -> Result<PeerRead, HostError> {
        if self
            .revoked_peers
            .lock()
            .map_err(|_| HostError::Configuration("peer revocation state unavailable".to_owned()))?
            .contains(peer)
        {
            return Err(HostError::Configuration(
                "peer is revoked until daemon configuration reload/restart".to_owned(),
            ));
        }
        let registry = self
            .peer_registries
            .get(peer)
            .cloned()
            .ok_or_else(|| HostError::Configuration("peer is not configured".to_owned()))?;
        tokio::task::spawn_blocking(move || registry.connect())
            .await
            .map_err(|_| HostError::Startup("peer connector task failed".to_owned()))?
            .map_err(|error| HostError::Startup(error.to_string()))?;
        self.peers()
            .into_iter()
            .find(|status| status.peer_id == peer.as_str())
            .ok_or_else(|| HostError::Startup("peer status disappeared".to_owned()))
    }

    /// Explicitly disconnects and drains one peer's local adapter registrations.
    pub(crate) async fn disconnect_peer(&self, peer: &PeerId) -> Result<PeerRead, HostError> {
        let registry = self
            .peer_registries
            .get(peer)
            .cloned()
            .ok_or_else(|| HostError::Configuration("peer is not configured".to_owned()))?;
        tokio::task::spawn_blocking(move || registry.disconnect())
            .await
            .map_err(|_| HostError::Shutdown("peer disconnect task failed".to_owned()))?
            .map_err(|error| HostError::Shutdown(error.to_string()))?;
        self.peers()
            .into_iter()
            .find(|status| status.peer_id == peer.as_str())
            .ok_or_else(|| HostError::Shutdown("peer status disappeared".to_owned()))
    }

    /// Revokes one live relationship and drains its registrations until reload/restart.
    pub(crate) async fn revoke_peer(&self, peer: &PeerId) -> Result<PeerRead, HostError> {
        let durable_peer = peer.clone();
        self.dispatch(false, move |owner| owner.revoke_peer(&durable_peer))
            .await
            .map_err(|error| HostError::Configuration(error.message))?;
        self.revoked_peers
            .lock()
            .map_err(|_| HostError::Configuration("peer revocation state unavailable".to_owned()))?
            .insert(peer.clone());
        self.disconnect_peer(peer).await
    }
}

impl Owner {
    pub(super) fn begin_peer_drain(&self) -> Result<(), PublicFailure> {
        let Some(service) = self.peer_service.as_ref().and_then(Weak::upgrade) else {
            return if self.peer_service.is_some() {
                Err(peer_unavailable())
            } else {
                Ok(())
            };
        };
        service.begin_drain().map_err(public_peer)
    }

    pub(super) fn revoke_peer(&self, peer: &PeerId) -> Result<(), PublicFailure> {
        let service = self
            .peer_service
            .as_ref()
            .and_then(Weak::upgrade)
            .ok_or_else(peer_unavailable)?;
        service.revoke_peer(peer).map_err(public_peer)
    }
}

pub(super) struct PeerRuntime {
    pub(super) service: Option<Arc<PeerService>>,
    pub(super) artifacts: Option<Arc<CorePeerArtifactStore>>,
    pub(super) registries: BTreeMap<PeerId, Arc<PeerRegistry>>,
    pub(super) clock: DurableClock,
}

pub(super) fn build_peer_runtime(
    peers: &PeerHostConfig,
    execution_lease_ms: u64,
    host: &CapabilityHost,
    store: Arc<RedbStore>,
    secrets: Arc<LocalSecretResolver>,
    owner_queue: OwnerQueue,
    clock: DurableClock,
) -> Result<PeerRuntime, String> {
    let PeerHostConfig::Enabled {
        local_peer_id,
        relationships: configured_relationships,
        serving,
    } = peers
    else {
        return Ok(PeerRuntime {
            service: None,
            artifacts: None,
            registries: BTreeMap::new(),
            clock,
        });
    };
    let local_peer = PeerId::new(local_peer_id.clone()).map_err(|error| error.to_string())?;
    let mut session_hasher = blake3::Hasher::new();
    session_hasher.update(b"milkdrift.peer.session.v1\0");
    session_hasher.update(local_peer.as_str().as_bytes());
    let session_now = clock
        .now()
        .map(TimestampMillis::get)
        .map_err(|_| "daemon clock unavailable during peer initialization".to_owned())?;
    session_hasher.update(&session_now.to_be_bytes());
    let session = SessionId::new(format!("session:{}", session_hasher.finalize().to_hex()))
        .map_err(|error| error.to_string())?;
    let versions = ProtocolVersionRange::default();
    let mut relationships = Vec::new();
    let mut clients = Vec::new();
    let mut authentication = Vec::new();
    for configured in configured_relationships {
        let reference =
            SecretRef::new(configured.credential_ref.clone()).map_err(|error| error.to_string())?;
        let credential = Arc::new(
            secrets
                .resolve(&reference)
                .map_err(|error| error.to_string())?,
        );
        let credential_source = Arc::new(ConfiguredPeerCredential {
            resolver: secrets.clone(),
            reference: reference.clone(),
        });
        let remote_peer =
            PeerId::new(configured.peer_id.clone()).map_err(|error| error.to_string())?;
        let relationship_versions = ProtocolVersionRange::new(
            ProtocolVersion {
                major: PROTOCOL_MAJOR_V1,
                minor: configured.minimum_minor,
            },
            ProtocolVersion {
                major: PROTOCOL_MAJOR_V1,
                minor: configured.maximum_minor,
            },
        )
        .map_err(|error| error.to_string())?;
        let capability_allow = configured
            .capability_allow
            .iter()
            .cloned()
            .map(CapabilityId::new)
            .collect::<Result<BTreeSet<_>, _>>()
            .map_err(|error| error.to_string())?;
        let capability_deny = configured
            .capability_deny
            .iter()
            .cloned()
            .map(CapabilityId::new)
            .collect::<Result<BTreeSet<_>, _>>()
            .map_err(|error| error.to_string())?;
        let operation_allow = configured
            .operation_allow
            .iter()
            .cloned()
            .map(milkdrift_capability::OperationId::new)
            .collect::<Result<BTreeSet<_>, _>>()
            .map_err(|error| error.to_string())?;
        let maximum_side_effect = match configured.maximum_side_effect {
            PeerSideEffectConfig::None => SideEffectClass::None,
            PeerSideEffectConfig::ReadOnly => SideEffectClass::ReadOnly,
            PeerSideEffectConfig::IdempotentWrite => SideEffectClass::IdempotentWrite,
            PeerSideEffectConfig::NonIdempotentWrite => SideEffectClass::NonIdempotentWrite,
            PeerSideEffectConfig::Unknown => SideEffectClass::Unknown,
        };
        let relationship = PeerRelationship {
            remote_peer: remote_peer.clone(),
            bearer_credential: credential.clone(),
            versions: relationship_versions,
            authority: PeerAuthority {
                actions: configured.actions.clone(),
            },
            capability_allow,
            capability_deny,
            operation_allow,
            maximum_side_effect,
            execution_filesystem: configured.execution_filesystem.clone(),
            execution_network_profiles: configured.execution_network_profiles.clone(),
            execution_network_destinations: configured.execution_network_destinations.clone(),
            execution_secrets: configured.execution_secrets.clone(),
            execution_limits: ExecutionLimits {
                artifact_bytes: configured.maximum_artifact_bytes,
                duration_ms: configured.maximum_duration_ms,
                cost_micros: configured.maximum_cost_micros,
                observations: configured.maximum_observations,
            },
            maximum_concurrent: configured.maximum_concurrent,
            maximum_requests_per_minute: configured.maximum_requests_per_minute,
            maximum_artifact_bytes: configured.maximum_artifact_bytes,
            artifact_sensitivities: configured.artifact_sensitivities.clone(),
            catalog_ttl_ms: configured.catalog_ttl_ms,
            trust_zone: milkdrift_capability::TrustZone::new(configured.trust_zone.clone())
                .map_err(|error| error.to_string())?,
            delegation: DelegationRef::new(configured.delegation_ref.clone())
                .map_err(|error| error.to_string())?,
            revocation_generation: configured.revocation_generation,
            expires_at_unix_ms: configured.expires_at_unix_ms,
            enabled: configured.enabled,
        };
        relationship.validate().map_err(|error| error.to_string())?;
        let endpoint = url::Url::parse(&configured.endpoint).map_err(|error| error.to_string())?;
        let client = PeerHttpClient::new_with_credential_source(
            PeerClientConfig {
                endpoint,
                local_peer: local_peer.clone(),
                expected_remote_peer: remote_peer,
                session: session.clone(),
                versions: relationship_versions,
                bearer_credential: credential,
                insecure_loopback: if configured.insecure_loopback_development {
                    InsecureLoopbackMode::AllowInsecureLoopbackDevelopment
                } else {
                    InsecureLoopbackMode::Disabled
                },
                request_timeout: Duration::from_secs(30),
                observation_poll_interval: Duration::from_millis(100),
            },
            credential_source,
        )
        .map_err(|error| error.to_string())?;
        authentication.push(ConfiguredPeerAuthentication {
            peer: relationship.remote_peer.clone(),
            reference,
            enabled: relationship.enabled,
            expires_at_unix_ms: relationship.expires_at_unix_ms,
        });
        clients.push((client, relationship.clone()));
        relationships.push(relationship);
    }
    let peer_clock = clock.peer_adapter();
    let direct_artifacts = Arc::new(
        CorePeerArtifactStore::new(
            store.clone(),
            configured_relationships
                .iter()
                .map(|relationship| relationship.maximum_artifact_bytes)
                .max()
                .unwrap_or(1),
            10 * 1_073_741_824,
            peer_clock.clone(),
        )
        .map_err(|error| error.to_string())?,
    );
    let executions = Arc::new(OwnerPeerExecutionStore::new(
        owner_queue.clone(),
        Arc::downgrade(&store),
    ));
    let artifacts = Arc::new(OwnerPeerArtifactStore::new(
        owner_queue,
        Arc::downgrade(&direct_artifacts),
    ));
    let service = PeerService::new_with_artifacts_and_authenticator(
        PeerServerConfig {
            local_peer,
            session,
            versions,
            limits: HardLimits::default(),
            lease: HeartbeatLease {
                heartbeat_ms: 5_000,
                idle_timeout_ms: 20_000,
                execution_lease_ms,
            },
            relationships,
            workers: PeerWorkerConfig {
                threads: serving.worker_threads,
                maximum_global_active: serving.maximum_global_active,
                maximum_dispatch_queue: serving.maximum_dispatch_queue,
                maximum_hot_terminal_records: serving.maximum_hot_terminal_records,
                archive_batch_size: serving.archive_batch_size,
                observation_hot_retention: Duration::from_millis(
                    serving.observation_hot_retention_ms,
                ),
                recovery_page: serving.recovery_page,
                poll_interval: Duration::from_millis(serving.poll_interval_ms),
            },
        },
        host.clone(),
        executions,
        artifacts,
        Some(Arc::new(ConfiguredPeerAuthenticator {
            resolver: secrets,
            relationships: authentication,
        })),
        peer_clock.clone(),
    )
    .map_err(|error| error.to_string())?;
    let mut registries = BTreeMap::new();
    for (client, relationship) in clients {
        let peer = relationship.remote_peer.clone();
        let registry = Arc::new(
            PeerRegistry::new(host.clone(), client, relationship, peer_clock.clone())
                .map_err(|error| error.to_string())?,
        );
        registries.insert(peer, registry);
    }
    Ok(PeerRuntime {
        service: Some(service),
        artifacts: Some(direct_artifacts),
        registries,
        clock,
    })
}

struct ConfiguredPeerCredential {
    resolver: Arc<LocalSecretResolver>,
    reference: SecretRef,
}

impl PeerCredentialSource for ConfiguredPeerCredential {
    fn resolve(&self) -> Result<milkdrift_authority::SensitiveSecret, PeerHttpError> {
        self.resolver.resolve(&self.reference).map_err(|_| {
            PeerHttpError::Unavailable("peer credential source unavailable".to_owned())
        })
    }
}

struct ConfiguredPeerAuthentication {
    peer: PeerId,
    reference: SecretRef,
    enabled: bool,
    expires_at_unix_ms: u64,
}

struct ConfiguredPeerAuthenticator {
    resolver: Arc<LocalSecretResolver>,
    relationships: Vec<ConfiguredPeerAuthentication>,
}

impl PeerAuthenticator for ConfiguredPeerAuthenticator {
    fn authenticate(&self, supplied: &[u8], now_unix_ms: u64) -> Option<PeerId> {
        self.relationships
            .iter()
            .filter(|relationship| {
                relationship.enabled && now_unix_ms <= relationship.expires_at_unix_ms
            })
            .find(|relationship| {
                self.resolver
                    .resolve(&relationship.reference)
                    .ok()
                    .is_some_and(|expected| {
                        expected.expose(|bytes| {
                            bytes.len() == supplied.len() && bool::from(bytes.ct_eq(supplied))
                        })
                    })
            })
            .map(|relationship| relationship.peer.clone())
    }
}

pub(super) fn peer_unavailable() -> PublicFailure {
    PublicFailure::new(ErrorCode::Unavailable, "peer service is unavailable", true)
}

pub(super) fn public_peer(error: PeerHttpError) -> PublicFailure {
    match error {
        PeerHttpError::Unauthenticated | PeerHttpError::Unauthorized(_) => unauthorized(),
        PeerHttpError::NotFound(_) => not_found(),
        PeerHttpError::Overloaded(_) => PublicFailure::new(
            ErrorCode::Overload,
            "peer service capacity is exhausted",
            true,
        ),
        PeerHttpError::Persistence(_)
        | PeerHttpError::Unavailable(_)
        | PeerHttpError::Transport(_) => peer_unavailable(),
        PeerHttpError::Configuration(message) | PeerHttpError::Protocol(message) => {
            invalid(&bounded(&message))
        }
    }
}

use milkdrift_capability_host::SecretResolver as _;
