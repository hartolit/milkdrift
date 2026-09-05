//! Independent continuation event fact validation.
use super::{
    super::MAX_REPEAT_CONTINUATION_ADDITIONAL_ITERATIONS, super::MAX_REPEAT_EFFECTIVE_ITERATIONS,
    super::kind::RunEventKind, super::model::ControllerAssessmentOutcome,
    super::model::RepeatContinuationCause, super::model::RepeatContinuationDecision,
    ReferenceContext,
};
use crate::PersistenceError;

pub(super) fn validate(
    event: &RunEventKind,
    context: &ReferenceContext<'_>,
) -> Result<(), PersistenceError> {
    match event {
        RunEventKind::RepeatContinuationRequested {
            initial_iteration_limit,
            effective_iteration_limit,
            cause,
            ..
        } => {
            let limits_valid = *initial_iteration_limit > 0
                && *initial_iteration_limit <= *effective_iteration_limit
                && *effective_iteration_limit <= MAX_REPEAT_EFFECTIVE_ITERATIONS;
            let cause_valid = match cause {
                RepeatContinuationCause::IterationLimit => true,
                RepeatContinuationCause::DurationBudget {
                    maximum_ms,
                    observed_ms,
                } => *maximum_ms > 0 && observed_ms >= maximum_ms,
                RepeatContinuationCause::CostBudget {
                    maximum_micros,
                    observed_micros,
                    ..
                } => *maximum_micros > 0 && observed_micros >= maximum_micros,
                RepeatContinuationCause::ControllerCheckpoint {
                    checkpoint_id,
                    completed_cycles,
                } => {
                    !checkpoint_id.is_empty() && checkpoint_id.len() <= 192 && *completed_cycles > 0
                }
            };
            if !limits_valid || !cause_valid {
                return Err(PersistenceError::InvalidDocument(format!(
                    "repeat continuation request requires limits within 1..={MAX_REPEAT_EFFECTIVE_ITERATIONS} and a truthfully exhausted typed cause"
                )));
            }
        }
        RunEventKind::RepeatContinuationDecided {
            outcome,
            approved_additional_iterations,
            evidence,
            ..
        } => {
            context.check_evidence(evidence)?;
            let valid = match (outcome, approved_additional_iterations) {
                (RepeatContinuationDecision::Approved, Some(additional)) => {
                    (1..=MAX_REPEAT_CONTINUATION_ADDITIONAL_ITERATIONS).contains(additional)
                }
                (RepeatContinuationDecision::Rejected, None) => true,
                (RepeatContinuationDecision::Approved, None)
                | (RepeatContinuationDecision::Rejected, Some(_)) => false,
            };
            if !valid {
                return Err(PersistenceError::InvalidDocument(format!(
                    "repeat approval requires 1..={MAX_REPEAT_CONTINUATION_ADDITIONAL_ITERATIONS} additional iterations and rejection forbids them"
                )));
            }
        }
        RunEventKind::ControllerAssessmentRecorded {
            controller_id,
            policy_digest,
            assessment_id,
            cycle_id,
            through_sequence: _,
            outcome,
            ..
        } => {
            let bounded_identity = |value: &str| !value.is_empty() && value.len() <= 192;
            let outcome_valid = match outcome {
                ControllerAssessmentOutcome::Continue => true,
                ControllerAssessmentOutcome::HumanCheckpoint { checkpoint_id } => {
                    bounded_identity(checkpoint_id)
                }
                ControllerAssessmentOutcome::BoundReached {
                    bound,
                    current,
                    limit,
                    unknown_usage,
                } => {
                    bounded_identity(bound)
                        && *limit > 0
                        && if bound == "account_integrity" {
                            current.is_none() && !unknown_usage
                        } else {
                            *unknown_usage == current.is_none()
                        }
                }
            };
            if !bounded_identity(controller_id)
                || !bounded_identity(policy_digest)
                || !bounded_identity(assessment_id)
                || cycle_id
                    .as_deref()
                    .is_some_and(|value| !bounded_identity(value))
                || !outcome_valid
            {
                return Err(PersistenceError::InvalidDocument(
                    "controller assessment identities, sequence, or outcome are invalid".to_owned(),
                ));
            }
        }
        _ => {}
    }
    Ok(())
}
