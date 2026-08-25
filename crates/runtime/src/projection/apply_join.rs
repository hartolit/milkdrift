use std::collections::BTreeSet;

use milkdrift_persistence::{JoinRule, RunEventEnvelope, RunEventKind, RunOutcome};

use crate::RuntimeError;

use super::helpers::{ensure_unique, ensure_unique_by, invalid_at};
use super::run::RunProjection;
use super::structured::{BranchState, JoinProjection};

impl RunProjection {
    pub(super) fn apply_join_kind(&mut self, event: &RunEventEnvelope) -> Result<(), RuntimeError> {
        let sequence = event.sequence();
        match event.kind() {
            RunEventKind::JoinSatisfied {
                execution,
                rule,
                branches,
                retained_branches,
            } => {
                self.execution(execution, event)?;
                if self.joins.contains_key(execution) {
                    return Err(invalid_at(event, "join was already satisfied"));
                }
                ensure_unique_by(
                    branches,
                    |result| result.branch.clone(),
                    event,
                    "join branch",
                )?;
                ensure_unique(retained_branches, event, "retained branch")?;
                let result_ids: BTreeSet<_> =
                    branches.iter().map(|result| &result.branch).collect();
                if retained_branches
                    .iter()
                    .any(|branch| result_ids.contains(branch))
                {
                    return Err(invalid_at(
                        event,
                        "a completed join result cannot also be retained",
                    ));
                }
                let fork_execution = branches
                    .first()
                    .and_then(|result| self.branches.get(&result.branch))
                    .map(|branch| branch.fork_execution.clone())
                    .ok_or_else(|| invalid_at(event, "join has no known owning fork"))?;
                let fork_scope = self
                    .current_node_execution(&fork_execution)
                    .ok_or_else(|| invalid_at(event, "join fork is outside the current frontier"))?
                    .scope()
                    .clone();
                if self.execution(execution, event)?.scope != fork_scope {
                    return Err(invalid_at(
                        event,
                        "join execution and owning fork must share a structured scope",
                    ));
                }
                for result in branches {
                    let branch = self
                        .branches
                        .get(&result.branch)
                        .ok_or_else(|| invalid_at(event, "join references an unknown branch"))?;
                    if branch.state != BranchState::Completed(result.outcome)
                        || branch.fork_execution != fork_execution
                        || branch.scope.reference() != &result.scope
                        || branch.outputs != result.outputs
                    {
                        return Err(invalid_at(
                            event,
                            "join result disagrees with the branch's durable terminal fact",
                        ));
                    }
                    for output in &result.outputs {
                        self.validate_known_workspace_value(output, event)?;
                        if !self.scope_descends_from(output.scope(), &result.scope) {
                            return Err(invalid_at(
                                event,
                                "branch result output is outside its scope",
                            ));
                        }
                    }
                }
                for retained in retained_branches {
                    let branch = self
                        .branches
                        .get(retained)
                        .ok_or_else(|| invalid_at(event, "join retains an unknown branch"))?;
                    if branch.state != BranchState::Active
                        || branch.fork_execution != fork_execution
                    {
                        return Err(invalid_at(
                            event,
                            "join retains a terminal, cancelling, or foreign branch",
                        ));
                    }
                }
                let owned = self
                    .branch_ids_by_fork_execution
                    .get(&fork_execution)
                    .cloned()
                    .unwrap_or_default();
                let named: BTreeSet<_> = branches
                    .iter()
                    .map(|result| result.branch.clone())
                    .chain(retained_branches.iter().cloned())
                    .collect();
                let unnamed_are_cancelling = owned.difference(&named).all(|branch| {
                    self.branches
                        .get(branch)
                        .is_some_and(|branch| branch.state == BranchState::Cancelling)
                });
                let successes = branches
                    .iter()
                    .filter(|result| result.outcome == RunOutcome::Succeeded)
                    .count();
                let satisfied = match rule {
                    JoinRule::All => {
                        !branches.is_empty()
                            && retained_branches.is_empty()
                            && result_ids.len() == owned.len()
                            && owned.iter().all(|branch| result_ids.contains(branch))
                    }
                    JoinRule::AnyCompletion => !branches.is_empty() && unnamed_are_cancelling,
                    JoinRule::FirstSuccess => {
                        successes >= 1 && retained_branches.is_empty() && unnamed_are_cancelling
                    }
                    JoinRule::Quorum { required } => {
                        usize::try_from(*required).is_ok_and(|required| successes >= required)
                            && retained_branches.is_empty()
                            && unnamed_are_cancelling
                    }
                };
                if !satisfied {
                    return Err(invalid_at(
                        event,
                        "recorded branch results do not satisfy the join rule",
                    ));
                }
                for result in branches {
                    let branch = self
                        .branches
                        .get(&result.branch)
                        .ok_or_else(|| invalid_at(event, "unknown branch"))?;
                    if branch.state != BranchState::Completed(result.outcome) {
                        return Err(invalid_at(event, "branch terminal outcome changed at join"));
                    }
                }
                for retained in retained_branches {
                    let scope = self
                        .branches
                        .get(retained)
                        .ok_or_else(|| invalid_at(event, "unknown branch"))?
                        .scope
                        .reference()
                        .clone();
                    let fork_execution = self
                        .branches
                        .get(retained)
                        .ok_or_else(|| invalid_at(event, "unknown branch"))?
                        .fork_execution
                        .clone();
                    self.branches
                        .get_mut(retained)
                        .ok_or_else(|| invalid_at(event, "unknown branch"))?
                        .state = BranchState::Retained;
                    self.active_branch_ids.remove(retained);
                    self.cancelling_branch_ids.remove(retained);
                    self.adjust_scope_ownership(&scope, false, event)?;
                    self.adjust_structured_child_count(&fork_execution, false, event)?;
                }
                self.joins.insert(
                    execution.clone(),
                    JoinProjection {
                        execution: execution.clone(),
                        rule: *rule,
                        branches: branches.clone(),
                        retained_branches: retained_branches.clone(),
                        sequence,
                    },
                );
            }
            _ => unreachable!("structured dispatch owns join settlement routing"),
        }
        Ok(())
    }
}
