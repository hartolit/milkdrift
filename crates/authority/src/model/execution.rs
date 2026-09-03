use milkdrift_blueprint::{NodeId, RevisionId, WorkflowId};
use milkdrift_capability::{IdempotencyBehavior, PeerId};
use milkdrift_workspace::RunId;
use serde::{Deserialize, Serialize};

use crate::{
    ActorRef, AuthorityError, DecisionId, GrantDigest, GrantId, PolicyId, document::canonical_json,
};

use super::{
    decision::{AuthorityDecisionSnapshot, AuthorityRequest, RequestedResourceFacts},
    resource::{AuthorityBudget, AuthorityOperation, BoundaryTimeMillis},
};

/// Exact execution coordinates bound into a future authority decision.
#[derive(Clone, Debug, Default, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AuthorityExecutionProvenance {
    /// Immutable revision whose node requests the capability.
    pub revision: Option<RevisionId>,
    /// Stable semantic node identity.
    pub node: Option<NodeId>,
    /// Exact runtime execution identity in safe canonical text.
    pub execution: Option<String>,
    /// Exact runtime attempt identity in safe canonical text.
    pub attempt: Option<String>,
    /// Exact descriptor generation considered at resolution or entry.
    pub descriptor_revision: Option<u64>,
    /// Authenticated remote peer when the candidate is remote.
    pub peer: Option<PeerId>,
    /// Exact idempotency behavior advertised by the selected operation.
    pub idempotency: Option<IdempotencyBehavior>,
}

impl AuthorityExecutionProvenance {
    pub(super) fn validate(&self) -> Result<(), AuthorityError> {
        if self.descriptor_revision == Some(0)
            || self
                .execution
                .as_ref()
                .is_some_and(|value| !safe_reference(value))
            || self
                .attempt
                .as_ref()
                .is_some_and(|value| !safe_reference(value))
        {
            return Err(AuthorityError::InvalidContract(
                "authority execution provenance contains an invalid identity or generation"
                    .to_owned(),
            ));
        }
        Ok(())
    }
}

fn safe_reference(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 192
        && value.is_ascii()
        && value.as_bytes()[0].is_ascii_alphanumeric()
        && value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':' | b'/')
        })
}

/// Immutable run-level authority pinned when external execution is accepted.
///
/// The basis stores exact references and digests, not a duplicate grant document. Every
/// capability attempt derives a fresh request from it and is evaluated against the current
/// revocation state while historical acceptance remains unchanged.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutionAuthorityBasis {
    schema_version: u32,
    actor: ActorRef,
    grant: GrantId,
    grant_revision: u64,
    grant_digest: GrantDigest,
    policy: PolicyId,
    policy_version: u32,
    workflow: WorkflowId,
    root_run: RunId,
    lineage_revision: RevisionId,
    accepted_decision: DecisionId,
    accepted_decision_digest: String,
    revocation_generation: u64,
    digest: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ExecutionAuthorityBasisWire {
    schema_version: u32,
    actor: ActorRef,
    grant: GrantId,
    grant_revision: u64,
    grant_digest: GrantDigest,
    policy: PolicyId,
    policy_version: u32,
    workflow: WorkflowId,
    root_run: RunId,
    lineage_revision: RevisionId,
    accepted_decision: DecisionId,
    accepted_decision_digest: String,
    revocation_generation: u64,
    digest: String,
}

impl<'de> Deserialize<'de> for ExecutionAuthorityBasis {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let wire = ExecutionAuthorityBasisWire::deserialize(deserializer)?;
        let value = Self {
            schema_version: wire.schema_version,
            actor: wire.actor,
            grant: wire.grant,
            grant_revision: wire.grant_revision,
            grant_digest: wire.grant_digest,
            policy: wire.policy,
            policy_version: wire.policy_version,
            workflow: wire.workflow,
            root_run: wire.root_run,
            lineage_revision: wire.lineage_revision,
            accepted_decision: wire.accepted_decision,
            accepted_decision_digest: wire.accepted_decision_digest,
            revocation_generation: wire.revocation_generation,
            digest: wire.digest,
        };
        value.validate().map_err(serde::de::Error::custom)?;
        Ok(value)
    }
}

impl ExecutionAuthorityBasis {
    /// Freezes the exact allowed start decision as the run's execution basis.
    pub fn from_start_decision(
        decision: &AuthorityDecisionSnapshot,
        workflow: WorkflowId,
        root_run: RunId,
        lineage_revision: RevisionId,
    ) -> Result<Self, AuthorityError> {
        let request = decision.request();
        if !decision.is_allowed() || request.operation != AuthorityOperation::StartRun {
            return Err(AuthorityError::InvalidContract(
                "execution authority requires an allowed start-run decision".to_owned(),
            ));
        }
        let mut value = Self {
            schema_version: 1,
            actor: request.actor.clone(),
            grant: request.grant.clone(),
            grant_revision: request.grant_revision,
            grant_digest: request.grant_digest.clone(),
            policy: decision.policy().clone(),
            policy_version: decision.policy_version(),
            workflow,
            root_run,
            lineage_revision,
            accepted_decision: request.decision.clone(),
            accepted_decision_digest: decision.digest().to_owned(),
            revocation_generation: request.revocation_generation,
            digest: String::new(),
        };
        value.digest = value.compute_digest()?;
        value.validate()?;
        Ok(value)
    }

    /// Derives a new exact request without widening the frozen grant reference.
    pub fn request(
        &self,
        decision: DecisionId,
        operation: AuthorityOperation,
        mut resources: RequestedResourceFacts,
        budget: AuthorityBudget,
        evaluated_at: BoundaryTimeMillis,
        provenance: AuthorityExecutionProvenance,
    ) -> AuthorityRequest {
        resources.workflow = Some(self.workflow.clone());
        resources.run = Some(self.root_run.clone());
        AuthorityRequest {
            decision,
            actor: self.actor.clone(),
            grant: self.grant.clone(),
            grant_revision: self.grant_revision,
            grant_digest: self.grant_digest.clone(),
            revocation_generation: self.revocation_generation,
            operation,
            resources,
            budget,
            evaluated_at,
            provenance,
        }
    }

    /// Actor whose grant is carried into execution.
    #[must_use]
    pub const fn actor(&self) -> &ActorRef {
        &self.actor
    }
    /// Exact grant lineage.
    #[must_use]
    pub const fn grant(&self) -> &GrantId {
        &self.grant
    }
    /// Exact grant revision.
    #[must_use]
    pub const fn grant_revision(&self) -> u64 {
        self.grant_revision
    }
    /// Exact immutable grant digest.
    #[must_use]
    pub const fn grant_digest(&self) -> &GrantDigest {
        &self.grant_digest
    }
    /// Evaluator policy lineage.
    #[must_use]
    pub const fn policy(&self) -> &PolicyId {
        &self.policy
    }
    /// Exact evaluator version used at acceptance.
    #[must_use]
    pub const fn policy_version(&self) -> u32 {
        self.policy_version
    }
    /// Root workflow authority scope.
    #[must_use]
    pub const fn workflow(&self) -> &WorkflowId {
        &self.workflow
    }
    /// Root run authority scope inherited by structured child runs.
    #[must_use]
    pub const fn root_run(&self) -> &RunId {
        &self.root_run
    }
    /// Initial accepted revision-lineage boundary.
    #[must_use]
    pub const fn lineage_revision(&self) -> &RevisionId {
        &self.lineage_revision
    }
    /// Command authorization decision that established the basis.
    #[must_use]
    pub const fn accepted_decision(&self) -> &DecisionId {
        &self.accepted_decision
    }
    /// Digest of the accepted command authorization decision.
    #[must_use]
    pub fn accepted_decision_digest(&self) -> &str {
        &self.accepted_decision_digest
    }
    /// Acceptance-time revocation generation.
    #[must_use]
    pub const fn revocation_generation(&self) -> u64 {
        self.revocation_generation
    }
    /// Domain-separated digest of this complete minimal basis.
    #[must_use]
    pub fn digest(&self) -> &str {
        &self.digest
    }

    fn validate(&self) -> Result<(), AuthorityError> {
        if self.schema_version != 1
            || self.grant_revision == 0
            || self.policy_version == 0
            || !milkdrift_contracts::is_canonical_blake3_digest(&self.accepted_decision_digest)
            || self.digest != self.compute_digest()?
        {
            return Err(AuthorityError::InvalidContract(
                "execution authority basis invariant or digest mismatch".to_owned(),
            ));
        }
        Ok(())
    }

    fn compute_digest(&self) -> Result<String, AuthorityError> {
        #[derive(Serialize)]
        struct Digest<'a> {
            domain: &'static str,
            schema_version: u32,
            actor: &'a ActorRef,
            grant: &'a GrantId,
            grant_revision: u64,
            grant_digest: &'a GrantDigest,
            policy: &'a PolicyId,
            policy_version: u32,
            workflow: &'a WorkflowId,
            root_run: &'a RunId,
            lineage_revision: &'a RevisionId,
            accepted_decision: &'a DecisionId,
            accepted_decision_digest: &'a str,
            revocation_generation: u64,
        }
        let bytes = canonical_json(&Digest {
            domain: "milkdrift.execution-authority-basis.v1",
            schema_version: self.schema_version,
            actor: &self.actor,
            grant: &self.grant,
            grant_revision: self.grant_revision,
            grant_digest: &self.grant_digest,
            policy: &self.policy,
            policy_version: self.policy_version,
            workflow: &self.workflow,
            root_run: &self.root_run,
            lineage_revision: &self.lineage_revision,
            accepted_decision: &self.accepted_decision,
            accepted_decision_digest: &self.accepted_decision_digest,
            revocation_generation: self.revocation_generation,
        })?;
        let mut hasher = blake3::Hasher::new();
        hasher.update(b"milkdrift.execution-authority-basis.v1\0");
        hasher.update(&bytes);
        Ok(format!("b3_{}", hasher.finalize()))
    }
}
