//! Subworkflow intent creation and attached child-aggregate lifecycle.

use super::super::RuntimeService;
use super::super::support::{CommandPlan, bounded_projection_set};
use crate::projection::{RunLifecycle, RunProjection, SubworkflowState};
use crate::{RunCommand, RunCommandDocument, RuntimeError, SystemTransition};
use milkdrift_blueprint::{Node, NodeKind, PortId, RevisionId};
use milkdrift_persistence::{
    BoundedDetail, CommandDisposition, NodeExecutionId, NodeOutcome, PageSize, Reason,
    RunEventEnvelope, RunEventKind, RunSequence, SubworkflowOwnership, TimestampMillis,
    WorkspaceMutation,
};
use milkdrift_workspace::{
    RunId, ScopeId, ScopeReference, SubworkflowId, ValueKey, WorkspaceScope, WorkspaceValueEntry,
    WorkspaceValueReference,
};
use std::collections::BTreeMap;
use std::sync::atomic::Ordering;

struct ActiveChild {
    subworkflow: SubworkflowId,
    parent_execution: NodeExecutionId,
    run: RunId,
    revision: RevisionId,
    scope: WorkspaceScope,
    inputs: Vec<WorkspaceValueReference>,
    state: SubworkflowState,
}

impl RuntimeService {
    pub(in crate::engine) fn drive_child_aggregates(
        &self,
        now: TimestampMillis,
        limit: PageSize,
    ) -> Result<(), RuntimeError> {
        for summary in self.next_nonterminal_page(&self.child_cursor, limit, "child-aggregate")? {
            if self.structured_scan_budget.load(Ordering::Acquire) == 0 {
                break;
            }
            let parent = self.projection(&summary.run)?;
            for child in self.active_children(&summary.run, &parent)? {
                let child_head = self.ensure_child_created(now, &parent, &child)?;
                let child_projection = self.advance_child_lifecycle(now, child_head, &child)?;
                self.observe_child_terminal(now, &summary.run, &child, &child_projection)?;
            }
        }
        Ok(())
    }

    fn active_children(
        &self,
        parent_run: &RunId,
        parent: &RunProjection,
    ) -> Result<Vec<ActiveChild>, RuntimeError> {
        let claimed = self.claim_structured_scan_visits(parent.active_subworkflow_ids().len());
        let mut allowance = claimed;
        bounded_projection_set(
            parent_run,
            parent.active_subworkflow_ids(),
            &self.child_subworkflow_cursors,
            &mut allowance,
            "active child scan cursor",
        )?
        .into_iter()
        .map(|subworkflow| {
            let child = parent.subworkflows().get(&subworkflow).ok_or_else(|| {
                RuntimeError::InvalidHistory("active child frontier identity is absent".to_owned())
            })?;
            Ok(ActiveChild {
                subworkflow: child.subworkflow().clone(),
                parent_execution: child.parent_execution().clone(),
                run: child.child_run().clone(),
                revision: child.child_revision().clone(),
                scope: child.scope().clone(),
                inputs: child.inputs().to_vec(),
                state: child.state(),
            })
        })
        .collect()
    }

    fn ensure_child_created(
        &self,
        now: TimestampMillis,
        parent: &RunProjection,
        child: &ActiveChild,
    ) -> Result<RunSequence, RuntimeError> {
        let child_blueprint = self.load_validated_revision(&child.revision, None)?;
        let root_scope =
            WorkspaceScope::run_root(child.run.clone(), child_root_scope(parent, child)?);
        let mut inputs_by_key = BTreeMap::new();
        for reference in &child.inputs {
            let entry = self.projected_workspace_value(parent, reference, &[])?;
            if inputs_by_key
                .insert(entry.reference().key().clone(), entry.value().clone())
                .is_some()
            {
                return Err(RuntimeError::InvalidTransition(
                    "subworkflow inputs must map to distinct child keys".to_owned(),
                ));
            }
        }
        let inputs: Vec<_> = inputs_by_key
            .into_iter()
            .map(|(key, value)| {
                WorkspaceValueEntry::initial(root_scope.reference().clone(), key, value)
            })
            .collect();
        let budget = parent.workspace_budget().ok_or_else(|| {
            RuntimeError::InvalidHistory("parent run has no workspace budget".to_owned())
        })?;
        let child_head = self.store.head(&child.run)?;
        if child_head != RunSequence::ZERO {
            let existing = self.projection(&child.run)?;
            let expected_references: Vec<_> = inputs
                .iter()
                .map(|entry| entry.reference().clone())
                .collect();
            if existing.run_id() != Some(&child.run)
                || existing.workflow() != Some(child_blueprint.semantic().workflow())
                || existing.revision() != Some(&child.revision)
                || existing.root_scope() != Some(&root_scope)
                || existing.workspace_budget() != Some(budget)
                || existing.inputs() != expected_references
            {
                return Err(RuntimeError::InvalidHistory(
                    "pre-existing child run does not match its parent-bound creation facts"
                        .to_owned(),
                ));
            }
            for expected in &inputs {
                let actual =
                    self.projected_workspace_value(&existing, expected.reference(), &[])?;
                if actual != *expected {
                    return Err(RuntimeError::InvalidHistory(
                        "pre-existing child run input does not match its parent-bound value"
                            .to_owned(),
                    ));
                }
            }
            return Ok(child_head);
        }
        let create = RunCommandDocument::new(
            self.next_command_id()?,
            child.run.clone(),
            self.config.internal_actor.clone(),
            RunSequence::ZERO,
            now,
            Reason::new("parent materialized a pinned child run aggregate")?,
            Vec::new(),
            RunCommand::CreateRun {
                workflow: child_blueprint.semantic().workflow().clone(),
                revision: child.revision.clone(),
                root_scope,
                workspace_budget: budget.clone(),
                inputs,
            },
        )?;
        let created = self.handle_command(&create)?;
        if created.result().disposition() != CommandDisposition::Accepted {
            return Err(RuntimeError::InvalidTransition(
                "pinned child run creation was durably rejected".to_owned(),
            ));
        }
        Ok(created.result().resulting_sequence())
    }

    fn advance_child_lifecycle(
        &self,
        now: TimestampMillis,
        child_head: RunSequence,
        attached: &ActiveChild,
    ) -> Result<RunProjection, RuntimeError> {
        let mut child = self.projection(&attached.run)?;
        if attached.state == SubworkflowState::Cancelling
            && !child.lifecycle().is_completed()
            && child.lifecycle() != RunLifecycle::Cancelling
        {
            let cancel = RunCommandDocument::new(
                self.next_command_id()?,
                attached.run.clone(),
                self.config.internal_actor.clone(),
                child.sequence(),
                now,
                Reason::new("attached parent propagated structured cancellation")?,
                Vec::new(),
                RunCommand::RequestCancellation,
            )?;
            let _ = self.handle_command(&cancel)?;
            child = self.projection(&attached.run)?;
        } else if child.lifecycle() == RunLifecycle::Created {
            let start = RunCommandDocument::new(
                self.next_command_id()?,
                attached.run.clone(),
                self.config.internal_actor.clone(),
                child_head,
                now,
                Reason::new("parent started its pinned child run")?,
                Vec::new(),
                RunCommand::StartRun,
            )?;
            let _ = self.handle_command(&start)?;
            child = self.projection(&attached.run)?;
        }
        Ok(child)
    }

    fn observe_child_terminal(
        &self,
        now: TimestampMillis,
        parent_run: &RunId,
        attached: &ActiveChild,
        child: &RunProjection,
    ) -> Result<(), RuntimeError> {
        let Some(terminal) = child.terminal() else {
            return Ok(());
        };
        let parent = self.projection(parent_run)?;
        let child_view = parent
            .subworkflows()
            .get(&attached.subworkflow)
            .ok_or_else(|| {
                RuntimeError::InvalidHistory("parent lost its durable child link".to_owned())
            })?;
        if child_view.is_completed() {
            return Ok(());
        }
        let parent_execution = parent
            .node_executions()
            .get(&attached.parent_execution)
            .ok_or_else(|| {
                RuntimeError::InvalidHistory("subworkflow parent execution is absent".to_owned())
            })?;
        let parent_revision = self.revision_for_execution(&parent, &attached.parent_execution)?;
        let parent_node = parent_revision
            .semantic()
            .nodes()
            .get(parent_execution.node())
            .ok_or_else(|| {
                RuntimeError::InvalidHistory("subworkflow parent node is absent".to_owned())
            })?;
        let publish_on_parent = matches!(parent_node.kind(), NodeKind::Subworkflow { .. });
        let import_scope = attached.scope.reference().clone();
        let mut plan = CommandPlan::one(RunEventKind::SubworkflowTerminal {
            subworkflow: attached.subworkflow.clone(),
            child_run: attached.run.clone(),
            outcome: terminal.outcome(),
            outputs: terminal.outputs().to_vec(),
            cost_micros: child.resource_usage().cost_micros().clone(),
        });
        for child_value in terminal.outputs() {
            let source = self.projected_workspace_value(child, child_value, &[])?;
            let imported = self.projected_imported_output_entry(
                &parent,
                &import_scope,
                source.reference().key().clone(),
                child_value.clone(),
                source.value().clone(),
                &plan.workspace,
            )?;
            let parent_value = imported.reference().clone();
            plan.workspace
                .push(WorkspaceMutation::PutValue { entry: imported });
            plan.events.push(RunEventKind::SubworkflowOutputImported {
                subworkflow: attached.subworkflow.clone(),
                child_value: child_value.clone(),
                parent_value: parent_value.clone(),
            });
            if publish_on_parent {
                let published = self.projected_output_entry(
                    &parent,
                    parent_execution.scope(),
                    source.reference().key().clone(),
                    source.value().clone(),
                    &plan.workspace,
                )?;
                let published_value = published.reference().clone();
                plan.workspace
                    .push(WorkspaceMutation::PutValue { entry: published });
                plan.events
                    .push(RunEventKind::DeterministicOutputPublished {
                        execution: attached.parent_execution.clone(),
                        value: published_value,
                        artifact: None,
                    });
            }
        }
        let _ = self.commit_internal_plan(
            parent_run,
            now,
            SystemTransition::ObserveChildTerminal,
            plan,
        )?;
        Ok(())
    }

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

fn child_root_scope(parent: &RunProjection, child: &ActiveChild) -> Result<ScopeId, RuntimeError> {
    let parent_run = parent.run_id().ok_or_else(|| {
        RuntimeError::InvalidHistory("subworkflow parent aggregate identity is absent".to_owned())
    })?;
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"milkdrift.attached-child-root-scope.v1\0");
    for component in [
        parent_run.as_str(),
        child.subworkflow.as_str(),
        child.run.as_str(),
    ] {
        let length = u64::try_from(component.len()).map_err(|_error| {
            RuntimeError::InvalidHistory("child identity length exceeds u64".to_owned())
        })?;
        hasher.update(&length.to_be_bytes());
        hasher.update(component.as_bytes());
    }
    ScopeId::new(format!("child-root-{}", hasher.finalize().to_hex()))
        .map_err(|error| RuntimeError::InvalidHistory(error.to_string()))
}
