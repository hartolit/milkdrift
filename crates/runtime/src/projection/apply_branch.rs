use std::collections::BTreeSet;

use milkdrift_persistence::{RunEventEnvelope, RunEventKind, RunOutcome};
use milkdrift_workspace::ScopeKind;

use crate::RuntimeError;

use super::helpers::{ensure_unique, invalid_at};
use super::run::RunProjection;
use super::structured::{BranchProjection, BranchState};

impl RunProjection {
    pub(super) fn apply_branch_kind(
        &mut self,
        event: &RunEventEnvelope,
    ) -> Result<(), RuntimeError> {
        let _sequence = event.sequence();
        match event.kind() {
            RunEventKind::BranchScopeCreated {
                fork_execution,
                port,
                branch,
                scope,
            } => {
                let owner_scope = self.execution(fork_execution, event)?.scope.clone();
                if self.branches.contains_key(branch)
                    || self
                        .branch_by_fork_port
                        .contains_key(&(fork_execution.clone(), port.clone()))
                    || !matches!(scope.kind(), ScopeKind::Branch { branch: identity } if identity == branch)
                    || scope.parent() != Some(&owner_scope)
                {
                    return Err(invalid_at(
                        event,
                        "branch scope identity, port, kind, or parent is invalid",
                    ));
                }
                self.register_child_scope(scope, event)?;
                self.branches.insert(
                    branch.clone(),
                    BranchProjection {
                        branch: branch.clone(),
                        fork_execution: fork_execution.clone(),
                        port: port.clone(),
                        scope: scope.clone(),
                        children: BTreeSet::new(),
                        state: BranchState::Active,
                        cancellation_reason: None,
                        outputs: Vec::new(),
                    },
                );
                self.branch_by_fork_port
                    .insert((fork_execution.clone(), port.clone()), branch.clone());
                self.branch_ids_by_fork_execution
                    .entry(fork_execution.clone())
                    .or_default()
                    .insert(branch.clone());
                self.active_branch_ids.insert(branch.clone());
                self.adjust_scope_ownership(scope.reference(), true, event)?;
                self.adjust_structured_child_count(fork_execution, true, event)?;
            }
            RunEventKind::BranchRouteSelected {
                execution,
                selected_port,
            } => {
                let execution_view = self.execution(execution, event)?;
                if execution_view.is_completed() || self.branch_routes.contains_key(execution) {
                    return Err(invalid_at(
                        event,
                        "branch route is duplicate or follows terminal execution",
                    ));
                }
                self.branch_routes
                    .insert(execution.clone(), selected_port.clone());
            }
            RunEventKind::BranchChildAdded { branch, execution } => {
                let child_scope = self.execution(execution, event)?.scope.clone();
                let branch_view = self.branches.get(branch).ok_or_else(|| {
                    invalid_at(event, "branch child references an unknown branch")
                })?;
                if !branch_view.is_active()
                    || self.branch_owner.contains_key(execution)
                    || !self.scope_descends_from(&child_scope, branch_view.scope.reference())
                {
                    return Err(invalid_at(
                        event,
                        "branch child is duplicate, out of state, or outside its scope",
                    ));
                }
                self.branches
                    .get_mut(branch)
                    .ok_or_else(|| invalid_at(event, "unknown branch"))?
                    .children
                    .insert(execution.clone());
                self.branch_owner.insert(execution.clone(), branch.clone());
            }
            RunEventKind::BranchCancellationRequested { branch, reason } => {
                let branch_view = self.branches.get_mut(branch).ok_or_else(|| {
                    invalid_at(event, "cancellation references an unknown branch")
                })?;
                if branch_view.state != BranchState::Active {
                    return Err(invalid_at(
                        event,
                        "branch cancellation is duplicate or terminal",
                    ));
                }
                branch_view.state = BranchState::Cancelling;
                branch_view.cancellation_reason = Some(reason.clone());
                self.cancelling_branch_ids.insert(branch.clone());
            }
            RunEventKind::BranchTerminal {
                branch,
                outcome,
                outputs,
            } => {
                let branch_view = self.branches.get(branch).ok_or_else(|| {
                    invalid_at(event, "terminal fact references an unknown branch")
                })?;
                if !branch_view.is_active()
                    || (*outcome == RunOutcome::Cancelled
                        && branch_view.state != BranchState::Cancelling)
                    || self.branch_has_active_descendant_ownership(branch)
                {
                    return Err(invalid_at(
                        event,
                        "branch terminal fact is duplicate, contradicts cancellation, or abandons a child",
                    ));
                }
                ensure_unique(outputs, event, "branch terminal output")?;
                for output in outputs {
                    self.validate_known_workspace_value(output, event)?;
                    if !self.scope_descends_from(output.scope(), branch_view.scope.reference()) {
                        return Err(invalid_at(
                            event,
                            "branch terminal output is outside its isolated scope",
                        ));
                    }
                }
                let branch_scope = branch_view.scope.reference().clone();
                let fork_execution = branch_view.fork_execution.clone();
                let branch_view = self
                    .branches
                    .get_mut(branch)
                    .ok_or_else(|| invalid_at(event, "unknown branch"))?;
                branch_view.state = BranchState::Completed(*outcome);
                branch_view.outputs = outputs.clone();
                self.active_branch_ids.remove(branch);
                self.cancelling_branch_ids.remove(branch);
                self.adjust_scope_ownership(&branch_scope, false, event)?;
                self.adjust_structured_child_count(&fork_execution, false, event)?;
            }
            _ => unreachable!("structured dispatch owns branch ownership routing"),
        }
        Ok(())
    }
}
