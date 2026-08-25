//! Deterministic structured-node execution, reducers, repeats, and subworkflow intent creation.

use super::super::RuntimeService;
use crate::RuntimeError;
use crate::projection::RunProjection;
use milkdrift_blueprint::{Node, PortId};
use milkdrift_persistence::{
    BoundedDetail, NodeExecutionId, NodeOutcome, RunEventEnvelope, RunEventKind,
    SubworkflowOwnership, TimestampMillis, WorkspaceMutation,
};
use milkdrift_workspace::{RunId, ScopeReference, ValueKey, WorkspaceScope};

impl RuntimeService {
    #[allow(clippy::too_many_arguments)]
    pub(super) fn create_subworkflow_intent(
        &self,
        run: &RunId,
        occurred_at: TimestampMillis,
        projection: &mut RunProjection,
        events: &mut Vec<RunEventEnvelope>,
        workspace: &mut Vec<WorkspaceMutation>,
        node: &Node,
        parent_execution: &NodeExecutionId,
        occurrence_scope: &ScopeReference,
        parent_scope: &ScopeReference,
        reference: &milkdrift_blueprint::PinnedSubworkflow,
    ) -> Result<(), RuntimeError> {
        let child_revision =
            self.load_validated_revision(reference.revision(), Some(reference.workflow()))?;
        let parent_revision = self.revision_for_execution(projection, parent_execution)?;
        let mut resolved_inputs = Vec::new();
        for (field, interface_field) in child_revision.semantic().interface().inputs() {
            let port = PortId::new(field.as_str().to_owned())
                .map_err(|error| RuntimeError::Scheduling(error.to_string()))?;
            let Some(parent_declaration) = node.data_inputs().get(&port) else {
                if interface_field.is_required() {
                    return self.complete_deterministic_with_outcome(
                        run,
                        occurred_at,
                        projection,
                        events,
                        node,
                        parent_execution,
                        NodeOutcome::Failed,
                        Some(BoundedDetail::new(
                            "required subworkflow input has no parent node data port",
                        )?),
                    );
                }
                continue;
            };
            let resolved = match self.resolve_node_port_inputs(
                &parent_revision,
                projection,
                node,
                &port,
                occurrence_scope,
                workspace,
            ) {
                Ok(resolved) => resolved,
                Err(RuntimeError::Scheduling(_)) => {
                    return self.complete_deterministic_with_outcome(
                        run,
                        occurred_at,
                        projection,
                        events,
                        node,
                        parent_execution,
                        NodeOutcome::Failed,
                        Some(BoundedDetail::new(
                            "subworkflow inputs could not be resolved from immutable parent data",
                        )?),
                    );
                }
                Err(error) => return Err(error),
            };
            if resolved.is_empty() {
                if interface_field.is_required() || parent_declaration.is_required() {
                    return self.complete_deterministic_with_outcome(
                        run,
                        occurred_at,
                        projection,
                        events,
                        node,
                        parent_execution,
                        NodeOutcome::Failed,
                        Some(BoundedDetail::new(
                            "required subworkflow input is absent from immutable parent data",
                        )?),
                    );
                }
                continue;
            }
            if resolved.len() != 1 {
                return self.complete_deterministic_with_outcome(
                    run,
                    occurred_at,
                    projection,
                    events,
                    node,
                    parent_execution,
                    NodeOutcome::Failed,
                    Some(BoundedDetail::new(
                        "subworkflow input resolved to more than one immutable value",
                    )?),
                );
            }
            let key = ValueKey::new(field.as_str().to_owned())
                .map_err(|error| RuntimeError::Scheduling(error.to_string()))?;
            let resolved_value = resolved.into_iter().next().ok_or_else(|| {
                RuntimeError::InvalidHistory("resolved subworkflow input disappeared".to_owned())
            })?;
            resolved_inputs.push((key, resolved_value));
        }
        let parent = projection.scopes().get(parent_scope).ok_or_else(|| {
            RuntimeError::InvalidHistory("subworkflow parent scope is absent".to_owned())
        })?;
        let subworkflow = self.next_subworkflow_id()?;
        let scope = WorkspaceScope::subworkflow(self.next_scope_id()?, parent, subworkflow.clone())
            .map_err(|error| RuntimeError::InvalidTransition(error.to_string()))?;
        let scope_reference = scope.reference().clone();
        workspace.push(WorkspaceMutation::CreateScope {
            scope: scope.clone(),
        });
        let mut inputs = Vec::new();
        for (key, resolved_value) in resolved_inputs {
            let entry = self.materialize_subworkflow_input(
                projection,
                workspace,
                &scope_reference,
                parent_scope,
                key,
                resolved_value,
            )?;
            inputs.push(entry.reference().clone());
            workspace.push(WorkspaceMutation::PutValue { entry });
        }
        self.push_projected_event(
            run,
            occurred_at,
            projection,
            events,
            RunEventKind::SubworkflowCreated {
                subworkflow,
                parent_execution: parent_execution.clone(),
                child_run: self.next_run_id()?,
                child_revision: reference.revision().clone(),
                scope: scope.clone(),
                ownership: SubworkflowOwnership::Attached,
                inputs,
            },
        )?;
        Ok(())
    }
}
