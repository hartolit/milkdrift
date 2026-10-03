use crate::{MethodCommand, error::CliError, session::CliSession};
use milkdrift_control_protocol::Command;

pub(super) async fn execute(session: &CliSession, command: &MethodCommand) -> Result<(), CliError> {
    if let MethodCommand::Prepare { file, output } = command {
        use std::io::Write as _;
        let mut destination = crate::output::PendingFile::create(output)?;
        let document = session
            .read_json(
                file,
                milkdrift_control_protocol::MAX_DOCUMENT_BYTES,
                "publication choices",
            )
            .await?;
        let request = session.command_request(Command::PrepareMethod { document })?;
        let prepared = session.client().submit(&request).await?;
        let bytes = serde_json::to_vec_pretty(&prepared.value)
            .map_err(|error| CliError::Internal(error.to_string()))?;
        destination
            .write_all(&bytes)
            .and_then(|()| destination.commit())
            .map_err(|error| CliError::Internal(error.to_string()))?;
        return session.output(
            "method.prepared",
            &serde_json::json!({"file":output,"method":prepared.value}),
        );
    }
    let (body, label) = match command {
        MethodCommand::Prepare { .. } => {
            return Err(CliError::Internal("preparation already handled".into()));
        }
        MethodCommand::Publish {
            file,
            expected_previous_version,
        } => (
            Command::PublishMethod {
                document: session
                    .read_json(
                        file,
                        milkdrift_control_protocol::MAX_DOCUMENT_BYTES,
                        "published method",
                    )
                    .await?,
                expected_previous_version: *expected_previous_version,
            },
            "method.publish",
        ),
        MethodCommand::Show {
            capability,
            generation,
        } => (
            Command::InspectMethod {
                capability: capability.clone(),
                generation: *generation,
            },
            "method.inspect",
        ),
        MethodCommand::List {
            after_capability,
            after_generation,
            limit,
        } => (
            Command::ListMethods {
                after_capability: after_capability.clone(),
                after_generation: *after_generation,
                limit: *limit,
            },
            "method.list",
        ),
        MethodCommand::Retire {
            capability,
            generation,
            expected_version,
        } => (
            Command::RetireMethod {
                capability: capability.clone(),
                generation: *generation,
                expected_version: *expected_version,
            },
            "method.retire",
        ),
    };
    let request = session.command_request(body)?;
    session.output(label, &session.client().submit(&request).await?)
}
