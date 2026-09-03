use crate::{DaemonCommand, error::CliError, session::CliSession};

pub(super) async fn execute(session: &CliSession, command: &DaemonCommand) -> Result<(), CliError> {
    match command {
        DaemonCommand::Health(arguments) => {
            session.output("daemon.health", &session.client().health().await?)?;
            if arguments.follow {
                let cursor = session.cursor(arguments.cursor.as_deref())?;
                super::stream::follow(
                    session,
                    "v1/stream/health".to_owned(),
                    cursor,
                    "daemon.observation",
                )
                .await?;
            }
            Ok(())
        }
        DaemonCommand::Readiness => {
            session.output("daemon.readiness", &session.client().readiness().await?)
        }
        DaemonCommand::Authority => {
            session.output("daemon.authority", &session.client().authority().await?)
        }
    }
}
