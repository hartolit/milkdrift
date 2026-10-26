//! Bounded accepted connections for rendering a canvas without fetching every task's content.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::tasks::{
    TaskCapability, TaskOrderConditionDto, TaskOutputDeclarationDto, TaskPublishedOutputDto,
    TaskSuppliedInputDto,
};

/// One accepted input edge and an optional authorized source preview.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[cfg_attr(feature = "web-codegen", derive(ts_rs::TS))]
#[serde(deny_unknown_fields)]
pub struct TopologyInputDto {
    /// Inline, supplied, task_output or settled_outcome.
    pub kind: String,
    /// Stable inline/source identity or accepted producer task identity.
    pub source: String,
    /// Named producer output for a data edge, absent for other kinds.
    pub output: Option<String>,
    /// Whether accepted absence satisfies this input.
    pub optional: bool,
    /// Current source value within the page preview allowance; none does not mean missing.
    pub preview: Option<TaskSuppliedInputDto>,
}

/// Accepted task ports and connections, without instructions or result content.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[cfg_attr(feature = "web-codegen", derive(ts_rs::TS))]
#[serde(deny_unknown_fields)]
pub struct TopologyTaskDto {
    /// Stable workflow-local task identity.
    pub id: String,
    /// Exact accepted definition version.
    #[cfg_attr(feature = "web-codegen", ts(type = "number"))]
    pub version: u64,
    /// Executor selected by the accepted definition.
    pub capability: TaskCapability,
    /// Named accepted input bindings, including supplied source identity.
    pub inputs: BTreeMap<String, TopologyInputDto>,
    /// Actual accepted output declarations.
    pub outputs: BTreeMap<String, TaskOutputDeclarationDto>,
    /// Retained named output metadata under current artifact authority; no result text.
    pub published_outputs: BTreeMap<String, TaskPublishedOutputDto>,
    /// Exact independent or retained historical order predicates.
    pub dependencies: BTreeMap<String, TaskOrderConditionDto>,
    /// Historical combined ordering needs explicit review before changing connection meaning.
    pub dependencies_legacy: bool,
}

/// One generation-bound page of accepted workflow topology.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[cfg_attr(feature = "web-codegen", derive(ts_rs::TS))]
#[serde(deny_unknown_fields)]
pub struct WorkflowTopologyDto {
    /// Workflow generation shared by every row and continuation in this page series.
    #[cfg_attr(feature = "web-codegen", ts(type = "number"))]
    pub workflow_generation: u64,
    /// At most 200 accepted tasks within the 2 MiB page bound.
    pub tasks: Vec<TopologyTaskDto>,
    /// Actor, authority, daemon, workflow and generation-bound continuation.
    pub next_cursor: Option<String>,
    /// True means more rows remain and the caller must continue the page series.
    pub truncated: bool,
}
