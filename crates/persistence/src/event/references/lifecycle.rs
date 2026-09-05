//! Independent lifecycle event fact validation.
use super::{
    super::kind::RunEventKind, super::model::AuthorityDecision, super::model::RunOutcome,
    ReferenceContext,
};
use crate::PersistenceError;

pub(super) fn validate(
    event: &RunEventKind,
    context: &ReferenceContext<'_>,
) -> Result<(), PersistenceError> {
    match event {
        RunEventKind::RunCreated { inputs, .. }
        | RunEventKind::SubworkflowCreated { inputs, .. } => {
            context.check_references("event.inputs", inputs.len())?;
            if inputs
                .iter()
                .collect::<std::collections::BTreeSet<_>>()
                .len()
                != inputs.len()
            {
                return Err(PersistenceError::InvalidDocument(
                    "event input references must be distinct".to_owned(),
                ));
            }
        }
        RunEventKind::RunTerminal {
            outputs, artifacts, ..
        } => {
            context.check_references("event.outputs", outputs.len())?;
            context.check_references("event.artifacts", artifacts.len())?;
            if outputs
                .iter()
                .collect::<std::collections::BTreeSet<_>>()
                .len()
                != outputs.len()
                || artifacts
                    .iter()
                    .collect::<std::collections::BTreeSet<_>>()
                    .len()
                    != artifacts.len()
            {
                return Err(PersistenceError::InvalidDocument(
                    "terminal output and artifact references must be distinct".to_owned(),
                ));
            }
        }
        RunEventKind::RunTerminationRequested { outcome, .. } if *outcome != RunOutcome::Failed => {
            return Err(PersistenceError::InvalidDocument(
                "internal run termination currently supports only an explicit failed outcome"
                    .to_owned(),
            ));
        }
        RunEventKind::SignalBroadcastScanAdvanced {
            through_execution: None,
            complete: false,
            ..
        } => {
            return Err(PersistenceError::InvalidDocument(
                "an incomplete broadcast scan must advance through one wait execution".to_owned(),
            ));
        }
        RunEventKind::RecoveryDecisionRecorded {
            outcome: AuthorityDecision::ResolveSucceeded | AuthorityDecision::ResolveFailed,
            evidence,
            ..
        } if evidence.is_empty() => {
            return Err(PersistenceError::InvalidDocument(
                "terminal external-work resolution requires at least one evidence reference"
                    .to_owned(),
            ));
        }
        RunEventKind::RunPaused { evidence, .. }
        | RunEventKind::RunResumed { evidence, .. }
        | RunEventKind::RunCancellationRequested { evidence, .. }
        | RunEventKind::ExternalOutcomeUncertain { evidence, .. }
        | RunEventKind::ReconciliationDecisionRecorded { evidence, .. }
        | RunEventKind::RecoveryDecisionRecorded { evidence, .. } => {
            context.check_evidence(evidence)?
        }
        _ => {}
    }
    Ok(())
}
