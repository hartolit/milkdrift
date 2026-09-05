//! Independent execution event fact validation.
use super::{super::kind::RunEventKind, super::model::NodeOutcome, ReferenceContext};
use crate::PersistenceError;
use milkdrift_capability::TerminalStatus;

pub(super) fn validate(
    event: &RunEventKind,
    _context: &ReferenceContext<'_>,
) -> Result<(), PersistenceError> {
    match event {
        RunEventKind::NodeScheduled {
            invocation,
            idempotency_key,
            request,
            ..
        } if request.invocation() != invocation
            || request.idempotency_key() != idempotency_key.as_ref() =>
        {
            return Err(PersistenceError::InvalidDocument(
                "scheduled invocation/idempotency facts contradict the persisted request"
                    .to_owned(),
            ));
        }
        RunEventKind::CapabilityResolved {
            requirement,
            snapshot,
            ..
        } => {
            requirement
                .validate()
                .map_err(|error| PersistenceError::InvalidDocument(error.to_string()))?;
            if requirement.operation() != snapshot.operation()
                || requirement
                    .exact_capability()
                    .is_some_and(|identity| identity != snapshot.capability())
                || requirement
                    .provider_profile_ref()
                    .is_some_and(|profile| Some(profile) != snapshot.provider_profile())
            {
                return Err(PersistenceError::InvalidDocument(
                    "resolved capability snapshot contradicts the recorded requirement".to_owned(),
                ));
            }
        }
        RunEventKind::CapabilityResolutionDecisionRecorded {
            attempt,
            snapshot,
            authorization,
            ..
        } if !authorization.is_allowed()
            || authorization.request().resources.capability.as_ref()
                != Some(snapshot.capability())
            || authorization
                .request()
                .resources
                .capability_operation
                .as_ref()
                != Some(snapshot.operation())
            || authorization.request().resources.provider_profile.as_ref()
                != snapshot.provider_profile()
            || authorization.request().provenance.descriptor_revision
                != Some(snapshot.descriptor_revision())
            || authorization.request().provenance.attempt.as_deref() != Some(attempt.as_str()) =>
        {
            return Err(PersistenceError::InvalidDocument(
                "capability authorization does not bind the resolved generation".to_owned(),
            ));
        }
        RunEventKind::NodeProgressRecorded {
            completed_units,
            total_units: Some(total),
            ..
        } if completed_units.is_some_and(|completed| completed > *total) => {
            return Err(PersistenceError::InvalidDocument(
                "completed progress units exceed total units".to_owned(),
            ));
        }
        RunEventKind::LateTerminalEvidenceRecorded { terminal, .. }
            if terminal.status() == TerminalStatus::Uncertain =>
        {
            return Err(PersistenceError::InvalidDocument(
                "late terminal evidence must add a known terminal observation".to_owned(),
            ));
        }
        RunEventKind::NodeRetryScheduled {
            attempt_number: 0, ..
        }
        | RunEventKind::RepeatIterationCreated {
            iteration_number: 0,
            ..
        } => {
            return Err(PersistenceError::InvalidDocument(
                "attempt and iteration numbers are one-based".to_owned(),
            ));
        }
        RunEventKind::NodeTerminal {
            outcome,
            error_class,
            ..
        }
        | RunEventKind::DeterministicNodeTerminal {
            outcome,
            error_class,
            ..
        } if matches!(outcome, NodeOutcome::Failed | NodeOutcome::Rejected)
            != error_class.is_some() =>
        {
            return Err(PersistenceError::InvalidDocument(
                    "node failure/rejection requires an error class and success/cancellation forbids one"
                        .to_owned(),
                ));
        }
        RunEventKind::AttemptUsageRecorded { usage, .. }
            if usage.input_units.is_none()
                && usage.output_units.is_none()
                && usage.duration_ms.is_none()
                && usage.cost.is_none() =>
        {
            return Err(PersistenceError::InvalidDocument(
                "an attempt usage fact must contain at least one observation".to_owned(),
            ));
        }
        _ => {}
    }
    Ok(())
}
