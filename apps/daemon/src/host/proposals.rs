//! Proposal discovery projection and exact control-state query ownership.

use super::{
    ActorSession, ApplicationCommandEffect, ApplicationCursor, ApplicationEffectReference,
    ApplicationPageQuery, AuthorityOperation, CommandAccepted, CommandId, CommandRequest,
    ControlCommand, ControlResult, Cursor, Owner, Page, PageSize, ProposalId, ProposalIndexEntry,
    ProposalIndexStore, ProposalRead, PublicFailure, RunId, TimestampMillis, Value,
    WorkflowProposalDocument, bounded, corruption, cursor_binding, internal, invalid,
    parse_revision_id, public_persistence, public_protocol, snake_debug,
};

pub(super) fn application_effect(
    session: &ActorSession,
    request: &CommandRequest,
    document: &Value,
    result: &CommandAccepted,
    completed_at: TimestampMillis,
) -> Result<(Option<ApplicationEffectReference>, ApplicationCommandEffect), PublicFailure> {
    let bytes = serde_json::to_vec(document).map_err(|_| invalid("invalid proposal JSON"))?;
    let proposal = WorkflowProposalDocument::from_json(&bytes)
        .map_err(|error| invalid(&bounded(&error.to_string())))?;
    let Some(run) = proposal.proposal().run().cloned() else {
        return Ok((None, ApplicationCommandEffect::None));
    };
    let revision = result
        .value
        .get("proposed_revision")
        .and_then(Value::as_str)
        .ok_or_else(internal)
        .and_then(parse_revision_id)?;
    let identity = proposal.proposal().identity().as_str().to_owned();
    let reference = ApplicationEffectReference::Proposal {
        run: run.clone(),
        proposal: identity.clone(),
        proposed_revision: revision.clone(),
    };
    Ok((
        Some(reference),
        ApplicationCommandEffect::IndexProposal(ProposalIndexEntry {
            run,
            proposal: identity,
            proposed_revision: revision,
            receipt_actor: session.actor.clone(),
            receipt_command: CommandId::new(request.command_id.clone())
                .map_err(public_persistence)?,
            created_at: completed_at,
        }),
    ))
}

pub(super) fn page(
    owner: &Owner,
    session: &ActorSession,
    run: &str,
    cursor: Option<&Cursor>,
    limit: u32,
) -> Result<Page<ProposalRead>, PublicFailure> {
    let run_id = RunId::new(run.to_owned()).map_err(|error| invalid(&error.to_string()))?;
    let feed = format!("proposals:{run}");
    let decision = owner.authorize_run_read(
        session,
        run,
        AuthorityOperation::InspectProposal,
        "read:proposals",
    )?;
    let binding = cursor_binding(session, &feed)?;
    let after = cursor
        .map(|cursor| {
            cursor
                .key_for_bound(&feed, &binding, session.cursor_key())
                .map_err(public_protocol)
        })
        .transpose()?
        .map(|value| ApplicationCursor::new(value.into_bytes()).map_err(public_persistence))
        .transpose()?;
    let page = owner
        .store
        .proposal_index(
            &run_id,
            &ApplicationPageQuery {
                after,
                limit: PageSize::new(limit).map_err(public_persistence)?,
            },
        )
        .map_err(public_persistence)?;
    let mut items = Vec::with_capacity(page.items.len());
    for proposal in page.items {
        items.push(exact(
            owner,
            session,
            run,
            &proposal.proposal,
            proposal.proposed_revision.as_str(),
        )?);
    }
    let next_cursor = page
        .next
        .map(|cursor| {
            let key = std::str::from_utf8(cursor.as_bytes())
                .map_err(|_| corruption("proposal index returned an invalid cursor"))?;
            Cursor::new_bound_key(
                &feed,
                key,
                binding.clone(),
                decision.digest(),
                session.cursor_key(),
            )
            .map_err(public_protocol)
        })
        .transpose()?;
    Ok(Page {
        items,
        next_cursor,
        observed_cursor: None,
    })
}

pub(super) fn exact(
    owner: &Owner,
    session: &ActorSession,
    run: &str,
    proposal: &str,
    revision: &str,
) -> Result<ProposalRead, PublicFailure> {
    let run = RunId::new(run.to_owned()).map_err(|error| invalid(&error.to_string()))?;
    let proposal =
        ProposalId::new(proposal.to_owned()).map_err(|error| invalid(&error.to_string()))?;
    let revision = parse_revision_id(revision)?;
    let result = owner.inspect_control(
        session,
        ControlCommand::QueryProposal {
            run,
            proposal,
            proposed_revision: revision,
        },
        None,
        "proposal-status",
    )?;
    let ControlResult::ProposalStatus { value } = result else {
        return Err(internal());
    };
    Ok(ProposalRead {
        proposal_id: value.proposal.as_str().to_owned(),
        proposed_revision: value.proposed_revision.as_str().to_owned(),
        status: snake_debug(&value.reconciliation.state),
        approved: value.reconciliation.approved,
        applied_sequence: value
            .reconciliation
            .applied_sequence
            .map(|sequence| sequence.get()),
    })
}
