//! Proposal parsing, decision planning, application, and public result mapping.

use serde_json::Value;

use super::super::{
    ActorSession, CommandAccepted, CommandRequest, ControlCommand, ControlResult, Owner,
    ProposalDecision, ProposalDigest, ProposalId, PublicFailure, ReconciliationDecisionId, RunId,
    WorkflowProposalDocument, accepted_sequence, bounded, internal, invalid, parse_revision_id,
};

pub(super) fn submit(
    owner: &Owner,
    session: &ActorSession,
    request: &CommandRequest,
    document: &Value,
) -> Result<CommandAccepted, PublicFailure> {
    let bytes = serde_json::to_vec(document).map_err(|_| invalid("invalid proposal JSON"))?;
    let proposal = WorkflowProposalDocument::from_json(&bytes)
        .map_err(|error| invalid(&bounded(&error.to_string())))?;
    let digest = proposal.proposal().digest().clone();
    let value = owner.execute_control_result(
        session,
        request,
        request.expected_sequence,
        Some(digest),
        ControlCommand::SubmitProposal { proposal },
        "proposal",
    )?;
    match value {
        ControlResult::ProposalSubmitted { value } => Ok(CommandAccepted {
            command_id: request.command_id.clone(),
            replayed: false,
            resulting_sequence: value
                .reconciliation
                .as_ref()
                .and_then(|item| item.applied_sequence)
                .map(|sequence| sequence.get()),
            result_type: "proposal_submitted".to_owned(),
            value: serde_json::to_value(value).map_err(|_| internal())?,
        }),
        _ => Err(internal()),
    }
}

pub(super) struct DecisionArguments<'a> {
    pub(super) run_id: &'a str,
    pub(super) proposal_id: &'a str,
    pub(super) proposal_digest: &'a str,
    pub(super) proposed_revision: &'a str,
    pub(super) decision_id: &'a str,
    pub(super) decision: ProposalDecision,
}

pub(super) fn decide(
    owner: &Owner,
    session: &ActorSession,
    request: &CommandRequest,
    arguments: DecisionArguments<'_>,
) -> Result<CommandAccepted, PublicFailure> {
    let run =
        RunId::new(arguments.run_id.to_owned()).map_err(|error| invalid(&error.to_string()))?;
    let proposal = ProposalId::new(arguments.proposal_id.to_owned())
        .map_err(|error| invalid(&error.to_string()))?;
    let digest: ProposalDigest =
        serde_json::from_value(Value::String(arguments.proposal_digest.to_owned()))
            .map_err(|error| invalid(&error.to_string()))?;
    let revision = parse_revision_id(arguments.proposed_revision)?;
    let decision_id = ReconciliationDecisionId::new(arguments.decision_id.to_owned())
        .map_err(|error| invalid(&error.to_string()))?;
    let command = match arguments.decision {
        ProposalDecision::Approve => ControlCommand::ApproveProposal {
            run,
            proposal,
            proposal_digest: digest.clone(),
            proposed_revision: revision,
            decision: decision_id,
        },
        ProposalDecision::Reject => ControlCommand::RejectProposal {
            run,
            proposal,
            proposal_digest: digest.clone(),
            proposed_revision: revision,
            decision: decision_id,
        },
    };
    let sequence = owner.execute_control_guarded(
        session,
        request,
        request.expected_sequence,
        Some(digest),
        command,
        "decision",
    )?;
    accepted_sequence(request, sequence, "proposal_decided")
}

pub(super) fn apply(
    owner: &Owner,
    session: &ActorSession,
    request: &CommandRequest,
    run_id: &str,
    proposal_id: &str,
    proposal_digest: &str,
    proposed_revision: &str,
) -> Result<CommandAccepted, PublicFailure> {
    let run = RunId::new(run_id.to_owned()).map_err(|error| invalid(&error.to_string()))?;
    let digest: ProposalDigest = serde_json::from_value(Value::String(proposal_digest.to_owned()))
        .map_err(|error| invalid(&error.to_string()))?;
    let command = ControlCommand::ApplyProposal {
        run,
        proposal: ProposalId::new(proposal_id.to_owned())
            .map_err(|error| invalid(&error.to_string()))?,
        proposal_digest: digest.clone(),
        proposed_revision: parse_revision_id(proposed_revision)?,
    };
    let sequence = owner.execute_control_guarded(
        session,
        request,
        request.expected_sequence,
        Some(digest),
        command,
        "apply",
    )?;
    accepted_sequence(request, sequence, "proposal_applied")
}
