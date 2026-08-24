//! Projection-derived index transitions, revision loading, bounded scans, and identity boundaries.

use super::RuntimeService;
use super::support::{bounded_projection_set, execution_branch_state};
use crate::RuntimeError;
use crate::projection::{
    AttemptState, BranchState, NodeExecutionState, RunLifecycle, RunProjection,
};
use milkdrift_blueprint::{BlueprintRevision, NodeKind, ReducerStrategy, RevisionId, WorkflowId};
use milkdrift_capability::InvocationId;
use milkdrift_persistence::{
    AttemptId, CommandId, EventId, IndexedRunState, LeaseId, LeaseIndexEntry, LeaseIndexMutation,
    NodeExecutionId, ReconciliationPlanId, RunIndexUpdate, RunSummaryIndex, RunnableIndexEntry,
    RunnableIndexMutation, TimerId, TimerIndexEntry, TimerIndexMutation, TimestampMillis,
    WorkspaceMutation,
};
use milkdrift_workspace::{
    ArtifactReference, BranchId, IterationId, RunId, ScopeId, SubworkflowId, WorkspaceBudget,
    WorkspaceUsage,
};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::Ordering;

impl RuntimeService {
    pub(super) fn workspace_accounting_transition(
        &self,
        projection: &RunProjection,
        mutations: &[WorkspaceMutation],
        budget: &WorkspaceBudget,
        required_artifacts: &BTreeSet<ArtifactReference>,
    ) -> Result<(WorkspaceUsage, WorkspaceUsage, BTreeSet<ArtifactReference>), RuntimeError> {
        let run = projection.run_id().ok_or_else(|| {
            RuntimeError::InvalidHistory(
                "workspace accounting transition has no run identity".to_owned(),
            )
        })?;
        let expected = self.store.workspace_usage(run)?;
        let mut newly_referenced_artifacts = BTreeSet::new();
        for artifact in required_artifacts {
            if !self.store.is_referenced_by_run(run, artifact)? {
                newly_referenced_artifacts.insert(artifact.clone());
            }
        }
        let mut resulting = expected;
        for artifact in &newly_referenced_artifacts {
            resulting = budget
                .admit_artifact_reference(&resulting, artifact)
                .map_err(|error| RuntimeError::InvalidHistory(error.to_string()))?;
        }
        for mutation in mutations {
            if let WorkspaceMutation::PutValue { entry } = mutation {
                resulting = budget
                    .admit_value(&resulting, entry.value())
                    .map_err(|error| RuntimeError::InvalidTransition(error.to_string()))?;
            }
        }
        Ok((expected, resulting, newly_referenced_artifacts))
    }

    pub(super) fn index_update(
        &self,
        run: &RunId,
        old: &RunProjection,
        new: &RunProjection,
        updated_at: TimestampMillis,
    ) -> Result<RunIndexUpdate, RuntimeError> {
        let through = new.sequence();
        let workflow = new.workflow().ok_or_else(|| {
            RuntimeError::InvalidHistory("indexed run has no workflow".to_owned())
        })?;
        let revision_id = new.revision().ok_or_else(|| {
            RuntimeError::InvalidHistory("indexed run has no revision pin".to_owned())
        })?;
        let runnable = self.runnable_executions(new)?;
        let state = if new.lifecycle().is_completed() {
            IndexedRunState::Terminal
        } else if new.lifecycle() == RunLifecycle::Cancelling {
            IndexedRunState::Cancelling
        } else if new.lifecycle() == RunLifecycle::Paused {
            IndexedRunState::Paused
        } else if new.unresolved_attempts().next().is_some() {
            IndexedRunState::Uncertain
        } else if !runnable.is_empty() {
            IndexedRunState::Runnable
        } else if new.lifecycle() == RunLifecycle::Created {
            IndexedRunState::Created
        } else if new.timers().values().any(|timer| timer.is_pending())
            || new.waits().values().any(|wait| wait.is_pending())
            || new.subworkflows().values().any(|child| child.is_active())
            || new.reconciliation().is_active()
        {
            IndexedRunState::Waiting
        } else {
            IndexedRunState::Active
        };
        let summary = RunSummaryIndex {
            run: run.clone(),
            workflow: workflow.clone(),
            revision: revision_id.clone(),
            state,
            through_sequence: through,
            updated_at,
        };
        let mut runnable_mutations = Vec::new();
        let mut timer_mutations = Vec::new();
        let mut lease_mutations = Vec::new();

        let old_runnable = self.runnable_executions(old)?;
        for (execution, eligible_at) in &runnable {
            if old_runnable.get(execution) == Some(eligible_at) {
                continue;
            }
            runnable_mutations.push(RunnableIndexMutation::Upsert {
                entry: RunnableIndexEntry {
                    run: run.clone(),
                    execution: execution.clone(),
                    eligible_at: *eligible_at,
                    priority: 0,
                    through_sequence: through,
                },
            });
        }
        for execution in old_runnable
            .keys()
            .filter(|execution| !runnable.contains_key(*execution))
        {
            runnable_mutations.push(RunnableIndexMutation::Remove {
                run: run.clone(),
                execution: execution.clone(),
            });
        }

        let old_timers: BTreeMap<_, _> = old
            .timers()
            .values()
            .filter(|timer| timer.is_pending())
            .map(|timer| (timer.timer().clone(), timer.fire_at()))
            .collect();
        let new_timers: BTreeMap<_, _> = new
            .timers()
            .values()
            .filter(|timer| timer.is_pending())
            .map(|timer| (timer.timer().clone(), timer.fire_at()))
            .collect();
        for (timer, fire_at) in &new_timers {
            if old_timers.get(timer) == Some(fire_at) {
                continue;
            }
            timer_mutations.push(TimerIndexMutation::Upsert {
                entry: TimerIndexEntry {
                    run: run.clone(),
                    timer: timer.clone(),
                    fire_at: *fire_at,
                    through_sequence: through,
                },
            });
        }
        for timer in old_timers
            .keys()
            .filter(|timer| !new_timers.contains_key(*timer))
        {
            timer_mutations.push(TimerIndexMutation::Remove {
                run: run.clone(),
                timer: timer.clone(),
            });
        }

        let old_leases: BTreeSet<_> = old
            .leases()
            .values()
            .filter(|lease| lease.is_active())
            .map(|lease| lease.lease().clone())
            .collect();
        let new_leases: BTreeSet<_> = new
            .leases()
            .values()
            .filter(|lease| lease.is_active())
            .map(|lease| lease.lease().clone())
            .collect();
        for lease in &new_leases {
            let candidate = new.leases().get(lease).ok_or_else(|| {
                RuntimeError::InvalidHistory("active lease projection disappeared".to_owned())
            })?;
            if old.leases().get(lease).is_some_and(|previous| {
                previous.is_active()
                    && previous.attempt() == candidate.attempt()
                    && previous.worker() == candidate.worker()
                    && previous.expires_at() == candidate.expires_at()
            }) {
                continue;
            }
            lease_mutations.push(LeaseIndexMutation::Upsert {
                entry: LeaseIndexEntry {
                    run: run.clone(),
                    lease: lease.clone(),
                    attempt: candidate.attempt().clone(),
                    worker: candidate.worker().clone(),
                    expires_at: candidate.expires_at(),
                    through_sequence: through,
                },
            });
        }
        for lease in old_leases.difference(&new_leases) {
            lease_mutations.push(LeaseIndexMutation::Remove {
                run: run.clone(),
                lease: lease.clone(),
            });
        }
        Ok(RunIndexUpdate::new(
            Some(summary),
            runnable_mutations,
            timer_mutations,
            lease_mutations,
        ))
    }

    pub(super) fn current_revision(
        &self,
        projection: &RunProjection,
    ) -> Result<BlueprintRevision, RuntimeError> {
        let revision = projection
            .revision()
            .ok_or_else(|| RuntimeError::InvalidHistory("run has no pinned revision".to_owned()))?;
        self.load_validated_revision(revision, projection.workflow())
    }

    pub(super) fn revision_for_execution(
        &self,
        projection: &RunProjection,
        execution: &NodeExecutionId,
    ) -> Result<BlueprintRevision, RuntimeError> {
        let execution_view = projection
            .node_executions()
            .get(execution)
            .ok_or_else(|| RuntimeError::InvalidHistory("node execution is absent".to_owned()))?;
        let revision = execution_view.revision();
        self.load_validated_revision(revision, projection.workflow())
    }

    pub(super) fn scan_eligible_execution_ids(
        &self,
        run: &RunId,
        projection: &RunProjection,
        remaining: &mut usize,
    ) -> Result<Vec<NodeExecutionId>, RuntimeError> {
        let claimed = self.claim_structured_scan_visits(
            (*remaining).min(projection.eligible_execution_ids().len()),
        );
        let mut allowance = claimed;
        let selected = bounded_projection_set(
            run,
            projection.eligible_execution_ids(),
            &self.structured_eligible_cursors,
            &mut allowance,
            "structured eligible scan cursor",
        )?;
        *remaining = remaining.saturating_sub(claimed.saturating_sub(allowance));
        Ok(selected)
    }

    pub(super) fn scan_branch_ids(
        &self,
        run: &RunId,
        projection: &RunProjection,
        remaining: &mut usize,
    ) -> Result<Vec<BranchId>, RuntimeError> {
        let claimed = self
            .claim_structured_scan_visits((*remaining).min(projection.active_branch_ids().len()));
        let mut allowance = claimed;
        let selected = bounded_projection_set(
            run,
            projection.active_branch_ids(),
            &self.structured_branch_cursors,
            &mut allowance,
            "structured branch scan cursor",
        )?;
        *remaining = remaining.saturating_sub(claimed.saturating_sub(allowance));
        Ok(selected)
    }

    pub(super) fn claim_structured_scan_visits(&self, requested: usize) -> usize {
        if requested == 0 || !self.structured_scan_budget_active.load(Ordering::Acquire) {
            return requested;
        }
        let mut claimed = 0;
        let _ = self.structured_scan_budget.fetch_update(
            Ordering::AcqRel,
            Ordering::Acquire,
            |remaining| {
                claimed = requested.min(remaining);
                Some(remaining.saturating_sub(claimed))
            },
        );
        claimed
    }

    pub(super) fn runnable_executions(
        &self,
        projection: &RunProjection,
    ) -> Result<BTreeMap<NodeExecutionId, TimestampMillis>, RuntimeError> {
        if projection.lifecycle() != RunLifecycle::Running || projection.termination().is_some() {
            return Ok(BTreeMap::new());
        }
        let mut result = BTreeMap::new();
        let mut revisions = BTreeMap::new();
        for execution_id in projection.active_execution_ids() {
            let execution = projection
                .node_executions()
                .get(execution_id)
                .ok_or_else(|| {
                    RuntimeError::InvalidHistory(
                        "active execution frontier identity is absent".to_owned(),
                    )
                })?;
            let eligible_at = match execution.state() {
                NodeExecutionState::Eligible => TimestampMillis::new(0),
                NodeExecutionState::RetryPending(attempt)
                    if projection
                        .attempts()
                        .get(attempt)
                        .is_some_and(|value| value.state() == &AttemptState::ReadyToSchedule) =>
                {
                    projection
                        .retries()
                        .values()
                        .find(|retry| retry.next_attempt() == attempt)
                        .map_or(TimestampMillis::new(0), |retry| retry.fire_at())
                }
                NodeExecutionState::RetryPending(_)
                | NodeExecutionState::Scheduled(_)
                | NodeExecutionState::Running(_)
                | NodeExecutionState::Uncertain(_)
                | NodeExecutionState::CancelledBeforeDispatch
                | NodeExecutionState::RemovedProspectively(_)
                | NodeExecutionState::Terminal(_) => continue,
            };
            if execution_branch_state(projection, execution.execution())
                .is_some_and(|state| state != BranchState::Active)
            {
                continue;
            }
            let revision_id = execution.revision().clone();
            if !revisions.contains_key(&revision_id) {
                revisions.insert(
                    revision_id.clone(),
                    self.load_validated_revision(&revision_id, projection.workflow())?,
                );
            }
            let is_task = revisions
                .get(&revision_id)
                .and_then(|revision| revision.semantic().nodes().get(execution.node()))
                .is_some_and(|node| {
                    matches!(node.kind(), NodeKind::Task { .. })
                        || matches!(
                            node.kind(),
                            NodeKind::Reducer { config }
                                if matches!(config.strategy(), ReducerStrategy::Capability(_))
                        )
                });
            if !is_task {
                continue;
            }
            result.insert(execution.execution().clone(), eligible_at);
        }
        Ok(result)
    }

    pub(super) fn load_validated_revision(
        &self,
        revision: &RevisionId,
        expected_workflow: Option<&WorkflowId>,
    ) -> Result<BlueprintRevision, RuntimeError> {
        let root = self.store.revision(revision)?.ok_or_else(|| {
            RuntimeError::InvalidTransition(format!("revision {revision} does not exist"))
        })?;
        if expected_workflow.is_some_and(|workflow| root.semantic().workflow() != workflow) {
            return Err(RuntimeError::InvalidTransition(
                "revision belongs to another workflow lineage".to_owned(),
            ));
        }
        let mut visiting = BTreeSet::new();
        let mut verified = BTreeSet::new();
        self.validate_pinned_children(&root, &mut visiting, &mut verified, 0)?;
        Ok(root)
    }

    fn validate_pinned_children(
        &self,
        revision: &BlueprintRevision,
        visiting: &mut BTreeSet<RevisionId>,
        verified: &mut BTreeSet<RevisionId>,
        depth: usize,
    ) -> Result<(), RuntimeError> {
        if depth > 64 {
            return Err(RuntimeError::InvalidTransition(
                "pinned subworkflow nesting exceeds 64 revisions".to_owned(),
            ));
        }
        if verified.contains(revision.id()) {
            return Ok(());
        }
        if !visiting.insert(revision.id().clone()) {
            return Err(RuntimeError::InvalidTransition(format!(
                "pinned subworkflow cycle reaches revision {}",
                revision.id()
            )));
        }
        for node in revision.semantic().nodes().values() {
            let reference = match node.kind() {
                NodeKind::Subworkflow { reference } => Some(reference),
                NodeKind::Repeat { config } => Some(config.body()),
                NodeKind::Task { .. }
                | NodeKind::Branch { .. }
                | NodeKind::Fork { .. }
                | NodeKind::Join { .. }
                | NodeKind::Reducer { .. }
                | NodeKind::Wait { .. }
                | NodeKind::SignalWait { .. }
                | NodeKind::Terminal { .. } => None,
            };
            if let Some(reference) = reference {
                let child = self.store.revision(reference.revision())?.ok_or_else(|| {
                    RuntimeError::InvalidTransition(format!(
                        "pinned child revision {} does not exist",
                        reference.revision()
                    ))
                })?;
                if child.semantic().workflow() != reference.workflow()
                    || child.semantic().interface() != reference.interface()
                {
                    return Err(RuntimeError::InvalidTransition(format!(
                        "pinned child revision {} has a different workflow or interface",
                        reference.revision()
                    )));
                }
                self.validate_pinned_children(&child, visiting, verified, depth + 1)?;
            }
        }
        visiting.remove(revision.id());
        verified.insert(revision.id().clone());
        Ok(())
    }

    pub(super) fn next_command_id(&self) -> Result<CommandId, RuntimeError> {
        Ok(CommandId::new(self.ids.next("command")?)?)
    }

    pub(super) fn next_event_id(&self) -> Result<EventId, RuntimeError> {
        Ok(EventId::new(self.ids.next("event")?)?)
    }

    pub(super) fn next_execution_id(&self) -> Result<NodeExecutionId, RuntimeError> {
        Ok(NodeExecutionId::new(self.ids.next("execution")?)?)
    }

    pub(super) fn next_attempt_id(&self) -> Result<AttemptId, RuntimeError> {
        Ok(AttemptId::new(self.ids.next("attempt")?)?)
    }

    pub(super) fn next_lease_id(&self) -> Result<LeaseId, RuntimeError> {
        Ok(LeaseId::new(self.ids.next("lease")?)?)
    }

    pub(super) fn next_timer_id(&self) -> Result<TimerId, RuntimeError> {
        Ok(TimerId::new(self.ids.next("timer")?)?)
    }

    pub(super) fn next_plan_id(&self) -> Result<ReconciliationPlanId, RuntimeError> {
        Ok(ReconciliationPlanId::new(
            self.ids.next("reconciliation-plan")?,
        )?)
    }

    pub(super) fn next_invocation_id(&self) -> Result<InvocationId, RuntimeError> {
        InvocationId::new(self.ids.next("invocation")?)
            .map_err(|error| RuntimeError::InvalidTransition(error.to_string()))
    }

    pub(super) fn next_scope_id(&self) -> Result<ScopeId, RuntimeError> {
        ScopeId::new(self.ids.next("scope")?)
            .map_err(|error| RuntimeError::InvalidTransition(error.to_string()))
    }

    pub(super) fn next_branch_id(&self) -> Result<BranchId, RuntimeError> {
        BranchId::new(self.ids.next("branch")?)
            .map_err(|error| RuntimeError::InvalidTransition(error.to_string()))
    }

    pub(super) fn next_iteration_id(&self) -> Result<IterationId, RuntimeError> {
        IterationId::new(self.ids.next("iteration")?)
            .map_err(|error| RuntimeError::InvalidTransition(error.to_string()))
    }

    pub(super) fn next_subworkflow_id(&self) -> Result<SubworkflowId, RuntimeError> {
        SubworkflowId::new(self.ids.next("subworkflow")?)
            .map_err(|error| RuntimeError::InvalidTransition(error.to_string()))
    }

    pub(super) fn next_run_id(&self) -> Result<RunId, RuntimeError> {
        RunId::new(self.ids.next("child-run")?)
            .map_err(|error| RuntimeError::InvalidTransition(error.to_string()))
    }
}
