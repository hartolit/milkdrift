//! Deterministic structured-node execution, reducers, repeats, and subworkflow intent creation.

use super::super::RuntimeService;
use super::super::transition::PlanTransition;
use crate::RuntimeError;
use milkdrift_blueprint::{BlueprintRevision, Node, ReducerStrategy};
use milkdrift_capability::BoundedJson;
use milkdrift_persistence::{
    BoundedDetail, NodeExecutionId, NodeOutcome, RunEventKind, WorkspaceMutation,
};
use milkdrift_workspace::{ScopeReference, ValueKey, WorkspaceValue};

impl RuntimeService {
    pub(super) fn drive_reducer(
        &self,
        transition: &mut PlanTransition<'_>,
        node: &Node,
        execution: &NodeExecutionId,
        scope_reference: &ScopeReference,
        config: &milkdrift_blueprint::ReducerConfig,
        revision: &BlueprintRevision,
    ) -> Result<(), RuntimeError> {
        if matches!(config.strategy(), ReducerStrategy::Capability(_)) {
            return Ok(());
        }
        if !transition
            .projection()
            .node_executions()
            .get(execution)
            .is_some_and(|value| value.outputs().is_empty())
        {
            return self.complete_deterministic(transition, node, execution);
        }
        let values = self.ordered_reducer_references(
            revision,
            transition.projection(),
            node,
            config.input_port(),
            scope_reference,
            transition.workspace(),
        )?;
        if values.len() < usize::from(config.minimum_items()) {
            return Ok(());
        }
        let output_port = node.data_outputs().keys().next().ok_or_else(|| {
            RuntimeError::Scheduling(format!("reducer node {} has no output port", node.id()))
        })?;
        let key = ValueKey::new(output_port.as_str().to_owned())
            .map_err(|error| RuntimeError::Scheduling(error.to_string()))?;
        let (value, artifact) = match config.strategy() {
            ReducerStrategy::Collect => {
                let Ok(json_value) = serde_json::to_value(&values) else {
                    return self.complete_deterministic_with_outcome(
                        transition,
                        node,
                        execution,
                        NodeOutcome::Failed,
                        Some(BoundedDetail::new(
                            "deterministic reducer result could not be serialized",
                        )?),
                    );
                };
                let Ok(collected) = BoundedJson::new(json_value) else {
                    return self.complete_deterministic_with_outcome(
                        transition,
                        node,
                        execution,
                        NodeOutcome::Failed,
                        Some(BoundedDetail::new(
                            "deterministic reducer result exceeds the bounded JSON contract",
                        )?),
                    );
                };
                (WorkspaceValue::Json(collected), None)
            }
            ReducerStrategy::First => {
                let reference = values.first().ok_or_else(|| {
                    RuntimeError::Scheduling("first reducer has no input".to_owned())
                })?;
                let entry = self.projected_workspace_value(
                    transition.projection(),
                    reference,
                    transition.workspace(),
                )?;
                let artifact = entry.value().as_artifact().cloned();
                (entry.value().clone(), artifact)
            }
            ReducerStrategy::Capability(_) => return Ok(()),
        };
        let entry = self.projected_output_entry(
            transition.projection(),
            scope_reference,
            key,
            value,
            transition.workspace(),
        )?;
        let reference = entry.reference().clone();
        transition.push_workspace(WorkspaceMutation::PutValue { entry })?;
        transition.push_event(RunEventKind::DeterministicOutputPublished {
            execution: execution.clone(),
            value: reference,
            artifact,
        })?;
        self.complete_deterministic(transition, node, execution)
    }
}
