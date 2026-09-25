//! Live capability catalog filtering, caching, and durable generation publication.

use std::collections::{BTreeMap, BTreeSet};

use crate::CatalogGenerationView;
use milkdrift_authority::{AuthorityBudget, AuthorityOperation, RequestedResourceFacts};
use milkdrift_capability::{CapabilityDescriptor, DescriptorBuilder, PeerId};
use milkdrift_peer_protocol::{CatalogEntry, CatalogSnapshot, DrainState};
use milkdrift_persistence::ServingCatalogState;

use super::config::PeerRelationship;
use super::{
    CachedCatalog, PeerService, ServingError, bounded, map_execution_persistence,
    relationship_generation,
};

impl PeerService {
    /// Derives a complete filtered, expiring catalog from the live capability host.
    pub fn catalog(&self, authenticated_peer: &PeerId) -> Result<CatalogSnapshot, ServingError> {
        let relationship = self.relationship(authenticated_peer)?;
        self.require_operation(
            &relationship,
            AuthorityOperation::InspectPeer,
            RequestedResourceFacts::empty(),
            AuthorityBudget::default(),
        )?;
        self.require_operation(
            &relationship,
            AuthorityOperation::ListCapabilities,
            RequestedResourceFacts::empty(),
            AuthorityBudget::default(),
        )?;
        self.require_operation(
            &relationship,
            AuthorityOperation::InspectCapabilityHealth,
            RequestedResourceFacts::empty(),
            AuthorityBudget::default(),
        )?;
        self.require_operation(
            &relationship,
            AuthorityOperation::InspectProviderProfile,
            RequestedResourceFacts::empty(),
            AuthorityBudget::default(),
        )?;
        self.check_rate(&relationship, "catalog")?;
        let now = self.now()?;
        let entries = self.catalog_entries(&relationship)?;
        let fingerprint = catalog_fingerprint(&entries)?;
        let mut catalogs = self
            .catalogs
            .lock()
            .map_err(|_| ServingError::Unavailable("catalog cache unavailable".to_owned()))?;
        if let Some(cached) = catalogs.get(authenticated_peer)
            && cached.fingerprint == fingerprint
            && cached.snapshot.is_live_at(now)
        {
            return Ok(cached.snapshot.clone());
        }
        let durable_generation = self
            .executions
            .peer_catalog(&self.peer_caller(authenticated_peer))
            .map_err(map_execution_persistence)?
            .map_or(0, |catalog| catalog.generation);
        let generation = catalogs.get(authenticated_peer).map_or(
            durable_generation.saturating_add(1).max(1),
            |cached| {
                cached
                    .snapshot
                    .generation
                    .max(durable_generation)
                    .saturating_add(1)
            },
        );
        let expires_at = now
            .saturating_add(relationship.catalog_ttl_ms)
            .min(relationship.expires_at_unix_ms);
        let snapshot = CatalogSnapshot::new(generation, now, expires_at, entries)
            .map_err(|error| ServingError::Protocol(error.to_string()))?;
        self.executions
            .publish_peer_catalog(&ServingCatalogState {
                caller: self.peer_caller(authenticated_peer),
                relationship_generation: relationship_generation(&relationship),
                generation: snapshot.generation,
                digest: snapshot.digest.as_str().to_owned(),
                expires_at_unix_ms: snapshot.expires_at_unix_ms,
            })
            .map_err(map_execution_persistence)?;
        catalogs.insert(
            authenticated_peer.clone(),
            CachedCatalog {
                fingerprint,
                snapshot: snapshot.clone(),
            },
        );
        Ok(snapshot)
    }

    fn catalog_entries(
        &self,
        relationship: &PeerRelationship,
    ) -> Result<Vec<CatalogEntry>, ServingError> {
        if self.drain_state() != DrainState::Ready {
            return Ok(Vec::new());
        }
        let scope = &self
            .grants
            .get(&relationship.remote_peer)
            .ok_or_else(|| ServingError::Unauthorized("peer grant is absent".to_owned()))?
            .resources()
            .capability;
        self.catalog_entries_in_scope(scope, false)
    }

    pub(super) fn catalog_entries_in_scope(
        &self,
        scope: &milkdrift_authority::CapabilityAuthorityScope,
        direct_only: bool,
    ) -> Result<Vec<CatalogEntry>, ServingError> {
        let generations = self.capability_host.catalog_generations(scope)?;
        let mut entries = Vec::new();
        for generation in generations {
            if !generation.current
                || generation.draining
                || (direct_only && !generation.accepts_direct_inputs)
            {
                continue;
            }
            let Some(ref observation) = generation.observation else {
                continue;
            };
            if !observation.available() {
                continue;
            }
            let operations = generation
                .descriptor
                .operations()
                .iter()
                .filter(|(identity, contract)| {
                    scope
                        .operation_selection()
                        .is_some_and(|selection| selection.matches(*identity))
                        && contract.side_effect() <= scope.maximum_side_effect()
                })
                .map(|(identity, contract)| (identity.clone(), contract.clone()))
                .collect::<BTreeMap<_, _>>();
            if operations.is_empty() {
                continue;
            }
            let invocable_operations = operations.keys().cloned().collect::<BTreeSet<_>>();
            let descriptor = filtered_descriptor(&generation, operations)?;
            let observation = milkdrift_capability::CapabilityObservation::new(
                descriptor.identity().clone(),
                observation.observed_at_unix_ms(),
                observation.available(),
                observation.current_load(),
                bounded(observation.health_summary(), 512),
            )
            .map_err(|error| ServingError::Protocol(error.to_string()))?;
            entries.push(CatalogEntry {
                descriptor,
                invocable_operations,
                observation,
                draining: false,
            });
        }
        Ok(entries)
    }
}

fn filtered_descriptor(
    generation: &CatalogGenerationView,
    operations: BTreeMap<
        milkdrift_capability::OperationId,
        milkdrift_capability::OperationContract,
    >,
) -> Result<CapabilityDescriptor, ServingError> {
    DescriptorBuilder::new(
        generation.descriptor.identity().clone(),
        generation.descriptor.descriptor_revision(),
        generation.descriptor.category().clone(),
        generation.descriptor.admission().clone(),
        generation.descriptor.locality(),
    )
    .provider_profile(generation.descriptor.provider_profile().cloned())
    .peer(generation.descriptor.peer().cloned())
    .operations(operations)
    .trust_zones(generation.descriptor.trust_zones().clone())
    .execution_trust(generation.descriptor.execution_trust())
    .resource_observations(generation.descriptor.resource_observations().cloned())
    .labels(generation.descriptor.labels().clone())
    .extensions(generation.descriptor.extensions().clone())
    .build()
    .map_err(|error| ServingError::Protocol(error.to_string()))
}

pub(super) fn catalog_fingerprint(entries: &[CatalogEntry]) -> Result<String, ServingError> {
    // A health heartbeat or load sample must not invalidate an otherwise exact prepared request.
    // The snapshot retains its original truthful observations until its TTL expires. Membership,
    // contracts and draining still change the generation; final entry checks live availability,
    // authority and capacity independently of this discovery snapshot.
    let selection = entries
        .iter()
        .map(|entry| {
            (
                &entry.descriptor,
                &entry.invocable_operations,
                entry.draining,
                entry.observation.available(),
            )
        })
        .collect::<Vec<_>>();
    let bytes = serde_json::to_vec(&selection)
        .map_err(|error| ServingError::Protocol(error.to_string()))?;
    Ok(blake3::hash(&bytes).to_hex().to_string())
}
