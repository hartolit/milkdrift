//! Daemon boundary authority requests and protected-operation audit.
use super::{
    Owner, PublicFailure, read_model::internal, read_model::invalid, read_model::not_found,
    read_model::public_persistence, read_model::snake_debug, read_model::unauthorized_decision,
};
use crate::auth::ActorSession;
use milkdrift_authority::{
    AuthorityBudget, AuthorityDecisionSnapshot, AuthorityExecutionProvenance, AuthorityOperation,
    AuthorityRequest, BoundaryTimeMillis, DecisionId, RequestedResourceFacts,
};
use milkdrift_capability::PeerId;
use milkdrift_persistence::{
    IntegrityDigest, SecurityAuditEntry, SecurityAuditStore, TimestampMillis,
};

impl Owner {
    pub(super) fn authorize(
        &self,
        session: &ActorSession,
        operation: AuthorityOperation,
        resources: RequestedResourceFacts,
        boundary: &str,
    ) -> Result<AuthorityDecisionSnapshot, PublicFailure> {
        let decision = self.evaluate_authority(session, operation, resources, boundary)?;
        if decision.is_allowed() {
            Ok(decision)
        } else {
            Err(unauthorized_decision(&decision))
        }
    }

    pub(super) fn evaluate_authority(
        &self,
        session: &ActorSession,
        operation: AuthorityOperation,
        resources: RequestedResourceFacts,
        boundary: &str,
    ) -> Result<AuthorityDecisionSnapshot, PublicFailure> {
        let claim = session.context.authority();
        let now = self.now()?;
        let mut hasher = blake3::Hasher::new();
        hasher.update(b"milkdrift.daemon-authority.v1\0");
        hasher.update(session.actor.as_str().as_bytes());
        hasher.update(boundary.as_bytes());
        hasher.update(format!("{operation:?}{resources:?}{now}").as_bytes());
        let request = AuthorityRequest {
            decision: DecisionId::new(format!("decision:{}", hasher.finalize()))
                .map_err(|error| invalid(&error.to_string()))?,
            actor: session.actor.clone(),
            grant: claim.grant().clone(),
            grant_revision: claim.grant_revision(),
            grant_digest: claim.grant_digest().clone(),
            revocation_generation: claim.revocation_generation(),
            operation,
            resources,
            budget: AuthorityBudget::default(),
            evaluated_at: BoundaryTimeMillis::new(now),
            provenance: AuthorityExecutionProvenance::default(),
        };
        self.authority.evaluate(&request).map_err(|_| internal())
    }

    pub(super) fn authorize_peer(
        &self,
        session: &ActorSession,
        peer: &str,
        operation: AuthorityOperation,
        boundary: &str,
    ) -> Result<AuthorityDecisionSnapshot, PublicFailure> {
        let peer = PeerId::new(peer.to_owned()).map_err(|error| invalid(&error.to_string()))?;
        let mut resources = RequestedResourceFacts::empty();
        resources.peer = Some(peer.clone());
        let decision = self.authorize(session, operation, resources, boundary)?;
        if !self.peer_registries.contains_key(&peer) {
            return Err(not_found());
        }
        Ok(decision)
    }

    pub(super) fn record_security_decision(
        &self,
        decision: &AuthorityDecisionSnapshot,
    ) -> Result<(), PublicFailure> {
        let request = decision.request();
        let operation = serde_json::to_value(request.operation)
            .ok()
            .and_then(|value| value.as_str().map(str::to_owned))
            .ok_or_else(internal)?;
        let mut resource_hasher = blake3::Hasher::new();
        resource_hasher.update(b"milkdrift.audit-resource.v1\0");
        resource_hasher.update(format!("{:?}", request.resources).as_bytes());
        self.store
            .append_security_audit(&SecurityAuditEntry {
                evaluated_at: TimestampMillis::new(request.evaluated_at.get()),
                actor: request.actor.clone(),
                grant: request.grant.clone(),
                grant_revision: request.grant_revision,
                grant_digest: request.grant_digest.clone(),
                operation,
                resource_digest: IntegrityDigest::new(format!("b3_{}", resource_hasher.finalize()))
                    .map_err(public_persistence)?,
                decision_digest: decision.digest().to_owned(),
                outcome: snake_debug(&decision.outcome()),
                reason_codes: decision.reason_codes().iter().map(snake_debug).collect(),
            })
            .map_err(public_persistence)?;
        Ok(())
    }
}

use milkdrift_authority::AuthorityEvaluator as _;
