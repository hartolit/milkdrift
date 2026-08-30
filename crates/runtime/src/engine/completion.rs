//! Structured completion, branch/join closure, successor materialization, and signal draining.

use super::support::{
    ResolvedInputValue, artifact_reference_as_bounded, control_nodes_before_join,
    execution_branch_state, node_execution_mode, node_occurrence_exists_for_current_pin,
    predecessors_ready, run_drain_reason, scope_has_inactive_branch,
    source_execution_is_valid_for_occurrence, wait_signal_matches, workspace_value_as_bounded,
};
use super::{RuntimeService, STRUCTURED_EVENT_SOFT_LIMIT};
use crate::projection::{BranchState, NodeExecutionState, RunLifecycle, RunProjection};
use crate::{EvaluationContext, RuntimeError};
use milkdrift_blueprint::{
    BindingSource, BlueprintRevision, EdgeKind, JoinPolicy, Node, NodeKind, PortId, TerminalOutcome,
};
use milkdrift_capability::{BoundedJson, ErrorClass};
use milkdrift_persistence::{
    BoundedDetail, BranchResultReference, JoinRule, MAX_WORKSPACE_MUTATIONS_PER_COMMIT,
    NodeExecutionId, NodeOutcome, Reason, RunEventEnvelope, RunEventKind, RunOutcome,
    TimestampMillis, WaitSatisfaction, WorkspaceMutation,
};
use milkdrift_workspace::{
    RunId, ScopeKind, ScopeReference, ValueKey, WorkspaceValue, WorkspaceValueEntry,
    WorkspaceValueReference,
};
use std::collections::BTreeSet;

impl RuntimeService {
    pub(super) fn complete_deterministic(
        &self,
        run: &RunId,
        occurred_at: TimestampMillis,
        projection: &mut RunProjection,
        events: &mut Vec<RunEventEnvelope>,
        node: &Node,
        execution: &NodeExecutionId,
    ) -> Result<(), RuntimeError> {
        self.complete_deterministic_with_outcome(
            run,
            occurred_at,
            projection,
            events,
            node,
            execution,
            NodeOutcome::Succeeded,
            None,
        )
    }

    #[allow(clippy::too_many_arguments)] // One atomic runtime transition owns these borrowed state views and durable outputs.
    pub(super) fn complete_deterministic_with_outcome(
        &self,
        run: &RunId,
        occurred_at: TimestampMillis,
        projection: &mut RunProjection,
        events: &mut Vec<RunEventEnvelope>,
        node: &Node,
        execution: &NodeExecutionId,
        outcome: NodeOutcome,
        detail: Option<BoundedDetail>,
    ) -> Result<(), RuntimeError> {
        if outcome == NodeOutcome::Cancelled {
            return self.push_projected_event(
                run,
                occurred_at,
                projection,
                events,
                RunEventKind::NodeExecutionCancelledBeforeDispatch {
                    execution: execution.clone(),
                    reason: Reason::new(
                        "deterministic execution was cancelled by its structured owner",
                    )?,
                },
            );
        }
        self.push_projected_event(
            run,
            occurred_at,
            projection,
            events,
            RunEventKind::DeterministicNodeTerminal {
                execution: execution.clone(),
                outcome,
                error_class: matches!(outcome, NodeOutcome::Failed | NodeOutcome::Rejected)
                    .then_some(ErrorClass::Unknown),
                detail,
            },
        )?;
        if outcome == NodeOutcome::Succeeded
            && matches!(
                node.kind(),
                NodeKind::Fork { .. } | NodeKind::Terminal { .. }
            )
        {
            self.push_projected_event(
                run,
                occurred_at,
                projection,
                events,
                RunEventKind::StructuredSuccessorScanCompleted {
                    execution: execution.clone(),
                },
            )?;
        }
        Ok(())
    }

    pub(super) fn try_finalize_run(
        &self,
        run: &RunId,
        occurred_at: TimestampMillis,
        revision: &BlueprintRevision,
        projection: &mut RunProjection,
        events: &mut Vec<RunEventEnvelope>,
        workspace: &[WorkspaceMutation],
    ) -> Result<(), RuntimeError> {
        if projection.is_completed()
            || !projection.pending_successor_execution_ids().is_empty()
            || projection.has_active_owned_work()
        {
            return Ok(());
        }

        let mut terminal_executions: Vec<_> = projection
            .current_node_executions()
            .filter_map(|execution| {
                let node = revision.semantic().nodes().get(execution.node())?;
                match node.kind() {
                    NodeKind::Terminal { outcome } => {
                        Some((execution.created_sequence(), execution, *outcome))
                    }
                    _ => None,
                }
            })
            .collect();
        terminal_executions.sort_by(|left, right| {
            left.0
                .cmp(&right.0)
                .then_with(|| left.1.execution().cmp(right.1.execution()))
        });
        let joined_branches: BTreeSet<_> = projection
            .joins()
            .values()
            .flat_map(|join| join.branches().iter().map(|result| result.branch.clone()))
            .collect();
        let unjoined_failed_terminal =
            terminal_executions.iter().any(|(_, execution, terminal)| {
                if *terminal != TerminalOutcome::Failure {
                    return false;
                }
                projection
                    .branches()
                    .values()
                    .find(|branch| branch.children().contains(execution.execution()))
                    .is_none_or(|branch| !joined_branches.contains(branch.branch()))
            });
        let unjoined_failed_branch = projection.branches().values().any(|branch| {
            branch.state() == BranchState::Completed(RunOutcome::Failed)
                && !joined_branches.contains(branch.branch())
        });
        let outcome = if projection.lifecycle() == RunLifecycle::Cancelling {
            RunOutcome::Cancelled
        } else if let Some(termination) = projection.termination() {
            termination.outcome()
        } else if unjoined_failed_terminal
            || unjoined_failed_branch
            || (terminal_executions.is_empty()
                && projection.current_node_executions().any(|execution| {
                    matches!(
                        execution.state(),
                        NodeExecutionState::Terminal(NodeOutcome::Failed | NodeOutcome::Rejected)
                    )
                }))
        {
            RunOutcome::Failed
        } else {
            RunOutcome::Succeeded
        };
        if outcome == RunOutcome::Cancelled && projection.cancellation().is_none() {
            return Ok(());
        }
        let mut outputs = BTreeSet::new();
        let mut artifacts = BTreeSet::new();
        if let Some((_, terminal_execution, _)) = terminal_executions.last() {
            let terminal_node = revision
                .semantic()
                .nodes()
                .get(terminal_execution.node())
                .ok_or_else(|| {
                    RuntimeError::InvalidHistory("terminal node is absent".to_owned())
                })?;
            for (field, declaration) in revision.semantic().interface().outputs() {
                let _terminal_port = terminal_node
                    .data_inputs()
                    .keys()
                    .find(|port| port.as_str() == field.as_str());
                let resolved = terminal_execution
                    .outputs()
                    .iter()
                    .find(|output| output.value().key().as_str() == field.as_str())
                    .map(|output| output.value().clone());
                match resolved {
                    Some(reference) => {
                        if let Some(artifact) = self
                            .projected_workspace_value(projection, &reference, workspace)?
                            .value()
                            .as_artifact()
                            .cloned()
                            && projection.artifacts().contains_key(artifact.artifact())
                        {
                            artifacts.insert(artifact);
                        }
                        outputs.insert(reference);
                    }
                    None if declaration.is_required() && outcome == RunOutcome::Succeeded => {
                        return Err(RuntimeError::InvalidHistory(format!(
                            "required workflow output {field} is unresolved at terminal boundary"
                        )));
                    }
                    None => {}
                }
            }
        }
        self.push_projected_event(
            run,
            occurred_at,
            projection,
            events,
            RunEventKind::RunTerminal {
                outcome,
                outputs: outputs.into_iter().collect(),
                artifacts: artifacts.into_iter().collect(),
                reason: projection
                    .cancellation()
                    .map(|cancellation| cancellation.reason().clone())
                    .or_else(|| {
                        projection
                            .termination()
                            .map(|termination| termination.reason().clone())
                    }),
            },
        )
    }

    #[allow(clippy::too_many_arguments)] // One atomic runtime transition owns these borrowed state views and durable outputs.
    pub(super) fn materialize_success_terminal_outputs(
        &self,
        run: &RunId,
        occurred_at: TimestampMillis,
        revision: &BlueprintRevision,
        projection: &mut RunProjection,
        events: &mut Vec<RunEventEnvelope>,
        workspace: &mut Vec<WorkspaceMutation>,
        terminal_node: &Node,
        execution: &NodeExecutionId,
        scope: &ScopeReference,
    ) -> Result<bool, RuntimeError> {
        for (field, declaration) in revision.semantic().interface().outputs() {
            let port = PortId::new(field.as_str().to_owned())
                .map_err(|error| RuntimeError::Scheduling(error.to_string()))?;
            if projection
                .node_executions()
                .get(execution)
                .is_some_and(|view| {
                    view.outputs()
                        .iter()
                        .any(|output| output.value().key().as_str() == field.as_str())
                })
            {
                continue;
            }
            let Some(_port_declaration) = terminal_node.data_inputs().get(&port) else {
                if declaration.is_required() {
                    return Err(RuntimeError::InvalidHistory(format!(
                        "required terminal workflow output {field} has no declared terminal port"
                    )));
                }
                continue;
            };
            let mut resolved = self.resolve_node_port_inputs(
                revision,
                projection,
                terminal_node,
                &port,
                scope,
                workspace,
            )?;
            if resolved.is_empty() {
                if declaration.is_required() {
                    return Err(RuntimeError::Scheduling(format!(
                        "required terminal workflow output {field} did not resolve from immutable inputs"
                    )));
                }
                continue;
            }
            if resolved.len() != 1 {
                return Err(RuntimeError::InvalidHistory(format!(
                    "terminal workflow output {field} resolved to more than one exact value"
                )));
            }
            if events.len().saturating_add(2) >= STRUCTURED_EVENT_SOFT_LIMIT {
                return Ok(false);
            }
            let resolved = resolved.pop().ok_or_else(|| {
                RuntimeError::InvalidHistory(
                    "resolved terminal workflow output disappeared".to_owned(),
                )
            })?;
            let key = ValueKey::new(field.as_str().to_owned())
                .map_err(|error| RuntimeError::Scheduling(error.to_string()))?;
            let value = match resolved {
                ResolvedInputValue::Inline { value, .. } => WorkspaceValue::Json(value),
                ResolvedInputValue::Workspace(reference) => {
                    let entry =
                        self.projected_workspace_value(projection, &reference, workspace)?;
                    entry.value().clone()
                }
                ResolvedInputValue::Artifact(reference) => WorkspaceValue::Artifact(reference),
            };
            let artifact = value.as_artifact().cloned();
            if let Some(artifact) = &artifact
                && !projection.artifacts().contains_key(artifact.artifact())
            {
                let metadata = self.store.metadata(artifact.artifact())?.ok_or_else(|| {
                    RuntimeError::InvalidHistory(
                        "terminal output artifact metadata is absent".to_owned(),
                    )
                })?;
                if metadata.reference() != artifact {
                    return Err(RuntimeError::InvalidHistory(
                        "terminal output artifact metadata contradicts its binding".to_owned(),
                    ));
                }
                self.push_projected_event(
                    run,
                    occurred_at,
                    projection,
                    events,
                    RunEventKind::ArtifactPublished { metadata },
                )?;
            }
            let entry = self.projected_output_entry(projection, scope, key, value, workspace)?;
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
        }
        Ok(true)
    }

    pub(super) fn close_finished_branches(
        &self,
        run: &RunId,
        occurred_at: TimestampMillis,
        revision: &BlueprintRevision,
        projection: &mut RunProjection,
        events: &mut Vec<RunEventEnvelope>,
        scan_remaining: &mut usize,
    ) -> Result<(), RuntimeError> {
        let mut terminal = Vec::new();
        for branch_id in self.scan_branch_ids(run, projection, scan_remaining)? {
            let branch = projection.branches().get(&branch_id).ok_or_else(|| {
                RuntimeError::InvalidHistory("scanned branch identity is absent".to_owned())
            })?;
            if !matches!(
                branch.state(),
                BranchState::Active | BranchState::Cancelling
            ) || projection.branch_has_active_descendant_ownership(branch.branch())
            {
                continue;
            }
            let fork = projection
                .node_executions()
                .get(branch.fork_execution())
                .ok_or_else(|| {
                    RuntimeError::InvalidHistory("branch owner fork execution is absent".to_owned())
                })?;
            let branch_revision = self.revision_for_execution(projection, fork.execution())?;
            let mut frontier = Vec::new();
            for child_node in branch_revision.semantic().nodes().values() {
                let Some(child) = projection
                    .latest_descendant_execution(branch.scope().reference(), child_node.id())
                else {
                    continue;
                };
                let reaches_owning_join = branch_revision.semantic().edges().values().any(|edge| {
                    edge.kind() == EdgeKind::Control
                        && edge.source_node() == child.node()
                        && branch_revision
                            .semantic()
                            .nodes()
                            .get(edge.target_node())
                            .is_some_and(|target| {
                                matches!(
                                    target.kind(),
                                    NodeKind::Join { config } if config.fork() == fork.node()
                                )
                            })
                });
                let is_explicit_terminal = branch_revision
                    .semantic()
                    .nodes()
                    .get(child.node())
                    .is_some_and(|node| matches!(node.kind(), NodeKind::Terminal { .. }));
                let stopped_before_join =
                    child.state() != &NodeExecutionState::Terminal(NodeOutcome::Succeeded);
                if reaches_owning_join || is_explicit_terminal || stopped_before_join {
                    frontier.push(child);
                }
            }
            // A successfully completed nested fork is not the enclosing branch's
            // terminal frontier. Its inner join (or a later outer-scope successor)
            // must become durable before the enclosing branch may close.
            if frontier.is_empty() {
                continue;
            }
            let outcome = if branch.state() == BranchState::Cancelling {
                RunOutcome::Cancelled
            } else if frontier
                .iter()
                .all(|child| child.state() == &NodeExecutionState::Terminal(NodeOutcome::Succeeded))
            {
                RunOutcome::Succeeded
            } else if frontier.iter().all(|child| {
                matches!(
                    child.state(),
                    NodeExecutionState::Terminal(NodeOutcome::Succeeded | NodeOutcome::Cancelled)
                )
            }) {
                RunOutcome::Cancelled
            } else {
                RunOutcome::Failed
            };
            let owning_join =
                branch_revision
                    .semantic()
                    .nodes()
                    .values()
                    .find_map(|node| match node.kind() {
                        NodeKind::Join { config } if config.fork() == fork.node() => {
                            Some(node.id().clone())
                        }
                        NodeKind::Branch { .. }
                        | NodeKind::Fork { .. }
                        | NodeKind::Join { .. }
                        | NodeKind::Repeat { .. }
                        | NodeKind::Wait { .. }
                        | NodeKind::SignalWait { .. }
                        | NodeKind::Subworkflow { .. }
                        | NodeKind::Terminal { .. }
                        | NodeKind::Task { .. }
                        | NodeKind::Reducer { .. } => None,
                    });
            let mut outputs = BTreeSet::new();
            if let Some(ref owning_join) = owning_join {
                let branch_start = branch_revision
                    .semantic()
                    .edges()
                    .values()
                    .find(|edge| {
                        edge.kind() == EdgeKind::Control
                            && edge.source_node() == fork.node()
                            && edge.source_port() == branch.port()
                    })
                    .map(|edge| edge.target_node().clone())
                    .ok_or_else(|| {
                        RuntimeError::InvalidHistory(
                            "fork branch has no declared control-flow start".to_owned(),
                        )
                    })?;
                let branch_nodes =
                    control_nodes_before_join(&branch_revision, &branch_start, owning_join);
                let mut routes: Vec<_> = branch_revision
                    .semantic()
                    .edges()
                    .values()
                    .filter(|edge| {
                        edge.kind() == EdgeKind::Data
                            && branch_nodes.contains(edge.source_node())
                            && !branch_nodes.contains(edge.target_node())
                    })
                    .map(|edge| (edge.source_node().clone(), edge.source_port().clone()))
                    .collect();
                routes.sort();
                routes.dedup();
                for (source_node, source_port) in routes {
                    let selected = projection
                        .latest_descendant_execution(branch.scope().reference(), &source_node)
                        .filter(|execution| {
                            execution.state()
                                == &NodeExecutionState::Terminal(NodeOutcome::Succeeded)
                        })
                        .and_then(|execution| {
                            execution
                                .outputs()
                                .iter()
                                .filter(|output| {
                                    output.value().key().as_str() == source_port.as_str()
                                })
                                .max_by_key(|output| output.sequence())
                                .map(|output| output.value().clone())
                        });
                    if let Some(selected) = selected {
                        outputs.insert(selected);
                    }
                }
            }
            let join = owning_join.and_then(|join| {
                revision
                    .semantic()
                    .nodes()
                    .get(&join)
                    .filter(|node| {
                        matches!(node.kind(), NodeKind::Join { config } if config.fork() == fork.node())
                    })
                    .map(|node| (node.clone(), fork.scope().clone()))
            });
            terminal.push((branch.branch().clone(), outcome, outputs, join));
        }
        for (branch, outcome, outputs, join) in terminal {
            let join_needs_materialization = join.as_ref().is_some_and(|(node, scope)| {
                !node_occurrence_exists_for_current_pin(projection, node.id(), scope)
            });
            let join_needs_membership = join_needs_materialization
                && join.as_ref().is_some_and(|(_, scope)| {
                    projection
                        .scopes()
                        .get(scope)
                        .is_some_and(|scope| matches!(scope.kind(), ScopeKind::Branch { .. }))
                });
            let required_events = 1_usize
                .saturating_add(usize::from(join_needs_materialization))
                .saturating_add(usize::from(join_needs_membership));
            if events.len().saturating_add(required_events) > STRUCTURED_EVENT_SOFT_LIMIT {
                return Ok(());
            }
            self.push_projected_event(
                run,
                occurred_at,
                projection,
                events,
                RunEventKind::BranchTerminal {
                    branch,
                    outcome,
                    outputs: outputs.into_iter().collect(),
                },
            )?;
            if join_needs_materialization {
                let (join, scope) = join.ok_or_else(|| {
                    RuntimeError::InvalidHistory(
                        "branch join materialization candidate disappeared".to_owned(),
                    )
                })?;
                if !scope_has_inactive_branch(projection, &scope)
                    && predecessors_ready(revision, projection, &join, &scope)
                {
                    let execution = self.next_execution_id()?;
                    let owning_branch = projection.scopes().get(&scope).and_then(|scope| {
                        if let ScopeKind::Branch { branch } = scope.kind() {
                            Some(branch.clone())
                        } else {
                            None
                        }
                    });
                    self.push_projected_event(
                        run,
                        occurred_at,
                        projection,
                        events,
                        RunEventKind::NodeBecameEligible {
                            node: join.id().clone(),
                            execution: execution.clone(),
                            scope,
                            mode: node_execution_mode(&join),
                        },
                    )?;
                    if let Some(branch) = owning_branch {
                        self.push_projected_event(
                            run,
                            occurred_at,
                            projection,
                            events,
                            RunEventKind::BranchChildAdded { branch, execution },
                        )?;
                    }
                }
            }
        }
        Ok(())
    }

    #[allow(deprecated)] // Legacy join handling remains isolated to the compatibility transition.
    #[allow(clippy::too_many_arguments)] // One atomic runtime transition owns these borrowed state views and durable outputs.
    pub(super) fn try_satisfy_join(
        &self,
        run: &RunId,
        occurred_at: TimestampMillis,
        _revision: &BlueprintRevision,
        projection: &mut RunProjection,
        events: &mut Vec<RunEventEnvelope>,
        node: &Node,
        execution: &NodeExecutionId,
        config: &milkdrift_blueprint::JoinConfig,
    ) -> Result<(), RuntimeError> {
        let join_scope = projection
            .node_executions()
            .get(execution)
            .ok_or_else(|| RuntimeError::InvalidHistory("join execution is absent".to_owned()))?
            .scope()
            .clone();
        let fork_execution = projection
            .executions_for_node(config.fork())
            .filter(|value| {
                value.scope() == &join_scope
                    && source_execution_is_valid_for_occurrence(
                        projection,
                        *value,
                        node.id(),
                        &join_scope,
                    )
            })
            .last()
            .map(|value| value.execution().clone());
        let Some(fork_execution) = fork_execution else {
            return Ok(());
        };
        let mut completed = Vec::new();
        let mut active = Vec::new();
        for branch in projection.branches_for_fork(&fork_execution) {
            match branch.state() {
                BranchState::Completed(outcome) => completed.push(BranchResultReference {
                    branch: branch.branch().clone(),
                    scope: branch.scope().reference().clone(),
                    outcome,
                    outputs: branch.outputs().to_vec(),
                }),
                BranchState::Active | BranchState::Cancelling => {
                    active.push(branch.branch().clone());
                }
                BranchState::Retained => {}
            }
        }
        completed.sort_by(|left, right| left.branch.cmp(&right.branch));
        active.sort();
        let (rule, selected, retained) = match config.policy() {
            JoinPolicy::All if active.is_empty() && !completed.is_empty() => {
                (JoinRule::All, completed, Vec::new())
            }
            JoinPolicy::Any if !completed.is_empty() => {
                for branch in &active {
                    if projection
                        .branches()
                        .get(branch)
                        .is_some_and(|branch| branch.state() == BranchState::Active)
                    {
                        self.push_projected_event(
                            run,
                            occurred_at,
                            projection,
                            events,
                            RunEventKind::BranchCancellationRequested {
                                branch: branch.clone(),
                                reason: Reason::new(
                                    "any-completion join cancelled an unfinished losing branch",
                                )?,
                            },
                        )?;
                    }
                }
                (JoinRule::AnyCompletion, completed, Vec::new())
            }
            JoinPolicy::FirstSuccess | JoinPolicy::AnySuccessful => {
                let has_success = completed
                    .iter()
                    .any(|result| result.outcome == RunOutcome::Succeeded);
                if !has_success && active.is_empty() {
                    return self.complete_deterministic_with_outcome(
                        run,
                        occurred_at,
                        projection,
                        events,
                        node,
                        execution,
                        NodeOutcome::Failed,
                        Some(BoundedDetail::new(
                            "all fork branches terminated without a successful result",
                        )?),
                    );
                }
                if !has_success {
                    return Ok(());
                }
                for branch in &active {
                    let state = projection
                        .branches()
                        .get(branch)
                        .map(|branch| branch.state());
                    if state == Some(BranchState::Active) {
                        self.push_projected_event(
                            run,
                            occurred_at,
                            projection,
                            events,
                            RunEventKind::BranchCancellationRequested {
                                branch: branch.clone(),
                                reason: Reason::new(
                                    "first-success join cancelled an unfinished losing branch",
                                )?,
                            },
                        )?;
                    }
                }
                (JoinRule::FirstSuccess, completed, Vec::new())
            }
            JoinPolicy::Quorum(required) => {
                let required_usize = usize::from(required);
                let successes = completed
                    .iter()
                    .filter(|result| result.outcome == RunOutcome::Succeeded)
                    .count();
                if successes < required_usize && active.is_empty() {
                    return self.complete_deterministic_with_outcome(
                        run,
                        occurred_at,
                        projection,
                        events,
                        node,
                        execution,
                        NodeOutcome::Failed,
                        Some(BoundedDetail::new(
                            "all fork branches terminated before the required quorum was reached",
                        )?),
                    );
                }
                if successes < required_usize {
                    return Ok(());
                }
                for branch in &active {
                    let state = projection
                        .branches()
                        .get(branch)
                        .map(|branch| branch.state());
                    if state == Some(BranchState::Active) {
                        self.push_projected_event(
                            run,
                            occurred_at,
                            projection,
                            events,
                            RunEventKind::BranchCancellationRequested {
                                branch: branch.clone(),
                                reason: Reason::new(
                                    "quorum join cancelled an unfinished losing branch",
                                )?,
                            },
                        )?;
                    }
                }
                (
                    JoinRule::Quorum {
                        required: u32::from(required),
                    },
                    completed,
                    Vec::new(),
                )
            }
            JoinPolicy::All | JoinPolicy::Any => return Ok(()),
        };
        self.push_projected_event(
            run,
            occurred_at,
            projection,
            events,
            RunEventKind::JoinSatisfied {
                execution: execution.clone(),
                rule,
                branches: selected,
                retained_branches: retained,
            },
        )?;
        self.complete_deterministic(run, occurred_at, projection, events, node, execution)
    }

    pub(super) fn add_ready_successors(
        &self,
        run: &RunId,
        occurred_at: TimestampMillis,
        revision: &BlueprintRevision,
        projection: &mut RunProjection,
        events: &mut Vec<RunEventEnvelope>,
        scan_remaining: &mut usize,
    ) -> Result<(), RuntimeError> {
        if run_drain_reason(projection).is_some() {
            return Ok(());
        }
        let mut candidates = BTreeSet::new();
        let requested = (*scan_remaining).min(projection.pending_successor_execution_ids().len());
        let claimed = self.claim_structured_scan_visits(requested);
        let pending_sources: Vec<_> = projection
            .pending_successor_execution_ids()
            .iter()
            .take(claimed)
            .cloned()
            .collect();
        *scan_remaining = scan_remaining.saturating_sub(pending_sources.len());
        let mut processed_sources = Vec::with_capacity(pending_sources.len());
        for source_execution in pending_sources {
            let Some(execution) = projection.node_executions().get(&source_execution) else {
                return Err(RuntimeError::InvalidHistory(
                    "scanned successor execution identity is absent".to_owned(),
                ));
            };
            if let Some(branch) = projection
                .branch_for_execution(execution.execution())
                .filter(|branch| matches!(branch.state(), BranchState::Completed(_)))
            {
                let fork = projection
                    .current_node_execution(branch.fork_execution())
                    .ok_or_else(|| {
                        RuntimeError::InvalidHistory(
                            "completed branch owner fork is absent".to_owned(),
                        )
                    })?;
                if let Some(join) = revision.semantic().nodes().values().find(|target| {
                    matches!(target.kind(), NodeKind::Join { config } if config.fork() == fork.node())
                }) {
                    candidates.insert((join.id().clone(), fork.scope().clone()));
                }
            }
            if execution.state() != &NodeExecutionState::Terminal(NodeOutcome::Succeeded)
                || !execution.is_current_epoch()
                || execution_branch_state(projection, execution.execution())
                    .is_some_and(|state| state != BranchState::Active)
            {
                processed_sources.push(source_execution);
                continue;
            }
            let source_node = execution.node().clone();
            let source_scope = execution.scope().clone();
            let Some(source) = revision.semantic().nodes().get(&source_node) else {
                // Reconciliation may prospectively remove a node while preserving
                // its immutable completed execution. It has no successors in the
                // adopted graph and must remain inert rather than being reinterpreted.
                processed_sources.push(source_execution);
                continue;
            };
            if matches!(
                source.kind(),
                NodeKind::Fork { .. } | NodeKind::Terminal { .. }
            ) {
                processed_sources.push(source_execution);
                continue;
            }
            let selected_port = projection.route_for_execution(&source_execution);
            for edge in revision
                .semantic()
                .edges()
                .values()
                .filter(|edge| edge.source_node() == &source_node)
            {
                let admits_target = match edge.kind() {
                    EdgeKind::Control => {
                        selected_port.is_none_or(|port| edge.source_port() == port)
                    }
                    EdgeKind::Data => !revision.semantic().edges().values().any(|candidate| {
                        candidate.kind() == EdgeKind::Control
                            && candidate.target_node() == edge.target_node()
                    }),
                };
                if admits_target {
                    candidates.insert((edge.target_node().clone(), source_scope.clone()));
                }
            }
            processed_sources.push(source_execution);
        }

        for (target, scope) in candidates {
            if events.len() >= STRUCTURED_EVENT_SOFT_LIMIT {
                return Ok(());
            }
            let target_node = revision.semantic().nodes().get(&target).ok_or_else(|| {
                RuntimeError::InvalidHistory("control edge target is absent".to_owned())
            })?;
            if scope_has_inactive_branch(projection, &scope) {
                continue;
            }
            if node_occurrence_exists_for_current_pin(projection, &target, &scope) {
                continue;
            }
            if !predecessors_ready(revision, projection, target_node, &scope) {
                continue;
            }
            let execution = self.next_execution_id()?;
            let owning_branch = projection.scopes().get(&scope).and_then(|scope| {
                if let ScopeKind::Branch { branch } = scope.kind() {
                    Some(branch.clone())
                } else {
                    None
                }
            });
            self.push_projected_event(
                run,
                occurred_at,
                projection,
                events,
                RunEventKind::NodeBecameEligible {
                    node: target,
                    execution: execution.clone(),
                    scope,
                    mode: node_execution_mode(target_node),
                },
            )?;
            if let Some(branch) = owning_branch {
                self.push_projected_event(
                    run,
                    occurred_at,
                    projection,
                    events,
                    RunEventKind::BranchChildAdded { branch, execution },
                )?;
            }
        }
        for execution in processed_sources {
            if events.len() >= STRUCTURED_EVENT_SOFT_LIMIT {
                return Ok(());
            }
            self.push_projected_event(
                run,
                occurred_at,
                projection,
                events,
                RunEventKind::StructuredSuccessorScanCompleted { execution },
            )?;
        }
        Ok(())
    }

    pub(super) fn evaluation_context(
        &self,
        node: &Node,
        projection: &RunProjection,
        occurrence_scope: &ScopeReference,
        pending_workspace: &[WorkspaceMutation],
    ) -> Result<EvaluationContext, RuntimeError> {
        let mut context = EvaluationContext::default();
        for port in node.data_inputs().values() {
            let Some(source) = port.binding() else {
                continue;
            };
            if matches!(source, BindingSource::Literal { .. }) {
                continue;
            }
            let Some(resolved) = self.resolve_optional_binding(
                projection,
                node.id(),
                occurrence_scope,
                source,
                pending_workspace,
                false,
            )?
            else {
                continue;
            };
            let value = match resolved {
                ResolvedInputValue::Inline { value, .. } => value,
                ResolvedInputValue::Workspace(reference) => {
                    let entry =
                        self.projected_workspace_value(projection, &reference, pending_workspace)?;
                    workspace_value_as_bounded(entry.value())?
                }
                ResolvedInputValue::Artifact(reference) => {
                    artifact_reference_as_bounded(&reference)?
                }
            };
            context.insert(source, value)?;
        }
        Ok(context)
    }

    pub(super) fn projected_output_entry(
        &self,
        projection: &RunProjection,
        scope: &ScopeReference,
        key: ValueKey,
        value: WorkspaceValue,
        pending_workspace: &[WorkspaceMutation],
    ) -> Result<WorkspaceValueEntry, RuntimeError> {
        match self.projected_latest_workspace_value(projection, scope, &key, pending_workspace)? {
            Some(previous) => WorkspaceValueEntry::successor(previous.reference().clone(), value)
                .map_err(|error| RuntimeError::InvalidTransition(error.to_string())),
            None => Ok(WorkspaceValueEntry::initial(scope.clone(), key, value)),
        }
    }

    pub(super) fn projected_imported_output_entry(
        &self,
        projection: &RunProjection,
        scope: &ScopeReference,
        key: ValueKey,
        source: WorkspaceValueReference,
        value: WorkspaceValue,
        pending_workspace: &[WorkspaceMutation],
    ) -> Result<WorkspaceValueEntry, RuntimeError> {
        match self.projected_latest_workspace_value(projection, scope, &key, pending_workspace)? {
            Some(previous) => WorkspaceValueEntry::successor(previous.reference().clone(), value)
                .map_err(|error| RuntimeError::InvalidTransition(error.to_string())),
            None => WorkspaceValueEntry::imported(scope.clone(), key, source, value)
                .map_err(|error| RuntimeError::InvalidTransition(error.to_string())),
        }
    }

    pub(super) fn signal_payload_entries(
        &self,
        projection: &RunProjection,
        execution: &NodeExecutionId,
        payload: &BoundedJson,
        pending_workspace: &[WorkspaceMutation],
    ) -> Result<Vec<WorkspaceValueEntry>, RuntimeError> {
        let execution_view = projection.node_executions().get(execution).ok_or_else(|| {
            RuntimeError::InvalidHistory("signal wait execution is absent".to_owned())
        })?;
        let revision = self.revision_for_execution(projection, execution)?;
        let node = revision
            .semantic()
            .nodes()
            .get(execution_view.node())
            .ok_or_else(|| RuntimeError::InvalidHistory("signal wait node is absent".to_owned()))?;
        let mut entries = Vec::with_capacity(node.data_outputs().len());
        for port in node.data_outputs().keys() {
            let key = ValueKey::new(port.as_str().to_owned())
                .map_err(|error| RuntimeError::Scheduling(error.to_string()))?;
            let entry = self.projected_output_entry(
                projection,
                execution_view.scope(),
                key,
                WorkspaceValue::Json(payload.clone()),
                pending_workspace,
            )?;
            entries.push(entry);
        }
        Ok(entries)
    }

    pub(super) fn drain_broadcast_signals(
        &self,
        run: &RunId,
        occurred_at: TimestampMillis,
        projection: &mut RunProjection,
        events: &mut Vec<RunEventEnvelope>,
        workspace: &mut Vec<WorkspaceMutation>,
    ) -> Result<(), RuntimeError> {
        if events.len().saturating_add(1) > STRUCTURED_EVENT_SOFT_LIMIT {
            return Ok(());
        }
        let Some((_, signal)) = projection
            .pending_broadcast_signals()
            .iter()
            .next()
            .cloned()
        else {
            return Ok(());
        };
        let signal_view = projection.signals().get(&signal).ok_or_else(|| {
            RuntimeError::InvalidHistory(
                "pending broadcast scan references an absent signal".to_owned(),
            )
        })?;
        let signal_type = signal_view.signal_type().clone();
        let correlation = signal_view.correlation().cloned();
        let received_sequence = signal_view.received_sequence();
        let payload = signal_view.payload().clone();
        let original_cursor = signal_view.broadcast_scan_through().cloned();
        let mut through = original_cursor.clone();
        let mut exhausted = false;
        let scan_limit = usize::from(self.config.maximum_tick_items);
        let mut scanned = 0_usize;

        while scanned < scan_limit {
            let lower = through
                .as_ref()
                .map_or(std::ops::Bound::Unbounded, std::ops::Bound::Excluded);
            let next_wait = projection
                .waits()
                .range((lower, std::ops::Bound::Unbounded))
                .next()
                .map(|(_, wait)| {
                    (
                        wait.execution().clone(),
                        wait.registered_sequence(),
                        wait.condition().clone(),
                        wait.is_pending(),
                    )
                });
            let Some((execution, registered_sequence, condition, pending)) = next_wait else {
                exhausted = true;
                break;
            };
            let consumed = projection
                .signals()
                .get(&signal)
                .is_some_and(|received| received.consumed_by().contains(&execution));
            let eligible = pending
                && registered_sequence < received_sequence
                && !consumed
                && wait_signal_matches(&condition, &signal_type, correlation.as_ref());
            if eligible {
                let entries =
                    self.signal_payload_entries(projection, &execution, &payload, workspace)?;
                let event_cost = entries.len().checked_add(2).ok_or_else(|| {
                    RuntimeError::Scheduling("broadcast signal event cost overflow".to_owned())
                })?;
                if event_cost.saturating_add(1) > STRUCTURED_EVENT_SOFT_LIMIT
                    || entries.len() > MAX_WORKSPACE_MUTATIONS_PER_COMMIT
                {
                    return Err(RuntimeError::InvalidHistory(
                        "one broadcast signal consumer exceeds atomic runtime bounds".to_owned(),
                    ));
                }
                if events.len().saturating_add(event_cost).saturating_add(1)
                    > STRUCTURED_EVENT_SOFT_LIMIT
                    || workspace.len().saturating_add(entries.len())
                        > MAX_WORKSPACE_MUTATIONS_PER_COMMIT
                {
                    break;
                }
                self.push_projected_event(
                    run,
                    occurred_at,
                    projection,
                    events,
                    RunEventKind::SignalConsumed {
                        signal: signal.clone(),
                        execution: execution.clone(),
                    },
                )?;
                for entry in entries {
                    let value = entry.reference().clone();
                    workspace.push(WorkspaceMutation::PutValue { entry });
                    self.push_projected_event(
                        run,
                        occurred_at,
                        projection,
                        events,
                        RunEventKind::DeterministicOutputPublished {
                            execution: execution.clone(),
                            value,
                            artifact: None,
                        },
                    )?;
                }
                self.push_projected_event(
                    run,
                    occurred_at,
                    projection,
                    events,
                    RunEventKind::WaitSatisfied {
                        execution: execution.clone(),
                        cause: WaitSatisfaction::Signal {
                            signal: signal.clone(),
                        },
                    },
                )?;
            }
            through = Some(execution);
            scanned = scanned.saturating_add(1);
        }
        if !exhausted && scanned == scan_limit {
            let lower = through
                .as_ref()
                .map_or(std::ops::Bound::Unbounded, std::ops::Bound::Excluded);
            exhausted = projection
                .waits()
                .range((lower, std::ops::Bound::Unbounded))
                .next()
                .is_none();
        }
        if through != original_cursor || exhausted {
            self.push_projected_event(
                run,
                occurred_at,
                projection,
                events,
                RunEventKind::SignalBroadcastScanAdvanced {
                    signal,
                    through_execution: through,
                    complete: exhausted,
                },
            )?;
        }
        Ok(())
    }
}
