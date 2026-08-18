use std::collections::BTreeSet;

use serde::{Deserialize, Deserializer, Serialize};

use crate::{
    BranchId, IterationId, RunId, ScopeId, SubworkflowId, WorkspaceError, WorkspaceValueReference,
};

/// Maximum supported nesting of structured workspace scopes.
pub const MAX_SCOPE_DEPTH: usize = 64;

/// Durable reference to one exact scope in one run.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ScopeReference {
    run: RunId,
    scope: ScopeId,
}

impl ScopeReference {
    /// Constructs a scope reference from already validated identities.
    #[must_use]
    pub const fn new(run: RunId, scope: ScopeId) -> Self {
        Self { run, scope }
    }

    /// Returns the owning run.
    #[must_use]
    pub const fn run(&self) -> &RunId {
        &self.run
    }

    /// Returns the scope identity within the run.
    #[must_use]
    pub const fn scope(&self) -> &ScopeId {
        &self.scope
    }
}

/// Structured semantic kind of a workspace scope.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Deserialize, Serialize)]
#[serde(rename_all = "snake_case", tag = "type", deny_unknown_fields)]
pub enum ScopeKind {
    /// The single root scope of a run. It has no parent.
    RunRoot,
    /// A branch-local scope created by a structured fork.
    Branch {
        /// Stable branch identity from the fork decision.
        branch: BranchId,
    },
    /// An isolated scope for one explicit repeat iteration.
    Iteration {
        /// Stable repeat-iteration identity.
        iteration: IterationId,
    },
    /// An isolated scope owned by a pinned child-subworkflow execution.
    Subworkflow {
        /// Stable child execution identity.
        subworkflow: SubworkflowId,
    },
}

impl ScopeKind {
    /// Returns whether this is a run-root scope.
    #[must_use]
    pub const fn is_run_root(&self) -> bool {
        matches!(self, Self::RunRoot)
    }
}

/// Immutable declaration of a durable workspace scope and its direct parent.
///
/// A run root has no parent. Every branch, iteration, and subworkflow scope has
/// exactly one parent in the same run. The parent reference is a durable fact;
/// storage implementations must not reconstruct it from naming conventions.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct WorkspaceScope {
    reference: ScopeReference,
    kind: ScopeKind,
    parent: Option<ScopeReference>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WorkspaceScopeWire {
    reference: ScopeReference,
    kind: ScopeKind,
    parent: Option<ScopeReference>,
}

impl<'de> Deserialize<'de> for WorkspaceScope {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire = WorkspaceScopeWire::deserialize(deserializer)?;
        Self::new(wire.reference, wire.kind, wire.parent).map_err(serde::de::Error::custom)
    }
}

impl WorkspaceScope {
    /// Creates the only parentless scope kind for a run.
    #[must_use]
    pub const fn run_root(run: RunId, scope: ScopeId) -> Self {
        Self {
            reference: ScopeReference::new(run, scope),
            kind: ScopeKind::RunRoot,
            parent: None,
        }
    }

    /// Creates a branch-local child scope.
    pub fn branch(scope: ScopeId, parent: &Self, branch: BranchId) -> Result<Self, WorkspaceError> {
        Self::child(scope, parent, ScopeKind::Branch { branch })
    }

    /// Creates an isolated repeat-iteration child scope.
    pub fn iteration(
        scope: ScopeId,
        parent: &Self,
        iteration: IterationId,
    ) -> Result<Self, WorkspaceError> {
        Self::child(scope, parent, ScopeKind::Iteration { iteration })
    }

    /// Creates an isolated child-subworkflow scope.
    pub fn subworkflow(
        scope: ScopeId,
        parent: &Self,
        subworkflow: SubworkflowId,
    ) -> Result<Self, WorkspaceError> {
        Self::child(scope, parent, ScopeKind::Subworkflow { subworkflow })
    }

    fn child(scope: ScopeId, parent: &Self, kind: ScopeKind) -> Result<Self, WorkspaceError> {
        let reference = ScopeReference::new(parent.reference.run().clone(), scope);
        Self::new(reference, kind, Some(parent.reference.clone()))
    }

    fn new(
        reference: ScopeReference,
        kind: ScopeKind,
        parent: Option<ScopeReference>,
    ) -> Result<Self, WorkspaceError> {
        match (&kind, &parent) {
            (ScopeKind::RunRoot, None) => {}
            (ScopeKind::RunRoot, Some(_)) => {
                return Err(WorkspaceError::InvalidScope(
                    "a run-root scope must not have a parent".to_owned(),
                ));
            }
            (_, None) => {
                return Err(WorkspaceError::InvalidScope(
                    "a branch, iteration, or subworkflow scope requires a parent".to_owned(),
                ));
            }
            (_, Some(parent)) if parent.run() != reference.run() => {
                return Err(WorkspaceError::InvalidScope(
                    "a child scope and its parent must belong to the same run".to_owned(),
                ));
            }
            (_, Some(parent)) if parent == &reference => {
                return Err(WorkspaceError::InvalidScope(
                    "a scope cannot be its own parent".to_owned(),
                ));
            }
            (_, Some(_)) => {}
        }
        Ok(Self {
            reference,
            kind,
            parent,
        })
    }

    /// Returns this scope's durable reference.
    #[must_use]
    pub const fn reference(&self) -> &ScopeReference {
        &self.reference
    }

    /// Returns this scope's structured kind.
    #[must_use]
    pub const fn kind(&self) -> &ScopeKind {
        &self.kind
    }

    /// Returns the exact direct parent, or `None` for the run root.
    #[must_use]
    pub const fn parent(&self) -> Option<&ScopeReference> {
        self.parent.as_ref()
    }

    /// Returns whether this scope is a direct child of `candidate`.
    #[must_use]
    pub fn is_direct_child_of(&self, candidate: &Self) -> bool {
        self.parent.as_ref() == Some(candidate.reference())
    }
}

/// Validated root-to-leaf ancestry used for branch-local visibility decisions.
///
/// Exact immutable values in any ancestor scope are readable by the leaf. Only
/// the leaf owns new versions of its local streams. A sibling scope is absent
/// from the lineage and is therefore neither readable nor writable.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ScopeLineage {
    scopes: Vec<WorkspaceScope>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ScopeLineageWire {
    scopes: Vec<WorkspaceScope>,
}

impl<'de> Deserialize<'de> for ScopeLineage {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Self::new(ScopeLineageWire::deserialize(deserializer)?.scopes)
            .map_err(serde::de::Error::custom)
    }
}

impl ScopeLineage {
    /// Validates a complete root-to-leaf scope chain.
    pub fn new(scopes: Vec<WorkspaceScope>) -> Result<Self, WorkspaceError> {
        if scopes.is_empty() {
            return Err(WorkspaceError::InvalidScope(
                "a scope lineage must contain a run root".to_owned(),
            ));
        }
        if scopes.len() > MAX_SCOPE_DEPTH {
            return Err(WorkspaceError::InvalidScope(format!(
                "a scope lineage may contain at most {MAX_SCOPE_DEPTH} scopes"
            )));
        }
        if !scopes[0].kind().is_run_root() {
            return Err(WorkspaceError::InvalidScope(
                "the first lineage entry must be a run root".to_owned(),
            ));
        }

        let mut seen = BTreeSet::new();
        for (index, scope) in scopes.iter().enumerate() {
            if !seen.insert(scope.reference().clone()) {
                return Err(WorkspaceError::InvalidScope(
                    "a scope lineage cannot contain duplicate scopes".to_owned(),
                ));
            }
            if index > 0 && scope.parent() != Some(scopes[index - 1].reference()) {
                return Err(WorkspaceError::InvalidScope(format!(
                    "scope at lineage index {index} does not name the preceding scope as its parent"
                )));
            }
        }
        Ok(Self { scopes })
    }

    /// Returns the ordered root-to-leaf scopes.
    #[must_use]
    pub fn scopes(&self) -> &[WorkspaceScope] {
        &self.scopes
    }

    /// Returns the run-root scope.
    #[must_use]
    pub fn root(&self) -> &WorkspaceScope {
        &self.scopes[0]
    }

    /// Returns the active leaf scope.
    #[must_use]
    pub fn leaf(&self) -> &WorkspaceScope {
        &self.scopes[self.scopes.len() - 1]
    }

    /// Returns whether an exact scope is this leaf or one of its ancestors.
    #[must_use]
    pub fn can_read_from(&self, source: &ScopeReference) -> bool {
        self.scopes.iter().any(|scope| scope.reference() == source)
    }

    /// Returns whether the leaf may read the exact immutable value reference.
    #[must_use]
    pub fn can_read(&self, value: &WorkspaceValueReference) -> bool {
        self.can_read_from(value.scope())
    }

    /// Returns whether the value stream belongs to the leaf scope.
    #[must_use]
    pub fn owns_value_stream(&self, value: &WorkspaceValueReference) -> bool {
        self.leaf().reference() == value.scope()
    }
}
