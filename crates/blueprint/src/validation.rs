use std::collections::{BTreeMap, BTreeSet, VecDeque};

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::{
    BindingSource, EdgeKind, JoinPolicy, Node, NodeId, NodeKind, SemanticBlueprint,
    model::{MAX_EDGES, MAX_NODES},
};

const MAX_DIAGNOSTICS: usize = 256;

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
    /// A serialized digest or derived revision identity was not authentic.
    IntegrityMismatch,
    /// A schema version is unsupported.
    UnsupportedVersion,
}

/// Bounded structured diagnostic for a GUI, CLI, or controller.
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

    /// Mutation operation associated with this error when known.
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
    if value.len() <= limit {
        return value;
    }
    while !value.is_char_boundary(limit) {
        value.remove(limit);
    }
    value.truncate(limit);
    value
}

/// One or more independent semantic validation failures.
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

pub(crate) fn validate_semantic(semantic: &SemanticBlueprint) -> Result<(), ValidationError> {
    let mut diagnostics = Vec::new();
    validate_bounds_and_local(semantic, &mut diagnostics);
    validate_edges_and_bindings(semantic, &mut diagnostics);
    validate_acyclic(semantic, &mut diagnostics);
    validate_control_topology(semantic, &mut diagnostics);
    validate_fork_join(semantic, &mut diagnostics);
    validate_reducers_and_subworkflows(semantic, &mut diagnostics);
    if diagnostics.is_empty() {
        Ok(())
    } else {
        Err(ValidationError::new(diagnostics))
    }
}

fn push(diagnostics: &mut Vec<Diagnostic>, diagnostic: Diagnostic) {
    if diagnostics.len() < MAX_DIAGNOSTICS {
        diagnostics.push(diagnostic);
    }
}

fn validate_bounds_and_local(semantic: &SemanticBlueprint, diagnostics: &mut Vec<Diagnostic>) {
    if semantic.nodes().is_empty() || semantic.nodes().len() > MAX_NODES {
        push(
            diagnostics,
            Diagnostic::new(
                DiagnosticCode::BoundsExceeded,
                "nodes",
                format!("node count must be between 1 and {MAX_NODES}"),
            ),
        );
    }
    if semantic.edges().len() > MAX_EDGES {
        push(
            diagnostics,
            Diagnostic::new(
                DiagnosticCode::BoundsExceeded,
                "edges",
                format!("edge count must not exceed {MAX_EDGES}"),
            ),
        );
    }
    for (identity, node) in semantic.nodes() {
        if identity != node.id() {
            push(
                diagnostics,
                Diagnostic::new(
                    DiagnosticCode::DuplicateIdentity,
                    format!("nodes.{identity}"),
                    "node map key does not equal embedded identity",
                ),
            );
        }
        if let Err(error) = node.validate_local() {
            push(
                diagnostics,
                Diagnostic::new(
                    DiagnosticCode::InvalidNodeConfiguration,
                    format!("nodes.{identity}.{}", error.location()),
                    error.to_string(),
                ),
            );
        }
        let input_overlap = node
            .control_inputs()
            .iter()
            .find(|port| node.data_inputs().contains_key(*port));
        let output_overlap = node
            .control_outputs()
            .iter()
            .find(|port| node.data_outputs().contains_key(*port));
        if input_overlap.is_some() || output_overlap.is_some() {
            push(
                diagnostics,
                Diagnostic::new(
                    DiagnosticCode::InvalidNodeConfiguration,
                    format!("nodes.{identity}.ports"),
                    "control and data ports in the same direction must have distinct identities",
                ),
            );
        }
    }
    for (identity, edge) in semantic.edges() {
        if identity != edge.id() {
            push(
                diagnostics,
                Diagnostic::new(
                    DiagnosticCode::DuplicateIdentity,
                    format!("edges.{identity}"),
                    "edge map key does not equal embedded identity",
                ),
            );
        }
    }
}

fn validate_edges_and_bindings(semantic: &SemanticBlueprint, diagnostics: &mut Vec<Diagnostic>) {
    let mut incoming_data: BTreeMap<(&NodeId, &crate::PortId), Vec<&crate::Edge>> = BTreeMap::new();
    for edge in semantic.edges().values() {
        let source = semantic.nodes().get(edge.source_node());
        let target = semantic.nodes().get(edge.target_node());
        let location = format!("edges.{}", edge.id());
        if source.is_none() || target.is_none() {
            push(
                diagnostics,
                Diagnostic::new(
                    DiagnosticCode::DanglingReference,
                    location,
                    "edge source or target node does not exist",
                ),
            );
            continue;
        }
        let (Some(source), Some(target)) = (source, target) else {
            continue;
        };
        match edge.kind() {
            EdgeKind::Control => {
                if !source.control_outputs().contains(edge.source_port())
                    || !target.control_inputs().contains(edge.target_port())
                {
                    push(
                        diagnostics,
                        Diagnostic::new(
                            DiagnosticCode::DanglingReference,
                            location,
                            "control edge must connect declared control output to control input",
                        ),
                    );
                }
            }
            EdgeKind::Data => {
                let source_port = source.data_outputs().get(edge.source_port());
                let target_port = target.data_inputs().get(edge.target_port());
                match (source_port, target_port) {
                    (Some(source_port), Some(target_port)) => {
                        if !source_port.schema().compatible_with(target_port.schema()) {
                            push(
                                diagnostics,
                                Diagnostic::new(
                                    DiagnosticCode::SchemaMismatch,
                                    location.clone(),
                                    "data edge schemas must match exactly in schema v1",
                                ),
                            );
                        }
                        incoming_data
                            .entry((edge.target_node(), edge.target_port()))
                            .or_default()
                            .push(edge);
                    }
                    _ => push(
                        diagnostics,
                        Diagnostic::new(
                            DiagnosticCode::DanglingReference,
                            location,
                            "data edge must connect declared data output to data input",
                        ),
                    ),
                }
            }
        }
    }

    for (node_id, node) in semantic.nodes() {
        for (port_id, port) in node.data_inputs() {
            let incoming = incoming_data
                .get(&(node_id, port_id))
                .map_or(&[][..], Vec::as_slice);
            let is_reducer_input = matches!(
                node.kind(),
                NodeKind::Reducer { config } if config.input_port() == port_id
            );
            if incoming.len() > 1 && !is_reducer_input {
                push(
                    diagnostics,
                    Diagnostic::new(
                        DiagnosticCode::AmbiguousControlFlow,
                        format!("nodes.{node_id}.data_inputs.{port_id}"),
                        "a non-reducer input cannot have multiple incoming data edges",
                    ),
                );
            }
            if port.required() && port.binding().is_none() && incoming.is_empty() {
                push(
                    diagnostics,
                    Diagnostic::new(
                        DiagnosticCode::MissingInput,
                        format!("nodes.{node_id}.data_inputs.{port_id}"),
                        "required input needs an explicit binding or incoming data edge",
                    ),
                );
            }
            if port.binding().is_some() && !incoming.is_empty() {
                push(
                    diagnostics,
                    Diagnostic::new(
                        DiagnosticCode::AmbiguousControlFlow,
                        format!("nodes.{node_id}.data_inputs.{port_id}"),
                        "input cannot have both a binding and an incoming data edge",
                    ),
                );
            }
            if let Some(binding) = port.binding() {
                validate_binding(
                    semantic,
                    node_id,
                    port_id,
                    port.schema(),
                    binding,
                    diagnostics,
                );
            }
        }
    }
}

fn validate_binding(
    semantic: &SemanticBlueprint,
    node_id: &NodeId,
    port_id: &crate::PortId,
    target_schema: &crate::SchemaRef,
    binding: &BindingSource,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let location = format!("nodes.{node_id}.data_inputs.{port_id}.binding");
    match binding {
        BindingSource::WorkflowInput { field } | BindingSource::SubworkflowParameter { field } => {
            match semantic.interface().inputs().get(field) {
                Some(input) if input.schema().compatible_with(target_schema) => {}
                Some(_) => push(
                    diagnostics,
                    Diagnostic::new(
                        DiagnosticCode::SchemaMismatch,
                        location,
                        "workflow input and node input schemas differ",
                    ),
                ),
                None => push(
                    diagnostics,
                    Diagnostic::new(
                        DiagnosticCode::DanglingReference,
                        location,
                        "workflow input field does not exist",
                    ),
                ),
            }
        }
        BindingSource::NodeOutput { node, port, path } => {
            match semantic
                .nodes()
                .get(node)
                .and_then(|source| source.data_outputs().get(port))
            {
                Some(output)
                    if path.segments().is_empty()
                        && !output.schema().compatible_with(target_schema) =>
                {
                    push(
                        diagnostics,
                        Diagnostic::new(
                            DiagnosticCode::SchemaMismatch,
                            location.clone(),
                            "direct node-output binding schemas differ",
                        ),
                    );
                }
                Some(_) => {}
                None => push(
                    diagnostics,
                    Diagnostic::new(
                        DiagnosticCode::DanglingReference,
                        location.clone(),
                        "bound node output does not exist",
                    ),
                ),
            }
            let has_data_edge = semantic.edges().values().any(|edge| {
                edge.kind() == EdgeKind::Data
                    && edge.source_node() == node
                    && edge.source_port() == port
                    && edge.target_node() == node_id
                    && edge.target_port() == port_id
            });
            if !has_data_edge {
                push(
                    diagnostics,
                    Diagnostic::new(
                        DiagnosticCode::DanglingReference,
                        location,
                        "node-output binding requires its explicit data dependency edge",
                    ),
                );
            }
        }
        BindingSource::WorkspaceValue { contract, .. }
        | BindingSource::Artifact { contract, .. }
            if !contract.compatible_with(target_schema) =>
        {
            push(
                diagnostics,
                Diagnostic::new(
                    DiagnosticCode::SchemaMismatch,
                    location,
                    "workspace/artifact contract and node input schemas differ",
                ),
            );
        }
        _ => {}
    }
}

fn dependency_adjacency(semantic: &SemanticBlueprint) -> BTreeMap<NodeId, BTreeSet<NodeId>> {
    let mut adjacency: BTreeMap<NodeId, BTreeSet<NodeId>> = semantic
        .nodes()
        .keys()
        .cloned()
        .map(|node| (node, BTreeSet::new()))
        .collect();
    for edge in semantic.edges().values() {
        if semantic.nodes().contains_key(edge.source_node())
            && semantic.nodes().contains_key(edge.target_node())
            && let Some(targets) = adjacency.get_mut(edge.source_node())
        {
            targets.insert(edge.target_node().clone());
        }
    }
    adjacency
}

fn control_adjacency(semantic: &SemanticBlueprint) -> BTreeMap<NodeId, BTreeSet<NodeId>> {
    let mut adjacency: BTreeMap<NodeId, BTreeSet<NodeId>> = semantic
        .nodes()
        .keys()
        .cloned()
        .map(|node| (node, BTreeSet::new()))
        .collect();
    for edge in semantic
        .edges()
        .values()
        .filter(|edge| edge.kind() == EdgeKind::Control)
    {
        if let Some(targets) = adjacency.get_mut(edge.source_node()) {
            targets.insert(edge.target_node().clone());
        }
    }
    adjacency
}

fn validate_acyclic(semantic: &SemanticBlueprint, diagnostics: &mut Vec<Diagnostic>) {
    let adjacency = dependency_adjacency(semantic);
    let mut indegree: BTreeMap<NodeId, usize> = semantic
        .nodes()
        .keys()
        .cloned()
        .map(|node| (node, 0))
        .collect();
    for targets in adjacency.values() {
        for target in targets {
            if let Some(value) = indegree.get_mut(target) {
                *value += 1;
            }
        }
    }
    let mut queue: VecDeque<NodeId> = indegree
        .iter()
        .filter(|(_, degree)| **degree == 0)
        .map(|(node, _)| node.clone())
        .collect();
    let mut visited = 0;
    while let Some(node) = queue.pop_front() {
        visited += 1;
        if let Some(targets) = adjacency.get(&node) {
            for target in targets {
                if let Some(degree) = indegree.get_mut(target) {
                    *degree -= 1;
                    if *degree == 0 {
                        queue.push_back(target.clone());
                    }
                }
            }
        }
    }
    if visited != semantic.nodes().len() {
        push(
            diagnostics,
            Diagnostic::new(
                DiagnosticCode::IllegalCycle,
                "graph",
                "semantic graph must be acyclic; use an explicit repeat node",
            ),
        );
    }
}

fn validate_control_topology(semantic: &SemanticBlueprint, diagnostics: &mut Vec<Diagnostic>) {
    let mut incoming: BTreeMap<NodeId, usize> = semantic
        .nodes()
        .keys()
        .cloned()
        .map(|node| (node, 0))
        .collect();
    let mut outgoing_by_port: BTreeMap<(NodeId, crate::PortId), usize> = BTreeMap::new();
    for edge in semantic
        .edges()
        .values()
        .filter(|edge| edge.kind() == EdgeKind::Control)
    {
        if let Some(count) = incoming.get_mut(edge.target_node()) {
            *count += 1;
        }
        *outgoing_by_port
            .entry((edge.source_node().clone(), edge.source_port().clone()))
            .or_default() += 1;
    }
    let entries: Vec<_> = incoming
        .iter()
        .filter(|(_, count)| **count == 0)
        .map(|(node, _)| node.clone())
        .collect();
    if entries.len() != 1 {
        push(
            diagnostics,
            Diagnostic::new(
                DiagnosticCode::InvalidTopology,
                "graph.entry",
                "workflow must have exactly one control-flow entry node",
            )
            .with_context("entry_count", entries.len().to_string()),
        );
    }
    let terminals: Vec<_> = semantic
        .nodes()
        .iter()
        .filter(|(_, node)| matches!(node.kind(), NodeKind::Terminal { .. }))
        .map(|(id, _)| id)
        .collect();
    if terminals.is_empty() {
        push(
            diagnostics,
            Diagnostic::new(
                DiagnosticCode::InvalidTopology,
                "graph.terminals",
                "workflow must declare at least one explicit terminal node",
            ),
        );
    }

    for (node_id, node) in semantic.nodes() {
        let outgoing_ports: BTreeSet<_> = outgoing_by_port
            .keys()
            .filter(|(source, _)| source == node_id)
            .map(|(_, port)| port.clone())
            .collect();
        let each_once = node.control_outputs().iter().all(|port| {
            outgoing_by_port
                .get(&(node_id.clone(), port.clone()))
                .copied()
                == Some(1)
        });
        match node.kind() {
            NodeKind::Terminal { .. } => {
                if !node.control_outputs().is_empty() || !outgoing_ports.is_empty() {
                    push(
                        diagnostics,
                        Diagnostic::new(
                            DiagnosticCode::InvalidTopology,
                            format!("nodes.{node_id}"),
                            "terminal nodes cannot have outgoing control flow",
                        ),
                    );
                }
            }
            NodeKind::Branch { config } => {
                if config.ports() != *node.control_outputs() || !each_once {
                    push(
                        diagnostics,
                        Diagnostic::new(
                            DiagnosticCode::AmbiguousControlFlow,
                            format!("nodes.{node_id}.branch"),
                            "branch arms must exactly equal single-use control output ports",
                        ),
                    );
                }
            }
            NodeKind::Fork { config } => {
                if config.branches() != node.control_outputs() || !each_once {
                    push(
                        diagnostics,
                        Diagnostic::new(
                            DiagnosticCode::AmbiguousControlFlow,
                            format!("nodes.{node_id}.fork"),
                            "fork branches must exactly equal single-use control output ports",
                        ),
                    );
                }
            }
            _ => {
                if node.control_outputs().len() != 1 || !each_once {
                    push(
                        diagnostics,
                        Diagnostic::new(
                            DiagnosticCode::AmbiguousControlFlow,
                            format!("nodes.{node_id}.control_outputs"),
                            "non-branch, non-fork work must have exactly one used control output",
                        ),
                    );
                }
            }
        }
        if !matches!(node.kind(), NodeKind::Join { .. })
            && incoming.get(node_id).copied().unwrap_or_default() > 1
        {
            push(
                diagnostics,
                Diagnostic::new(
                    DiagnosticCode::AmbiguousControlFlow,
                    format!("nodes.{node_id}.control_inputs"),
                    "only a join may receive multiple control edges",
                ),
            );
        }
    }

    if let Some(entry) = entries.first() {
        let adjacency = control_adjacency(semantic);
        let reachable = reachable_from(entry, &adjacency, None);
        for node in semantic.nodes().keys() {
            if !reachable.contains(node) {
                push(
                    diagnostics,
                    Diagnostic::new(
                        DiagnosticCode::UnreachableNode,
                        format!("nodes.{node}"),
                        "node is not reachable from the control-flow entry",
                    ),
                );
            }
        }
    }
}

fn validate_fork_join(semantic: &SemanticBlueprint, diagnostics: &mut Vec<Diagnostic>) {
    let adjacency = control_adjacency(semantic);
    for (join_id, node) in semantic.nodes() {
        let NodeKind::Join { config } = node.kind() else {
            continue;
        };
        let Some(fork_node) = semantic.nodes().get(config.fork()) else {
            push(
                diagnostics,
                Diagnostic::new(
                    DiagnosticCode::InvalidForkJoin,
                    format!("nodes.{join_id}.join.fork"),
                    "owning fork does not exist",
                ),
            );
            continue;
        };
        let NodeKind::Fork { config: fork } = fork_node.kind() else {
            push(
                diagnostics,
                Diagnostic::new(
                    DiagnosticCode::InvalidForkJoin,
                    format!("nodes.{join_id}.join.fork"),
                    "join owner is not a fork node",
                ),
            );
            continue;
        };
        let incoming_sources: BTreeSet<_> = semantic
            .edges()
            .values()
            .filter(|edge| edge.kind() == EdgeKind::Control && edge.target_node() == join_id)
            .map(|edge| edge.source_node().clone())
            .collect();
        let branch_starts: Vec<_> = semantic
            .edges()
            .values()
            .filter(|edge| edge.kind() == EdgeKind::Control && edge.source_node() == config.fork())
            .map(|edge| edge.target_node().clone())
            .collect();
        let covered = branch_starts
            .iter()
            .filter(|start| {
                let reachable = reachable_from(start, &adjacency, Some(join_id));
                incoming_sources
                    .iter()
                    .any(|source| reachable.contains(source))
            })
            .count();
        if incoming_sources.is_empty() || covered != branch_starts.len() {
            push(
                diagnostics,
                Diagnostic::new(
                    DiagnosticCode::InvalidForkJoin,
                    format!("nodes.{join_id}.join"),
                    "every owned fork branch must reach this join without crossing it",
                ),
            );
        }
        let branch_count = fork.branches().len();
        match config.policy() {
            JoinPolicy::All if incoming_sources.len() != branch_count => push(
                diagnostics,
                Diagnostic::new(
                    DiagnosticCode::InvalidForkJoin,
                    format!("nodes.{join_id}.join.policy"),
                    "an all-join requires one incoming branch completion per fork branch",
                ),
            ),
            JoinPolicy::Quorum(quorum)
                if quorum == 0
                    || usize::from(quorum) > branch_count
                    || usize::from(quorum) > incoming_sources.len() =>
            {
                push(
                    diagnostics,
                    Diagnostic::new(
                        DiagnosticCode::ImpossibleQuorum,
                        format!("nodes.{join_id}.join.policy"),
                        "quorum must be nonzero and satisfiable by the owned branches",
                    ),
                );
            }
            _ => {}
        }
    }
}

fn reachable_from(
    start: &NodeId,
    adjacency: &BTreeMap<NodeId, BTreeSet<NodeId>>,
    stop: Option<&NodeId>,
) -> BTreeSet<NodeId> {
    let mut seen = BTreeSet::new();
    let mut queue = VecDeque::from([start.clone()]);
    while let Some(node) = queue.pop_front() {
        if !seen.insert(node.clone()) || stop == Some(&node) {
            continue;
        }
        if let Some(targets) = adjacency.get(&node) {
            queue.extend(targets.iter().cloned());
        }
    }
    seen
}

fn validate_reducers_and_subworkflows(
    semantic: &SemanticBlueprint,
    diagnostics: &mut Vec<Diagnostic>,
) {
    for (node_id, node) in semantic.nodes() {
        match node.kind() {
            NodeKind::Reducer { config } => {
                let Some(port) = node.data_inputs().get(config.input_port()) else {
                    push(
                        diagnostics,
                        Diagnostic::new(
                            DiagnosticCode::ReducerMismatch,
                            format!("nodes.{node_id}.reducer.input_port"),
                            "configured reducer input port does not exist",
                        ),
                    );
                    continue;
                };
                let incoming = semantic
                    .edges()
                    .values()
                    .filter(|edge| {
                        edge.kind() == EdgeKind::Data
                            && edge.target_node() == node_id
                            && edge.target_port() == config.input_port()
                    })
                    .count();
                if !port.schema().compatible_with(config.item_schema())
                    || incoming < usize::from(config.minimum_items())
                {
                    push(
                        diagnostics,
                        Diagnostic::new(
                            DiagnosticCode::ReducerMismatch,
                            format!("nodes.{node_id}.reducer"),
                            "reducer item schema or minimum input count does not match data edges",
                        ),
                    );
                }
            }
            NodeKind::Subworkflow { reference } => {
                validate_subworkflow_ports(node_id, node, reference.interface(), diagnostics);
            }
            NodeKind::Repeat { config } => {
                validate_subworkflow_ports(node_id, node, config.body().interface(), diagnostics);
            }
            NodeKind::Task {
                requirement,
                operation,
            } if requirement.operation() != operation => push(
                diagnostics,
                Diagnostic::new(
                    DiagnosticCode::MissingCapabilityRequirement,
                    format!("nodes.{node_id}.task"),
                    "task requirement does not name its configured operation",
                ),
            ),
            _ => {}
        }
    }
}

fn validate_subworkflow_ports(
    node_id: &NodeId,
    node: &Node,
    interface: &crate::WorkflowInterface,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let inputs_match = interface.inputs().iter().all(|(field, expected)| {
        crate::PortId::new(field.as_str())
            .ok()
            .is_some_and(|port_id| {
                node.data_inputs()
                    .get(&port_id)
                    .is_some_and(|port| port.schema().compatible_with(expected.schema()))
            })
    });
    let outputs_match = interface.outputs().iter().all(|(field, expected)| {
        crate::PortId::new(field.as_str())
            .ok()
            .is_some_and(|port_id| {
                node.data_outputs()
                    .get(&port_id)
                    .is_some_and(|port| port.schema().compatible_with(expected.schema()))
            })
    });
    if !inputs_match || !outputs_match {
        push(
            diagnostics,
            Diagnostic::new(
                DiagnosticCode::IncompatibleSubworkflow,
                format!("nodes.{node_id}.subworkflow.interface"),
                "node data ports must cover the pinned subworkflow interface with exact schemas",
            ),
        );
    }
}
