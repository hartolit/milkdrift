use std::error::Error;
use std::fmt;

use super::policy::{DependencyKind, Layer};

/// One architecture policy violation.
#[derive(Debug, PartialEq, Eq)]
pub struct Violation {
    source: String,
    target: String,
    dependency_kind: Option<DependencyKind>,
    source_layer: Option<Layer>,
    target_layer: Option<Layer>,
    rule: &'static str,
    reason: String,
}

impl Violation {
    pub(super) fn new(
        source: String,
        target: String,
        dependency_kind: Option<DependencyKind>,
        source_layer: Option<Layer>,
        target_layer: Option<Layer>,
        rule: &'static str,
        reason: String,
    ) -> Self {
        Self {
            source,
            target,
            dependency_kind,
            source_layer,
            target_layer,
            rule,
            reason,
        }
    }

    /// Returns the stable identifier of the policy rule that was violated.
    #[must_use]
    pub const fn rule(&self) -> &'static str {
        self.rule
    }

    /// Returns the human-readable reason the policy rejected the item.
    #[must_use]
    pub fn reason(&self) -> &str {
        &self.reason
    }

    /// Returns the source package or manifest description.
    #[must_use]
    pub fn source(&self) -> &str {
        &self.source
    }

    /// Returns the dependency package or location description.
    #[must_use]
    pub fn target(&self) -> &str {
        &self.target
    }

    /// Returns the dependency kind when the violation represents an edge.
    #[must_use]
    pub const fn dependency_kind(&self) -> Option<DependencyKind> {
        self.dependency_kind
    }
}

impl fmt::Display for Violation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.dependency_kind {
            Some(kind) => write!(
                formatter,
                "forbidden architecture dependency: {} ({:?}) --{}--> {} ({:?}); policy rule {}: {}",
                self.source,
                self.source_layer,
                kind,
                self.target,
                self.target_layer,
                self.rule,
                self.reason
            ),
            None => write!(
                formatter,
                "architecture policy violation: {} -> {}; policy rule {}: {}",
                self.source, self.target, self.rule, self.reason
            ),
        }
    }
}

/// The complete result of validating one Cargo workspace.
#[derive(Debug, Default)]
pub struct ValidationReport {
    violations: Vec<Violation>,
}

impl ValidationReport {
    pub(super) fn push(&mut self, violation: Violation) {
        self.violations.push(violation);
    }

    /// Returns true when the workspace satisfies every architecture rule.
    #[must_use]
    pub const fn is_valid(&self) -> bool {
        self.violations.is_empty()
    }

    /// Returns all violations discovered in the workspace.
    #[must_use]
    pub fn violations(&self) -> &[Violation] {
        &self.violations
    }
}

/// An error that prevented Cargo metadata from being loaded.
#[derive(Debug)]
pub struct ValidationError(cargo_metadata::Error);

impl fmt::Display for ValidationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "could not load locked Cargo metadata: {}",
            self.0
        )
    }
}

impl Error for ValidationError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        Some(&self.0)
    }
}

impl From<cargo_metadata::Error> for ValidationError {
    fn from(error: cargo_metadata::Error) -> Self {
        Self(error)
    }
}
