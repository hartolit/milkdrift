//! Workspace provenance validation and structured input/value resolution.

use super::RuntimeService;
use super::support::{
    ResolvedInputValue, execution_scope_related, select_json_path,
    source_execution_is_valid_for_occurrence,
};
use crate::RuntimeError;
use crate::projection::{NodeExecutionState, RunProjection};
use milkdrift_blueprint::{BindingSource, BlueprintRevision, EdgeKind, Node, NodeId, PortId};
use milkdrift_persistence::{
    NodeOutcome, RunEventEnvelope, RunEventKind, TimestampMillis, WorkspaceMutation,
};
use milkdrift_workspace::{
    ArtifactReference, RunId, ScopeReference, ValueKey, ValueVersion, WorkspaceScope,
    WorkspaceValue, WorkspaceValueEntry, WorkspaceValueReference,
};
use std::collections::BTreeSet;

impl RuntimeService {
    pub(super) fn resolve_node_port_inputs(
        &self,
        revision: &BlueprintRevision,
        projection: &RunProjection,
        node: &Node,
        port: &PortId,
        occurrence_scope: &ScopeReference,
        pending_workspace: &[WorkspaceMutation],
    ) -> Result<Vec<ResolvedInputValue>, RuntimeError> {
        self.validate_projected_scope(projection, occurrence_scope, pending_workspace)?;
        let declaration = node.data_inputs().get(port).ok_or_else(|| {
            RuntimeError::InvalidHistory(format!(
                "node {} has no declared data input {port}",
                node.id()
            ))
        })?;
        if let Some(binding) = declaration.binding() {
            return Ok(self
                .resolve_optional_binding(
                    projection,
                    node.id(),
                    occurrence_scope,
                    binding,
                    pending_workspace,
                    true,
                )?
                .into_iter()
                .collect());
        }
        let references =
            self.incoming_data_references(revision, projection, node, port, occurrence_scope);
        for reference in &references {
            self.projected_workspace_value(projection, reference, pending_workspace)?;
        }
        Ok(references
            .into_iter()
            .map(ResolvedInputValue::Workspace)
            .collect())
    }

    fn incoming_data_references(
        &self,
        revision: &BlueprintRevision,
        projection: &RunProjection,
        node: &Node,
        port: &PortId,
        occurrence_scope: &ScopeReference,
    ) -> BTreeSet<WorkspaceValueReference> {
        let mut references = BTreeSet::new();
        for edge in revision.semantic().edges().values().filter(|edge| {
            edge.kind() == EdgeKind::Data
                && edge.target_node() == node.id()
                && edge.target_port() == port
        }) {
            for execution in projection
                .executions_for_node(edge.source_node())
                .filter(|source| {
                    source_execution_is_valid_for_occurrence(
                        projection,
                        *source,
                        node.id(),
                        occurrence_scope,
                    ) && execution_scope_related(projection, source.scope(), occurrence_scope)
                        && source.state() == &NodeExecutionState::Terminal(NodeOutcome::Succeeded)
                })
            {
                references.extend(
                    execution
                        .outputs()
                        .iter()
                        .filter(|output| {
                            output.value().key().as_str() == edge.source_port().as_str()
                        })
                        .map(|output| output.value().clone()),
                );
            }
        }
        references
    }

    pub(super) fn ordered_reducer_references(
        &self,
        revision: &BlueprintRevision,
        projection: &RunProjection,
        node: &Node,
        port: &PortId,
        occurrence_scope: &ScopeReference,
        pending_workspace: &[WorkspaceMutation],
    ) -> Result<Vec<WorkspaceValueReference>, RuntimeError> {
        self.validate_projected_scope(projection, occurrence_scope, pending_workspace)?;
        let mut candidates = Vec::new();
        for (edge_order, edge) in revision
            .semantic()
            .edges()
            .values()
            .filter(|edge| {
                edge.kind() == EdgeKind::Data
                    && edge.target_node() == node.id()
                    && edge.target_port() == port
            })
            .enumerate()
        {
            for execution in projection
                .executions_for_node(edge.source_node())
                .filter(|source| {
                    source_execution_is_valid_for_occurrence(
                        projection,
                        *source,
                        node.id(),
                        occurrence_scope,
                    ) && execution_scope_related(projection, source.scope(), occurrence_scope)
                        && source.state() == &NodeExecutionState::Terminal(NodeOutcome::Succeeded)
                })
            {
                let branch_order = projection
                    .branches()
                    .values()
                    .find(|branch| branch.children().contains(execution.execution()))
                    .and_then(|branch| {
                        projection
                            .current_node_execution(branch.fork_execution())
                            .map(|fork| {
                                (
                                    fork.created_sequence().get(),
                                    branch.port().as_str().to_owned(),
                                )
                            })
                    });
                for output in execution
                    .outputs()
                    .iter()
                    .filter(|output| output.value().key().as_str() == edge.source_port().as_str())
                {
                    let (class, owner_order, port_order) = branch_order.clone().map_or_else(
                        || {
                            (
                                1_u8,
                                u64::try_from(edge_order).unwrap_or(u64::MAX),
                                String::new(),
                            )
                        },
                        |(fork_sequence, branch_port)| (0_u8, fork_sequence, branch_port),
                    );
                    candidates.push((
                        class,
                        owner_order,
                        port_order,
                        execution.created_sequence().get(),
                        output.sequence().get(),
                        output.value().clone(),
                    ));
                }
            }
        }
        candidates.sort();
        let mut seen = BTreeSet::new();
        let mut ordered = Vec::new();
        for (_, _, _, _, _, reference) in candidates {
            if seen.insert(reference.clone()) {
                self.projected_workspace_value(projection, &reference, pending_workspace)?;
                ordered.push(reference);
            }
        }
        Ok(ordered)
    }

    pub(super) fn resolve_optional_binding(
        &self,
        projection: &RunProjection,
        occurrence_node: &NodeId,
        occurrence_scope: &ScopeReference,
        binding: &BindingSource,
        pending_workspace: &[WorkspaceMutation],
        apply_path: bool,
    ) -> Result<Option<ResolvedInputValue>, RuntimeError> {
        self.validate_projected_scope(projection, occurrence_scope, pending_workspace)?;
        match binding {
            BindingSource::Literal { value } => Ok(Some(ResolvedInputValue::Inline {
                value: value.clone(),
                source: None,
            })),
            BindingSource::WorkflowInput { field }
            | BindingSource::SubworkflowParameter { field } => {
                let key = ValueKey::new(field.as_str().to_owned())
                    .map_err(|error| RuntimeError::Scheduling(error.to_string()))?;
                let reference = projection
                    .inputs()
                    .iter()
                    .find(|reference| reference.key() == &key)
                    .cloned();
                if reference.is_none() {
                    let input_scope = projection
                        .root_scope()
                        .ok_or_else(|| {
                            RuntimeError::InvalidHistory(
                                "workflow input lookup has no projected root scope".to_owned(),
                            )
                        })?
                        .reference();
                    // Absence is itself an integrity claim. Never select a later
                    // value by key as an immutable creation input, but still
                    // compare the durable latest row with replay-derived state so
                    // an injected unprojected row cannot turn omission into input.
                    let initial = WorkspaceValueReference::new(
                        input_scope.clone(),
                        key.clone(),
                        ValueVersion::FIRST,
                    );
                    if !projection.workspace_values().contains(&initial)
                        && self.workspace_value(&initial, pending_workspace)?.is_some()
                    {
                        return Err(RuntimeError::InvalidHistory(format!(
                            "durable workspace contains an orphan initial input {}:{}",
                            input_scope.scope(),
                            key
                        )));
                    }
                    let _ = self.projected_latest_workspace_value(
                        projection,
                        input_scope,
                        &key,
                        pending_workspace,
                    )?;
                }
                reference
                    .map(|reference| {
                        self.projected_workspace_value(projection, &reference, pending_workspace)
                            .map(|_| ResolvedInputValue::Workspace(reference))
                    })
                    .transpose()
            }
            BindingSource::NodeOutput { node, port, path } => {
                let references: BTreeSet<_> = projection
                    .executions_for_node(node)
                    .filter(|source| {
                        source_execution_is_valid_for_occurrence(
                            projection,
                            *source,
                            occurrence_node,
                            occurrence_scope,
                        ) && execution_scope_related(projection, source.scope(), occurrence_scope)
                            && source.state()
                                == &NodeExecutionState::Terminal(NodeOutcome::Succeeded)
                    })
                    .flat_map(|source| source.outputs())
                    .filter(|output| output.value().key().as_str() == port.as_str())
                    .map(|output| output.value().clone())
                    .collect();
                for reference in &references {
                    self.projected_workspace_value(projection, reference, pending_workspace)?;
                }
                if references.is_empty() {
                    return Ok(None);
                }
                if references.len() != 1 {
                    return Err(RuntimeError::Scheduling(format!(
                        "required node output {node}:{port} resolved to {} values",
                        references.len()
                    )));
                }
                let reference = references.into_iter().next().ok_or_else(|| {
                    RuntimeError::InvalidHistory("resolved node output disappeared".to_owned())
                })?;
                if path.segments().is_empty() || !apply_path {
                    return Ok(Some(ResolvedInputValue::Workspace(reference)));
                }
                let entry =
                    self.projected_workspace_value(projection, &reference, pending_workspace)?;
                let json_value = entry.value().as_json().ok_or_else(|| {
                    RuntimeError::Scheduling(format!(
                        "node output {node}:{port} is an artifact and cannot be path-selected"
                    ))
                })?;
                let selected = select_json_path(json_value, path.segments())?;
                Ok(Some(ResolvedInputValue::Inline {
                    value: selected,
                    source: Some(reference),
                }))
            }
            BindingSource::WorkspaceValue { reference, .. } => {
                let parsed = serde_json::from_str::<WorkspaceValueReference>(reference).map_err(
                    |error| {
                        RuntimeError::Scheduling(format!(
                            "workspace binding is not an exact canonical reference: {error}"
                        ))
                    },
                )?;
                self.projected_workspace_value(projection, &parsed, pending_workspace)?;
                self.ensure_readable_ancestor(
                    projection,
                    parsed.scope(),
                    occurrence_scope,
                    pending_workspace,
                )?;
                Ok(Some(ResolvedInputValue::Workspace(parsed)))
            }
            BindingSource::Artifact { reference, .. } => {
                let parsed =
                    serde_json::from_str::<ArtifactReference>(reference).map_err(|error| {
                        RuntimeError::Scheduling(format!(
                            "artifact binding is not an exact canonical reference: {error}"
                        ))
                    })?;
                if !self.store.is_committed(&parsed)? {
                    return Err(RuntimeError::Scheduling(
                        "artifact binding references uncommitted content".to_owned(),
                    ));
                }
                Ok(Some(ResolvedInputValue::Artifact(parsed)))
            }
        }
    }

    fn workspace_value(
        &self,
        reference: &WorkspaceValueReference,
        pending_workspace: &[WorkspaceMutation],
    ) -> Result<Option<WorkspaceValueEntry>, RuntimeError> {
        if let Some(entry) = pending_workspace
            .iter()
            .rev()
            .find_map(|mutation| match mutation {
                WorkspaceMutation::PutValue { entry } if entry.reference() == reference => {
                    Some(entry.clone())
                }
                WorkspaceMutation::CreateScope { .. } | WorkspaceMutation::PutValue { .. } => None,
            })
        {
            return Ok(Some(entry));
        }
        self.store.value(reference).map_err(RuntimeError::from)
    }

    pub(super) fn validate_projected_scope(
        &self,
        projection: &RunProjection,
        reference: &ScopeReference,
        pending_workspace: &[WorkspaceMutation],
    ) -> Result<(), RuntimeError> {
        let expected = projection.scopes().get(reference).ok_or_else(|| {
            RuntimeError::InvalidHistory(format!(
                "projected workspace scope {}:{} is absent",
                reference.run(),
                reference.scope()
            ))
        })?;
        if let Some(pending) = pending_workspace
            .iter()
            .rev()
            .find_map(|mutation| match mutation {
                WorkspaceMutation::CreateScope { scope } if scope.reference() == reference => {
                    Some(scope)
                }
                WorkspaceMutation::CreateScope { .. } | WorkspaceMutation::PutValue { .. } => None,
            })
        {
            if pending != expected {
                return Err(RuntimeError::InvalidHistory(format!(
                    "pending workspace scope {}:{} contradicts its projection",
                    reference.run(),
                    reference.scope()
                )));
            }
            return Ok(());
        }
        let durable = self
            .store
            .scope(reference.run(), reference.scope())?
            .ok_or_else(|| {
                RuntimeError::InvalidHistory(format!(
                    "projected workspace scope {}:{} is absent from durable storage",
                    reference.run(),
                    reference.scope()
                ))
            })?;
        if &durable != expected {
            return Err(RuntimeError::InvalidHistory(format!(
                "durable workspace scope {}:{} contradicts its projection",
                reference.run(),
                reference.scope()
            )));
        }
        Ok(())
    }

    pub(super) fn projected_workspace_value(
        &self,
        projection: &RunProjection,
        reference: &WorkspaceValueReference,
        pending_workspace: &[WorkspaceMutation],
    ) -> Result<WorkspaceValueEntry, RuntimeError> {
        self.validate_projected_scope(projection, reference.scope(), pending_workspace)?;
        if !projection.workspace_values().contains(reference) {
            return Err(RuntimeError::InvalidHistory(format!(
                "workspace value {}:{}:{} is absent from its event projection",
                reference.scope().scope(),
                reference.key(),
                reference.version()
            )));
        }
        let entry = self
            .workspace_value(reference, pending_workspace)?
            .ok_or_else(|| {
                RuntimeError::InvalidHistory(format!(
                    "projected workspace value {}:{}:{} is absent from durable storage",
                    reference.scope().scope(),
                    reference.key(),
                    reference.version()
                ))
            })?;
        if entry.reference() != reference {
            return Err(RuntimeError::InvalidHistory(
                "durable workspace value contradicts its exact projected reference".to_owned(),
            ));
        }
        Ok(entry)
    }

    pub(super) fn projected_latest_workspace_value(
        &self,
        projection: &RunProjection,
        scope: &ScopeReference,
        key: &ValueKey,
        pending_workspace: &[WorkspaceMutation],
    ) -> Result<Option<WorkspaceValueEntry>, RuntimeError> {
        self.validate_projected_scope(projection, scope, pending_workspace)?;
        let expected = projection
            .workspace_values()
            .iter()
            .filter(|reference| reference.scope() == scope && reference.key() == key)
            .max_by_key(|reference| reference.version());
        let pending = pending_workspace
            .iter()
            .filter_map(|mutation| match mutation {
                WorkspaceMutation::PutValue { entry }
                    if entry.reference().scope() == scope && entry.reference().key() == key =>
                {
                    Some(entry)
                }
                WorkspaceMutation::CreateScope { .. } | WorkspaceMutation::PutValue { .. } => None,
            })
            .max_by_key(|entry| entry.reference().version())
            .cloned();
        if pending
            .as_ref()
            .is_some_and(|entry| !projection.workspace_values().contains(entry.reference()))
        {
            return Err(RuntimeError::InvalidHistory(format!(
                "pending workspace contains an unprojected latest value {}:{}",
                scope.scope(),
                key
            )));
        }
        let durable = self.store.latest_value(scope, key)?;
        if durable
            .as_ref()
            .is_some_and(|entry| !projection.workspace_values().contains(entry.reference()))
        {
            return Err(RuntimeError::InvalidHistory(format!(
                "durable workspace contains orphan latest value {}:{}",
                scope.scope(),
                key
            )));
        }
        if pending
            .as_ref()
            .zip(durable.as_ref())
            .is_some_and(|(pending, durable)| pending.reference() == durable.reference())
        {
            return Err(RuntimeError::InvalidHistory(format!(
                "pending workspace duplicates durable latest value {}:{}",
                scope.scope(),
                key
            )));
        }
        let observed = match (pending, durable) {
            (Some(pending), Some(durable))
                if pending.reference().version() < durable.reference().version() =>
            {
                Some(durable)
            }
            (Some(pending), Some(_)) => Some(pending),
            (Some(pending), None) => Some(pending),
            (None, Some(durable)) => Some(durable),
            (None, None) => None,
        };
        match (expected, observed) {
            (None, None) => Ok(None),
            (None, Some(_)) => Err(RuntimeError::InvalidHistory(format!(
                "durable workspace contains orphan latest value {}:{}",
                scope.scope(),
                key
            ))),
            (Some(expected), None) => Err(RuntimeError::InvalidHistory(format!(
                "projected latest workspace value {}:{}:{} is absent from durable storage",
                scope.scope(),
                key,
                expected.version()
            ))),
            (Some(expected), Some(entry)) if entry.reference() == expected => Ok(Some(entry)),
            (Some(expected), Some(entry)) => Err(RuntimeError::InvalidHistory(format!(
                "durable latest workspace value {}:{}:{} contradicts projected version {}",
                scope.scope(),
                key,
                entry.reference().version(),
                expected.version()
            ))),
        }
    }

    pub(super) fn materialize_subworkflow_input(
        &self,
        projection: &RunProjection,
        pending_workspace: &[WorkspaceMutation],
        target_scope: &ScopeReference,
        target_parent_scope: &ScopeReference,
        key: ValueKey,
        value: ResolvedInputValue,
    ) -> Result<WorkspaceValueEntry, RuntimeError> {
        match value {
            ResolvedInputValue::Inline { value, source } => {
                if let Some(source) = source {
                    self.ensure_readable_ancestor(
                        projection,
                        source.scope(),
                        target_parent_scope,
                        pending_workspace,
                    )?;
                    WorkspaceValueEntry::inherited(
                        target_scope.clone(),
                        key,
                        source,
                        WorkspaceValue::Json(value),
                    )
                    .map_err(|error| RuntimeError::InvalidTransition(error.to_string()))
                } else {
                    Ok(WorkspaceValueEntry::initial(
                        target_scope.clone(),
                        key,
                        WorkspaceValue::Json(value),
                    ))
                }
            }
            ResolvedInputValue::Workspace(source) => {
                self.ensure_readable_ancestor(
                    projection,
                    source.scope(),
                    target_parent_scope,
                    pending_workspace,
                )?;
                let entry =
                    self.projected_workspace_value(projection, &source, pending_workspace)?;
                WorkspaceValueEntry::inherited(
                    target_scope.clone(),
                    key,
                    source,
                    entry.value().clone(),
                )
                .map_err(|error| RuntimeError::InvalidTransition(error.to_string()))
            }
            ResolvedInputValue::Artifact(reference) => Ok(WorkspaceValueEntry::initial(
                target_scope.clone(),
                key,
                WorkspaceValue::Artifact(reference),
            )),
        }
    }

    fn ensure_readable_ancestor(
        &self,
        projection: &RunProjection,
        source: &ScopeReference,
        target_parent: &ScopeReference,
        pending_workspace: &[WorkspaceMutation],
    ) -> Result<(), RuntimeError> {
        self.validate_projected_scope(projection, source, pending_workspace)?;
        let mut cursor = Some(target_parent);
        for _ in 0..=milkdrift_workspace::MAX_SCOPE_DEPTH {
            let Some(scope) = cursor else {
                break;
            };
            self.validate_projected_scope(projection, scope, pending_workspace)?;
            if scope == source {
                return Ok(());
            }
            cursor = projection
                .scopes()
                .get(scope)
                .and_then(WorkspaceScope::parent);
        }
        Err(RuntimeError::InvalidTransition(
            "workspace input aliases a sibling or unrelated scope; scope isolation forbids the read"
                .to_owned(),
        ))
    }

    pub(super) fn push_projected_event(
        &self,
        run: &RunId,
        occurred_at: TimestampMillis,
        projection: &mut RunProjection,
        events: &mut Vec<RunEventEnvelope>,
        kind: RunEventKind,
    ) -> Result<(), RuntimeError> {
        if events.len() >= milkdrift_persistence::MAX_EVENTS_PER_COMMIT {
            return Err(RuntimeError::Scheduling(
                "event commit bound reached while driving structured work".to_owned(),
            ));
        }
        let event = RunEventEnvelope::new(
            self.next_event_id()?,
            run.clone(),
            projection.sequence().next()?,
            occurred_at,
            kind,
        )?;
        projection.apply_replayed(&event)?;
        events.push(event);
        Ok(())
    }
}
