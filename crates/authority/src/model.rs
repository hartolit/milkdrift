use std::collections::{BTreeMap, BTreeSet};

use milkdrift_blueprint::{NodeId, RevisionId, WorkflowId};
use milkdrift_capability::{
    BoundedJson, CapabilityCategory, CapabilityId, CapabilityRequirement, ExecutionTrustClass,
    IdempotencyBehavior, Locality, OperationId, ProviderProfileRef, SideEffectClass, TrustZone,
};
use milkdrift_workspace::{ArtifactId, ArtifactSensitivity, RunId, ScopeId};
use serde::{Deserialize, Serialize};

use crate::{
    ActorRef, AuthorityError, DecisionId, GrantDigest, GrantId, NetworkProfileRef, PeerId,
    PolicyId, SecretRef, Selection,
    document::{AUTHORITY_GRANT_SCHEMA_VERSION_V4, canonical_json},
};

const MAX_SCOPE_ITEMS: usize = 128;
const MAX_DIAGNOSTIC_CODES: usize = 16;
const DECISION_DIGEST_DOMAIN: &[u8] = b"milkdrift.authority-decision.v2\0";

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
    /// Propose a revision without a live run; controller policy changes remain excluded.
    ProposeOffline,
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
    /// Validate an immutable blueprint document without storing it.
    ValidateBlueprint,
    /// Import an immutable blueprint/workflow revision.
    ImportBlueprint,
    /// Inspect one revision, its lineage, or a semantic diff.
    InspectRevision,
    /// Inspect one run summary.
    InspectRun,
    /// Inspect one run's append-only external timeline.
    InspectTimeline,
    /// Inspect one node execution in a run.
    InspectNodeExecution,
    /// Inspect one exact execution attempt and its provenance.
    InspectAttempt,
    /// Inspect reconciliation, proposal, or approval state.
    InspectProposal,
    /// List authority-filtered capability descriptor generations.
    ListCapabilities,
    /// Inspect mutable capability health, load, and admission limits.
    InspectCapabilityHealth,
    /// Inspect provider/model profile identity.
    InspectProviderProfile,
    /// Administer capability or provider registrations and configuration.
    AdministerCapabilities,
    /// Read protected artifact metadata, including sensitivity and provenance.
    ReadArtifactMetadata,
    /// Read an explicit verified artifact byte range.
    ReadArtifactContent,
    /// Publish or import artifact content.
    PublishArtifact,
    /// Export artifact content across an external boundary.
    ExportArtifact,
    /// Delete artifact content or metadata.
    DeleteArtifact,
    /// Change artifact retention policy.
    AdministerArtifactRetention,
    /// Read a scoped workspace value.
    ReadWorkspaceValue,
    /// Read presentation-only layout state.
    ReadLayout,
    /// Create or replace presentation-only layout state.
    WriteLayout,
    /// Delete presentation-only layout state.
    DeleteLayout,
    /// Inspect an exact configured peer relationship or filtered catalog.
    InspectPeer,
    /// Negotiate an authenticated peer session.
    NegotiatePeerSession,
    /// Inspect one peer-owned execution and its observations.
    InspectPeerExecution,
    /// Invoke one exact capability through an authenticated peer relationship.
    InvokePeerCapability,
    /// Cancel one exact peer-owned capability execution.
    CancelPeerCapability,
    /// Upload verified artifact content across a peer relationship.
    PeerArtifactUpload,
    /// Download verified artifact content across a peer relationship.
    PeerArtifactDownload,
    /// Administer peer trust, configuration, connection, or grants.
    AdministerPeer,
    /// Negotiate the local control protocol version.
    NegotiateControlProtocol,
    /// Read coarse liveness/readiness without detailed diagnostics.
    ReadReadiness,
    /// Read detailed daemon health and load diagnostics.
    InspectDaemonHealth,
    /// Inspect the authenticated actor's exact immutable grant identity.
    InspectOwnAuthority,
    /// Inspect bounded security audit/diagnostic views.
    InspectAudit,
    /// Inspect daemon configuration without credential values.
    InspectConfiguration,
    /// Reload validated daemon configuration.
    ReloadConfiguration,
    /// Begin an orderly daemon drain.
    DrainDaemon,
    /// Shut down the daemon owner.
    ShutdownDaemon,
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
#[serde(transparent)]
pub struct CapabilityAuthorityScope(CapabilityAuthorityScopeKind);

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case", tag = "type", deny_unknown_fields)]
enum CapabilityAuthorityScopeKind {
    DenyAll,
    Allow(Box<CapabilityAuthorityAllowScope>),
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct CapabilityAuthorityAllowScope {
    identities: Selection<CapabilityId>,
    categories: Selection<CapabilityCategory>,
    operations: Selection<OperationId>,
    provider_profiles: Selection<ProviderProfileRef>,
    trust_zones: Selection<TrustZone>,
    execution_trust_classes: Selection<ExecutionTrustClass>,
    localities: Selection<Locality>,
    peers: Selection<PeerId>,
    maximum_side_effect: SideEffectClass,
}

impl CapabilityAuthorityScope {
    /// Explicitly denies every capability identity, descriptor, profile, and operation.
    #[must_use]
    pub const fn deny_all() -> Self {
        Self(CapabilityAuthorityScopeKind::DenyAll)
    }

    /// Explicitly permits every selector value subject to the supplied side-effect ceiling.
    #[must_use]
    pub fn allow_any(maximum_side_effect: SideEffectClass) -> Self {
        CapabilityAuthorityScopeBuilder::new(maximum_side_effect).build()
    }

    /// Constructs the exact semantic envelope requested by one workflow requirement.
    ///
    /// Unspecified requirement dimensions become explicit `Any` selectors. Exact identity,
    /// operation, provider profile, category, trust-zone, and execution-trust facts become
    /// nonempty `Only` selectors.
    pub fn requirement_envelope(
        requirement: &CapabilityRequirement,
    ) -> Result<Self, AuthorityError> {
        let mut builder =
            CapabilityAuthorityScopeBuilder::new(requirement.maximum_side_effect_class())
                .only_operations(BTreeSet::from([requirement.operation().clone()]))?;
        if let Some(identity) = requirement.exact_capability() {
            builder = builder.only_capabilities(BTreeSet::from([identity.clone()]))?;
        }
        if !requirement.categories().is_empty() {
            builder = builder.only_categories(requirement.categories().clone())?;
        }
        if let Some(profile) = requirement.provider_profile_ref() {
            builder = builder.only_provider_profiles(BTreeSet::from([profile.clone()]))?;
        }
        if !requirement.trust_zones().is_empty() {
            builder = builder.only_trust_zones(requirement.trust_zones().clone())?;
        }
        if let Some(trust_class) = requirement.execution_trust_class() {
            builder = builder.only_execution_trust_classes(BTreeSet::from([trust_class]))?;
        }
        Ok(builder.build())
    }

    /// Whether this scope is the explicit default-deny capability scope.
    #[must_use]
    pub const fn denies_all(&self) -> bool {
        matches!(self.0, CapabilityAuthorityScopeKind::DenyAll)
    }

    /// Capability identity selector for an allow scope.
    #[must_use]
    pub const fn identity_selection(&self) -> Option<&Selection<CapabilityId>> {
        match &self.0 {
            CapabilityAuthorityScopeKind::DenyAll => None,
            CapabilityAuthorityScopeKind::Allow(scope) => Some(&scope.identities),
        }
    }
    /// Capability category selector for an allow scope.
    #[must_use]
    pub const fn category_selection(&self) -> Option<&Selection<CapabilityCategory>> {
        match &self.0 {
            CapabilityAuthorityScopeKind::DenyAll => None,
            CapabilityAuthorityScopeKind::Allow(scope) => Some(&scope.categories),
        }
    }
    /// Capability operation selector for an allow scope.
    #[must_use]
    pub const fn operation_selection(&self) -> Option<&Selection<OperationId>> {
        match &self.0 {
            CapabilityAuthorityScopeKind::DenyAll => None,
            CapabilityAuthorityScopeKind::Allow(scope) => Some(&scope.operations),
        }
    }
    /// Provider profile selector for an allow scope.
    #[must_use]
    pub const fn provider_profile_selection(&self) -> Option<&Selection<ProviderProfileRef>> {
        match &self.0 {
            CapabilityAuthorityScopeKind::DenyAll => None,
            CapabilityAuthorityScopeKind::Allow(scope) => Some(&scope.provider_profiles),
        }
    }
    /// Trust-zone selector for an allow scope.
    #[must_use]
    pub const fn trust_zone_selection(&self) -> Option<&Selection<TrustZone>> {
        match &self.0 {
            CapabilityAuthorityScopeKind::DenyAll => None,
            CapabilityAuthorityScopeKind::Allow(scope) => Some(&scope.trust_zones),
        }
    }
    /// Execution trust-class selector for an allow scope.
    #[must_use]
    pub const fn execution_trust_class_selection(&self) -> Option<&Selection<ExecutionTrustClass>> {
        match &self.0 {
            CapabilityAuthorityScopeKind::DenyAll => None,
            CapabilityAuthorityScopeKind::Allow(scope) => Some(&scope.execution_trust_classes),
        }
    }
    /// Locality selector for an allow scope.
    #[must_use]
    pub const fn locality_selection(&self) -> Option<&Selection<Locality>> {
        match &self.0 {
            CapabilityAuthorityScopeKind::DenyAll => None,
            CapabilityAuthorityScopeKind::Allow(scope) => Some(&scope.localities),
        }
    }
    /// Authenticated peer selector for an allow scope.
    #[must_use]
    pub const fn peer_selection(&self) -> Option<&Selection<PeerId>> {
        match &self.0 {
            CapabilityAuthorityScopeKind::DenyAll => None,
            CapabilityAuthorityScopeKind::Allow(scope) => Some(&scope.peers),
        }
    }
    /// Maximum permitted side-effect class.
    #[must_use]
    pub const fn maximum_side_effect(&self) -> SideEffectClass {
        match self.0 {
            CapabilityAuthorityScopeKind::DenyAll => SideEffectClass::None,
            CapabilityAuthorityScopeKind::Allow(ref scope) => scope.maximum_side_effect,
        }
    }

    /// Whether this allow scope contains an explicit wildcard in any dimension.
    #[must_use]
    pub fn has_any_selector(&self) -> bool {
        match &self.0 {
            CapabilityAuthorityScopeKind::DenyAll => false,
            CapabilityAuthorityScopeKind::Allow(scope) => [
                scope.identities.is_any(),
                scope.categories.is_any(),
                scope.operations.is_any(),
                scope.provider_profiles.is_any(),
                scope.trust_zones.is_any(),
                scope.execution_trust_classes.is_any(),
                scope.localities.is_any(),
                scope.peers.is_any(),
            ]
            .into_iter()
            .any(|value| value),
        }
    }

    /// Tests exact containment using selector algebra and the side-effect ceiling.
    #[must_use]
    pub fn is_subset_of(&self, allowed: &Self) -> bool {
        match (&self.0, &allowed.0) {
            (CapabilityAuthorityScopeKind::DenyAll, _) => true,
            (_, CapabilityAuthorityScopeKind::DenyAll) => false,
            (
                CapabilityAuthorityScopeKind::Allow(requested),
                CapabilityAuthorityScopeKind::Allow(allowed),
            ) => {
                requested.identities.is_subset_of(&allowed.identities)
                    && requested.categories.is_subset_of(&allowed.categories)
                    && requested.operations.is_subset_of(&allowed.operations)
                    && requested
                        .provider_profiles
                        .is_subset_of(&allowed.provider_profiles)
                    && requested.trust_zones.is_subset_of(&allowed.trust_zones)
                    && requested
                        .execution_trust_classes
                        .is_subset_of(&allowed.execution_trust_classes)
                    && requested.localities.is_subset_of(&allowed.localities)
                    && requested.peers.is_subset_of(&allowed.peers)
                    && requested.maximum_side_effect <= allowed.maximum_side_effect
            }
        }
    }
}

/// Validating builder for an explicit conjunctive capability allow scope.
#[derive(Clone, Debug)]
pub struct CapabilityAuthorityScopeBuilder {
    identities: Selection<CapabilityId>,
    categories: Selection<CapabilityCategory>,
    operations: Selection<OperationId>,
    provider_profiles: Selection<ProviderProfileRef>,
    trust_zones: Selection<TrustZone>,
    execution_trust_classes: Selection<ExecutionTrustClass>,
    localities: Selection<Locality>,
    peers: Selection<PeerId>,
    maximum_side_effect: SideEffectClass,
}

impl CapabilityAuthorityScopeBuilder {
    /// Starts an explicit allow scope with `Any` in every dimension.
    #[must_use]
    pub const fn new(maximum_side_effect: SideEffectClass) -> Self {
        Self {
            identities: Selection::any(),
            categories: Selection::any(),
            operations: Selection::any(),
            provider_profiles: Selection::any(),
            trust_zones: Selection::any(),
            execution_trust_classes: Selection::any(),
            localities: Selection::any(),
            peers: Selection::any(),
            maximum_side_effect,
        }
    }

    /// Narrows capability identities to a nonempty exact allowlist.
    pub fn only_capabilities(
        mut self,
        values: BTreeSet<CapabilityId>,
    ) -> Result<Self, AuthorityError> {
        self.identities = Selection::only(values)?;
        Ok(self)
    }

    /// Narrows categories to a nonempty exact allowlist.
    pub fn only_categories(
        mut self,
        values: BTreeSet<CapabilityCategory>,
    ) -> Result<Self, AuthorityError> {
        self.categories = Selection::only(values)?;
        Ok(self)
    }

    /// Narrows operations to a nonempty exact allowlist.
    pub fn only_operations(
        mut self,
        values: BTreeSet<OperationId>,
    ) -> Result<Self, AuthorityError> {
        self.operations = Selection::only(values)?;
        Ok(self)
    }

    /// Narrows provider profiles to a nonempty exact allowlist.
    pub fn only_provider_profiles(
        mut self,
        values: BTreeSet<ProviderProfileRef>,
    ) -> Result<Self, AuthorityError> {
        self.provider_profiles = Selection::only(values)?;
        Ok(self)
    }

    /// Narrows trust zones to a nonempty exact allowlist.
    pub fn only_trust_zones(mut self, values: BTreeSet<TrustZone>) -> Result<Self, AuthorityError> {
        self.trust_zones = Selection::only(values)?;
        Ok(self)
    }

    /// Narrows execution trust classes to a nonempty exact allowlist.
    pub fn only_execution_trust_classes(
        mut self,
        values: BTreeSet<ExecutionTrustClass>,
    ) -> Result<Self, AuthorityError> {
        self.execution_trust_classes = Selection::only(values)?;
        Ok(self)
    }

    /// Narrows localities to a nonempty exact allowlist.
    pub fn only_localities(mut self, values: BTreeSet<Locality>) -> Result<Self, AuthorityError> {
        self.localities = Selection::only(values)?;
        Ok(self)
    }

    /// Narrows authenticated peers to a nonempty exact allowlist.
    pub fn only_peers(mut self, values: BTreeSet<PeerId>) -> Result<Self, AuthorityError> {
        self.peers = Selection::only(values)?;
        Ok(self)
    }

    /// Publishes the explicit allow scope.
    #[must_use]
    pub fn build(self) -> CapabilityAuthorityScope {
        CapabilityAuthorityScope(CapabilityAuthorityScopeKind::Allow(Box::new(
            CapabilityAuthorityAllowScope {
                identities: self.identities,
                categories: self.categories,
                operations: self.operations,
                provider_profiles: self.provider_profiles,
                trust_zones: self.trust_zones,
                execution_trust_classes: self.execution_trust_classes,
                localities: self.localities,
                peers: self.peers,
                maximum_side_effect: self.maximum_side_effect,
            },
        )))
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

/// Explicit artifact metadata/content scope.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(transparent)]
pub struct ArtifactAuthorityScope(ArtifactAuthorityScopeKind);

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case", tag = "type", deny_unknown_fields)]
enum ArtifactAuthorityScopeKind {
    DenyAll,
    Allow(Box<ArtifactAuthorityAllowScope>),
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct ArtifactAuthorityAllowScope {
    identities: Selection<ArtifactId>,
    sensitivities: BTreeSet<ArtifactSensitivity>,
}

impl Default for ArtifactAuthorityScope {
    fn default() -> Self {
        Self::none()
    }
}

impl ArtifactAuthorityScope {
    /// Constructs a bounded artifact allow scope with an explicit identity selector.
    pub fn new(
        identities: Selection<ArtifactId>,
        sensitivities: BTreeSet<ArtifactSensitivity>,
    ) -> Result<Self, AuthorityError> {
        if sensitivities.is_empty() || sensitivities.len() > 3 {
            return Err(AuthorityError::Bounds {
                location: "grant.artifacts",
                reason: "artifact sensitivity scope requires 1..=3 values".to_owned(),
            });
        }
        Ok(Self(ArtifactAuthorityScopeKind::Allow(Box::new(
            ArtifactAuthorityAllowScope {
                identities,
                sensitivities,
            },
        ))))
    }

    /// Grants no artifact metadata or content.
    #[must_use]
    pub fn none() -> Self {
        Self(ArtifactAuthorityScopeKind::DenyAll)
    }

    /// Deliberately broad artifact scope for acknowledged administration.
    #[must_use]
    pub fn dangerous_all() -> Self {
        Self(ArtifactAuthorityScopeKind::Allow(Box::new(
            ArtifactAuthorityAllowScope {
                identities: Selection::any(),
                sensitivities: BTreeSet::from([
                    ArtifactSensitivity::Public,
                    ArtifactSensitivity::Internal,
                    ArtifactSensitivity::Restricted,
                ]),
            },
        )))
    }

    /// Artifact identity selector for an allow scope.
    #[must_use]
    pub const fn identity_selection(&self) -> Option<&Selection<ArtifactId>> {
        match &self.0 {
            ArtifactAuthorityScopeKind::DenyAll => None,
            ArtifactAuthorityScopeKind::Allow(scope) => Some(&scope.identities),
        }
    }

    /// Explicitly visible sensitivity classes for an allow scope.
    #[must_use]
    pub const fn sensitivities(&self) -> Option<&BTreeSet<ArtifactSensitivity>> {
        match &self.0 {
            ArtifactAuthorityScopeKind::DenyAll => None,
            ArtifactAuthorityScopeKind::Allow(scope) => Some(&scope.sensitivities),
        }
    }

    /// Whether this allow scope contains an explicit identity wildcard.
    #[must_use]
    pub const fn has_any_selector(&self) -> bool {
        match &self.0 {
            ArtifactAuthorityScopeKind::DenyAll => false,
            ArtifactAuthorityScopeKind::Allow(scope) => scope.identities.is_any(),
        }
    }

    pub(crate) fn matches(&self, artifact: &ArtifactId, sensitivity: ArtifactSensitivity) -> bool {
        match &self.0 {
            ArtifactAuthorityScopeKind::DenyAll => false,
            ArtifactAuthorityScopeKind::Allow(scope) => {
                scope.sensitivities.contains(&sensitivity) && scope.identities.matches(artifact)
            }
        }
    }
}

/// Presentation-layout visibility within the separately evaluated workflow scope.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(transparent)]
pub struct LayoutAuthorityScope(LayoutAuthorityScopeKind);

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case", tag = "type", deny_unknown_fields)]
enum LayoutAuthorityScopeKind {
    DenyAll,
    Shared { revisions: Selection<RevisionId> },
}

impl Default for LayoutAuthorityScope {
    fn default() -> Self {
        Self::none()
    }
}

impl LayoutAuthorityScope {
    /// Constructs a shared-layout allow scope with an explicit revision selector.
    #[must_use]
    pub const fn shared(revisions: Selection<RevisionId>) -> Self {
        Self(LayoutAuthorityScopeKind::Shared { revisions })
    }

    /// Grants no layout reads or writes.
    #[must_use]
    pub fn none() -> Self {
        Self(LayoutAuthorityScopeKind::DenyAll)
    }

    /// Deliberately broad shared-layout scope.
    #[must_use]
    pub fn dangerous_all() -> Self {
        Self::shared(Selection::any())
    }

    /// Revision selector for a shared-layout allow scope.
    #[must_use]
    pub const fn revision_selection(&self) -> Option<&Selection<RevisionId>> {
        match &self.0 {
            LayoutAuthorityScopeKind::DenyAll => None,
            LayoutAuthorityScopeKind::Shared { revisions } => Some(revisions),
        }
    }

    /// Whether shared layouts are allowed.
    #[must_use]
    pub const fn allows_shared(&self) -> bool {
        matches!(self.0, LayoutAuthorityScopeKind::Shared { .. })
    }

    /// Whether this shared allow scope uses an explicit revision wildcard.
    #[must_use]
    pub const fn has_any_selector(&self) -> bool {
        match &self.0 {
            LayoutAuthorityScopeKind::DenyAll => false,
            LayoutAuthorityScopeKind::Shared { revisions } => revisions.is_any(),
        }
    }

    pub(crate) fn matches(&self, revision: &RevisionId, owner: &LayoutOwner) -> bool {
        matches!(owner, LayoutOwner::Shared)
            && self
                .revision_selection()
                .is_some_and(|selection| selection.matches(revision))
    }
}

/// Exact owner class of one layout request.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(
    rename_all = "snake_case",
    tag = "type",
    content = "actor",
    deny_unknown_fields
)]
pub enum LayoutOwner {
    /// Shared workflow/revision layout.
    Shared,
    /// Reserved actor-owned identity; production persistence does not implement private layouts.
    Actor(ActorRef),
}

/// Explicit configured-peer scope. Empty identities with `allow_any=false` denies all peers.
#[derive(Clone, Debug, Default, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PeerAuthorityScope {
    identities: BTreeSet<PeerId>,
    allow_any: bool,
}

impl PeerAuthorityScope {
    /// Constructs an exact or explicitly broad peer scope.
    pub fn new(identities: BTreeSet<PeerId>, allow_any: bool) -> Result<Self, AuthorityError> {
        if identities.len() > MAX_SCOPE_ITEMS || (allow_any && !identities.is_empty()) {
            return Err(AuthorityError::InvalidContract(
                "peer scope must be either an allowlist or an explicit wildcard".to_owned(),
            ));
        }
        Ok(Self {
            identities,
            allow_any,
        })
    }

    /// Grants no peer relationship access.
    #[must_use]
    pub fn none() -> Self {
        Self::default()
    }

    /// Deliberately broad peer scope.
    #[must_use]
    pub fn dangerous_all() -> Self {
        Self {
            identities: BTreeSet::new(),
            allow_any: true,
        }
    }

    /// Exact peer identities.
    #[must_use]
    pub const fn identities(&self) -> &BTreeSet<PeerId> {
        &self.identities
    }

    /// Whether every configured peer is explicitly included.
    #[must_use]
    pub const fn allows_any(&self) -> bool {
        self.allow_any
    }

    pub(crate) fn matches(&self, peer: &PeerId) -> bool {
        self.allow_any || self.identities.contains(peer)
    }
}

/// Daemon-local information scopes kept separate from authentication and workflow access.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DaemonAuthorityScope {
    /// Coarse liveness/readiness.
    pub readiness: bool,
    /// Queue, worker, lifecycle, and failure detail.
    pub detailed_health: bool,
    /// The caller's own grant identity and operation vocabulary.
    pub own_authority: bool,
    /// Redacted effective configuration.
    pub configuration: bool,
    /// Bounded security audit/diagnostic records.
    pub audit: bool,
}

impl DaemonAuthorityScope {
    /// Deliberately broad daemon-local read scope.
    #[must_use]
    pub const fn dangerous_all() -> Self {
        Self {
            readiness: true,
            detailed_health: true,
            own_authority: true,
            configuration: true,
            audit: true,
        }
    }
}

/// Workspace scope allowlist for future externally projected values.
#[derive(Clone, Debug, Default, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct WorkspaceAuthorityScope {
    scopes: BTreeSet<ScopeId>,
    allow_any_in_run: bool,
}

impl WorkspaceAuthorityScope {
    /// Constructs an exact or explicitly run-wide workspace scope.
    pub fn new(scopes: BTreeSet<ScopeId>, allow_any_in_run: bool) -> Result<Self, AuthorityError> {
        if scopes.len() > MAX_SCOPE_ITEMS || (allow_any_in_run && !scopes.is_empty()) {
            return Err(AuthorityError::InvalidContract(
                "workspace scope must be either an allowlist or an explicit run wildcard"
                    .to_owned(),
            ));
        }
        Ok(Self {
            scopes,
            allow_any_in_run,
        })
    }

    /// Grants no workspace values.
    #[must_use]
    pub fn none() -> Self {
        Self::default()
    }

    /// Deliberately grants all scopes within an otherwise authorized run.
    #[must_use]
    pub fn dangerous_all_in_run() -> Self {
        Self {
            scopes: BTreeSet::new(),
            allow_any_in_run: true,
        }
    }

    /// Exact workspace scope identities.
    #[must_use]
    pub const fn scopes(&self) -> &BTreeSet<ScopeId> {
        &self.scopes
    }

    /// Whether all scopes in an authorized run are included.
    #[must_use]
    pub const fn allows_any_in_run(&self) -> bool {
        self.allow_any_in_run
    }

    pub(crate) fn matches(&self, scope: &ScopeId) -> bool {
        self.allow_any_in_run || self.scopes.contains(scope)
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
    /// Artifact identity and sensitivity scope. Empty sensitivity scope denies metadata and bytes.
    pub artifacts: ArtifactAuthorityScope,
    /// Presentation-layout revision/owner scope.
    pub layouts: LayoutAuthorityScope,
    /// Configured peer relationship scope.
    pub peers: PeerAuthorityScope,
    /// Daemon-local read/diagnostic scope.
    pub daemon: DaemonAuthorityScope,
    /// Workspace-value scope.
    pub workspace: WorkspaceAuthorityScope,
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
    /// Strictly decodes and validates one schema-v4 grant.
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
        if version != AUTHORITY_GRANT_SCHEMA_VERSION_V4 {
            return Err(AuthorityError::UnsupportedVersion {
                document: "authority_grant",
                found: version,
                supported: AUTHORITY_GRANT_SCHEMA_VERSION_V4,
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
                schema_version: AUTHORITY_GRANT_SCHEMA_VERSION_V4,
                identity,
                revision,
                actor,
                operations: BTreeSet::new(),
                resources: ResourceScope {
                    workflow_run: WorkflowRunScope::Any,
                    capability: CapabilityAuthorityScope::deny_all(),
                    filesystem: Vec::new(),
                    network: NetworkScope {
                        profiles: BTreeSet::new(),
                        destinations: BTreeSet::new(),
                    },
                    secrets: BTreeSet::new(),
                    artifacts: ArtifactAuthorityScope::none(),
                    layouts: LayoutAuthorityScope::none(),
                    peers: PeerAuthorityScope::none(),
                    daemon: DaemonAuthorityScope::default(),
                    workspace: WorkspaceAuthorityScope::none(),
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
        if grant.schema_version != AUTHORITY_GRANT_SCHEMA_VERSION_V4 {
            return Err(AuthorityError::UnsupportedVersion {
                document: "authority_grant",
                found: grant.schema_version,
                supported: AUTHORITY_GRANT_SCHEMA_VERSION_V4,
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
        NetworkScope::new(
            grant.resources.network.profiles.clone(),
            grant.resources.network.destinations.clone(),
        )?;
        for scope in &grant.resources.filesystem {
            FilesystemScope::new(scope.root.clone(), scope.access.clone())?;
        }
        if let ArtifactAuthorityScopeKind::Allow(scope) = &grant.resources.artifacts.0 {
            ArtifactAuthorityScope::new(scope.identities.clone(), scope.sensitivities.clone())?;
        }
        PeerAuthorityScope::new(
            grant.resources.peers.identities.clone(),
            grant.resources.peers.allow_any,
        )?;
        WorkspaceAuthorityScope::new(
            grant.resources.workspace.scopes.clone(),
            grant.resources.workspace.allow_any_in_run,
        )?;
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
            FilesystemScope::new(scope.root.clone(), scope.access.clone())?;
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
