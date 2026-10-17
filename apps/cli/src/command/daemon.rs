use crate::{DaemonCommand, error::CliError, session::CliSession};
use milkdrift_control_protocol::RenameDaemonRequest;

pub(super) async fn execute(session: &CliSession, command: &DaemonCommand) -> Result<(), CliError> {
    match command {
        DaemonCommand::Name => {
            session.output("daemon.name", &session.client().daemon_metadata().await?)
        }
        DaemonCommand::Rename {
            name,
            expected_generation,
            request_id,
        } => {
            let metadata = session.client().daemon_metadata().await?;
            let client = session
                .client()
                .clone()
                .with_daemon_identity(&metadata.daemon_id)?;
            session.output(
                "daemon.rename",
                &client
                    .rename_daemon(&RenameDaemonRequest {
                        request_id: request_id.clone(),
                        expected_generation: *expected_generation,
                        name: name.clone(),
                    })
                    .await?,
            )
        }
        DaemonCommand::RenameReceipt { request_id } => {
            let metadata = session.client().daemon_metadata().await?;
            let client = session
                .client()
                .clone()
                .with_daemon_identity(&metadata.daemon_id)?;
            session.output(
                "daemon.rename_receipt",
                &client.daemon_rename_receipt(request_id).await?,
            )
        }
        DaemonCommand::Health(arguments) => {
            session.output("daemon.health", &session.client().health().await?)?;
            if arguments.follow {
                let cursor = CliSession::cursor(arguments.cursor.as_deref())?;
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
