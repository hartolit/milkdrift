use milkdrift_persistence::{RunEventEnvelope, RunEventKind, RunOutcome, SubworkflowOwnership};
use milkdrift_workspace::ScopeKind;

use crate::RuntimeError;

use super::helpers::{ensure_unique, invalid_at};
use super::run::RunProjection;
use super::structured::{
    IterationState, SubworkflowOutputImport, SubworkflowProjection, SubworkflowState,
};

impl RunProjection {
    pub(super) fn apply_subworkflow_kind(
        &mut self,
        event: &RunEventEnvelope,
    ) -> Result<(), RuntimeError> {
        let sequence = event.sequence();
        match event.kind() {
            RunEventKind::SubworkflowCreated {
                subworkflow,
                parent_execution,
                child_run,
                child_revision,
                scope,
                ownership,
                inputs,
            } => {
                let parent_scope = self.execution(parent_execution, event)?.scope.clone();
                let valid_parent_scope = scope.parent() == Some(&parent_scope)
                    || self.iterations.values().any(|iteration| {
                        iteration.repeat_execution == *parent_execution
                            && iteration.state == IterationState::Active
                            && scope.parent() == Some(iteration.scope.reference())
                    });
                if self.subworkflows.contains_key(subworkflow)
                    || self.child_runs.contains(child_run)
                    || child_run == event.run_id()
                    || !matches!(scope.kind(), ScopeKind::Subworkflow { subworkflow: identity } if identity == subworkflow)
                    || !valid_parent_scope
                {
                    return Err(invalid_at(
                        event,
                        "subworkflow identity, child run, scope kind, or parent is invalid",
                    ));
                }
                ensure_unique(inputs, event, "subworkflow input")?;
                for input in inputs {
                    if input.scope() != scope.reference() {
                        self.validate_known_workspace_value(input, event)?;
                        let accessible_from_parent = scope.parent().is_some_and(|parent| {
                            input.scope() == parent
                                || self.scope_descends_from(parent, input.scope())
                        });
                        if !accessible_from_parent {
                            return Err(invalid_at(
                                event,
                                "pre-existing subworkflow input is not owned by an ancestor scope",
                            ));
                        }
                    }
                }
                self.register_child_scope(scope, event)?;
                for input in inputs {
                    if input.scope() == scope.reference() {
                        self.record_workspace_value(input, event)?;
                    }
                }
                self.child_runs.insert(child_run.clone());
                self.subworkflows.insert(
                    subworkflow.clone(),
                    SubworkflowProjection {
                        subworkflow: subworkflow.clone(),
                        parent_execution: parent_execution.clone(),
                        created_sequence: sequence,
                        child_run: child_run.clone(),
                        child_revision: child_revision.clone(),
                        scope: scope.clone(),
                        ownership: *ownership,
                        inputs: inputs.clone(),
                        state: SubworkflowState::Active,
                        cancellation_reason: None,
                        outputs: Vec::new(),
                        imports: Vec::new(),
                    },
                );
                self.active_subworkflow_ids.insert(subworkflow.clone());
                if *ownership == SubworkflowOwnership::Attached {
                    self.active_attached_subworkflow_ids
                        .insert(subworkflow.clone());
                }
                self.adjust_structured_child_count(parent_execution, true, event)?;
            }
            RunEventKind::SubworkflowTerminal {
                subworkflow,
                child_run,
                outcome,
                outputs,
                cost_micros,
                usage: child_usage,
            } => {
                let child = self.subworkflows.get(subworkflow).ok_or_else(|| {
                    invalid_at(event, "child terminal references an unknown subworkflow")
                })?;
                if child.child_run != *child_run
                    || child.is_completed()
                    || (*outcome == RunOutcome::Cancelled
                        && child.state != SubworkflowState::Cancelling)
                {
                    return Err(invalid_at(
                        event,
                        "child terminal is duplicate or names the wrong run",
                    ));
                }
                ensure_unique(outputs, event, "subworkflow output")?;
                for output in outputs {
                    if output.scope().run() != child_run {
                        return Err(invalid_at(
                            event,
                            "subworkflow terminal output belongs to another run",
                        ));
                    }
                }
                let parent_execution = child.parent_execution.clone();
                let child = self
                    .subworkflows
                    .get_mut(subworkflow)
                    .ok_or_else(|| invalid_at(event, "unknown subworkflow"))?;
                child.state = SubworkflowState::Terminal(*outcome);
                child.outputs = outputs.clone();
                let usage = self
                    .subworkflow_usage_by_execution
                    .entry(parent_execution.clone())
                    .or_default();
                usage.completed_children =
                    usage.completed_children.checked_add(1).unwrap_or_else(|| {
                        usage.overflowed = true;
                        usage.completed_children
                    });
                if *outcome != RunOutcome::Succeeded {
                    usage.failed_children =
                        usage.failed_children.checked_add(1).unwrap_or_else(|| {
                            usage.overflowed = true;
                            usage.failed_children
                        });
                }
                if !child_usage.cost_micros.is_empty() && child_usage.cost_micros != *cost_micros {
                    return Err(invalid_at(
                        event,
                        "child compact usage contradicts its legacy cost summary",
                    ));
                }
                for (currency, cost) in cost_micros {
                    let total = usage.cost_micros.entry(currency.clone()).or_default();
                    if let Some(next) = total.checked_add(*cost) {
                        *total = next;
                    } else {
                        usage.overflowed = true;
                    }
                }
                macro_rules! add_usage {
                    ($field:ident) => {
                        if let Some(next) = usage.$field.checked_add(child_usage.$field) {
                            usage.$field = next;
                        } else {
                            usage.overflowed = true;
                        }
                    };
                }
                macro_rules! add_optional_usage {
                    ($field:ident) => {
                        usage.$field = match (usage.$field, child_usage.$field) {
                            (None, value) => value,
                            (value, None) => value,
                            (Some(left), Some(right)) => match left.checked_add(right) {
                                Some(value) => Some(value),
                                None => {
                                    usage.overflowed = true;
                                    Some(left)
                                }
                            },
                        };
                    };
                }
                add_optional_usage!(input_units);
                add_optional_usage!(output_units);
                add_usage!(artifact_bytes);
                add_usage!(process_invocations);
                add_usage!(model_invocations);
                add_usage!(unknown_input_usage);
                add_usage!(unknown_output_usage);
                add_usage!(unknown_cost_usage);
                self.active_subworkflow_ids.remove(subworkflow);
                self.active_attached_subworkflow_ids.remove(subworkflow);
                self.adjust_structured_child_count(&parent_execution, false, event)?;
            }
            RunEventKind::SubworkflowOutputImported {
                subworkflow,
                child_value,
                parent_value,
            } => {
                let child = self.subworkflows.get(subworkflow).ok_or_else(|| {
                    invalid_at(event, "output import references an unknown subworkflow")
                })?;
                let parent_scope = self
                    .execution(&child.parent_execution, event)?
                    .scope
                    .clone();
                if !child.is_completed()
                    || child_value.scope().run() != &child.child_run
                    || !child.outputs.contains(child_value)
                    || child.imports.iter().any(|import| {
                        import.child_value == *child_value || import.parent_value == *parent_value
                    })
                    || self.workspace_values.contains(parent_value)
                {
                    return Err(invalid_at(
                        event,
                        "subworkflow import is duplicate or not backed by its terminal child output",
                    ));
                }
                self.validate_workspace_value(parent_value, event)?;
                if !self.scope_descends_from(parent_value.scope(), &parent_scope) {
                    return Err(invalid_at(
                        event,
                        "subworkflow import target is outside its parent execution scope",
                    ));
                }
                self.subworkflows
                    .get_mut(subworkflow)
                    .ok_or_else(|| invalid_at(event, "unknown subworkflow"))?
                    .imports
                    .push(SubworkflowOutputImport {
                        child_value: child_value.clone(),
                        parent_value: parent_value.clone(),
                        sequence,
                    });
                self.record_workspace_value(parent_value, event)?;
            }
            RunEventKind::SubworkflowCancellationRequested {
                subworkflow,
                child_run,
                reason,
            } => {
                let child = self.subworkflows.get_mut(subworkflow).ok_or_else(|| {
                    invalid_at(
                        event,
                        "child cancellation references an unknown subworkflow",
                    )
                })?;
                if child.child_run != *child_run
                    || child.ownership != SubworkflowOwnership::Attached
                    || child.state != SubworkflowState::Active
                {
                    return Err(invalid_at(
                        event,
                        "child cancellation is duplicate, detached, or mismatched",
                    ));
                }
                child.state = SubworkflowState::Cancelling;
                child.cancellation_reason = Some(reason.clone());
            }
            _ => unreachable!(
                "structured dispatch owns subworkflow child ownership and imports routing"
            ),
        }
        Ok(())
    }
}
