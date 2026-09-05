use milkdrift_capability::BoundedJson;
use serde::{Deserialize, Serialize};

use crate::{ArtifactReference, ScopeReference, ValueKey, ValueVersion, WorkspaceError};

/// Exact durable reference to one immutable workspace value version.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct WorkspaceValueReference {
    scope: ScopeReference,
    key: ValueKey,
    version: ValueVersion,
}

impl WorkspaceValueReference {
    /// Constructs an exact value reference.
    #[must_use]
    pub const fn new(scope: ScopeReference, key: ValueKey, version: ValueVersion) -> Self {
        Self {
            scope,
            key,
            version,
        }
    }

    /// Returns the owning scope.
    #[must_use]
    pub const fn scope(&self) -> &ScopeReference {
        &self.scope
    }

    /// Returns the scope-local value key.
    #[must_use]
    pub const fn key(&self) -> &ValueKey {
        &self.key
    }

    /// Returns the exact immutable version.
    #[must_use]
    pub const fn version(&self) -> ValueVersion {
        self.version
    }
}

/// Bounded content of one workspace value version.
///
/// JSON uses [`BoundedJson`]'s depth, string, item, and encoded-size limits.
/// Artifact values contain only an immutable, content-addressed reference.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(
    rename_all = "snake_case",
    tag = "type",
    content = "value",
    deny_unknown_fields
)]
pub enum WorkspaceValue {
    /// Small inline structured data.
    Json(BoundedJson),
    /// Reference to separately stored immutable artifact bytes.
    Artifact(ArtifactReference),
}

impl WorkspaceValue {
    /// Returns the inline value when this is JSON.
    #[must_use]
    pub const fn as_json(&self) -> Option<&BoundedJson> {
        match self {
            Self::Json(value) => Some(value),
            Self::Artifact(_) => None,
        }
    }

    /// Returns the artifact reference when this is an artifact value.
    #[must_use]
    pub const fn as_artifact(&self) -> Option<&ArtifactReference> {
        match self {
            Self::Artifact(reference) => Some(reference),
            Self::Json(_) => None,
        }
    }

    pub(crate) fn inline_size_bytes(&self) -> Result<u64, WorkspaceError> {
        match self {
            Self::Json(value) => u64::try_from(serde_json::to_vec(value)?.len())
                .map_err(|_| WorkspaceError::AccountingOverflow("inline value bytes")),
            Self::Artifact(_) => Ok(0),
        }
    }
}

/// Immutable origin fact for one scope-local value version.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case", tag = "type", deny_unknown_fields)]
pub enum ValueOrigin {
    /// First value in a stream with no inherited source.
    Initial,
    /// Next version in the same scope-local stream.
    Successor {
        /// Exact preceding version in the same scope and key.
        previous: WorkspaceValueReference,
    },
    /// First local version derived from an immutable ancestor value.
    Inherited {
        /// Exact source value in an ancestor scope.
        source: WorkspaceValueReference,
    },
    /// First local version imported from an exact value in another run.
    Imported {
        /// Exact immutable source value owned by the child/foreign run.
        source: WorkspaceValueReference,
    },
}

/// One immutable and fully versioned workspace value record.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct WorkspaceValueEntry {
    reference: WorkspaceValueReference,
    value: WorkspaceValue,
    origin: ValueOrigin,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WorkspaceValueEntryWire {
    reference: WorkspaceValueReference,
    value: WorkspaceValue,
    origin: ValueOrigin,
}

milkdrift_contracts::deserialize_via!(WorkspaceValueEntry, WorkspaceValueEntryWire, |wire| {
    Self::new(wire.reference, wire.value, wire.origin)
});

impl WorkspaceValueEntry {
    /// Creates version one of a new scope-local value stream.
    #[must_use]
    pub fn initial(scope: ScopeReference, key: ValueKey, value: WorkspaceValue) -> Self {
        Self {
            reference: WorkspaceValueReference::new(scope, key, ValueVersion::FIRST),
            value,
            origin: ValueOrigin::Initial,
        }
    }

    /// Creates version one of a new local stream derived from an ancestor value.
    ///
    /// The source and target must be different scopes in the same run. A
    /// [`crate::ScopeLineage`] can additionally prove that the source is an
    /// ancestor rather than a sibling before this record is persisted.
    pub fn inherited(
        scope: ScopeReference,
        key: ValueKey,
        source: WorkspaceValueReference,
        value: WorkspaceValue,
    ) -> Result<Self, WorkspaceError> {
        Self::new(
            WorkspaceValueReference::new(scope, key, ValueVersion::FIRST),
            value,
            ValueOrigin::Inherited { source },
        )
    }

    /// Creates version one of a local stream imported from another run.
    ///
    /// Storage proves the exact source exists. Cross-run imports deliberately do
    /// not use scope ancestry; the corresponding runtime event proves the
    /// parent/subworkflow ownership relationship.
    pub fn imported(
        scope: ScopeReference,
        key: ValueKey,
        source: WorkspaceValueReference,
        value: WorkspaceValue,
    ) -> Result<Self, WorkspaceError> {
        Self::new(
            WorkspaceValueReference::new(scope, key, ValueVersion::FIRST),
            value,
            ValueOrigin::Imported { source },
        )
    }

    /// Creates the exact next version of an existing scope-local value stream.
    pub fn successor(
        previous: WorkspaceValueReference,
        value: WorkspaceValue,
    ) -> Result<Self, WorkspaceError> {
        let version = previous.version().next()?;
        Self::new(
            WorkspaceValueReference::new(previous.scope().clone(), previous.key().clone(), version),
            value,
            ValueOrigin::Successor { previous },
        )
    }

    fn new(
        reference: WorkspaceValueReference,
        value: WorkspaceValue,
        origin: ValueOrigin,
    ) -> Result<Self, WorkspaceError> {
        match &origin {
            ValueOrigin::Initial if reference.version() != ValueVersion::FIRST => {
                return Err(WorkspaceError::InvalidValue(
                    "an initial value must have version one".to_owned(),
                ));
            }
            ValueOrigin::Initial => {}
            ValueOrigin::Successor { previous } => {
                if previous.scope() != reference.scope() || previous.key() != reference.key() {
                    return Err(WorkspaceError::InvalidValue(
                        "a successor must keep the previous scope and key".to_owned(),
                    ));
                }
                if previous.version().next()? != reference.version() {
                    return Err(WorkspaceError::InvalidValue(
                        "a successor version must be exactly one greater than its predecessor"
                            .to_owned(),
                    ));
                }
            }
            ValueOrigin::Inherited { source } => {
                if reference.version() != ValueVersion::FIRST {
                    return Err(WorkspaceError::InvalidValue(
                        "an inherited local stream must begin at version one".to_owned(),
                    ));
                }
                if source.scope().run() != reference.scope().run() {
                    return Err(WorkspaceError::InvalidValue(
                        "an inherited value must remain within one run".to_owned(),
                    ));
                }
                if source.scope() == reference.scope() {
                    return Err(WorkspaceError::InvalidValue(
                        "same-scope changes must be represented as successors".to_owned(),
                    ));
                }
            }
            ValueOrigin::Imported { source } => {
                if reference.version() != ValueVersion::FIRST {
                    return Err(WorkspaceError::InvalidValue(
                        "an imported local stream must begin at version one".to_owned(),
                    ));
                }
                if source.scope().run() == reference.scope().run() {
                    return Err(WorkspaceError::InvalidValue(
                        "an imported value source must belong to a different run".to_owned(),
                    ));
                }
            }
        }
        Ok(Self {
            reference,
            value,
            origin,
        })
    }

    /// Returns this entry's exact durable reference.
    #[must_use]
    pub const fn reference(&self) -> &WorkspaceValueReference {
        &self.reference
    }

    /// Returns the immutable bounded content.
    #[must_use]
    pub const fn value(&self) -> &WorkspaceValue {
        &self.value
    }

    /// Returns the immutable stream-origin fact.
    #[must_use]
    pub const fn origin(&self) -> &ValueOrigin {
        &self.origin
    }
}
