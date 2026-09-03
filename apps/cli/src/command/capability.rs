use crate::{CapabilityCommand, error::CliError, session::CliSession};

pub(super) async fn execute(
    session: &CliSession,
    command: &CapabilityCommand,
) -> Result<(), CliError> {
    let capabilities = session.client().capabilities().await?;
    match command {
        CapabilityCommand::List(arguments) => {
            session.output("capability.list", &capabilities)?;
            if arguments.follow {
                let cursor = session.cursor(arguments.cursor.as_deref())?;
                super::stream::follow(
                    session,
                    "v1/stream/capabilities".to_owned(),
                    cursor,
                    "capability.observation",
                )
                .await?;
            }
            Ok(())
        }
        CapabilityCommand::Show { capability } => {
            let matches = capabilities
                .into_iter()
                .filter(|item| &item.capability_id == capability)
                .collect::<Vec<_>>();
            if matches.is_empty() {
                return Err(CliError::NotFound("capability was not found".to_owned()));
            }
            session.output("capability.show", &matches)
        }
    }
}
