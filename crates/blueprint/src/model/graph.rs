use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::{BlueprintId, EdgeId, NodeId, PortId, WorkflowId};

use super::{BlueprintMetadata, ModelError, Node, WorkflowInterface};

/// Relationship between declared source and target ports.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EdgeKind {
    /// Scheduling/control dependency.
    Control,
    /// Typed value dependency.
    Data,
}

/// Explicit graph edge.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Edge {
    id: EdgeId,
    kind: EdgeKind,
    source_node: NodeId,
    source_port: PortId,
    target_node: NodeId,
    target_port: PortId,
}

impl Edge {
    /// Constructs an explicit port-to-port edge.
    #[must_use]
    pub const fn new(
        id: EdgeId,
        kind: EdgeKind,
        source_node: NodeId,
        source_port: PortId,
        target_node: NodeId,
        target_port: PortId,
    ) -> Self {
        Self {
            id,
            kind,
            source_node,
            source_port,
            target_node,
            target_port,
        }
    }

    /// Edge identity.
    #[must_use]
    pub const fn id(&self) -> &EdgeId {
        &self.id
    }

    /// Whether this is a control or typed data dependency.
    #[must_use]
    pub const fn kind(&self) -> EdgeKind {
        self.kind
    }

    /// Source node identity.
    #[must_use]
    pub const fn source_node(&self) -> &NodeId {
        &self.source_node
    }

    /// Declared source port.
    #[must_use]
    pub const fn source_port(&self) -> &PortId {
        &self.source_port
    }

    /// Target node identity.
    #[must_use]
    pub const fn target_node(&self) -> &NodeId {
        &self.target_node
    }

    /// Declared target port.
    #[must_use]
    pub const fn target_port(&self) -> &PortId {
        &self.target_port
    }
}

/// Validated semantic content from which revision identity is derived.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct SemanticBlueprint {
    workflow: WorkflowId,
    blueprint: BlueprintId,
    metadata: BlueprintMetadata,
    interface: WorkflowInterface,
    nodes: BTreeMap<NodeId, Node>,
    edges: BTreeMap<EdgeId, Edge>,
}

impl SemanticBlueprint {
    pub(crate) fn empty(workflow: WorkflowId) -> Result<Self, ModelError> {
        let blueprint = BlueprintId::new(workflow.as_str())
            .map_err(|error| ModelError::new("blueprint.identity", error.to_string()))?;
        let metadata = BlueprintMetadata::default_for(&workflow)?;
        Ok(Self {
            workflow,
            blueprint,
            metadata,
            interface: WorkflowInterface::new([], [])?,
            nodes: BTreeMap::new(),
            edges: BTreeMap::new(),
        })
    }

    pub(crate) fn from_parts(
        workflow: WorkflowId,
        blueprint: BlueprintId,
        metadata: BlueprintMetadata,
        interface: WorkflowInterface,
        nodes: BTreeMap<NodeId, Node>,
        edges: BTreeMap<EdgeId, Edge>,
    ) -> Self {
        Self {
            workflow,
            blueprint,
            metadata,
            interface,
            nodes,
            edges,
        }
    }

    /// Workflow identity owning the revision lineage.
    #[must_use]
    pub const fn workflow(&self) -> &WorkflowId {
        &self.workflow
    }

    /// Stable reusable package identity.
    #[must_use]
    pub const fn blueprint(&self) -> &BlueprintId {
        &self.blueprint
    }

    /// Bounded package metadata.
    #[must_use]
    pub const fn metadata(&self) -> &BlueprintMetadata {
        &self.metadata
    }

    /// Declared workflow interface.
    #[must_use]
    pub const fn interface(&self) -> &WorkflowInterface {
        &self.interface
    }

    /// Nodes in deterministic identity order.
    #[must_use]
    pub const fn nodes(&self) -> &BTreeMap<NodeId, Node> {
        &self.nodes
    }

    /// Edges in deterministic identity order.
    #[must_use]
    pub const fn edges(&self) -> &BTreeMap<EdgeId, Edge> {
        &self.edges
    }

    pub(crate) fn nodes_mut(&mut self) -> &mut BTreeMap<NodeId, Node> {
        &mut self.nodes
    }

    pub(crate) fn edges_mut(&mut self) -> &mut BTreeMap<EdgeId, Edge> {
        &mut self.edges
    }

    pub(crate) fn set_interface(&mut self, interface: WorkflowInterface) {
        self.interface = interface;
    }

    pub(crate) fn set_metadata(&mut self, metadata: BlueprintMetadata) {
        self.metadata = metadata;
    }

    pub(crate) fn replace_node(&mut self, node: Node) -> Option<Node> {
        self.nodes.insert(node.id().clone(), node)
    }
}
