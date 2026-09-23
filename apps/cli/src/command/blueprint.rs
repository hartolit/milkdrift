use std::path::Path;

use milkdrift_control_protocol::{Command, RevisionRead};
use serde_json::json;

use crate::{BlueprintCommand, error::CliError, session::CliSession};

pub(super) async fn execute(
    session: &CliSession,
    command: &BlueprintCommand,
) -> Result<(), CliError> {
    match command {
        BlueprintCommand::Govern { .. }
        | BlueprintCommand::Create { .. }
        | BlueprintCommand::EffectPolicy { .. } => Err(CliError::Internal(
            "local agreement authoring reached a connected session".to_owned(),
        )),
        BlueprintCommand::Validate { file } => {
            let document = session
                .read_json(
                    file,
                    milkdrift_control_protocol::MAX_DOCUMENT_BYTES,
                    "blueprint document",
                )
                .await?;
            let request = session.command_request(Command::ValidateBlueprint { document })?;
            session.output(
                "blueprint.validate",
                &session.client().submit(&request).await?,
            )
        }
        BlueprintCommand::Import { file } => {
            let document = session
                .read_json(
                    file,
                    milkdrift_control_protocol::MAX_DOCUMENT_BYTES,
                    "blueprint document",
                )
                .await?;
            let request = session.command_request(Command::ImportBlueprint { document })?;
            session.output(
                "blueprint.import",
                &session.client().submit(&request).await?,
            )
        }
        BlueprintCommand::Show {
            revision,
            document,
            output,
        } => show(session, revision, *document, output.as_deref()).await,
        BlueprintCommand::Export { revision, output } => {
            show(session, revision, false, Some(output)).await
        }
        BlueprintCommand::List(page) => {
            let request = session.page_request(page.limit, page.cursor.as_deref())?;
            session.output(
                "blueprint.list",
                &session
                    .client()
                    .revisions(page.workflow.as_deref(), &request)
                    .await?,
            )
        }
        BlueprintCommand::Diff { from, to } => session.output(
            "blueprint.diff",
            &session.client().revision_diff(from, to).await?,
        ),
    }
}

async fn show(
    session: &CliSession,
    revision: &str,
    document: bool,
    output: Option<&Path>,
) -> Result<(), CliError> {
    let mut read = session.client().revision(revision).await?;
    if !document && output.is_none() {
        read.document = None;
        return session.output("blueprint.show", &read);
    }
    let bytes = canonical_document(&read)?;
    session.write_exact_document(output, &bytes)?;
    if let Some(destination) = output {
        session.output(
            "blueprint.document",
            &json!({
                "revision_id": read.summary.revision_id,
                "semantic_digest": read.summary.semantic_digest,
                "output": destination,
                "size": bytes.len(),
            }),
        )?;
    }
    Ok(())
}

fn canonical_document(read: &RevisionRead) -> Result<Vec<u8>, CliError> {
    let value = read
        .document
        .as_ref()
        .ok_or_else(|| CliError::Internal("revision document is unavailable".to_owned()))?;
    // The daemon owns semantic validation. Value maps preserve canonical key order.
    milkdrift_control_protocol::encode_json(value)
        .map_err(|error| CliError::Internal(error.to_string()))
}

pub(super) async fn govern(cli: &crate::Cli) -> Result<(), CliError> {
    use milkdrift_blueprint::{
        AdaptationScope, AuthorRef, BlueprintRevisionDocument, GoverningAgreement, Mutation,
        MutationBatch, NodeId,
    };
    use std::io::Write as _;
    let crate::TopCommand::Blueprint {
        command:
            BlueprintCommand::Govern {
                file,
                scope,
                name,
                effect_policy,
                author,
                output,
            },
    } = &cli.command
    else {
        return Err(CliError::Internal(
            "agreement authoring command required".to_owned(),
        ));
    };
    let invalid = |error: &dyn std::fmt::Display| CliError::Invalid(error.to_string());
    let bytes = crate::input::read_bounded(file, 4_194_304, "blueprint").await?;
    let (_, baseline) = BlueprintRevisionDocument::from_json(&bytes).map_err(|e| invalid(&e))?;
    let scope = AdaptationScope::from_json(
        &crate::input::read_bounded(scope, 65_536, "adaptation scope").await?,
    )
    .map_err(|e| invalid(&e))?;
    let agreement = GoverningAgreement::seal(
        NodeId::new(name).map_err(|e| invalid(&e))?,
        &baseline,
        scope,
        effect_policy.clone(),
    )
    .map_err(|e| invalid(&e))?;
    let agreement_id = agreement.digest().to_owned();
    let mutation = MutationBatch::new(vec![Mutation::SetAgreement {
        agreement: Some(agreement),
    }])
    .map_err(|e| invalid(&e))?;
    let revision = baseline
        .revise(
            baseline.id(),
            mutation,
            AuthorRef::new(author).map_err(|e| invalid(&e))?,
            cli.reason.clone(),
        )
        .map_err(|e| invalid(&e))?;
    let bytes = BlueprintRevisionDocument::new(&revision)
        .to_canonical_json()
        .map_err(|e| invalid(&e))?;
    let mut destination = crate::output::PendingFile::create(output)?;
    destination
        .write_all(&bytes)
        .and_then(|()| destination.commit())
        .map_err(|e| CliError::Internal(e.to_string()))?;
    crate::output::success(
        cli,
        "blueprint.govern",
        &serde_json::json!({
            "revision_id": revision.id(), "agreement": agreement_id, "output": output, "size": bytes.len(),
        }),
    )
}

pub(super) async fn author(cli: &crate::Cli) -> Result<(), CliError> {
    let invalid = |error: &dyn std::fmt::Display| CliError::Invalid(error.to_string());
    match &cli.command {
        crate::TopCommand::Blueprint {
            command: BlueprintCommand::EffectPolicy { file },
        } => {
            let bytes = crate::input::read_bounded(file, 65_536, "effect policy").await?;
            let policy = milkdrift_authority::ProtectedEffectPolicy::from_json(&bytes)
                .map_err(|e| invalid(&e))?;
            crate::output::success(
                cli,
                "blueprint.effect-policy",
                &json!({"digest": policy.digest().map_err(|e| invalid(&e))?, "policy": policy}),
            )
        }
        crate::TopCommand::Blueprint {
            command:
                BlueprintCommand::Create {
                    file,
                    workflow,
                    author,
                    output,
                },
        } => {
            use std::io::Write as _;
            let bytes = crate::input::read_bounded(file, 4_194_304, "method mutations").await?;
            let value = milkdrift_control_protocol::decode_json::<serde_json::Value>(&bytes)
                .map_err(|e| invalid(&e))?;
            let operations: Vec<milkdrift_blueprint::Mutation> =
                serde_json::from_value(value).map_err(|e| invalid(&e))?;
            let revision = milkdrift_blueprint::BlueprintRevision::genesis(
                milkdrift_blueprint::WorkflowId::new(workflow).map_err(|e| invalid(&e))?,
                milkdrift_blueprint::MutationBatch::new(operations).map_err(|e| invalid(&e))?,
                milkdrift_blueprint::AuthorRef::new(author).map_err(|e| invalid(&e))?,
                cli.reason.clone(),
            )
            .map_err(|e| invalid(&e))?;
            let bytes = milkdrift_blueprint::BlueprintRevisionDocument::new(&revision)
                .to_canonical_json()
                .map_err(|e| invalid(&e))?;
            let mut destination = crate::output::PendingFile::create(output)?;
            destination
                .write_all(&bytes)
                .and_then(|()| destination.commit())
                .map_err(|e| CliError::Internal(e.to_string()))?;
            crate::output::success(
                cli,
                "blueprint.create",
                &json!({"revision_id":revision.id(), "output":output,"size":bytes.len()}),
            )
        }
        _ => govern(cli).await,
    }
}
