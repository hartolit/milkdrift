use std::collections::{BTreeMap, BTreeSet};

use milkdrift_blueprint::{NodeId, RevisionId, WorkflowId};
use milkdrift_capability::{
    BoundedJson, CapabilityCategory, CapabilityId, IdempotencyBehavior, Locality, OperationId,
    ProviderProfileRef, SideEffectClass, TrustZone,
};
use milkdrift_workspace::RunId;
use serde::{Deserialize, Serialize};

use crate::{
    ActorRef, AuthorityError, DecisionId, GrantDigest, GrantId, NetworkProfileRef, PeerId,
    PolicyId, SecretRef,
    document::{AUTHORITY_GRANT_SCHEMA_VERSION_V1, canonical_json},
};

const MAX_SCOPE_ITEMS: usize = 128;
const MAX_DIAGNOSTIC_CODES: usize = 16;
const DECISION_DIGEST_DOMAIN: &[u8] = b"milkdrift.authority-decision.v1\0";

/// Caller-supplied epoch-millisecond fact used at a deterministic policy boundary.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Deserialize, Serialize)]
#[serde(transparent)]
pub struct BoundaryTimeMillis(u64);

impl BoundaryTimeMillis {
    /// Constructs a boundary timestamp fact.
    #[must_use]
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    /// Returns epoch milliseconds.
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }
}

/// Closed command, control, and capability action understood by the core evaluator.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthorityOperation {
    /// Inspect durable run or registry facts.
    Inspect,
    /// Propose a prospective revision or action.
    Propose,
    /// Approve an awaiting decision.
    Approve,
    /// Apply an approved prospective change.
    Apply,
    /// Create a new run aggregate.
    CreateRun,
    /// Start a created run.
    StartRun,
    /// Pause new run work.
    Pause,
    /// Resume paused work.
    Resume,
    /// Request cancellation.
    Cancel,
    /// Authorize a new execution attempt.
    Retry,
    /// Force terminal resolution from evidence.
    Terminate,
    /// Deliver an external signal.
    DeliverSignal,
    /// Submit a caller-clock timer observation.
    FireTimer,
    /// Resolve and invoke a capability.
    InvokeCapability,
    /// Cancel an exact capability invocation.
    CancelCapability,
}

/// Workflow/run portion of an immutable grant.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case", tag = "type", deny_unknown_fields)]
pub enum WorkflowRunScope {
    /// Explicit wildcard across workflows and runs.
    Any,
    /// Every run in one workflow lineage.
    Workflow {
        /// Exact workflow lineage.
        workflow: WorkflowId,
    },
    /// One exact run, optionally bound to its workflow lineage.
    Run {
        /// Exact run aggregate.
        run: RunId,
        /// Optional workflow cross-check.
        workflow: Option<WorkflowId>,
    },
}

impl WorkflowRunScope {
    pub(crate) fn matches(&self, facts: &RequestedResourceFacts) -> bool {
        match self {
            Self::Any => true,
            Self::Workflow { workflow } => facts.workflow.as_ref() == Some(workflow),
            Self::Run { run, workflow } => {
                facts.run.as_ref() == Some(run)
                    && workflow
                        .as_ref()
                        .is_none_or(|expected| facts.workflow.as_ref() == Some(expected))
            }
        }
    }
}

/// Normalized future-facing filesystem access mode.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AccessMode {
    /// Read metadata or content.
    Read,
    /// Create or modify content.
    Write,
    /// Execute a file or traverse an execution boundary.
    Execute,
}

/// One normalized absolute filesystem root and allowed access modes.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct FilesystemScope {
    root: String,
    access: BTreeSet<AccessMode>,
}

impl FilesystemScope {
    /// Deliberately broad root scope for explicitly acknowledged administration.
    #[must_use]
    pub fn dangerous_all_access_root() -> Self {
        Self {
            root: "/".to_owned(),
            access: BTreeSet::from([AccessMode::Read, AccessMode::Write, AccessMode::Execute]),
        }
    }

    /// Constructs a lexical absolute root without `.` or `..` segments.
    pub fn new(
        root: impl Into<String>,
        access: BTreeSet<AccessMode>,
    ) -> Result<Self, AuthorityError> {
        let root = root.into();
        if root.len() > 4_096
            || !root.starts_with('/')
            || root.contains('\0')
            || root.split('/').any(|part| matches!(part, "." | ".."))
            || (root.len() > 1 && root.ends_with('/'))
            || root.contains("//")
            || access.is_empty()
        {
            return Err(AuthorityError::InvalidContract(
                "filesystem scope requires a normalized absolute root and nonempty access set"
                    .to_owned(),
            ));
        }
        Ok(Self { root, access })
    }

    /// Normalized root text.
    #[must_use]
    pub fn root(&self) -> &str {
        &self.root
    }

    /// Allowed access modes.
    #[must_use]
    pub const fn access(&self) -> &BTreeSet<AccessMode> {
        &self.access
    }
}

/// Credential-free network profile and destination allowlist.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NetworkScope {
    profiles: BTreeSet<NetworkProfileRef>,
    destinations: BTreeSet<String>,
}

impl NetworkScope {
    /// Empty network scope granting no profiles or destinations.
    #[must_use]
    pub fn empty() -> Self {
        Self {
            profiles: BTreeSet::new(),
            destinations: BTreeSet::new(),
        }
    }

    /// Constructs a bounded non-secret network scope.
    pub fn new(
        profiles: BTreeSet<NetworkProfileRef>,
        destinations: BTreeSet<String>,
    ) -> Result<Self, AuthorityError> {
        if profiles.len() > MAX_SCOPE_ITEMS
            || destinations.len() > MAX_SCOPE_ITEMS
            || destinations.iter().any(|destination| {
                destination.is_empty()
                    || destination.len() > 255
                    || !destination.is_ascii()
                    || destination.contains(['/', '@', ' ', '\t', '\n'])
            })
        {
            return Err(AuthorityError::Bounds {
                location: "grant.network",
                reason: "network scope exceeds bounds or contains a credential-like destination"
                    .to_owned(),
            });
        }
        Ok(Self {
            profiles,
            destinations,
        })
    }

    /// Allowed network profiles.
    #[must_use]
    pub const fn profiles(&self) -> &BTreeSet<NetworkProfileRef> {
        &self.profiles
    }

    /// Allowed host or host-and-port destination facts.
    #[must_use]
    pub const fn destinations(&self) -> &BTreeSet<String> {
        &self.destinations
    }
}

/// Capability-selection constraints in a grant.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CapabilityAuthorityScope {
    identities: BTreeSet<CapabilityId>,
    categories: BTreeSet<CapabilityCategory>,
    operations: BTreeSet<OperationId>,
    provider_profiles: BTreeSet<ProviderProfileRef>,
    trust_zones: BTreeSet<TrustZone>,
    localities: BTreeSet<Locality>,
    #[serde(default)]
    peers: BTreeSet<PeerId>,
    maximum_side_effect: SideEffectClass,
}

impl CapabilityAuthorityScope {
    /// Constructs a capability scope; empty sets are explicit wildcards.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        identities: BTreeSet<CapabilityId>,
        categories: BTreeSet<CapabilityCategory>,
        operations: BTreeSet<OperationId>,
        provider_profiles: BTreeSet<ProviderProfileRef>,
        trust_zones: BTreeSet<TrustZone>,
        localities: BTreeSet<Locality>,
        maximum_side_effect: SideEffectClass,
    ) -> Result<Self, AuthorityError> {
        if [
            identities.len(),
            categories.len(),
            operations.len(),
            provider_profiles.len(),
            trust_zones.len(),
            localities.len(),
        ]
        .into_iter()
        .any(|count| count > MAX_SCOPE_ITEMS)
        {
            return Err(AuthorityError::Bounds {
                location: "grant.capability_scope",
                reason: format!("each scope set is limited to {MAX_SCOPE_ITEMS} entries"),
            });
        }
        Ok(Self {
            identities,
            categories,
            operations,
            provider_profiles,
            trust_zones,
            localities,
            peers: BTreeSet::new(),
            maximum_side_effect,
        })
    }

    /// Unconstrained identity/category/profile scope capped at the supplied side effect.
    #[must_use]
    pub fn any(maximum_side_effect: SideEffectClass) -> Self {
        Self {
            identities: BTreeSet::new(),
            categories: BTreeSet::new(),
            operations: BTreeSet::new(),
            provider_profiles: BTreeSet::new(),
            trust_zones: BTreeSet::new(),
            localities: BTreeSet::new(),
            peers: BTreeSet::new(),
            maximum_side_effect,
        }
    }

    /// Narrows remote selection to exact authenticated peer identities.
    pub fn with_peers(mut self, peers: BTreeSet<PeerId>) -> Result<Self, AuthorityError> {
        if peers.len() > MAX_SCOPE_ITEMS {
            return Err(AuthorityError::Bounds {
                location: "grant.capability_scope.peers",
                reason: format!("at most {MAX_SCOPE_ITEMS} peers are allowed"),
            });
        }
        self.peers = peers;
        Ok(self)
    }

    /// Allowed capability identities; empty means any.
    #[must_use]
    pub const fn identities(&self) -> &BTreeSet<CapabilityId> {
        &self.identities
    }
    /// Allowed categories; empty means any.
    #[must_use]
    pub const fn categories(&self) -> &BTreeSet<CapabilityCategory> {
        &self.categories
    }
    /// Allowed operations; empty means any.
    #[must_use]
    pub const fn operations(&self) -> &BTreeSet<OperationId> {
        &self.operations
    }
    /// Allowed provider profiles; empty means any.
    #[must_use]
    pub const fn provider_profiles(&self) -> &BTreeSet<ProviderProfileRef> {
        &self.provider_profiles
    }
    /// Required/allowed trust-zone labels; empty means any.
    #[must_use]
    pub const fn trust_zones(&self) -> &BTreeSet<TrustZone> {
        &self.trust_zones
    }
    /// Allowed localities; empty means any.
    #[must_use]
    pub const fn localities(&self) -> &BTreeSet<Locality> {
        &self.localities
    }
    /// Allowed authenticated remote peers; empty means any peer within other constraints.
    #[must_use]
    pub const fn peers(&self) -> &BTreeSet<PeerId> {
        &self.peers
    }
    /// Maximum permitted side-effect class.
    #[must_use]
    pub const fn maximum_side_effect(&self) -> SideEffectClass {
        self.maximum_side_effect
    }
}

/// Numeric ceilings evaluated for a command or capability request.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AuthorityBudget {
    /// Maximum minor currency units.
    pub cost_minor: Option<u64>,
    /// Maximum boundary duration.
    pub duration_ms: Option<u64>,
    /// Maximum invocations.
    pub invocations: Option<u64>,
    /// Maximum artifact bytes.
    pub artifact_bytes: Option<u64>,
    /// Maximum provider-defined input/output units.
    pub units: Option<u64>,
    /// Maximum concurrent work.
    pub concurrency: Option<u32>,
}

/// Immutable adapter-declared resource facts added to every exact candidate request.
///
/// These values contain references and ceilings only. Secret values and live health do not
/// cross the authority boundary.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CapabilityExecutionRequirements {
    /// Normalized host filesystem roots and access modes used by this generation.
    pub filesystem: Vec<FilesystemScope>,
    /// Credential-free network profiles used by this generation.
    pub network_profiles: BTreeSet<NetworkProfileRef>,
    /// Credential-free host or host-and-port destinations used by this generation.
    pub network_destinations: BTreeSet<String>,
    /// Opaque secret references resolved only after authorization.
    pub secrets: BTreeSet<SecretRef>,
    /// Per-invocation resource ceilings declared by the adapter profile.
    pub budget: AuthorityBudget,
}

impl AuthorityBudget {
    pub(crate) fn fits_within(self, ceiling: Self) -> bool {
        within(self.cost_minor, ceiling.cost_minor)
            && within(self.duration_ms, ceiling.duration_ms)
            && within(self.invocations, ceiling.invocations)
            && within(self.artifact_bytes, ceiling.artifact_bytes)
            && within(self.units, ceiling.units)
            && within(
                self.concurrency.map(u64::from),
                ceiling.concurrency.map(u64::from),
            )
    }
}

fn within(requested: Option<u64>, ceiling: Option<u64>) -> bool {
    match (requested, ceiling) {
        (Some(requested), Some(ceiling)) => requested <= ceiling,
        (Some(_), None) => false,
        (None, _) => true,
    }
}

/// All typed resources to which a grant applies.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ResourceScope {
    /// Workflow and run scope.
    pub workflow_run: WorkflowRunScope,
    /// Capability-selection scope.
    pub capability: CapabilityAuthorityScope,
    /// Normalized filesystem roots.
    pub filesystem: Vec<FilesystemScope>,
    /// Credential-free network scope.
    pub network: NetworkScope,
    /// Opaque secret references that may be resolved.
    pub secrets: BTreeSet<SecretRef>,
}

/// Immutable, exact revision of one authority grant.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AuthorityGrant {
    schema_version: u32,
    identity: GrantId,
    revision: u64,
    actor: ActorRef,
    operations: BTreeSet<AuthorityOperation>,
    resources: ResourceScope,
    budget: AuthorityBudget,
    valid_from: BoundaryTimeMillis,
    valid_until: BoundaryTimeMillis,
    revocation_generation: u64,
    extensions: BTreeMap<String, BoundedJson>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AuthorityGrantWire {
    schema_version: u32,
    identity: GrantId,
    revision: u64,
    actor: ActorRef,
    operations: BTreeSet<AuthorityOperation>,
    resources: ResourceScope,
    budget: AuthorityBudget,
    valid_from: BoundaryTimeMillis,
    valid_until: BoundaryTimeMillis,
    revocation_generation: u64,
    extensions: BTreeMap<String, BoundedJson>,
}

impl<'de> Deserialize<'de> for AuthorityGrant {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let wire = AuthorityGrantWire::deserialize(deserializer)?;
        AuthorityGrantBuilder::new(wire.identity, wire.revision, wire.actor)
            .operations(wire.operations)
            .resources(wire.resources)
            .budget(wire.budget)
            .validity(wire.valid_from, wire.valid_until)
            .revocation_generation(wire.revocation_generation)
            .extensions(wire.extensions)
            .schema_version(wire.schema_version)
            .build()
            .map_err(serde::de::Error::custom)
    }
}

impl AuthorityGrant {
    /// Explicit contract schema.
    #[must_use]
    pub const fn schema_version(&self) -> u32 {
        self.schema_version
    }
    /// Grant lineage identity.
    #[must_use]
    pub const fn identity(&self) -> &GrantId {
        &self.identity
    }
    /// Exact nonzero revision.
    #[must_use]
    pub const fn revision(&self) -> u64 {
        self.revision
    }
    /// Actor receiving authority.
    #[must_use]
    pub const fn actor(&self) -> &ActorRef {
        &self.actor
    }
    /// Closed allowed operations.
    #[must_use]
    pub const fn operations(&self) -> &BTreeSet<AuthorityOperation> {
        &self.operations
    }
    /// Typed resource scope.
    #[must_use]
    pub const fn resources(&self) -> &ResourceScope {
        &self.resources
    }
    /// Numeric ceilings.
    #[must_use]
    pub const fn budget(&self) -> AuthorityBudget {
        self.budget
    }
    /// Inclusive validity start.
    #[must_use]
    pub const fn valid_from(&self) -> BoundaryTimeMillis {
        self.valid_from
    }
    /// Inclusive validity end.
    #[must_use]
    pub const fn valid_until(&self) -> BoundaryTimeMillis {
        self.valid_until
    }
    /// Exact revocation generation expected by this revision.
    #[must_use]
    pub const fn revocation_generation(&self) -> u64 {
        self.revocation_generation
    }
    /// Canonical bounded JSON encoding.
    pub fn to_canonical_json(&self) -> Result<Vec<u8>, AuthorityError> {
        canonical_json(self)
    }
    /// Domain-separated digest of this exact immutable grant revision.
    pub fn digest(&self) -> Result<GrantDigest, AuthorityError> {
        Ok(GrantDigest::for_bytes(&self.to_canonical_json()?))
    }
    /// Strictly decodes and validates one schema-v1 grant.
    pub fn from_json(bytes: &[u8]) -> Result<Self, AuthorityError> {
        if bytes.len() > crate::MAX_AUTHORITY_DOCUMENT_BYTES {
            return Err(AuthorityError::Bounds {
                location: "grant.document",
                reason: "document too large".to_owned(),
            });
        }
        let value = milkdrift_contracts::parse_json_without_duplicates(bytes)
            .map_err(|error| AuthorityError::Json(error.to_string()))?;
        let version = value
            .get("schema_version")
            .and_then(serde_json::Value::as_u64)
            .and_then(|value| u32::try_from(value).ok())
            .ok_or_else(|| {
                AuthorityError::InvalidContract("grant requires numeric schema_version".to_owned())
            })?;
        if version != AUTHORITY_GRANT_SCHEMA_VERSION_V1 {
            return Err(AuthorityError::UnsupportedVersion {
                document: "authority_grant",
                found: version,
                supported: AUTHORITY_GRANT_SCHEMA_VERSION_V1,
            });
        }
        serde_json::from_value(value).map_err(|error| AuthorityError::Json(error.to_string()))
    }
}

/// Builder that publishes a grant only after complete invariant validation.
pub struct AuthorityGrantBuilder {
    grant: AuthorityGrant,
}

impl AuthorityGrantBuilder {
    /// Starts an exact grant revision with deliberately empty permissions.
    #[must_use]
    pub fn new(identity: GrantId, revision: u64, actor: ActorRef) -> Self {
        Self {
            grant: AuthorityGrant {
                schema_version: AUTHORITY_GRANT_SCHEMA_VERSION_V1,
                identity,
                revision,
                actor,
                operations: BTreeSet::new(),
                resources: ResourceScope {
                    workflow_run: WorkflowRunScope::Any,
                    capability: CapabilityAuthorityScope::any(SideEffectClass::None),
                    filesystem: Vec::new(),
                    network: NetworkScope {
                        profiles: BTreeSet::new(),
                        destinations: BTreeSet::new(),
                    },
                    secrets: BTreeSet::new(),
                },
                budget: AuthorityBudget::default(),
                valid_from: BoundaryTimeMillis::new(0),
                valid_until: BoundaryTimeMillis::new(u64::MAX),
                revocation_generation: 0,
                extensions: BTreeMap::new(),
            },
        }
    }
    /// Replaces allowed operations.
    #[must_use]
    pub fn operations(mut self, value: BTreeSet<AuthorityOperation>) -> Self {
        self.grant.operations = value;
        self
    }
    /// Replaces typed resources.
    #[must_use]
    pub fn resources(mut self, value: ResourceScope) -> Self {
        self.grant.resources = value;
        self
    }
    /// Replaces numeric ceilings.
    #[must_use]
    pub const fn budget(mut self, value: AuthorityBudget) -> Self {
        self.grant.budget = value;
        self
    }
    /// Replaces the inclusive validity interval.
    #[must_use]
    pub const fn validity(mut self, from: BoundaryTimeMillis, until: BoundaryTimeMillis) -> Self {
        self.grant.valid_from = from;
        self.grant.valid_until = until;
        self
    }
    /// Sets the exact revocation generation.
    #[must_use]
    pub const fn revocation_generation(mut self, value: u64) -> Self {
        self.grant.revocation_generation = value;
        self
    }
    /// Replaces bounded namespaced extensions.
    #[must_use]
    pub fn extensions(mut self, value: BTreeMap<String, BoundedJson>) -> Self {
        self.grant.extensions = value;
        self
    }
    pub(crate) const fn schema_version(mut self, value: u32) -> Self {
        self.grant.schema_version = value;
        self
    }
    /// Validates and publishes the immutable grant revision.
    pub fn build(self) -> Result<AuthorityGrant, AuthorityError> {
        let grant = self.grant;
        if grant.schema_version != AUTHORITY_GRANT_SCHEMA_VERSION_V1 {
            return Err(AuthorityError::UnsupportedVersion {
                document: "authority_grant",
                found: grant.schema_version,
                supported: AUTHORITY_GRANT_SCHEMA_VERSION_V1,
            });
        }
        if grant.revision == 0
            || grant.operations.is_empty()
            || grant.operations.len() > MAX_SCOPE_ITEMS
        {
            return Err(AuthorityError::InvalidContract(
                "grant revision must be nonzero and operations must contain 1..=128 entries"
                    .to_owned(),
            ));
        }
        if grant.valid_from > grant.valid_until {
            return Err(AuthorityError::InvalidContract(
                "grant validity interval is inverted".to_owned(),
            ));
        }
        if grant.resources.filesystem.len() > MAX_SCOPE_ITEMS
            || grant.resources.secrets.len() > MAX_SCOPE_ITEMS
            || grant.extensions.len() > 64
        {
            return Err(AuthorityError::Bounds {
                location: "grant.resources",
                reason: "scope or extension count exceeded".to_owned(),
            });
        }
        CapabilityAuthorityScope::new(
            grant.resources.capability.identities.clone(),
            grant.resources.capability.categories.clone(),
            grant.resources.capability.operations.clone(),
            grant.resources.capability.provider_profiles.clone(),
            grant.resources.capability.trust_zones.clone(),
            grant.resources.capability.localities.clone(),
            grant.resources.capability.maximum_side_effect,
        )?
        .with_peers(grant.resources.capability.peers.clone())?;
        NetworkScope::new(
            grant.resources.network.profiles.clone(),
            grant.resources.network.destinations.clone(),
        )?;
        for scope in &grant.resources.filesystem {
            FilesystemScope::new(scope.root.clone(), scope.access.clone())?;
        }
        if grant.extensions.keys().any(|key| {
            key.len() > 192
                || !key
                    .split_once('/')
                    .is_some_and(|(namespace, name)| namespace.contains('.') && !name.is_empty())
        }) {
            return Err(AuthorityError::InvalidContract(
                "authority extension keys must be DNS-namespaced".to_owned(),
            ));
        }
        let _ = canonical_json(&grant)?;
        Ok(grant)
    }
}

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
    fn validate(&self) -> Result<(), AuthorityError> {
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
            || !valid_digest(&self.accepted_decision_digest)
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

fn valid_digest(value: &str) -> bool {
    value.strip_prefix("b3_").is_some_and(|hex| {
        hex.len() == 64
            && hex
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    })
}

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
            locality: None,
            peer: None,
            capability_envelope: None,
            side_effect: SideEffectClass::None,
            filesystem: Vec::new(),
            network_profiles: BTreeSet::new(),
            network_destinations: BTreeSet::new(),
            secrets: BTreeSet::new(),
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
            FilesystemScope::new(scope.root.clone(), scope.access.clone())?;
        }
        NetworkScope::new(
            self.network_profiles.clone(),
            self.network_destinations.clone(),
        )?;
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
            schema_version: 1,
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
        if self.schema_version != 1
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
            domain: "milkdrift.authority-decision.v1",
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
