use milkdrift_control_protocol::Command;

use crate::{BlueprintCommand, error::CliError, session::CliSession};

pub(super) async fn execute(
    session: &CliSession,
    command: &BlueprintCommand,
) -> Result<(), CliError> {
    match command {
        BlueprintCommand::Import { file } => {
            let document = session.read_json(file)?;
            let request = session.command_request(Command::ImportBlueprint { document });
            session.output(
                "blueprint.import",
                &session.client().submit(&request).await?,
            )
        }
        BlueprintCommand::Show { revision } => session.output(
            "blueprint.show",
            &session.client().revision(revision).await?,
        ),
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
