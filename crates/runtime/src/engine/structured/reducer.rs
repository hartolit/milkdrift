//! Deterministic structured-node execution, reducers, repeats, and subworkflow intent creation.

use super::super::RuntimeService;
use crate::RuntimeError;
use crate::projection::RunProjection;
use milkdrift_blueprint::{BlueprintRevision, Node, ReducerStrategy};
use milkdrift_capability::BoundedJson;
use milkdrift_persistence::{
    BoundedDetail, NodeExecutionId, NodeOutcome, RunEventEnvelope, RunEventKind, TimestampMillis,
    WorkspaceMutation,
};
use milkdrift_workspace::{RunId, ScopeReference, ValueKey, WorkspaceValue};

impl RuntimeService {
    #[allow(clippy::too_many_arguments)]
    pub(super) fn drive_reducer(
        &self,
        run: &RunId,
        occurred_at: TimestampMillis,
        projection: &mut RunProjection,
        events: &mut Vec<RunEventEnvelope>,
        workspace: &mut Vec<WorkspaceMutation>,
        node: &Node,
        execution: &NodeExecutionId,
        scope_reference: &ScopeReference,
        config: &milkdrift_blueprint::ReducerConfig,
        revision: &BlueprintRevision,
    ) -> Result<(), RuntimeError> {
        if matches!(config.strategy(), ReducerStrategy::Capability(_)) {
            return Ok(());
        }
        if !projection
            .node_executions()
            .get(execution)
            .is_some_and(|value| value.outputs().is_empty())
        {
            return self.complete_deterministic(
                run,
                occurred_at,
                projection,
                events,
                node,
                execution,
            );
        }
        let values = self.ordered_reducer_references(
            revision,
            projection,
            node,
            config.input_port(),
            scope_reference,
            workspace,
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
                        run,
                        occurred_at,
                        projection,
                        events,
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
                        run,
                        occurred_at,
                        projection,
                        events,
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
                let entry = self.projected_workspace_value(projection, reference, workspace)?;
                let artifact = entry.value().as_artifact().cloned();
                (entry.value().clone(), artifact)
            }
            ReducerStrategy::Capability(_) => return Ok(()),
        };
        let entry =
            self.projected_output_entry(projection, scope_reference, key, value, workspace)?;
        let reference = entry.reference().clone();
        workspace.push(WorkspaceMutation::PutValue { entry });
        self.push_projected_event(
            run,
            occurred_at,
            projection,
            events,
            RunEventKind::DeterministicOutputPublished {
                execution: execution.clone(),
                value: reference,
                artifact,
            },
        )?;
        self.complete_deterministic(run, occurred_at, projection, events, node, execution)
    }
}
