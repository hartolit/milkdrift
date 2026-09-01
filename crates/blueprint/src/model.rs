use thiserror::Error;

mod contract;
mod graph;
mod node;
mod structured;

pub use contract::{
    BindingSource, BlueprintMetadata, DataPort, InterfaceField, SchemaRef, WorkflowInterface,
};
pub use graph::{Edge, EdgeKind, SemanticBlueprint};
pub use node::{Node, NodeKind, TaskConfig, TerminalOutcome};
pub use structured::{
    BranchConfig, CostCurrencyCode, ForkConfig, JoinConfig, JoinPolicy, PinnedSubworkflow,
    ReducerConfig, ReducerStrategy, RepeatBudget, RepeatConfig, RepeatTermination,
};

use contract::PortDirection;

pub(crate) const MAX_NODES: usize = 1_024;
pub(crate) const MAX_EDGES: usize = 4_096;

/// Error returned when constructing a locally invalid semantic component.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
#[error("invalid blueprint model at {location}: {reason}")]
pub struct ModelError {
    location: String,
    reason: String,
}

impl ModelError {
    pub(crate) fn new(location: impl Into<String>, reason: impl Into<String>) -> Self {
        Self {
            location: location.into(),
            reason: reason.into(),
        }
    }

    pub(crate) fn location(&self) -> &str {
        &self.location
    }
}
