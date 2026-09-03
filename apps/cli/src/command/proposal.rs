use milkdrift_control::WorkflowProposalDocument;
use milkdrift_control_protocol::{Command, ProposalDecision};

use crate::{ProposalCommand, ProposalDecisionArgs, error::CliError, session::CliSession};

pub(super) async fn execute(
    session: &CliSession,
    command: &ProposalCommand,
) -> Result<(), CliError> {
    match command {
        ProposalCommand::Submit { file } => {
            let document = session.read_json(file)?;
            let bytes = serde_json::to_vec(&document)
                .map_err(|error| CliError::Internal(error.to_string()))?;
            let proposal = WorkflowProposalDocument::from_json(&bytes)
                .map_err(|error| CliError::Invalid(error.to_string()))?;
            let mut request = session.command_request(Command::SubmitProposal { document });
            request.expected_revision =
                Some(proposal.proposal().base_revision().as_str().to_owned());
            session.output("proposal.submit", &session.client().submit(&request).await?)
        }
        ProposalCommand::List { run, limit, cursor } => {
            let page = session.page_request(*limit, cursor.as_deref())?;
            session.output(
                "proposal.list",
                &session.client().proposals(run, &page).await?,
            )
        }
        ProposalCommand::Show {
            run,
            proposal,
            revision,
        } => session.output(
            "proposal.show",
            &session.client().proposal(run, proposal, revision).await?,
        ),
        ProposalCommand::Approve(arguments) => {
            session.confirm("approve this exact workflow proposal")?;
            decide(session, arguments, ProposalDecision::Approve).await
        }
        ProposalCommand::Reject(arguments) => {
            session.confirm("reject this exact workflow proposal")?;
            decide(session, arguments, ProposalDecision::Reject).await
        }
        ProposalCommand::Apply(arguments) => {
            session.confirm("apply this exact workflow proposal")?;
            let mut request = session.command_request(Command::ApplyProposal {
                run_id: arguments.run.clone(),
                proposal_id: arguments.proposal.clone(),
                proposal_digest: arguments.proposal_digest.clone(),
                proposed_revision: arguments.proposed_revision.clone(),
            });
            request.expected_revision = Some(arguments.proposed_revision.clone());
            session.output("proposal.apply", &session.client().submit(&request).await?)
        }
    }
}

async fn decide(
    session: &CliSession,
    arguments: &ProposalDecisionArgs,
    decision: ProposalDecision,
) -> Result<(), CliError> {
    let mut request = session.command_request(Command::DecideProposal {
        run_id: arguments.run.clone(),
        proposal_id: arguments.proposal.clone(),
        proposal_digest: arguments.proposal_digest.clone(),
        proposed_revision: arguments.proposed_revision.clone(),
        decision_id: arguments.decision_id.clone(),
        decision,
    });
    request.expected_revision = Some(arguments.proposed_revision.clone());
    session.output("proposal.decide", &session.client().submit(&request).await?)
}
