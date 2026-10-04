//! Independent calls enter the same durable admission and worker path as peer delegations.
use super::{
    CachedCatalog, PeerService, ServingError, map_execution_persistence, rejection, store,
};
use milkdrift_authority::{
    ActorRef, AuthorityBudget, AuthorityExecutionProvenance, AuthorityOperation,
    RequestedResourceFacts,
};
use milkdrift_peer_protocol::{
    CatalogSnapshot, ClientInvocationAuthorization, DirectInvocationDraft, DirectInvocationRequest,
    DrainState, InvocationAcceptance, InvocationLookup, PeerExecutionId, PeerRequestId,
    ServingAuthorization, ServingCaller, ServingPrincipal,
};
use milkdrift_persistence::{PeerExecutionSnapshot, ServingCatalogState};

impl PeerService {
    /// Prepares one current authorized direct or published call without accepting work.
    ///
    /// Submission rechecks catalog, authority, inputs and limits. Preparation reserves neither
    /// a generation nor execution capacity and must never replace exact replay after submission.
    ///
    /// # Errors
    /// Rejects the wrong host, unavailable or unauthorized operations, invalid inputs or limits,
    /// and deadline overflow. Discovery, authority, and clock failures also propagate.
    pub fn prepare_client_invocation(
        &self,
        actor: &ActorRef,
        draft: &DirectInvocationDraft,
    ) -> Result<DirectInvocationRequest, ServingError> {
        use milkdrift_capability::{
            IdempotencyKey, InvocationId, InvocationRequest, ResolvedCapabilitySnapshot,
            SideEffectClass,
        };
        let protocol = |error: &dyn std::fmt::Display| ServingError::Protocol(error.to_string());
        if draft.host != self.config.local_peer {
            return Err(ServingError::Protocol(
                "direct request targets another host".into(),
            ));
        }
        let discovery = self.client_discovery(actor)?;
        let entry = discovery
            .catalog
            .entries
            .iter()
            .find(|entry| {
                entry.descriptor.identity() == &draft.capability
                    && entry.invocable_operations.contains(&draft.operation)
                    && !entry.draining
            })
            .ok_or_else(|| {
                ServingError::Unauthorized("capability operation is not currently callable".into())
            })?;
        let selection =
            ResolvedCapabilitySnapshot::from_descriptor(&entry.descriptor, &draft.operation)
                .map_err(|error| protocol(&error))?;
        let key = (selection.operation_contract().side_effect()
            == SideEffectClass::IdempotentWrite)
            .then(|| IdempotencyKey::new(draft.request_id.as_str()))
            .transpose()
            .map_err(|error| protocol(&error))?;
        let limits = draft.limits.clone().unwrap_or(discovery.limits);
        let deadline_unix_ms = self.now()?.checked_add(limits.duration_ms).ok_or_else(|| {
            ServingError::Protocol("invocation deadline exceeds the clock range".into())
        })?;
        let request = DirectInvocationRequest {
            host: discovery.host,
            request_id: draft.request_id.clone(),
            catalog_generation: discovery.catalog.generation,
            catalog_digest: discovery.catalog.digest,
            selection,
            request: InvocationRequest::new(
                InvocationId::new(draft.request_id.as_str()).map_err(|error| protocol(&error))?,
                draft.capability.clone(),
                draft.operation.clone(),
                entry.descriptor.provider_profile().cloned(),
                key,
                draft.inputs.clone(),
                Default::default(),
            )
            .map_err(|error| protocol(&error))?,
            limits,
            deadline_unix_ms,
        };
        let bound = request
            .bind(self.client_authorization(actor)?)
            .map_err(|error| protocol(&error))?;
        self.authorize_serving_request(&self.client_caller(actor), &bound)?;
        Ok(request)
    }

    /// Discovers this installation and the client's currently authorized exact generations.
    ///
    /// # Errors
    /// Rejects absent client policy or insufficient current catalog authority; propagates
    /// catalog rate, clock, registry, encoding, and persistence failures.
    pub fn client_discovery(
        &self,
        actor: &ActorRef,
    ) -> Result<milkdrift_peer_protocol::DirectDiscovery, ServingError> {
        Ok(milkdrift_peer_protocol::DirectDiscovery {
            host: self.config.local_peer.clone(),
            limits: self
                .client_policy
                .as_ref()
                .ok_or(ServingError::Unauthenticated)?
                .execution_limits
                .clone(),
            catalog: self.client_catalog(actor)?,
        })
    }

    /// Projects a protected invocation without exposing internal claims or storage documents.
    ///
    /// # Errors
    /// Rejects unknown or unowned executions and insufficient current inspection/output
    /// authority. Rate, clock, and persistence failures propagate.
    pub fn client_read(
        &self,
        actor: &ActorRef,
        execution: &PeerExecutionId,
    ) -> Result<milkdrift_peer_protocol::ServingInvocationRead, ServingError> {
        let snapshot = self.client_inspect(actor, execution)?;
        let (authorization, capability, revision, operation, accounting, cancellation) =
            match &snapshot {
                PeerExecutionSnapshot::Hot(record) => (
                    &record.request.authorization,
                    record.request.selection.capability(),
                    record.request.selection.descriptor_revision(),
                    record.request.selection.operation(),
                    &record.accounting,
                    &record.cancellation,
                ),
                PeerExecutionSnapshot::Archived(record) => (
                    &record.authorization,
                    &record.capability,
                    record.capability_generation,
                    &record.operation,
                    &record.accounting,
                    &record.cancellation,
                ),
            };
        Ok(milkdrift_peer_protocol::ServingInvocationRead {
            caller: snapshot.caller().clone(),
            origin: authorization.origin(),
            acceptance: store::lookup(&snapshot),
            capability: capability.clone(),
            descriptor_revision: revision,
            operation: operation.clone(),
            artifact_bytes: accounting.artifact_bytes,
            duration_ms: accounting.duration_ms,
            cost_micros: accounting.cost_micros,
            cancellation: cancellation
                .as_ref()
                .and_then(|record| record.acknowledgement.clone()),
        })
    }
    /// Requests durable cancellation; acknowledgement is separate from terminal evidence.
    ///
    /// # Errors
    /// Rejects malformed cancellation, unknown executions, insufficient current cancellation
    /// authority, and rate limits. Durable request or acknowledgement failures propagate;
    /// cancellation receipt alone is never terminal evidence.
    pub fn cancel_client(
        &self,
        actor: &ActorRef,
        request: &milkdrift_peer_protocol::PeerCancellationRequest,
    ) -> Result<milkdrift_peer_protocol::PeerCancellationAcknowledgement, ServingError> {
        self.check_client_rate(actor, "cancel")?;
        if request.sequence == 0 || request.reason.is_empty() || request.reason.len() > 512 {
            return Err(ServingError::Protocol(
                "invalid cancellation request".to_owned(),
            ));
        }
        let caller = self.client_caller(actor);
        let before = self
            .executions
            .peer_execution(&caller, &request.execution)
            .map_err(map_execution_persistence)?
            .ok_or_else(|| ServingError::NotFound("execution was not found".to_owned()))?;
        self.require_client_execution(actor, &before, AuthorityOperation::CancelCapability)?;
        self.cancel_owned(&caller, request, before)
    }

    /// Reads a bounded contiguous page after checking current access to all disclosed artifacts.
    ///
    /// # Errors
    /// Rejects invalid page bounds, missing executions, insufficient current inspection/output
    /// authority, and rate limits. Persistence and noncontiguous observation failures propagate.
    pub fn client_observations(
        &self,
        actor: &ActorRef,
        execution: &PeerExecutionId,
        after_sequence: u64,
        maximum: u32,
    ) -> Result<milkdrift_peer_protocol::ObservationPage, ServingError> {
        self.check_client_rate(actor, "observations")?;
        let record = self.client_inspect(actor, execution)?;
        self.require_client_execution(actor, &record, AuthorityOperation::Inspect)?;
        let page = self
            .executions
            .peer_observations(
                &self.client_caller(actor),
                execution,
                after_sequence,
                milkdrift_persistence::PageSize::new(
                    maximum.min(u32::from(self.config.limits.observation_items)),
                )
                .map_err(|error| ServingError::Protocol(error.to_string()))?,
            )
            .map_err(map_execution_persistence)?;
        // Archival may have replaced the inspected hot record. Authorize the snapshot
        // actually disclosed, including any retained output references in its summary.
        self.require_client_execution(actor, &page.execution, AuthorityOperation::Inspect)?;
        if page.observations.iter().any(|item| {
            item.event.kind().output().is_some()
                || item
                    .event
                    .kind()
                    .terminal()
                    .is_some_and(|terminal| !terminal.outputs().is_empty())
        }) {
            self.require_client_execution(
                actor,
                &page.execution,
                AuthorityOperation::ReadCapabilityOutput,
            )?;
        }
        store::observation_page(
            page,
            execution,
            after_sequence,
            usize::from(self.config.limits.observation_items),
        )
    }
    pub(super) fn client_caller(&self, actor: &ActorRef) -> ServingCaller {
        ServingCaller {
            host: self.config.local_peer.clone(),
            principal: ServingPrincipal::Client {
                actor: actor.clone(),
            },
        }
    }

    /// Discovers exact current generations after client authentication and resource authorization.
    ///
    /// # Errors
    /// Rejects unconfigured/revoked clients, insufficient catalog permissions, and rate limits.
    /// Clock, registry, encoding, cache, and durable catalog failures propagate.
    pub fn client_catalog(&self, actor: &ActorRef) -> Result<CatalogSnapshot, ServingError> {
        self.check_client_rate(actor, "catalog")?;
        let grant = self.client_grant(actor)?;
        let policy = self
            .client_policy
            .as_ref()
            .ok_or(ServingError::Unauthenticated)?;
        for operation in [
            AuthorityOperation::ListCapabilities,
            AuthorityOperation::InspectCapabilityHealth,
            AuthorityOperation::InspectProviderProfile,
        ] {
            self.evaluate_client(
                actor,
                operation,
                RequestedResourceFacts::empty(),
                AuthorityBudget::default(),
                AuthorityExecutionProvenance::default(),
            )?;
        }
        let mut entries = if self.drain_state() == DrainState::Ready {
            self.catalog_entries_in_scope(&grant.resources().capability, true)?
        } else {
            Vec::new()
        };
        entries.retain(|entry| {
            let mut resources = RequestedResourceFacts::empty();
            resources.capability = Some(entry.descriptor.identity().clone());
            resources.category = Some(entry.descriptor.category().clone());
            resources.provider_profile = entry.descriptor.provider_profile().cloned();
            resources.locality = Some(entry.descriptor.locality());
            [
                AuthorityOperation::ListCapabilities,
                AuthorityOperation::InspectCapabilityHealth,
                AuthorityOperation::InspectProviderProfile,
            ]
            .into_iter()
            .all(|operation| {
                self.evaluate_client(
                    actor,
                    operation,
                    resources.clone(),
                    AuthorityBudget::default(),
                    AuthorityExecutionProvenance::default(),
                )
                .is_ok()
            })
        });
        let fingerprint = super::catalog::catalog_fingerprint(&entries)?;
        let now = self.now()?;
        let mut catalogs = self.client_catalogs.lock().map_err(|_| {
            ServingError::Unavailable("client catalog state unavailable".to_owned())
        })?;
        if let Some(cached) = catalogs.get(actor)
            && cached.fingerprint == fingerprint
            && cached.snapshot.is_live_at(now)
        {
            return Ok(cached.snapshot.clone());
        }
        let caller = self.client_caller(actor);
        let generation = self
            .executions
            .peer_catalog(&caller)
            .map_err(map_execution_persistence)?
            .map_or(1, |catalog| catalog.generation.saturating_add(1));
        let snapshot = CatalogSnapshot::new(
            generation,
            now,
            now.saturating_add(policy.catalog_ttl_ms)
                .min(grant.valid_until().get()),
            entries,
        )
        .map_err(|error| ServingError::Protocol(error.to_string()))?;
        self.executions
            .publish_peer_catalog(&ServingCatalogState {
                caller,
                relationship_generation: policy.authority_generation(grant)?,
                generation,
                digest: snapshot.digest.as_str().to_owned(),
                expires_at_unix_ms: snapshot.expires_at_unix_ms,
            })
            .map_err(map_execution_persistence)?;
        catalogs.insert(
            actor.clone(),
            CachedCatalog {
                fingerprint,
                snapshot: snapshot.clone(),
            },
        );
        Ok(snapshot)
    }

    /// Accepts an independent client request after resolving its server-owned authority basis.
    ///
    /// # Errors
    /// Rejects missing client authority, malformed submissions, or inaccessible replay records;
    /// propagates rate, clock, and persistence failures. Admission refusals, including exact
    /// request conflicts, can also be returned as a successful rejection response.
    pub fn invoke_client(
        &self,
        actor: &ActorRef,
        submission: &DirectInvocationRequest,
    ) -> Result<InvocationAcceptance, ServingError> {
        self.check_client_rate(actor, "invoke")?;
        let caller = self.client_caller(actor);
        self.client_grant(actor)?;
        let existing = self
            .executions
            .peer_execution_by_request(&caller, &submission.request_id)
            .map_err(map_execution_persistence)?;
        // Replay binds the original accepted grant; current permission separately governs disclosure.
        let basis = if let Some(existing) = &existing {
            self.require_client_execution(actor, existing, AuthorityOperation::Inspect)?;
            let authorization = match existing {
                PeerExecutionSnapshot::Hot(record) => &record.request.authorization,
                PeerExecutionSnapshot::Archived(record) => &record.authorization,
            };
            match authorization {
                ServingAuthorization::Client(basis) => basis.clone(),
                ServingAuthorization::Peer(_) => {
                    return Err(ServingError::Persistence(
                        "client replay resolved a peer authorization".to_owned(),
                    ));
                }
            }
        } else {
            self.client_authorization(actor)?
        };
        let request = submission
            .bind(basis)
            .map_err(|error| ServingError::Protocol(error.to_string()))?;
        if let Some(existing) = existing {
            return if existing.request_digest() == request.request_digest {
                Ok(store::acceptance(&existing, true))
            } else {
                Ok(rejection(
                    &request,
                    "idempotency_conflict",
                    "request key already names different accepted bytes",
                    false,
                    Some(existing.execution().clone()),
                ))
            };
        }
        if self.drain_state() != DrainState::Ready {
            return Ok(rejection(
                &request,
                "draining",
                "serving admission is closed",
                true,
                None,
            ));
        }
        if self.now()? > request.deadline_unix_ms {
            return Ok(rejection(
                &request,
                "deadline",
                "request deadline elapsed",
                false,
                None,
            ));
        }
        let catalog = self.client_catalog(actor)?;
        if catalog.generation != request.catalog_generation
            || catalog.digest != request.catalog_digest
        {
            return Ok(rejection(
                &request,
                "catalog_stale",
                "selected catalog generation is no longer current",
                true,
                None,
            ));
        }
        let entry = catalog
            .entries
            .iter()
            .find(|entry| {
                entry.descriptor.identity() == request.selection.capability()
                    && entry.descriptor.descriptor_revision()
                        == request.selection.descriptor_revision()
                    && entry
                        .invocable_operations
                        .contains(request.selection.operation())
            })
            .ok_or_else(|| {
                ServingError::Unauthorized(
                    "selected generation and operation are not advertised".to_owned(),
                )
            })?;
        request
            .selection
            .validate_against(&entry.descriptor)
            .map_err(|error| ServingError::Protocol(error.to_string()))?;
        let (decision, generation) = self.authorize_serving_request(&caller, &request)?;
        self.accept_serving(request, decision, generation)
    }

    fn client_authorization(
        &self,
        actor: &ActorRef,
    ) -> Result<ClientInvocationAuthorization, ServingError> {
        let grant = self.client_grant(actor)?;
        Ok(ClientInvocationAuthorization {
            host: self.config.local_peer.clone(),
            actor: actor.clone(),
            grant: grant.identity().clone(),
            grant_revision: grant.revision(),
            grant_digest: grant
                .digest()
                .map_err(|error| ServingError::Configuration(error.to_string()))?,
            revocation_generation: grant.revocation_generation(),
        })
    }

    /// Looks up only the authenticated client's request namespace, including archived work.
    ///
    /// # Errors
    /// Rejects insufficient current inspection/output authority or rate limits, and propagates
    /// clock/storage failures. A successfully checked absent request returns `NotAccepted`.
    pub fn client_lookup(
        &self,
        actor: &ActorRef,
        request: &PeerRequestId,
    ) -> Result<InvocationLookup, ServingError> {
        self.check_client_rate(actor, "lookup")?;
        self.evaluate_client(
            actor,
            AuthorityOperation::Inspect,
            RequestedResourceFacts::empty(),
            AuthorityBudget::default(),
            AuthorityExecutionProvenance::default(),
        )?;
        let existing = self
            .executions
            .peer_execution_by_request(&self.client_caller(actor), request)
            .map_err(map_execution_persistence)?;
        if let Some(existing) = &existing {
            self.require_client_execution(actor, existing, AuthorityOperation::Inspect)?;
        }
        Ok(existing.as_ref().map_or_else(
            || InvocationLookup::NotAccepted {
                request_id: request.clone(),
            },
            store::lookup,
        ))
    }

    /// Inspects only an execution owned by this authenticated client, with current permissions.
    ///
    /// # Errors
    /// Rejects missing or unowned executions, insufficient current inspection/output authority,
    /// and rate limits. Clock and persistence failures propagate.
    pub fn client_inspect(
        &self,
        actor: &ActorRef,
        execution: &PeerExecutionId,
    ) -> Result<PeerExecutionSnapshot, ServingError> {
        self.check_client_rate(actor, "inspect")?;
        let record = self
            .executions
            .peer_execution(&self.client_caller(actor), execution)
            .map_err(map_execution_persistence)?
            .ok_or_else(|| ServingError::NotFound("execution was not found".to_owned()))?;
        self.require_client_execution(actor, &record, AuthorityOperation::Inspect)?;
        Ok(record)
    }

    pub(super) fn require_client_execution(
        &self,
        actor: &ActorRef,
        execution: &PeerExecutionSnapshot,
        operation: AuthorityOperation,
    ) -> Result<(), ServingError> {
        let mut resources = RequestedResourceFacts::empty();
        match execution {
            PeerExecutionSnapshot::Hot(record) => {
                for reference in record
                    .request
                    .request
                    .inputs()
                    .iter()
                    .filter_map(|input| input.value().artifact())
                {
                    self.authorize_client_artifact(actor, reference)?;
                }
                resources.capability = Some(record.request.selection.capability().clone());
                resources.capability_operation = Some(record.request.selection.operation().clone());
                resources.side_effect = record.request.selection.operation_contract().side_effect();
            }
            PeerExecutionSnapshot::Archived(record) => {
                resources.capability = Some(record.capability.clone());
                resources.capability_operation = Some(record.operation.clone());
                resources.side_effect = record.side_effect;
            }
        }
        // Archived lookup/replay embeds the retained terminal summary. Its output references
        // require the same current result permission as a hot observation page.
        if operation == AuthorityOperation::Inspect
            && matches!(execution, PeerExecutionSnapshot::Archived(record)
                if !record.output_observations.is_empty() || record.disposition.terminal_observation().is_some_and(|observation|
                    observation.event.kind().terminal().is_some_and(|terminal| !terminal.outputs().is_empty())))
        {
            self.evaluate_client(
                actor,
                AuthorityOperation::ReadCapabilityOutput,
                resources.clone(),
                AuthorityBudget::default(),
                AuthorityExecutionProvenance::default(),
            )?;
        }
        self.evaluate_client(
            actor,
            operation,
            resources,
            AuthorityBudget::default(),
            AuthorityExecutionProvenance::default(),
        )?;
        Ok(())
    }
}
