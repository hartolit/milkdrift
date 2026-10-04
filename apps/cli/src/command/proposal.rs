use milkdrift_control_protocol::{Command, ProposalDecision};

use crate::{ProposalCommand, ProposalDecisionArgs, error::CliError, session::CliSession};

pub(super) async fn execute(
    session: &CliSession,
    command: &ProposalCommand,
) -> Result<(), CliError> {
    match command {
        ProposalCommand::Repair(arguments) => prepare_repair(session, arguments).await,
        ProposalCommand::Submit { file } => {
            let document = session
                .read_json(
                    file,
                    milkdrift_control_protocol::MAX_DOCUMENT_BYTES,
                    "proposal document",
                )
                .await?;
            let request = session.command_request(Command::SubmitProposal { document })?;
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
            decide(session, arguments, ProposalDecision::Approve).await
        }
        ProposalCommand::Reject(arguments) => {
            decide(session, arguments, ProposalDecision::Reject).await
        }
        ProposalCommand::Apply(arguments) => {
            let preview = impact(
                session,
                &arguments.run,
                &arguments.proposal,
                &arguments.proposed_revision,
            )
            .await?;
            session
                .confirm("apply this exact workflow proposal")
                .await?;
            let mut request = session.command_request_with_revision(
                Command::ApplyProposal {
                    run_id: arguments.run.clone(),
                    proposal_id: arguments.proposal.clone(),
                    proposal_digest: arguments.proposal_digest.clone(),
                    proposed_revision: arguments.proposed_revision.clone(),
                },
                &arguments.proposed_revision,
            )?;
            request.expected_sequence.get_or_insert(preview.sequence);
            session.output("proposal.apply", &session.client().submit(&request).await?)
        }
    }
}

async fn decide(
    session: &CliSession,
    arguments: &ProposalDecisionArgs,
    decision: ProposalDecision,
) -> Result<(), CliError> {
    let preview = impact(
        session,
        &arguments.run,
        &arguments.proposal,
        &arguments.proposed_revision,
    )
    .await?;
    session
        .confirm(match decision {
            ProposalDecision::Approve => "approve this exact workflow proposal",
            ProposalDecision::Reject => "reject this exact workflow proposal",
        })
        .await?;
    let mut request = session.command_request_with_revision(
        Command::DecideProposal {
            run_id: arguments.run.clone(),
            proposal_id: arguments.proposal.clone(),
            proposal_digest: arguments.proposal_digest.clone(),
            proposed_revision: arguments.proposed_revision.clone(),
            decision_id: arguments.decision_id.clone(),
            decision,
        },
        &arguments.proposed_revision,
    )?;
    request.expected_sequence.get_or_insert(preview.sequence);
    session.output("proposal.decide", &session.client().submit(&request).await?)
}

async fn prepare_repair(session: &CliSession, args: &crate::RepairArgs) -> Result<(), CliError> {
    let bytes = crate::input::read_bounded(
        &args.prompt,
        milkdrift_control_protocol::MAX_DOCUMENT_BYTES,
        "repair prompt",
    )
    .await?;
    let prompt = String::from_utf8(bytes)
        .map_err(|_| CliError::Invalid("repair prompt must be UTF-8".into()))?;
    let state = session.client().run(&args.run).await?;
    let revision = state
        .revision_id
        .as_deref()
        .ok_or_else(|| CliError::Invalid("run has no revision".into()))?;
    let mut request = session.command_request_with_revision(
        Command::PrepareModelRepair {
            run_id: args.run.clone(),
            proposal_id: args.proposal.clone(),
            repair: milkdrift_control_protocol::ModelRepair {
                failed_step: args.failed_step.clone(),
                repair_step: args.new_step.clone(),
                capability: args.model.clone(),
                prompt,
                maximum_output_units: args.maximum_output_units,
            },
        },
        revision,
    )?;
    request.expected_sequence.get_or_insert(state.sequence);
    let prepared = session.client().submit(&request).await?;
    let document = prepared
        .value
        .get("document")
        .ok_or_else(|| CliError::Internal("repair response has no proposal".into()))?;
    let bytes = milkdrift_control_protocol::encode_json(document)
        .map_err(|error| CliError::Invalid(error.to_string()))?;
    crate::output::PendingFile::create(&args.file)?.write_complete(&bytes)?;
    session.output("proposal.repair", &serde_json::json!({"file":args.file,"proposal_id":args.proposal,"base_revision":revision,"sequence":state.sequence,"summary":prepared.value["summary"]}))
}

async fn impact(
    session: &CliSession,
    run: &str,
    proposal: &str,
    revision: &str,
) -> Result<milkdrift_control_protocol::ProposalRead, CliError> {
    let value = session.client().proposal(run, proposal, revision).await?;
    if session.cli().json {
        println!(
            "{}",
            crate::output::encode(
                "proposal.impact",
                session.cli().command_id.as_deref(),
                "success",
                serde_json::to_value(&value)
                    .map_err(|error| CliError::Internal(error.to_string()))?,
                serde_json::Value::Null,
                false
            )?
        );
    } else {
        println!("Affected work at run sequence {}:", value.sequence);
        if let Some(items) = &value.impact {
            for item in items {
                println!(
                    "  {}: {} — {}",
                    serde_json::to_string(&item.node).unwrap_or_default(),
                    item.action,
                    serde_json::to_string(&item.reason).unwrap_or_default()
                );
            }
        } else {
            println!("  Current plan detail is unavailable; inspect the proposal before applying.");
        }
    }
    Ok(value)
}
