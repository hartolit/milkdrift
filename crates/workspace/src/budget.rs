use serde::{Deserialize, Deserializer, Serialize};

use crate::{ArtifactMetadata, ArtifactReference, WorkspaceError, WorkspaceValue};

/// Immutable limits applied to one durable workspace accounting domain.
///
/// Limits are inclusive. A zero limit intentionally disables that resource.
/// A run's first admission of an artifact and each workspace value version are
/// charged separately. Persistence de-duplicates later references to the same
/// immutable artifact within that run.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct WorkspaceBudget {
    max_value_versions: u64,
    max_inline_bytes_per_value: u64,
    max_total_inline_bytes: u64,
    max_artifacts: u64,
    max_bytes_per_artifact: u64,
    max_total_artifact_bytes: u64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WorkspaceBudgetWire {
    max_value_versions: u64,
    max_inline_bytes_per_value: u64,
    max_total_inline_bytes: u64,
    max_artifacts: u64,
    max_bytes_per_artifact: u64,
    max_total_artifact_bytes: u64,
}

impl<'de> Deserialize<'de> for WorkspaceBudget {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire = WorkspaceBudgetWire::deserialize(deserializer)?;
        Self::new(
            wire.max_value_versions,
            wire.max_inline_bytes_per_value,
            wire.max_total_inline_bytes,
            wire.max_artifacts,
            wire.max_bytes_per_artifact,
            wire.max_total_artifact_bytes,
        )
        .map_err(serde::de::Error::custom)
    }
}

impl WorkspaceBudget {
    /// Constructs a consistent set of workspace limits.
    #[allow(clippy::too_many_arguments)] // One validated workspace budget keeps every independent resource ceiling explicit.
    pub fn new(
        max_value_versions: u64,
        max_inline_bytes_per_value: u64,
        max_total_inline_bytes: u64,
        max_artifacts: u64,
        max_bytes_per_artifact: u64,
        max_total_artifact_bytes: u64,
    ) -> Result<Self, WorkspaceError> {
        if max_inline_bytes_per_value > max_total_inline_bytes {
            return Err(WorkspaceError::InvalidBudget(
                "per-value inline bytes cannot exceed total inline bytes".to_owned(),
            ));
        }
        if max_bytes_per_artifact > max_total_artifact_bytes {
            return Err(WorkspaceError::InvalidBudget(
                "per-artifact bytes cannot exceed total artifact bytes".to_owned(),
            ));
        }
        Ok(Self {
            max_value_versions,
            max_inline_bytes_per_value,
            max_total_inline_bytes,
            max_artifacts,
            max_bytes_per_artifact,
            max_total_artifact_bytes,
        })
    }

    /// Returns the maximum number of durable value versions.
    #[must_use]
    pub const fn max_value_versions(&self) -> u64 {
        self.max_value_versions
    }

    /// Returns the maximum encoded bytes of one inline JSON value.
    #[must_use]
    pub const fn max_inline_bytes_per_value(&self) -> u64 {
        self.max_inline_bytes_per_value
    }

    /// Returns the aggregate maximum encoded bytes of inline value versions.
    #[must_use]
    pub const fn max_total_inline_bytes(&self) -> u64 {
        self.max_total_inline_bytes
    }

    /// Returns the maximum number of artifact metadata records.
    #[must_use]
    pub const fn max_artifacts(&self) -> u64 {
        self.max_artifacts
    }

    /// Returns the maximum exact size of one artifact.
    #[must_use]
    pub const fn max_bytes_per_artifact(&self) -> u64 {
        self.max_bytes_per_artifact
    }

    /// Returns the aggregate maximum bytes of admitted artifacts.
    #[must_use]
    pub const fn max_total_artifact_bytes(&self) -> u64 {
        self.max_total_artifact_bytes
    }

    /// Validates persisted/projected usage against this budget.
    pub fn validate_usage(&self, usage: &WorkspaceUsage) -> Result<(), WorkspaceError> {
        enforce(
            "value versions",
            self.max_value_versions,
            usage.value_versions,
        )?;
        enforce(
            "total inline bytes",
            self.max_total_inline_bytes,
            usage.inline_bytes,
        )?;
        enforce("artifacts", self.max_artifacts, usage.artifacts)?;
        enforce(
            "total artifact bytes",
            self.max_total_artifact_bytes,
            usage.artifact_bytes,
        )
    }

    /// Computes usage after admitting one immutable workspace value version.
    pub fn admit_value(
        &self,
        usage: &WorkspaceUsage,
        value: &WorkspaceValue,
    ) -> Result<WorkspaceUsage, WorkspaceError> {
        self.validate_usage(usage)?;
        let inline_bytes = value.inline_size_bytes()?;
        enforce(
            "inline bytes per value",
            self.max_inline_bytes_per_value,
            inline_bytes,
        )?;
        let value_versions = checked_add(usage.value_versions, 1, "value-version count")?;
        let total_inline = checked_add(usage.inline_bytes, inline_bytes, "inline bytes")?;
        enforce("value versions", self.max_value_versions, value_versions)?;
        enforce(
            "total inline bytes",
            self.max_total_inline_bytes,
            total_inline,
        )?;
        Ok(WorkspaceUsage {
            value_versions,
            inline_bytes: total_inline,
            ..*usage
        })
    }

    /// Computes usage after admitting one artifact metadata/content publication.
    pub fn admit_artifact(
        &self,
        usage: &WorkspaceUsage,
        artifact: &ArtifactMetadata,
    ) -> Result<WorkspaceUsage, WorkspaceError> {
        self.admit_artifact_reference(usage, artifact.reference())
    }

    /// Computes usage after this budget domain first references one already
    /// committed artifact. Repeated references to the same immutable content must
    /// be de-duplicated by the owning persistence transaction before this call.
    pub fn admit_artifact_reference(
        &self,
        usage: &WorkspaceUsage,
        artifact: &ArtifactReference,
    ) -> Result<WorkspaceUsage, WorkspaceError> {
        self.validate_usage(usage)?;
        let size = artifact.size_bytes();
        enforce("bytes per artifact", self.max_bytes_per_artifact, size)?;
        let artifacts = checked_add(usage.artifacts, 1, "artifact count")?;
        let artifact_bytes = checked_add(usage.artifact_bytes, size, "artifact bytes")?;
        enforce("artifacts", self.max_artifacts, artifacts)?;
        enforce(
            "total artifact bytes",
            self.max_total_artifact_bytes,
            artifact_bytes,
        )?;
        Ok(WorkspaceUsage {
            artifacts,
            artifact_bytes,
            ..*usage
        })
    }
}

/// Immutable accounted usage for a workspace budget domain.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct WorkspaceUsage {
    value_versions: u64,
    inline_bytes: u64,
    artifacts: u64,
    artifact_bytes: u64,
}

impl WorkspaceUsage {
    /// Empty usage for a newly created workspace.
    pub const EMPTY: Self = Self {
        value_versions: 0,
        inline_bytes: 0,
        artifacts: 0,
        artifact_bytes: 0,
    };

    /// Constructs projected usage from durable facts.
    #[must_use]
    pub const fn new(
        value_versions: u64,
        inline_bytes: u64,
        artifacts: u64,
        artifact_bytes: u64,
    ) -> Self {
        Self {
            value_versions,
            inline_bytes,
            artifacts,
            artifact_bytes,
        }
    }

    /// Returns the number of admitted value versions.
    #[must_use]
    pub const fn value_versions(self) -> u64 {
        self.value_versions
    }

    /// Returns aggregate encoded inline JSON bytes.
    #[must_use]
    pub const fn inline_bytes(self) -> u64 {
        self.inline_bytes
    }

    /// Returns the number of admitted artifact records.
    #[must_use]
    pub const fn artifacts(self) -> u64 {
        self.artifacts
    }

    /// Returns aggregate exact artifact bytes.
    #[must_use]
    pub const fn artifact_bytes(self) -> u64 {
        self.artifact_bytes
    }
}

fn enforce(resource: &'static str, limit: u64, attempted: u64) -> Result<(), WorkspaceError> {
    if attempted > limit {
        return Err(WorkspaceError::BudgetExceeded {
            resource,
            limit,
            attempted,
        });
    }
    Ok(())
}

fn checked_add(left: u64, right: u64, resource: &'static str) -> Result<u64, WorkspaceError> {
    left.checked_add(right)
        .ok_or(WorkspaceError::AccountingOverflow(resource))
}
