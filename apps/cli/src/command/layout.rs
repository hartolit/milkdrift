use milkdrift_control_protocol::Command;

use crate::{LayoutCommand, error::CliError, session::CliSession};

pub(super) async fn execute(session: &CliSession, command: &LayoutCommand) -> Result<(), CliError> {
    match command {
        LayoutCommand::Get { workflow, revision } => session.output(
            "layout.get",
            &session.client().layout(workflow, revision).await?,
        ),
        LayoutCommand::Put { file } => {
            let layout = session.read_layout(file)?;
            let request = session.command_request(Command::PutLayout { layout });
            session.output("layout.put", &session.client().submit(&request).await?)
        }
    }
}
