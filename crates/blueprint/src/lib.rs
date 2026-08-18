//! Immutable blueprint definitions and revision transactions.
//!
//! A blueprint is a reusable declarative workflow or subworkflow package. A workflow
//! gives a top-level blueprint identity and revision lineage. A revision is one
//! immutable semantic snapshot. Runs and node executions are later runtime concepts;
//! mutable execution state is never stored on a [`Node`]. Layout is presentation state
//! and is deliberately absent from semantic identity.
//!
//! ```
//! use milkdrift_blueprint::{
//!     AuthorRef, BlueprintRevision, FieldId, InterfaceField, Mutation, MutationBatch,
//!     Node, NodeId, NodeKind, PortId, SchemaRef, TerminalOutcome, WorkflowId,
//!     WorkflowInterface,
//! };
//! use milkdrift_capability::SchemaId;
//!
//! let schema = SchemaRef::new(SchemaId::new("milkdrift.unit")?, 1)?;
//! let interface = WorkflowInterface::new(
//!     [(FieldId::new("input")?, InterfaceField::required(schema))],
//!     [],
//! )?;
//! let node = Node::new(
//!     NodeId::new("done")?,
//!     NodeKind::Terminal { outcome: TerminalOutcome::Success },
//! )?;
//! let batch = MutationBatch::new(vec![
//!     Mutation::SetInterface { interface },
//!     Mutation::AddNode { node },
//! ])?;
//! let revision = BlueprintRevision::genesis(
//!     WorkflowId::new("example")?,
//!     batch,
//!     AuthorRef::new("human:example")?,
//!     "initial workflow",
//! )?;
//! assert_eq!(revision.sequence(), 1);
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```

mod condition;
mod document;
mod identity;
mod model;
mod mutation;
mod revision;
mod validation;

pub use condition::{
    Comparison, Condition, ConditionError, ConditionOperand, PathSegment, PathSelector,
};
pub use document::{BlueprintRevisionDocument, DocumentError, canonical_blueprint_json};
pub use identity::{
    AuthorRef, BlueprintId, ContentDigest, EdgeId, FieldId, IdentityError, MutationBatchId, NodeId,
    PortId, RevisionId, WorkflowId,
};
pub use model::{
    BindingSource, BlueprintMetadata, BranchConfig, DataPort, Edge, EdgeKind, ForkConfig,
    InterfaceField, JoinConfig, JoinPolicy, ModelError, Node, NodeKind, PinnedSubworkflow,
    ReducerConfig, ReducerStrategy, RepeatBudget, RepeatConfig, RepeatTermination, SchemaRef,
    SemanticBlueprint, TerminalOutcome, WorkflowInterface,
};
pub use mutation::{Mutation, MutationBatch, MutationError};
pub use revision::BlueprintRevision;
pub use validation::{Diagnostic, DiagnosticCode, ValidationError};

/// Portable blueprint document schema implemented by this crate.
pub const BLUEPRINT_SCHEMA_VERSION_V1: u32 = 1;
