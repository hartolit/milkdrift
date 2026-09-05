//! Independent structured event fact validation.
use super::{
    super::kind::RunEventKind, super::model::JoinRule, super::model::RunOutcome, ReferenceContext,
};
use crate::PersistenceError;

pub(super) fn validate(
    event: &RunEventKind,
    context: &ReferenceContext<'_>,
) -> Result<(), PersistenceError> {
    match event {
        RunEventKind::JoinSatisfied {
            rule: JoinRule::Quorum { required: 0 },
            ..
        } => {
            return Err(PersistenceError::InvalidDocument(
                "join quorum must be greater than zero".to_owned(),
            ));
        }
        RunEventKind::JoinSatisfied {
            branches,
            retained_branches,
            rule,
            ..
        } => {
            context.check_references("event.branches", branches.len())?;
            context.check_references("event.retained_branches", retained_branches.len())?;
            for branch in branches {
                context.check_references("event.branch.outputs", branch.outputs.len())?;
                if branch
                    .outputs
                    .iter()
                    .collect::<std::collections::BTreeSet<_>>()
                    .len()
                    != branch.outputs.len()
                {
                    return Err(PersistenceError::InvalidDocument(
                        "branch output references must be distinct".to_owned(),
                    ));
                }
            }
            let branch_ids: std::collections::BTreeSet<_> =
                branches.iter().map(|branch| &branch.branch).collect();
            let retained_ids: std::collections::BTreeSet<_> = retained_branches.iter().collect();
            let successful = branches
                .iter()
                .filter(|branch| branch.outcome == RunOutcome::Succeeded)
                .count();
            let rule_satisfied = match rule {
                JoinRule::All | JoinRule::AnyCompletion => !branches.is_empty(),
                JoinRule::FirstSuccess => successful > 0,
                JoinRule::Quorum { required } => {
                    usize::try_from(*required).is_ok_and(|required| successful >= required)
                }
            };
            if branch_ids.len() != branches.len()
                || retained_ids.len() != retained_branches.len()
                || !branch_ids.is_disjoint(&retained_ids)
                || !rule_satisfied
            {
                return Err(PersistenceError::InvalidDocument(
                        "join results must use distinct branches and truthfully satisfy the recorded rule"
                            .to_owned(),
                    ));
            }
        }
        RunEventKind::SubworkflowTerminal { outputs, .. }
        | RunEventKind::BranchTerminal { outputs, .. } => {
            context.check_references("event.outputs", outputs.len())?;
            if outputs
                .iter()
                .collect::<std::collections::BTreeSet<_>>()
                .len()
                != outputs.len()
            {
                return Err(PersistenceError::InvalidDocument(
                    "branch/subworkflow output references must be distinct".to_owned(),
                ));
            }
        }
        _ => {}
    }
    Ok(())
}
