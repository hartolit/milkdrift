use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::{
    BLUEPRINT_SCHEMA_VERSION_V1, BlueprintMetadata, Diagnostic, DiagnosticCode, Edge, EdgeId,
    MutationBatchId, Node, NodeId, NodeKind, PinnedSubworkflow, RevisionId, SemanticBlueprint,
    ValidationError, WorkflowInterface, validation::validate_semantic,
};

const MAX_BATCH_OPERATIONS: usize = 512;
const MAX_MERGE_PARENTS: usize = 16;

/// Closed schema-v1 command set for every semantic blueprint edit.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case", tag = "type", deny_unknown_fields)]
pub enum Mutation {
    /// Insert a new node identity.
    AddNode {
        /// Complete validated local node value.
        node: Node,
    },
    /// Remove an existing node; incident edges must be removed in the same batch.
    RemoveNode {
        /// Existing node identity.
        node: NodeId,
    },
    /// Replace a node, including its configuration and ports, atomically.
    ReplaceNode {
        /// Complete atomic replacement.
        node: Node,
    },
    /// Insert a new edge identity.
    AddEdge {
        /// Complete edge value.
        edge: Edge,
    },
    /// Remove an existing edge.
    RemoveEdge {
        /// Existing edge identity.
        edge: EdgeId,
    },
    /// Replace an edge atomically.
    ReplaceEdge {
        /// Complete atomic replacement.
        edge: Edge,
    },
    /// Instantiate an exact pinned subworkflow as a new node.
    InstantiateSubworkflow {
        /// New node whose kind is a pinned subworkflow.
        node: Node,
    },
    /// Upgrade one subworkflow node after checking its current pinned revision.
    UpgradeSubworkflow {
        /// Node being upgraded.
        node: NodeId,
        /// Revision the caller believes is currently pinned.
        expected_revision: RevisionId,
        /// Exact replacement target and interface.
        replacement: PinnedSubworkflow,
    },
    /// Replace the declared workflow interface.
    SetInterface {
        /// Complete replacement interface.
        interface: WorkflowInterface,
    },
    /// Replace all bounded blueprint metadata.
    SetMetadata {
        /// Complete replacement metadata.
        metadata: BlueprintMetadata,
    },
    /// Explicitly select multiple parents for a deliberate merge revision.
    SetMergeParents {
        /// Distinct explicit parent identities, including the exact base.
        parents: Vec<RevisionId>,
    },
}

/// Versioned atomic list of semantic mutations.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct MutationBatch {
    schema_version: u32,
    id: MutationBatchId,
    operations: Vec<Mutation>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MutationBatchWire {
    schema_version: u32,
    id: MutationBatchId,
    operations: Vec<Mutation>,
}

impl<'de> Deserialize<'de> for MutationBatch {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let wire = MutationBatchWire::deserialize(deserializer)?;
        if wire.schema_version != BLUEPRINT_SCHEMA_VERSION_V1 {
            return Err(serde::de::Error::custom(format!(
                "unsupported mutation schema version {}; supported version is {}",
                wire.schema_version, BLUEPRINT_SCHEMA_VERSION_V1
            )));
        }
        Self::from_parts(wire.id, wire.operations).map_err(serde::de::Error::custom)
    }
}

impl MutationBatch {
    /// Creates a deterministic batch identity from its canonical operation content.
    pub fn new(operations: Vec<Mutation>) -> Result<Self, MutationError> {
        let bytes = crate::document::canonical_value_bytes(&operations)
            .map_err(|error| MutationError::Serialization(error.to_string()))?;
        let id = MutationBatchId::from_hash(blake3::hash(&bytes));
        Self::from_parts(id, operations)
    }

    fn from_parts(id: MutationBatchId, operations: Vec<Mutation>) -> Result<Self, MutationError> {
        if operations.is_empty() || operations.len() > MAX_BATCH_OPERATIONS {
            return Err(MutationError::InvalidBatch(format!(
                "operation count must be between 1 and {MAX_BATCH_OPERATIONS}"
            )));
        }
        let batch = Self {
            schema_version: BLUEPRINT_SCHEMA_VERSION_V1,
            id,
            operations,
        };
        let expected_bytes = crate::document::canonical_value_bytes(&batch.operations)
            .map_err(|error| MutationError::Serialization(error.to_string()))?;
        let expected = MutationBatchId::from_hash(blake3::hash(&expected_bytes));
        if batch.id != expected {
            return Err(MutationError::InvalidBatch(
                "batch identity does not match canonical operations".to_owned(),
            ));
        }
        Ok(batch)
    }

    /// Deterministic batch identity.
    #[must_use]
    pub const fn id(&self) -> &MutationBatchId {
        &self.id
    }

    /// Ordered operations applied transactionally.
    #[must_use]
    pub fn operations(&self) -> &[Mutation] {
        &self.operations
    }
}

/// Failure returned without changing the immutable base revision.
#[derive(Clone, Debug, Error, PartialEq)]
pub enum MutationError {
    /// Optimistic base revision did not match.
    #[error("base revision conflict: expected {expected}, actual {actual}")]
    BaseRevisionConflict {
        /// Revision supplied by the caller.
        expected: RevisionId,
        /// Revision on which the transaction was invoked.
        actual: RevisionId,
    },
    /// One operation could not be applied to the private candidate.
    #[error("mutation operation failed: {0:?}")]
    Operation(Diagnostic),
    /// The complete candidate failed semantic validation.
    #[error(transparent)]
    Validation(#[from] ValidationError),
    /// The versioned batch itself was malformed.
    #[error("invalid mutation batch: {0}")]
    InvalidBatch(String),
    /// Canonical serialization failed.
    #[error("mutation serialization failed: {0}")]
    Serialization(String),
    /// Revision author/reason or ancestry was invalid.
    #[error("invalid revision metadata: {0}")]
    InvalidRevision(String),
}

pub(crate) struct AppliedCandidate {
    pub(crate) semantic: SemanticBlueprint,
    pub(crate) merge_parents: Option<Vec<RevisionId>>,
}

pub(crate) fn apply_batch(
    base: &SemanticBlueprint,
    batch: &MutationBatch,
) -> Result<AppliedCandidate, MutationError> {
    let mut semantic = base.clone();
    let mut merge_parents = None;
    for (index, operation) in batch.operations.iter().enumerate() {
        apply_operation(&mut semantic, &mut merge_parents, operation)
            .map_err(|diagnostic| MutationError::Operation(diagnostic.operation(index)))?;
    }
    validate_semantic(&semantic).map_err(|error| {
        MutationError::Validation(error.with_default_operation(batch.operations.len() - 1))
    })?;
    Ok(AppliedCandidate {
        semantic,
        merge_parents,
    })
}

fn apply_operation(
    semantic: &mut SemanticBlueprint,
    merge_parents: &mut Option<Vec<RevisionId>>,
    operation: &Mutation,
) -> Result<(), Diagnostic> {
    match operation {
        Mutation::AddNode { node } => {
            if semantic.nodes().contains_key(node.id()) {
                return Err(duplicate("nodes", node.id().to_string()));
            }
            semantic.nodes_mut().insert(node.id().clone(), node.clone());
        }
        Mutation::RemoveNode { node } => {
            if semantic.nodes_mut().remove(node).is_none() {
                return Err(missing("nodes", node.to_string()));
            }
        }
        Mutation::ReplaceNode { node } => {
            if semantic.replace_node(node.clone()).is_none() {
                semantic.nodes_mut().remove(node.id());
                return Err(missing("nodes", node.id().to_string()));
            }
        }
        Mutation::AddEdge { edge } => {
            if semantic.edges().contains_key(edge.id()) {
                return Err(duplicate("edges", edge.id().to_string()));
            }
            semantic.edges_mut().insert(edge.id().clone(), edge.clone());
        }
        Mutation::RemoveEdge { edge } => {
            if semantic.edges_mut().remove(edge).is_none() {
                return Err(missing("edges", edge.to_string()));
            }
        }
        Mutation::ReplaceEdge { edge } => {
            if semantic
                .edges_mut()
                .insert(edge.id().clone(), edge.clone())
                .is_none()
            {
                semantic.edges_mut().remove(edge.id());
                return Err(missing("edges", edge.id().to_string()));
            }
        }
        Mutation::InstantiateSubworkflow { node } => {
            if !matches!(node.kind(), NodeKind::Subworkflow { .. }) {
                return Err(Diagnostic::new(
                    DiagnosticCode::InvalidNodeConfiguration,
                    format!("nodes.{}", node.id()),
                    "instantiation command requires a subworkflow node",
                ));
            }
            if semantic.nodes().contains_key(node.id()) {
                return Err(duplicate("nodes", node.id().to_string()));
            }
            semantic.nodes_mut().insert(node.id().clone(), node.clone());
        }
        Mutation::UpgradeSubworkflow {
            node,
            expected_revision,
            replacement,
        } => {
            let Some(existing) = semantic.nodes_mut().get_mut(node) else {
                return Err(missing("nodes", node.to_string()));
            };
            let NodeKind::Subworkflow { reference } = existing.kind() else {
                return Err(Diagnostic::new(
                    DiagnosticCode::InvalidNodeConfiguration,
                    format!("nodes.{node}"),
                    "upgrade target is not a subworkflow node",
                ));
            };
            if reference.revision() != expected_revision {
                return Err(Diagnostic::new(
                    DiagnosticCode::RevisionConflict,
                    format!("nodes.{node}.subworkflow.revision"),
                    "pinned subworkflow revision changed",
                )
                .with_context("expected", expected_revision.to_string())
                .with_context("actual", reference.revision().to_string()));
            }
            existing
                .replace_kind(NodeKind::Subworkflow {
                    reference: replacement.clone(),
                })
                .map_err(|error| {
                    Diagnostic::new(
                        DiagnosticCode::InvalidNodeConfiguration,
                        format!("nodes.{node}"),
                        error.to_string(),
                    )
                })?;
        }
        Mutation::SetInterface { interface } => semantic.set_interface(interface.clone()),
        Mutation::SetMetadata { metadata } => semantic.set_metadata(metadata.clone()),
        Mutation::SetMergeParents { parents } => {
            let unique: BTreeSet<_> = parents.iter().collect();
            if !(2..=MAX_MERGE_PARENTS).contains(&parents.len()) || unique.len() != parents.len() {
                return Err(Diagnostic::new(
                    DiagnosticCode::RevisionConflict,
                    "revision.parents",
                    format!("a merge needs 2..={MAX_MERGE_PARENTS} distinct explicit parents"),
                ));
            }
            *merge_parents = Some(parents.clone());
        }
    }
    Ok(())
}

fn duplicate(collection: &str, identity: String) -> Diagnostic {
    Diagnostic::new(
        DiagnosticCode::DuplicateIdentity,
        format!("{collection}.{identity}"),
        "identity already exists",
    )
}

fn missing(collection: &str, identity: String) -> Diagnostic {
    Diagnostic::new(
        DiagnosticCode::DanglingReference,
        format!("{collection}.{identity}"),
        "identity does not exist",
    )
}
