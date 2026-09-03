use milkdrift_control_protocol::Command;

use crate::{ControllerCommand, error::CliError, session::CliSession};

pub(super) async fn execute(
    session: &CliSession,
    command: &ControllerCommand,
) -> Result<(), CliError> {
    match command {
        ControllerCommand::Status {
            run,
            controller_execution,
        } => {
            let request = session.command_request(Command::InspectController {
                run_id: run.clone(),
                controller_execution: controller_execution.clone(),
            })?;
            session.output(
                "controller.status",
                &session.client().submit(&request).await?,
            )
        }
        ControllerCommand::Continue {
            run,
            controller_execution,
            decision_id,
        } => {
            session.confirm("continue this exact controller checkpoint")?;
            let request = session.command_request(Command::ContinueController {
                run_id: run.clone(),
                controller_execution: controller_execution.clone(),
                decision_id: decision_id.clone(),
            })?;
            session.output(
                "controller.continue",
                &session.client().submit(&request).await?,
            )
        }
    }
}
