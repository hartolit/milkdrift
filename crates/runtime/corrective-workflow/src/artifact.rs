//! Immutable typed workflow artifact storage.

use std::collections::BTreeMap;
use std::num::NonZeroUsize;

use super::{
    ArtifactId, ArtifactKind, ArtifactReference, ArtifactRole, NormalizedValidationReport,
    ValidationReport, WorkflowError,
};

/// Stable discriminator for an artifact payload variant.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ArtifactContentKind {
    /// Root workflow specification text.
    Specification,
    /// Initial generated draft text.
    Draft,
    /// Raw typed validation report.
    RawValidation,
    /// Deterministically normalized validation report.
    NormalizedDiagnostics,
    /// Model-produced review text.
    Review,
    /// Model-produced revision text.
    Revision,
    /// Final typed validation report.
    FinalValidation,
}

/// Owned payload stored for one immutable artifact.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ArtifactContent {
    /// Root workflow specification text.
    Specification(String),
    /// Initial generated draft text.
    Draft(String),
    /// Raw typed validation report.
    RawValidation(ValidationReport),
    /// Deterministically normalized validation report.
    NormalizedDiagnostics(NormalizedValidationReport),
    /// Model-produced review text.
    Review(String),
    /// Model-produced revision text.
    Revision(String),
    /// Final typed validation report.
    FinalValidation(ValidationReport),
}

impl ArtifactContent {
    /// Returns the stable payload discriminator.
    #[must_use]
    pub const fn kind(&self) -> ArtifactContentKind {
        match self {
            Self::Specification(_) => ArtifactContentKind::Specification,
            Self::Draft(_) => ArtifactContentKind::Draft,
            Self::RawValidation(_) => ArtifactContentKind::RawValidation,
            Self::NormalizedDiagnostics(_) => ArtifactContentKind::NormalizedDiagnostics,
            Self::Review(_) => ArtifactContentKind::Review,
            Self::Revision(_) => ArtifactContentKind::Revision,
            Self::FinalValidation(_) => ArtifactContentKind::FinalValidation,
        }
    }
}

/// One immutable typed artifact and its graph-level reference.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Artifact {
    reference: ArtifactReference,
    content: ArtifactContent,
}

impl Artifact {
    /// Creates an artifact after validating its kind, role, and content variant.
    ///
    /// # Errors
    ///
    /// Returns [`WorkflowError::ArtifactContentMismatch`] when the reference does
    /// not describe the supplied payload variant.
    pub fn new(
        reference: ArtifactReference,
        content: ArtifactContent,
    ) -> Result<Self, WorkflowError> {
        validate_content(reference, &content)?;
        Ok(Self { reference, content })
    }

    /// Returns the immutable graph-level reference.
    #[must_use]
    pub const fn reference(&self) -> ArtifactReference {
        self.reference
    }

    /// Returns the immutable typed payload.
    #[must_use]
    pub const fn content(&self) -> &ArtifactContent {
        &self.content
    }
}

/// Ordered fixed-capacity immutable artifact repository.
#[derive(Debug)]
pub struct ArtifactStore {
    artifacts: BTreeMap<ArtifactId, Artifact>,
    maximum_artifacts: NonZeroUsize,
}

impl ArtifactStore {
    /// Creates an empty artifact store with a fixed non-zero capacity.
    #[must_use]
    pub const fn new(maximum_artifacts: NonZeroUsize) -> Self {
        Self {
            artifacts: BTreeMap::new(),
            maximum_artifacts,
        }
    }

    /// Inserts one validated artifact without permitting replacement or growth
    /// beyond the configured capacity.
    ///
    /// # Errors
    ///
    /// Returns [`WorkflowError::ArtifactContentMismatch`] for an invalid typed
    /// reference, [`WorkflowError::DuplicateArtifact`] when the identity is
    /// already present, or [`WorkflowError::ArtifactCapacityExceeded`] when the
    /// store is full. Existing artifacts are never overwritten.
    pub fn insert(&mut self, artifact: Artifact) -> Result<(), WorkflowError> {
        validate_content(artifact.reference, &artifact.content)?;
        let id = artifact.reference.id;
        if self.artifacts.contains_key(&id) {
            return Err(WorkflowError::DuplicateArtifact(id));
        }
        let available = self.remaining_capacity();
        if available == 0 {
            return Err(WorkflowError::ArtifactCapacityExceeded {
                required: 1,
                available,
            });
        }
        self.artifacts.insert(id, artifact);
        Ok(())
    }

    /// Returns an artifact by identity.
    #[must_use]
    pub fn get(&self, id: ArtifactId) -> Option<&Artifact> {
        self.artifacts.get(&id)
    }

    /// Returns the number of committed artifacts.
    #[must_use]
    pub fn len(&self) -> usize {
        self.artifacts.len()
    }

    /// Returns the fixed maximum number of committed artifacts.
    #[must_use]
    pub const fn capacity(&self) -> usize {
        self.maximum_artifacts.get()
    }

    /// Returns the number of artifact entries still available.
    #[must_use]
    pub fn remaining_capacity(&self) -> usize {
        let capacity = self.maximum_artifacts.get();
        if self.artifacts.len() >= capacity {
            0
        } else {
            capacity - self.artifacts.len()
        }
    }

    /// Returns whether no artifacts have been committed.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.artifacts.is_empty()
    }
}

/// Borrowed resolver restricted to one task's declared artifact identities.
///
/// The backing store and declaration slice are private, so a port cannot bypass
/// the identity allowlist through this view.
#[derive(Clone, Copy, Debug)]
pub struct ArtifactInputs<'a> {
    store: &'a ArtifactStore,
    ids: &'a [ArtifactId],
}

impl<'a> ArtifactInputs<'a> {
    pub(crate) const fn new(store: &'a ArtifactStore, ids: &'a [ArtifactId]) -> Self {
        Self { store, ids }
    }

    /// Returns the exact artifact identities declared for the task.
    #[must_use]
    pub const fn ids(&self) -> &'a [ArtifactId] {
        self.ids
    }

    /// Resolves an artifact only when its identity was declared for this task.
    #[must_use]
    pub fn get(&self, id: ArtifactId) -> Option<&'a Artifact> {
        if self.ids.contains(&id) {
            self.store.get(id)
        } else {
            None
        }
    }
}

const fn validate_content(
    reference: ArtifactReference,
    content: &ArtifactContent,
) -> Result<(), WorkflowError> {
    let valid = matches!(
        (reference.kind, reference.role, content),
        (
            ArtifactKind::Text,
            ArtifactRole::Specification,
            ArtifactContent::Specification(_)
        ) | (
            ArtifactKind::Text,
            ArtifactRole::Draft,
            ArtifactContent::Draft(_)
        ) | (
            ArtifactKind::Diagnostics,
            ArtifactRole::RawDiagnostics,
            ArtifactContent::RawValidation(_)
        ) | (
            ArtifactKind::Diagnostics,
            ArtifactRole::NormalizedDiagnostics,
            ArtifactContent::NormalizedDiagnostics(_)
        ) | (
            ArtifactKind::Text,
            ArtifactRole::Review,
            ArtifactContent::Review(_)
        ) | (
            ArtifactKind::Text,
            ArtifactRole::Revision,
            ArtifactContent::Revision(_)
        ) | (
            ArtifactKind::Diagnostics,
            ArtifactRole::FinalValidation,
            ArtifactContent::FinalValidation(_)
        )
    );
    if valid {
        Ok(())
    } else {
        Err(WorkflowError::ArtifactContentMismatch {
            reference,
            content: content.kind(),
        })
    }
}
