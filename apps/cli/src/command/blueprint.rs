use std::path::Path;

use milkdrift_control_protocol::{Command, RevisionRead};
use serde_json::json;

use crate::{BlueprintCommand, error::CliError, session::CliSession};

pub(super) async fn execute(
    session: &CliSession,
    command: &BlueprintCommand,
) -> Result<(), CliError> {
    match command {
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
