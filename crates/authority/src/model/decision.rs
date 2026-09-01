use std::collections::BTreeSet;

use milkdrift_blueprint::{RevisionId, WorkflowId};
use milkdrift_capability::{
    CapabilityCategory, CapabilityId, ExecutionTrustClass, Locality, OperationId,
    ProviderProfileRef, SideEffectClass, TrustZone,
};
use milkdrift_workspace::{ArtifactId, ArtifactSensitivity, RunId, ScopeId};
use serde::{Deserialize, Serialize};

use crate::{
    ActorRef, AuthorityError, DecisionId, GrantDigest, GrantId, NetworkProfileRef, PeerId,
    PolicyId, SecretRef, document::canonical_json,
};

use super::{
    capability::CapabilityAuthorityScope,
    execution::AuthorityExecutionProvenance,
    resource::{
        AuthorityBudget, AuthorityOperation, BoundaryTimeMillis, FilesystemScope, LayoutOwner,
        MAX_SCOPE_ITEMS, NetworkScope,
    },
};

const MAX_DIAGNOSTIC_CODES: usize = 16;
const DECISION_DIGEST_DOMAIN: &[u8] = b"milkdrift.authority-decision.v2\0";

/// Exact resource facts requested at one authorization boundary.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RequestedResourceFacts {
    /// Workflow lineage when known.
    pub workflow: Option<WorkflowId>,
    /// Run aggregate when known.
    pub run: Option<RunId>,
    /// Capability identity when selection is exact.
    pub capability: Option<CapabilityId>,
    /// Capability category when known.
    pub category: Option<CapabilityCategory>,
    /// Exact capability operation when known.
    pub capability_operation: Option<OperationId>,
    /// Provider profile when known.
    pub provider_profile: Option<ProviderProfileRef>,
    /// Trust zone requested or selected.
    pub trust_zone: Option<TrustZone>,
    /// Complete trust-zone set advertised by an exact candidate.
    #[serde(default)]
    pub trust_zones: BTreeSet<TrustZone>,
    /// Exact execution-isolation/trust class advertised by an exact candidate.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub execution_trust_class: Option<ExecutionTrustClass>,
    /// Execution locality when known.
    pub locality: Option<Locality>,
    /// Authenticated remote peer when a remote candidate is exact.
    pub peer: Option<PeerId>,
    /// Semantic selection envelope at workflow acceptance; absent for exact candidates.
    pub capability_envelope: Option<CapabilityAuthorityScope>,
    /// Maximum side effect of the requested operation.
    pub side_effect: SideEffectClass,
    /// Filesystem scopes requested.
    pub filesystem: Vec<FilesystemScope>,
    /// Network profiles requested.
    pub network_profiles: BTreeSet<NetworkProfileRef>,
    /// Network destinations requested.
    pub network_destinations: BTreeSet<String>,
    /// Secret references requested.
    pub secrets: BTreeSet<SecretRef>,
    /// Exact artifact identity, when artifact metadata or bytes are requested.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub artifact: Option<ArtifactId>,
    /// Exact artifact sensitivity obtained from immutable metadata before release.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub artifact_sensitivity: Option<ArtifactSensitivity>,
    /// Exact revision governing a revision/layout request.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub revision: Option<RevisionId>,
    /// Shared layout scope, or a reserved actor owner that production denies.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub layout_owner: Option<LayoutOwner>,
    /// Exact workspace scope identity.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_scope: Option<ScopeId>,
    /// Whether the request asks for coarse readiness data.
    #[serde(default, skip_serializing_if = "is_false")]
    pub daemon_readiness: bool,
    /// Whether the request asks for detailed daemon health/load data.
    #[serde(default, skip_serializing_if = "is_false")]
    pub daemon_detailed_health: bool,
    /// Whether the request asks for the caller's own grant details.
    #[serde(default, skip_serializing_if = "is_false")]
    pub daemon_own_authority: bool,
    /// Whether the request asks for redacted daemon configuration.
    #[serde(default, skip_serializing_if = "is_false")]
    pub daemon_configuration: bool,
    /// Whether the request asks for security audit/diagnostic data.
    #[serde(default, skip_serializing_if = "is_false")]
    pub daemon_audit: bool,
}

const fn is_false(value: &bool) -> bool {
    !*value
}

impl RequestedResourceFacts {
    /// Empty non-side-effecting resource request.
    #[must_use]
    pub fn empty() -> Self {
        Self {
            workflow: None,
            run: None,
            capability: None,
            category: None,
            capability_operation: None,
            provider_profile: None,
            trust_zone: None,
            trust_zones: BTreeSet::new(),
            execution_trust_class: None,
            locality: None,
            peer: None,
            capability_envelope: None,
            side_effect: SideEffectClass::None,
            filesystem: Vec::new(),
            network_profiles: BTreeSet::new(),
            network_destinations: BTreeSet::new(),
            secrets: BTreeSet::new(),
            artifact: None,
            artifact_sensitivity: None,
            revision: None,
            layout_owner: None,
            workspace_scope: None,
            daemon_readiness: false,
            daemon_detailed_health: false,
            daemon_own_authority: false,
            daemon_configuration: false,
            daemon_audit: false,
        }
    }

    fn validate(&self) -> Result<(), AuthorityError> {
        if self.filesystem.len() > MAX_SCOPE_ITEMS || self.secrets.len() > MAX_SCOPE_ITEMS {
            return Err(AuthorityError::Bounds {
                location: "authority_request.resources",
                reason: "filesystem or secret request count exceeded".to_owned(),
            });
        }
        for scope in &self.filesystem {
            FilesystemScope::new(scope.root().to_owned(), scope.access().clone())?;
        }
        NetworkScope::new(
            self.network_profiles.clone(),
            self.network_destinations.clone(),
        )?;
        if self.artifact.is_some() != self.artifact_sensitivity.is_some()
            || (self.layout_owner.is_some() && self.revision.is_none())
        {
            return Err(AuthorityError::InvalidContract(
                "artifact sensitivity and layout revision facts must be complete".to_owned(),
            ));
        }
        Ok(())
    }
}

/// Complete pure input to one authority evaluation.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AuthorityRequest {
    /// Stable decision identity supplied by the caller.
    pub decision: DecisionId,
    /// Claimed actor; authentication occurs elsewhere.
    pub actor: ActorRef,
    /// Exact grant lineage requested.
    pub grant: GrantId,
    /// Exact grant revision requested.
    pub grant_revision: u64,
    /// Exact immutable grant digest observed by the trusted authentication boundary.
    pub grant_digest: GrantDigest,
    /// Revocation generation observed by the trusted caller boundary.
    pub revocation_generation: u64,
    /// Closed requested operation.
    pub operation: AuthorityOperation,
    /// Exact requested resource facts.
    pub resources: RequestedResourceFacts,
    /// Exact requested ceilings/usage.
    pub budget: AuthorityBudget,
    /// Caller-supplied validity boundary.
    pub evaluated_at: BoundaryTimeMillis,
    /// Execution coordinates and exact candidate generation bound into this decision.
    pub provenance: AuthorityExecutionProvenance,
}

impl AuthorityRequest {
    pub(crate) fn validate(&self) -> Result<(), AuthorityError> {
        if self.grant_revision == 0 {
            return Err(AuthorityError::InvalidContract(
                "authority request grant revision must be nonzero".to_owned(),
            ));
        }
        self.provenance.validate()?;
        self.resources.validate()
    }
}

/// Stable authorization outcome.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DecisionOutcome {
    /// The exact grant permits the request.
    Allow,
    /// The exact grant does not permit the request.
    Deny,
}

/// Stable bounded diagnostic categories from the evaluator.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DecisionReasonCode {
    /// Every check passed.
    Allowed,
    /// Grant identity was not found.
    GrantNotFound,
    /// Requested revision is stale or absent.
    GrantRevisionMismatch,
    /// Configured content for the identity/revision differs from the frozen digest.
    GrantDigestMismatch,
    /// Actor differs from the grant subject.
    WrongActor,
    /// Revocation generation differs.
    Revoked,
    /// Boundary precedes grant validity.
    NotYetValid,
    /// Boundary follows grant validity.
    Expired,
    /// Operation is not granted.
    OperationMismatch,
    /// Workflow or run scope does not match.
    WorkflowRunMismatch,
    /// Capability identity/category/operation/profile does not match.
    CapabilityMismatch,
    /// Trust-zone or locality constraint does not match.
    PlacementMismatch,
    /// Side-effect class exceeds the grant.
    SideEffectExcess,
    /// Filesystem request exceeds normalized roots or modes.
    FilesystemMismatch,
    /// Network request exceeds destination/profile scope.
    NetworkMismatch,
    /// Secret reference is outside the allowlist.
    SecretScopeMismatch,
    /// Artifact identity or sensitivity is outside the grant.
    ArtifactScopeMismatch,
    /// Layout revision or owner is outside the grant.
    LayoutScopeMismatch,
    /// Configured peer identity is outside the grant.
    PeerScopeMismatch,
    /// Daemon health, authority, configuration, or audit detail is outside the grant.
    DaemonScopeMismatch,
    /// Workspace scope identity is outside the grant.
    WorkspaceScopeMismatch,
    /// Numeric ceiling is exceeded or ungranted.
    BudgetExcess,
}

/// Immutable exact authorization result durably attached to command acceptance/rejection.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AuthorityDecisionSnapshot {
    schema_version: u32,
    policy: PolicyId,
    policy_version: u32,
    request: AuthorityRequest,
    outcome: DecisionOutcome,
    reason_codes: Vec<DecisionReasonCode>,
    evaluated_budget: AuthorityBudget,
    evaluated_side_effect: SideEffectClass,
    digest: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DecisionWire {
    schema_version: u32,
    policy: PolicyId,
    policy_version: u32,
    request: AuthorityRequest,
    outcome: DecisionOutcome,
    reason_codes: Vec<DecisionReasonCode>,
    evaluated_budget: AuthorityBudget,
    evaluated_side_effect: SideEffectClass,
    digest: String,
}

#[derive(Serialize)]
struct DecisionDigest<'a> {
    domain: &'static str,
    schema_version: u32,
    policy: &'a PolicyId,
    policy_version: u32,
    request: &'a AuthorityRequest,
    outcome: DecisionOutcome,
    reason_codes: &'a [DecisionReasonCode],
    evaluated_budget: AuthorityBudget,
    evaluated_side_effect: SideEffectClass,
}

impl<'de> Deserialize<'de> for AuthorityDecisionSnapshot {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let wire = DecisionWire::deserialize(deserializer)?;
        let value = Self {
            schema_version: wire.schema_version,
            policy: wire.policy,
            policy_version: wire.policy_version,
            request: wire.request,
            outcome: wire.outcome,
            reason_codes: wire.reason_codes,
            evaluated_budget: wire.evaluated_budget,
            evaluated_side_effect: wire.evaluated_side_effect,
            digest: wire.digest,
        };
        value.validate().map_err(serde::de::Error::custom)?;
        Ok(value)
    }
}

impl AuthorityDecisionSnapshot {
    /// Constructs and digests the output of an authority evaluator implementation.
    pub fn from_evaluation(
        policy: PolicyId,
        policy_version: u32,
        request: AuthorityRequest,
        mut reason_codes: Vec<DecisionReasonCode>,
        evaluated_budget: AuthorityBudget,
        evaluated_side_effect: SideEffectClass,
    ) -> Result<Self, AuthorityError> {
        request.validate()?;
        if policy_version == 0 {
            return Err(AuthorityError::InvalidContract(
                "policy version must be nonzero".to_owned(),
            ));
        }
        reason_codes.sort_unstable();
        reason_codes.dedup();
        if reason_codes.is_empty() || reason_codes.len() > MAX_DIAGNOSTIC_CODES {
            return Err(AuthorityError::InvalidContract(
                "decision requires 1..=16 stable reason codes".to_owned(),
            ));
        }
        let outcome = if reason_codes == [DecisionReasonCode::Allowed] {
            DecisionOutcome::Allow
        } else {
            DecisionOutcome::Deny
        };
        let mut value = Self {
            schema_version: 2,
            policy,
            policy_version,
            request,
            outcome,
            reason_codes,
            evaluated_budget,
            evaluated_side_effect,
            digest: String::new(),
        };
        value.digest = value.compute_digest()?;
        Ok(value)
    }
    /// Evaluator policy lineage.
    #[must_use]
    pub const fn policy(&self) -> &PolicyId {
        &self.policy
    }
    /// Exact evaluator policy version.
    #[must_use]
    pub const fn policy_version(&self) -> u32 {
        self.policy_version
    }
    /// Exact evaluated request.
    #[must_use]
    pub const fn request(&self) -> &AuthorityRequest {
        &self.request
    }
    /// Allow or deny result.
    #[must_use]
    pub const fn outcome(&self) -> DecisionOutcome {
        self.outcome
    }
    /// Stable diagnostic codes.
    #[must_use]
    pub fn reason_codes(&self) -> &[DecisionReasonCode] {
        &self.reason_codes
    }
    /// Evaluated budget ceiling.
    #[must_use]
    pub const fn evaluated_budget(&self) -> AuthorityBudget {
        self.evaluated_budget
    }
    /// Evaluated maximum side effect.
    #[must_use]
    pub const fn evaluated_side_effect(&self) -> SideEffectClass {
        self.evaluated_side_effect
    }
    /// Domain-separated deterministic digest.
    #[must_use]
    pub fn digest(&self) -> &str {
        &self.digest
    }
    /// Whether the request was allowed.
    #[must_use]
    pub const fn is_allowed(&self) -> bool {
        matches!(self.outcome, DecisionOutcome::Allow)
    }
    /// Canonical bounded JSON encoding.
    pub fn to_canonical_json(&self) -> Result<Vec<u8>, AuthorityError> {
        canonical_json(self)
    }
    fn validate(&self) -> Result<(), AuthorityError> {
        self.request.validate()?;
        if self.schema_version != 2
            || self.policy_version == 0
            || self.reason_codes.is_empty()
            || self.reason_codes.len() > MAX_DIAGNOSTIC_CODES
            || self.reason_codes.windows(2).any(|pair| pair[0] >= pair[1])
            || (self.reason_codes.contains(&DecisionReasonCode::Allowed)
                && self.reason_codes.len() != 1)
            || self.digest != self.compute_digest()?
        {
            return Err(AuthorityError::InvalidContract(
                "authorization decision invariant or digest mismatch".to_owned(),
            ));
        }
        let derived = if self.reason_codes == [DecisionReasonCode::Allowed] {
            DecisionOutcome::Allow
        } else {
            DecisionOutcome::Deny
        };
        if self.outcome != derived {
            return Err(AuthorityError::InvalidContract(
                "decision outcome disagrees with reason codes".to_owned(),
            ));
        }
        Ok(())
    }
    fn compute_digest(&self) -> Result<String, AuthorityError> {
        let payload = DecisionDigest {
            domain: "milkdrift.authority-decision.v2",
            schema_version: self.schema_version,
            policy: &self.policy,
            policy_version: self.policy_version,
            request: &self.request,
            outcome: self.outcome,
            reason_codes: &self.reason_codes,
            evaluated_budget: self.evaluated_budget,
            evaluated_side_effect: self.evaluated_side_effect,
        };
        let bytes = canonical_json(&payload)?;
        let mut hasher = blake3::Hasher::new();
        hasher.update(DECISION_DIGEST_DOMAIN);
        hasher.update(&bytes);
        Ok(format!("b3_{}", hasher.finalize()))
    }
}
