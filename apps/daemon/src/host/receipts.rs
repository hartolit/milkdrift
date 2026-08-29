//! External command idempotency, deterministic rejection, and recovery boundary.

use super::*;

pub(super) fn execute(
    owner: &mut Owner,
    session: &ActorSession,
    mut request: CommandRequest,
) -> Result<CommandAccepted, PublicFailure> {
    request.validate().map_err(public_protocol)?;
    let command = CommandId::new(request.command_id.clone()).map_err(public_persistence)?;
    let digest = command_fingerprint(session, &request)?;
    if let Some(existing) = owner
        .store
        .application_command_receipt(&session.actor, &command)
        .map_err(public_persistence)?
    {
        if existing.command_digest() != &digest {
            return Err(conflict(
                "command identity was already used with different canonical content",
            ));
        }
        return stored_application_result(&existing);
    }
    if let Command::PutLayout { layout } = &mut request.command {
        layout.author = session.actor.as_str().to_owned();
        layout.digest = layout.computed_digest().map_err(public_protocol)?;
    }
    let created_at = TimestampMillis::new(unix_millis());
    match owner.execute_new_command(session, &request) {
        Ok(result) => {
            let (effect_reference, effect) = application_effect(
                session,
                &request,
                &result,
                TimestampMillis::new(unix_millis()),
            )?;
            let document = serde_json::to_vec(&StoredApplicationResult::Accepted(result.clone()))
                .map_err(|_| internal())?;
            let receipt = application_receipt(
                session,
                command.clone(),
                digest.clone(),
                created_at,
                ApplicationCommandResult::Accepted {
                    document,
                    effect: effect_reference,
                },
            )?;
            match owner
                .store
                .commit_application_command(&ApplicationCommandCommit { receipt, effect })
            {
                Ok(ApplicationCommandCommitOutcome::Committed) => Ok(result),
                Ok(ApplicationCommandCommitOutcome::Replayed(existing)) => {
                    stored_application_result(&existing)
                }
                Err(error) => {
                    let failure = public_persistence(error);
                    if receipt_rejection(&failure) {
                        persist_rejection(owner, session, command, digest, created_at, failure)
                    } else {
                        Err(failure)
                    }
                }
            }
        }
        Err(failure) if receipt_rejection(&failure) => {
            persist_rejection(owner, session, command, digest, created_at, failure)
        }
        Err(failure) => Err(failure),
    }
}

fn persist_rejection(
    owner: &Owner,
    session: &ActorSession,
    command: CommandId,
    digest: IntegrityDigest,
    created_at: TimestampMillis,
    failure: PublicFailure,
) -> Result<CommandAccepted, PublicFailure> {
    let document = serde_json::to_vec(&StoredApplicationResult::Rejected(failure.clone()))
        .map_err(|_| internal())?;
    let receipt = application_receipt(
        session,
        command,
        digest,
        created_at,
        ApplicationCommandResult::Rejected { document },
    )?;
    match owner
        .store
        .commit_application_command(&ApplicationCommandCommit {
            receipt,
            effect: ApplicationCommandEffect::None,
        })
        .map_err(public_persistence)?
    {
        ApplicationCommandCommitOutcome::Committed => Err(failure),
        ApplicationCommandCommitOutcome::Replayed(existing) => stored_application_result(&existing),
    }
}

fn command_fingerprint(
    session: &ActorSession,
    request: &CommandRequest,
) -> Result<IntegrityDigest, PublicFailure> {
    let bytes = milkdrift_control_protocol::encode_json(request).map_err(public_protocol)?;
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"milkdrift.daemon-command.v1\0");
    hasher.update(session.actor.as_str().as_bytes());
    hasher.update(session.grant.identity().as_str().as_bytes());
    hasher.update(&session.grant.revision().to_le_bytes());
    hasher.update(
        session
            .grant
            .digest()
            .map_err(|_| internal())?
            .as_str()
            .as_bytes(),
    );
    hasher.update(&bytes);
    IntegrityDigest::new(format!("b3_{}", hasher.finalize())).map_err(public_persistence)
}

#[derive(Deserialize, Serialize)]
#[serde(
    rename_all = "snake_case",
    tag = "outcome",
    content = "value",
    deny_unknown_fields
)]
enum StoredApplicationResult {
    Accepted(CommandAccepted),
    Rejected(PublicFailure),
}

fn application_receipt(
    session: &ActorSession,
    command: CommandId,
    command_digest: IntegrityDigest,
    created_at: TimestampMillis,
    result: ApplicationCommandResult,
) -> Result<ApplicationCommandReceipt, PublicFailure> {
    ApplicationCommandReceipt::new(
        session.actor.clone(),
        command,
        APPLICATION_COMMAND_SCHEMA_VERSION,
        command_digest,
        session.grant.identity().clone(),
        session.grant.revision(),
        session.grant.digest().map_err(|_| internal())?.clone(),
        None,
        created_at,
        TimestampMillis::new(unix_millis()),
        result,
    )
    .map_err(public_persistence)
}

fn stored_application_result(
    receipt: &ApplicationCommandReceipt,
) -> Result<CommandAccepted, PublicFailure> {
    let stored: StoredApplicationResult = serde_json::from_slice(receipt.result().document())
        .map_err(|_| corruption("stored application receipt result failed decoding"))?;
    match (receipt.result(), stored) {
        (
            ApplicationCommandResult::Accepted { .. },
            StoredApplicationResult::Accepted(mut value),
        ) => {
            value.replayed = true;
            Ok(value)
        }
        (ApplicationCommandResult::Rejected { .. }, StoredApplicationResult::Rejected(value)) => {
            Err(value)
        }
        _ => Err(corruption(
            "stored application receipt disposition disagrees with its result document",
        )),
    }
}

fn receipt_rejection(failure: &PublicFailure) -> bool {
    !failure.retryable
        && matches!(
            failure.code,
            ErrorCode::Unauthorized
                | ErrorCode::InvalidInput
                | ErrorCode::Conflict
                | ErrorCode::NotFound
        )
}

fn application_effect(
    session: &ActorSession,
    request: &CommandRequest,
    result: &CommandAccepted,
    completed_at: TimestampMillis,
) -> Result<(Option<ApplicationEffectReference>, ApplicationCommandEffect), PublicFailure> {
    match &request.command {
        Command::PutLayout { layout } => layouts::application_effect(session, layout, completed_at),
        Command::SubmitProposal { document } => {
            proposals::application_effect(session, request, document, result, completed_at)
        }
        Command::ImportBlueprint { document } | Command::ValidateBlueprint { document } => {
            let bytes =
                serde_json::to_vec(document).map_err(|_| invalid("invalid blueprint JSON"))?;
            let (_document, revision) = BlueprintRevisionDocument::from_json(&bytes)
                .map_err(|error| invalid(&bounded(&error.to_string())))?;
            Ok((
                Some(ApplicationEffectReference::Revision {
                    revision: revision.id().clone(),
                }),
                ApplicationCommandEffect::None,
            ))
        }
        command => {
            let Some(sequence) = result.resulting_sequence else {
                return Ok((None, ApplicationCommandEffect::None));
            };
            let Some(run_id) = command_run_identity(command) else {
                return Ok((None, ApplicationCommandEffect::None));
            };
            let run = RunId::new(run_id.to_owned()).map_err(|error| invalid(&error.to_string()))?;
            Ok((
                Some(ApplicationEffectReference::RunSequence {
                    run,
                    resulting_sequence: RunSequence::new(sequence),
                }),
                ApplicationCommandEffect::None,
            ))
        }
    }
}

fn command_run_identity(command: &Command) -> Option<&str> {
    match command {
        Command::StartRun { run_id, .. }
        | Command::PauseRun { run_id }
        | Command::ResumeRun { run_id }
        | Command::CancelRun { run_id }
        | Command::SignalRun { run_id, .. }
        | Command::ResolveWork { run_id, .. }
        | Command::DecideProposal { run_id, .. }
        | Command::ApplyProposal { run_id, .. } => Some(run_id),
        Command::ImportBlueprint { .. }
        | Command::ValidateBlueprint { .. }
        | Command::SubmitProposal { .. }
        | Command::PutLayout { .. } => None,
    }
}
