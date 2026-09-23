use super::MAX_DIAGNOSTICS;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use thiserror::Error;

/// Stable machine-readable blueprint invariant code.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DiagnosticCode {
    /// A map key and embedded identity disagree or a mutation duplicates an identity.
    DuplicateIdentity,
    /// A node, port, edge endpoint, or binding target does not exist.
    DanglingReference,
    /// A data source and target have incompatible declared schemas.
    SchemaMismatch,
    /// An explicit graph cycle was found.
    IllegalCycle,
    /// Semantic work cannot be reached from the unique entry.
    UnreachableNode,
    /// Entry or terminal topology is invalid.
    InvalidTopology,
    /// Control flow is ambiguous for the node kind.
    AmbiguousControlFlow,
    /// A fork/join relationship is not structured or has wrong ownership.
    InvalidForkJoin,
    /// Join quorum cannot be satisfied.
    ImpossibleQuorum,
    /// Reducer input shape/count does not match its configuration.
    ReducerMismatch,
    /// Repeat configuration is missing an effective hard bound.
    UnboundedRepeat,
    /// A required input has neither a binding nor an incoming data edge.
    MissingInput,
    /// A task is missing or contradicts its capability requirement.
    MissingCapabilityRequirement,
    /// A pinned subworkflow node does not match its recorded interface.
    IncompatibleSubworkflow,
    /// A supported deterministic document bound was exceeded.
    BoundsExceeded,
    /// A local node/model constructor invariant was violated.
    InvalidNodeConfiguration,
    /// A mutation conflicts with the selected immutable base.
    RevisionConflict,
    /// A serialized digest or revision identity contradicted its derived value.
    IntegrityMismatch,
    /// A schema version is unsupported.
    UnsupportedVersion,
}

/// Locates a definition error; branch on [`Self::code`] and display its message and location.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct Diagnostic {
    code: DiagnosticCode,
    location: String,
    message: String,
    operation_index: Option<usize>,
    context: BTreeMap<String, String>,
}

impl Diagnostic {
    pub(crate) fn new(
        code: DiagnosticCode,
        location: impl Into<String>,
        message: impl Into<String>,
    ) -> Self {
        Self {
            code,
            location: bound_text(location.into(), 512),
            message: bound_text(message.into(), 1_024),
            operation_index: None,
            context: BTreeMap::new(),
        }
    }

    pub(crate) fn operation(mut self, index: usize) -> Self {
        self.operation_index = Some(index);
        self
    }

    pub(crate) fn with_context(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        if self.context.len() < 16 {
            self.context
                .insert(bound_text(key.into(), 96), bound_text(value.into(), 256));
        }
        self
    }

    /// Stable invariant code.
    #[must_use]
    pub const fn code(&self) -> DiagnosticCode {
        self.code
    }

    /// JSON-like semantic location or identity.
    #[must_use]
    pub fn location(&self) -> &str {
        &self.location
    }

    /// Bounded human-readable summary; clients must branch on [`Self::code`].
    #[must_use]
    pub fn message(&self) -> &str {
        &self.message
    }

    /// Zero-based batch index; final graph validation may attach the last operation, not a unique cause.
    #[must_use]
    pub const fn operation_index(&self) -> Option<usize> {
        self.operation_index
    }

    /// Bounded stable structured context.
    #[must_use]
    pub const fn context(&self) -> &BTreeMap<String, String> {
        &self.context
    }
}

fn bound_text(mut value: String, limit: usize) -> String {
    let boundary = milkdrift_contracts::truncate_utf8(&value, limit).len();
    value.truncate(boundary);
    value
}

#[cfg(test)]
mod tests {
    use super::bound_text;

    #[test]
    fn unicode_diagnostic_truncation_uses_a_character_boundary() {
        assert_eq!(bound_text("abcdéf".to_owned(), 5), "abcd");
        assert_eq!(bound_text("ééé".to_owned(), 5), "éé");
    }
}

/// Up to 256 candidate-graph errors; inspect [`Self::diagnostics`] and resubmit a repaired batch.
#[derive(Clone, Debug, Error, PartialEq)]
#[error("blueprint validation failed with {} diagnostic(s)", .diagnostics.len())]
pub struct ValidationError {
    diagnostics: Vec<Diagnostic>,
}

impl ValidationError {
    pub(crate) fn new(mut diagnostics: Vec<Diagnostic>) -> Self {
        diagnostics.truncate(MAX_DIAGNOSTICS);
        Self { diagnostics }
    }

    /// Machine-readable diagnostics in deterministic discovery order.
    #[must_use]
    pub fn diagnostics(&self) -> &[Diagnostic] {
        &self.diagnostics
    }

    pub(crate) fn with_default_operation(mut self, index: usize) -> Self {
        for diagnostic in &mut self.diagnostics {
            if diagnostic.operation_index.is_none() {
                diagnostic.operation_index = Some(index);
            }
        }
        self
    }
}
