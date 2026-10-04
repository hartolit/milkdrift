use super::*;
use milkdrift_capability::{
    ErrorClass, InvocationEvent, InvocationEventKind, InvocationFailure, InvocationId,
    InvocationTerminal, SideEffectClass,
};
use milkdrift_peer_protocol::{
    ArchivedExecutionSummary, ObservationCategory, PeerExecutionId, PeerObservation,
    RemoteExecutionStatus,
};

fn completed(status: TerminalStatus) -> EvidenceResult<ObservationPage> {
    let execution = PeerExecutionId::new("driver-execution")?;
    let invocation = InvocationId::new("driver-invocation")?;
    let output = ArtifactReference::new(
        "stdout",
        "00".repeat(32),
        Some("text/plain".to_owned()),
        Some(39),
    )?;
    let observation = |sequence, category, kind| -> EvidenceResult<PeerObservation> {
        Ok(PeerObservation {
            execution: execution.clone(),
            sequence,
            category,
            observed_at_unix_ms: sequence,
            event: InvocationEvent::new(invocation.clone(), sequence, kind)?,
        })
    };
    let failure = (status != TerminalStatus::Success)
        .then(|| {
            InvocationFailure::new(
                ErrorClass::Provider,
                false,
                "exit",
                "process failed after emitting stdout",
                None,
            )
        })
        .transpose()?;
    let terminal =
        InvocationTerminal::new(status, Vec::new(), failure, None, SideEffectClass::Unknown)?;
    let observations = vec![
        observation(
            1,
            ObservationCategory::Artifact,
            InvocationEventKind::Output {
                name: "stdout".to_owned(),
                reference: output,
            },
        )?,
        observation(
            2,
            ObservationCategory::Terminal,
            InvocationEventKind::Terminal { terminal },
        )?,
    ];
    let page = ObservationPage {
        status: RemoteExecutionStatus::Terminal,
        execution,
        after_sequence: 0,
        observations,
        next_sequence: 2,
        terminal: true,
        closed: true,
        history: ObservationHistory::Hot,
    };
    page.validate(128)?;
    Ok(page)
}

#[test]
fn expected_output_does_not_hide_failed_direct_execution() -> EvidenceResult {
    let page = completed(TerminalStatus::Failure)?;
    assert!(successful_outputs(&[page]).is_err());
    let page = completed(TerminalStatus::Success)?;
    assert_eq!(successful_outputs(&[page])?.len(), 1);
    Ok(())
}

#[test]
fn retained_outputs_survive_page_boundaries_and_archival() -> EvidenceResult {
    let page = completed(TerminalStatus::Success)?;
    let mut first = page.clone();
    first.observations.pop();
    first.next_sequence = 1;
    first.terminal = false;
    first.closed = false;
    let mut last = page.clone();
    last.observations.remove(0);
    last.after_sequence = 1;
    first.validate(128)?;
    last.validate(128)?;
    let expected = successful_outputs(&[first, last])?;
    let mut archived = page.clone();
    archived.observations.clear();
    archived.next_sequence = 0;
    archived.history = ObservationHistory::Archived {
        summary: Box::new(ArchivedExecutionSummary {
            status: RemoteExecutionStatus::Terminal,
            last_sequence: 2,
            observation_digest: format!("b3_{}", "0".repeat(64)),
            archived_at_unix_ms: 3,
            final_observation: page.observations.last().cloned(),
            output_observations: vec![
                page.observations
                    .first()
                    .ok_or("output observation absent")?
                    .clone(),
            ],
            uncertainty_reason: None,
        }),
    };
    archived.validate(128)?;
    assert_eq!(successful_outputs(&[archived])?, expected);
    Ok(())
}

#[test]
fn closed_uncertainty_cannot_qualify_an_output() -> EvidenceResult {
    let mut page = completed(TerminalStatus::Success)?;
    page.observations.pop();
    page.next_sequence = 1;
    page.status = RemoteExecutionStatus::OutcomeUnknown;
    page.terminal = false;
    page.validate(128)?;
    assert!(successful_outputs(&[page]).is_err());
    Ok(())
}
