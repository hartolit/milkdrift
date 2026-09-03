use std::{collections::BTreeSet, path::Path};

use milkdrift_blueprint::{RevisionId, WorkflowId};
use milkdrift_workspace::{ArtifactId, ArtifactSensitivity, RunId, ScopeId};
use serde::{Deserialize, Deserializer, Serialize, Serializer, de::Error as _};

use crate::{ActorRef, AuthorityError, NetworkProfileRef, PeerId, SecretRef, Selection};

use super::{capability::CapabilityAuthorityScope, decision::RequestedResourceFacts};

pub(super) const MAX_SCOPE_ITEMS: usize = 128;

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

#[derive(Clone, Debug, Eq, PartialEq)]
enum FilesystemRootFamily {
    Unix,
    WindowsDrive(u8),
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct CanonicalFilesystemRoot {
    text: String,
    family: FilesystemRootFamily,
    components: Vec<String>,
}

impl CanonicalFilesystemRoot {
    fn parse(text: String) -> Result<Self, AuthorityError> {
        if text.is_empty()
            || text.len() > 4_096
            || text.contains('\0')
            || text.contains('\\')
            || text.chars().any(char::is_control)
        {
            return Err(invalid_filesystem_scope());
        }
        let (family, remainder) = if let Some(remainder) = text.strip_prefix('/') {
            (FilesystemRootFamily::Unix, remainder)
        } else if text.len() >= 3
            && text.as_bytes()[0].is_ascii_uppercase()
            && text.as_bytes()[0].is_ascii_alphabetic()
            && text.as_bytes()[1] == b':'
            && text.as_bytes()[2] == b'/'
        {
            (
                FilesystemRootFamily::WindowsDrive(text.as_bytes()[0]),
                &text[3..],
            )
        } else {
            return Err(invalid_filesystem_scope());
        };
        if remainder.ends_with('/') || remainder.contains("//") {
            return Err(invalid_filesystem_scope());
        }
        let components = if remainder.is_empty() {
            Vec::new()
        } else {
            remainder
                .split('/')
                .map(|component| {
                    if component.is_empty()
                        || matches!(component, "." | "..")
                        || (matches!(family, FilesystemRootFamily::WindowsDrive(_))
                            && !windows_component_is_canonical(component))
                    {
                        return Err(invalid_filesystem_scope());
                    }
                    Ok(component.to_owned())
                })
                .collect::<Result<Vec<_>, _>>()?
        };
        Ok(Self {
            text,
            family,
            components,
        })
    }

    fn contains(&self, requested: &Self) -> bool {
        self.family == requested.family && requested.components.starts_with(&self.components)
    }
}

fn windows_component_is_canonical(component: &str) -> bool {
    if component.ends_with(['.', ' ']) || component.contains(['<', '>', '"', '|', '?', '*', ':']) {
        return false;
    }
    let stem = component.split('.').next().unwrap_or_default();
    let upper = stem.to_ascii_uppercase();
    if matches!(upper.as_str(), "CON" | "PRN" | "AUX" | "NUL") {
        return false;
    }
    let bytes = upper.as_bytes();
    if bytes.len() == 4
        && (bytes.starts_with(b"COM") || bytes.starts_with(b"LPT"))
        && matches!(bytes[3], b'1'..=b'9')
    {
        return false;
    }
    !["COM¹", "COM²", "COM³", "LPT¹", "LPT²", "LPT³"]
        .iter()
        .any(|reserved| stem.eq_ignore_ascii_case(reserved))
}

fn invalid_filesystem_scope() -> AuthorityError {
    AuthorityError::InvalidContract(
        "filesystem scope requires one canonical Unix or Windows drive-absolute root and a nonempty access set"
            .to_owned(),
    )
}

/// One canonical Unix or Windows drive-absolute filesystem root and allowed access modes.
///
/// Durable roots use `/` separators. Unix roots begin with `/`; Windows roots begin with an
/// uppercase ASCII drive and `:/`. UNC, device, drive-relative, traversal, alternate-data-stream,
/// mixed-separator, and non-UTF-8 forms are deliberately not representable. Windows drive letters
/// are normalized by the host conversion boundary, while path components retain exact case so
/// authority evaluation fails closed instead of approximating filesystem Unicode case behavior.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FilesystemScope {
    root: CanonicalFilesystemRoot,
    access: BTreeSet<AccessMode>,
}

impl FilesystemScope {
    /// Deliberately broad Unix root scope for explicitly acknowledged administration.
    ///
    /// This does not grant access to any Windows drive.
    #[must_use]
    pub fn dangerous_all_access_unix_root() -> Self {
        Self {
            root: CanonicalFilesystemRoot {
                text: "/".to_owned(),
                family: FilesystemRootFamily::Unix,
                components: Vec::new(),
            },
            access: BTreeSet::from([AccessMode::Read, AccessMode::Write, AccessMode::Execute]),
        }
    }

    /// Deliberately broad all-access scope for one explicit Windows drive.
    pub fn dangerous_all_access_windows_drive(drive: char) -> Result<Self, AuthorityError> {
        if !drive.is_ascii_alphabetic() {
            return Err(invalid_filesystem_scope());
        }
        Self::new(
            format!("{}:/", drive.to_ascii_uppercase()),
            BTreeSet::from([AccessMode::Read, AccessMode::Write, AccessMode::Execute]),
        )
    }

    /// Deliberately broad all-access scopes for every ordinary Windows drive identity.
    #[must_use]
    pub fn dangerous_all_access_windows_drives() -> Vec<Self> {
        (b'A'..=b'Z')
            .map(|drive| Self {
                root: CanonicalFilesystemRoot {
                    text: format!("{}:/", char::from(drive)),
                    family: FilesystemRootFamily::WindowsDrive(drive),
                    components: Vec::new(),
                },
                access: BTreeSet::from([AccessMode::Read, AccessMode::Write, AccessMode::Execute]),
            })
            .collect()
    }

    /// Constructs one canonical durable Unix or Windows drive-absolute root.
    pub fn new(
        root: impl Into<String>,
        access: BTreeSet<AccessMode>,
    ) -> Result<Self, AuthorityError> {
        if access.is_empty() {
            return Err(invalid_filesystem_scope());
        }
        Ok(Self {
            root: CanonicalFilesystemRoot::parse(root.into())?,
            access,
        })
    }

    /// Converts a host-canonicalized native path into the durable authority representation.
    ///
    /// This performs no filesystem access. The caller remains responsible for canonicalizing and
    /// inspecting the native path at its trust boundary before calling this conversion.
    pub fn from_canonical_host_path(
        path: &Path,
        access: BTreeSet<AccessMode>,
    ) -> Result<Self, AuthorityError> {
        #[cfg(unix)]
        {
            if !path.is_absolute() {
                return Err(invalid_filesystem_scope());
            }
            let text = path.to_str().ok_or_else(invalid_filesystem_scope)?;
            Self::new(text, access)
        }
        #[cfg(windows)]
        {
            use std::path::{Component, Prefix};

            let mut components = path.components();
            let drive = match components.next() {
                Some(Component::Prefix(prefix)) => match prefix.kind() {
                    Prefix::Disk(drive) | Prefix::VerbatimDisk(drive)
                        if drive.is_ascii_alphabetic() =>
                    {
                        drive.to_ascii_uppercase()
                    }
                    Prefix::Verbatim(_)
                    | Prefix::VerbatimUNC(_, _)
                    | Prefix::UNC(_, _)
                    | Prefix::DeviceNS(_)
                    | Prefix::Disk(_)
                    | Prefix::VerbatimDisk(_) => return Err(invalid_filesystem_scope()),
                },
                Some(Component::RootDir)
                | Some(Component::CurDir)
                | Some(Component::ParentDir)
                | Some(Component::Normal(_))
                | None => return Err(invalid_filesystem_scope()),
            };
            if !matches!(components.next(), Some(Component::RootDir)) {
                return Err(invalid_filesystem_scope());
            }
            let mut text = String::from(char::from(drive));
            text.push_str(":/");
            for component in components {
                let Component::Normal(component) = component else {
                    return Err(invalid_filesystem_scope());
                };
                let component = component.to_str().ok_or_else(invalid_filesystem_scope)?;
                if component.is_empty()
                    || component.contains(['/', '\\', ':', '\0'])
                    || component.chars().any(char::is_control)
                {
                    return Err(invalid_filesystem_scope());
                }
                if !text.ends_with('/') {
                    text.push('/');
                }
                text.push_str(component);
            }
            Self::new(text, access)
        }
        #[cfg(not(any(unix, windows)))]
        {
            let _ = path;
            let _ = access;
            Err(invalid_filesystem_scope())
        }
    }

    /// Canonical durable root text.
    #[must_use]
    pub fn root(&self) -> &str {
        &self.root.text
    }

    /// Allowed access modes.
    #[must_use]
    pub const fn access(&self) -> &BTreeSet<AccessMode> {
        &self.access
    }

    pub(crate) fn contains(&self, requested: &Self) -> bool {
        self.root.contains(&requested.root) && requested.access.is_subset(&self.access)
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FilesystemScopeDocument {
    root: String,
    access: BTreeSet<AccessMode>,
}

#[derive(Serialize)]
struct FilesystemScopeRef<'a> {
    root: &'a str,
    access: &'a BTreeSet<AccessMode>,
}

impl Serialize for FilesystemScope {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        FilesystemScopeRef {
            root: self.root(),
            access: &self.access,
        }
        .serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for FilesystemScope {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let document = FilesystemScopeDocument::deserialize(deserializer)?;
        Self::new(document.root, document.access).map_err(D::Error::custom)
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
