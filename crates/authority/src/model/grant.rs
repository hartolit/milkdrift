use std::collections::{BTreeMap, BTreeSet};

use milkdrift_capability::BoundedJson;
use serde::{Deserialize, Serialize};

use crate::{
    ActorRef, AuthorityError, GrantDigest, GrantId,
    document::{AUTHORITY_GRANT_SCHEMA_VERSION_V4, canonical_json},
};

use super::{
    capability::CapabilityAuthorityScope,
    resource::{
        ArtifactAuthorityScope, AuthorityBudget, AuthorityOperation, BoundaryTimeMillis,
        DaemonAuthorityScope, FilesystemScope, LayoutAuthorityScope, MAX_SCOPE_ITEMS, NetworkScope,
        PeerAuthorityScope, ResourceScope, WorkflowRunScope, WorkspaceAuthorityScope,
    },
};

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

milkdrift_contracts::deserialize_via!(AuthorityGrant, AuthorityGrantWire, |wire| {
    AuthorityGrantBuilder::new(wire.identity, wire.revision, wire.actor)
        .operations(wire.operations)
        .resources(wire.resources)
        .budget(wire.budget)
        .validity(wire.valid_from, wire.valid_until)
        .revocation_generation(wire.revocation_generation)
        .extensions(wire.extensions)
        .schema_version(wire.schema_version)
        .build()
});

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
        if bytes.len() > crate::document::MAX_AUTHORITY_DOCUMENT_BYTES {
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
                    network: NetworkScope::empty(),
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
            grant.resources.network.profiles().clone(),
            grant.resources.network.destinations().clone(),
        )?;
        for scope in &grant.resources.filesystem {
            FilesystemScope::new(scope.root().to_owned(), scope.access().clone())?;
        }
        if let (Some(identities), Some(sensitivities)) = (
            grant.resources.artifacts.identity_selection(),
            grant.resources.artifacts.sensitivities(),
        ) {
            ArtifactAuthorityScope::new(identities.clone(), sensitivities.clone())?;
        }
        PeerAuthorityScope::new(
            grant.resources.peers.identities().clone(),
            grant.resources.peers.allows_any(),
        )?;
        WorkspaceAuthorityScope::new(
            grant.resources.workspace.scopes().clone(),
            grant.resources.workspace.allows_any_in_run(),
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
