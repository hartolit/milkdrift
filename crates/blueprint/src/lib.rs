//! Define a workflow and publish validated, immutable revisions of what it should do.
//!
//! Workflow authors and importers build [`Node`]s, ports, and edges, then submit them in
//! a [`MutationBatch`] to [`BlueprintRevision::genesis`] or to a new revision of an
//! existing blueprint. Graph validation refuses incompatible bindings, missing targets,
//! and cycles before a revision is published. This crate defines the program; runtime
//! scheduling, persistence, capability execution, and authority have separate owners.
//!
//! A blueprint is a reusable declarative workflow or subworkflow package. A workflow
//! gives a top-level blueprint identity and revision lineage. A revision is one
//! immutable semantic snapshot. Runs and node executions are later runtime concepts;
//! mutable execution state is never stored on a [`Node`]. Layout is presentation state
//! and is deliberately absent from semantic identity.
//!
//! For an external task, [`TaskConfig`] pairs a capability requirement with a
//! [`TaskContextPolicy`]. The policy asks for inputs and earlier evidence; the runtime
//! later records its actual selection in a context manifest. Start with
//! [`TaskConfig::direct_inputs`] for direct inputs only, or follow the executable
//! example on [`TaskContextPolicy`] to request bounded ancestor evidence.
//!
//! The example defines a review task that reads a workflow input and then reaches a
//! success terminal. No capability needs to be running to author this revision.
//!
//! ```
//! use milkdrift_blueprint::{
//!     AuthorRef, BindingSource, BlueprintRevision, DataPort, Edge, EdgeId, EdgeKind,
//!     FieldId, InterfaceField, Mutation, MutationBatch, Node, NodeId, NodeKind, PortId,
//!     SchemaRef, TerminalOutcome, WorkflowId, WorkflowInterface,
//! };
//! use milkdrift_capability::{CapabilityRequirement, OperationId, SchemaId};
//!
//! let schema = SchemaRef::new(SchemaId::new("example.review_request")?, 1)?;
//! let interface = WorkflowInterface::new(
//!     [(FieldId::new("request")?, InterfaceField::required(schema.clone()))],
//!     [],
//! )?;
//! let review = Node::new(
//!     NodeId::new("review")?,
//!     NodeKind::task_direct_inputs(CapabilityRequirement::new(
//!         OperationId::new("process.execute")?,
//!     ))?,
//! )?
//! .with_data_input(PortId::new("request")?, DataPort::input(
//!     schema, true, Some(BindingSource::WorkflowInput { field: FieldId::new("request")? }),
//! )?)?
//! .with_control_output(PortId::new("out")?)?;
//! let done = Node::new(
//!     NodeId::new("done")?,
//!     NodeKind::Terminal { outcome: TerminalOutcome::Success },
//! )?.with_control_input(PortId::new("in")?)?;
//! let batch = MutationBatch::new(vec![
//!     Mutation::SetInterface { interface },
//!     Mutation::AddNode { node: review },
//!     Mutation::AddNode { node: done },
//!     Mutation::AddEdge { edge: Edge::new(
//!         EdgeId::new("review-done")?, EdgeKind::Control,
//!         NodeId::new("review")?, PortId::new("out")?,
//!         NodeId::new("done")?, PortId::new("in")?,
//!     ) },
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
mod context;
mod document;
mod identity;
mod model;
mod mutation;
mod revision;
mod validation;

pub use condition::{
    Comparison, Condition, ConditionError, ConditionOperand, PathSegment, PathSelector,
};
pub use context::{
    ContextArtifactRetention, ContextArtifactSelector, ContextArtifactSensitivity, ContextBudget,
    ContextCategory, ContextOrdering, ContextProvenanceClass, ContextSemanticRole,
    ContextSessionPolicy, ContextTruncation, TaskContextPolicy,
};
pub use document::{
    BlueprintRevisionDocument, DocumentError, node_configuration_fingerprint,
    node_dependency_fingerprint,
};
pub use identity::{
    AuthorRef, BlueprintId, ContentDigest, EdgeId, FieldId, IdentityError, MutationBatchId,
    NodeFingerprint, NodeId, PortId, RevisionId, WorkflowId,
};
pub use model::{
    BindingSource, BlueprintMetadata, BranchConfig, CostCurrencyCode, DataPort, Edge, EdgeKind,
    ForkConfig, InterfaceField, JoinConfig, JoinPolicy, ModelError, Node, NodeKind,
    PinnedSubworkflow, ReducerConfig, ReducerStrategy, RepeatBudget, RepeatConfig,
    RepeatTermination, SchemaRef, SemanticBlueprint, TaskConfig, TerminalOutcome,
    WorkflowInterface,
};
pub use mutation::{Mutation, MutationBatch, MutationError};
pub use revision::BlueprintRevision;
pub use validation::{Diagnostic, DiagnosticCode, ValidationError};

/// Portable blueprint document schema with explicit task context policy.
pub(crate) const BLUEPRINT_SCHEMA_VERSION_V2: u32 = 2;
