use crate::{DaemonCommand, error::CliError, session::CliSession};

pub(super) async fn execute(session: &CliSession, command: &DaemonCommand) -> Result<(), CliError> {
    match command {
        DaemonCommand::Health => session.output("daemon.health", &session.client().health().await?),
        DaemonCommand::Readiness => {
            session.output("daemon.readiness", &session.client().readiness().await?)
        }
        DaemonCommand::Authority => {
            session.output("daemon.authority", &session.client().authority().await?)
        }
    }
}
