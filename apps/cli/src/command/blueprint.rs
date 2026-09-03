use std::path::Path;

use milkdrift_blueprint::BlueprintRevisionDocument;
use milkdrift_control_protocol::{Command, RevisionRead};
use serde_json::json;

use crate::{BlueprintCommand, error::CliError, session::CliSession};

pub(super) async fn execute(
    session: &CliSession,
    command: &BlueprintCommand,
) -> Result<(), CliError> {
    match command {
        BlueprintCommand::Validate { file } => {
            let document = session.read_json(
                file,
                milkdrift_control_protocol::MAX_DOCUMENT_BYTES,
                "blueprint document",
            )?;
            let request = session.command_request(Command::ValidateBlueprint { document })?;
            session.output(
                "blueprint.validate",
                &session.client().submit(&request).await?,
            )
        }
        BlueprintCommand::Import { file } => {
            let document = session.read_json(
                file,
                milkdrift_control_protocol::MAX_DOCUMENT_BYTES,
                "blueprint document",
            )?;
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
    let read = session.client().revision(revision).await?;
    if !document && output.is_none() {
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
    let encoded =
        serde_json::to_vec(value).map_err(|error| CliError::Internal(error.to_string()))?;
    let (document, revision) = BlueprintRevisionDocument::from_json(&encoded)
        .map_err(|error| CliError::Internal(error.to_string()))?;
    if revision.id().as_str() != read.summary.revision_id
        || revision.content_digest().as_str() != read.summary.semantic_digest
    {
        return Err(CliError::Internal(
            "revision document does not match its read-model identity".to_owned(),
        ));
    }
    document
        .to_canonical_json()
        .map_err(|error| CliError::Internal(error.to_string()))
}
