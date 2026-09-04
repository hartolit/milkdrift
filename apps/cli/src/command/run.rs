use milkdrift_control_protocol::Command;
use serde_json::Value;

use crate::{RunCommand, error::CliError, session::CliSession};

pub(super) async fn execute(session: &CliSession, command: &RunCommand) -> Result<(), CliError> {
    match command {
        RunCommand::Start {
            run,
            workflow,
            revision,
        } => {
            submit(
                session,
                "run.start",
                Command::StartRun {
                    run_id: run.clone(),
                    workflow_id: workflow.clone(),
                    revision_id: revision.clone(),
                },
            )
            .await
        }
        RunCommand::List(page) => {
            let request = session.page_request(page.limit, page.cursor.as_deref())?;
            session.output(
                "run.list",
                &session
                    .client()
                    .runs(page.state.as_deref(), page.workflow.as_deref(), &request)
                    .await?,
            )
        }
        RunCommand::Show { run } => {
            let run = session.client().run(run).await?;
            if run.terminal.as_deref() == Some("failed") {
                return Err(CliError::FailedTask(
                    "run reached a failed terminal outcome".to_owned(),
                ));
            }
            session.output("run.show", &run)
        }
        RunCommand::Pause { run } => {
            submit(
                session,
                "run.pause",
                Command::PauseRun {
                    run_id: run.clone(),
                },
            )
            .await
        }
        RunCommand::Resume { run } => {
            submit(
                session,
                "run.resume",
                Command::ResumeRun {
                    run_id: run.clone(),
                },
            )
            .await
        }
        RunCommand::Cancel { run } => {
            session.confirm("request durable run cancellation")?;
            submit(
                session,
                "run.cancel",
                Command::CancelRun {
                    run_id: run.clone(),
                },
            )
            .await
        }
        RunCommand::Signal {
            run,
            signal_id,
            signal_type,
            correlation,
            broadcast,
            payload,
        } => {
            let payload = signal_payload(payload)?;
            let command = Command::SignalRun {
                run_id: run.clone(),
                signal_id: signal_id.clone(),
                signal_type: signal_type.clone(),
                correlation: correlation.clone(),
                broadcast: *broadcast,
                payload,
            };
            submit(session, "run.signal", command).await
        }
        RunCommand::Timeline {
            run,
            limit,
            cursor,
            follow: should_follow,
        } => timeline(session, run, *limit, cursor.as_deref(), *should_follow).await,
    }
}

fn signal_payload(payload: &str) -> Result<Value, CliError> {
    if payload.len() > milkdrift_capability::MAX_DOCUMENT_BYTES {
        return Err(CliError::Invalid(format!(
            "signal payload exceeds {} bytes",
            milkdrift_capability::MAX_DOCUMENT_BYTES
        )));
    }
    let value = milkdrift_contracts::parse_json_without_duplicates(payload.as_bytes())
        .map_err(|error| CliError::Invalid(error.to_string()))?;
    milkdrift_capability::BoundedJson::new(value)
        .map(|bounded| bounded.value().clone())
        .map_err(|error| CliError::Invalid(error.to_string()))
}

async fn timeline(
    session: &CliSession,
    run: &str,
    limit: u32,
    cursor: Option<&str>,
    should_follow: bool,
) -> Result<(), CliError> {
    let request = session.page_request(limit, cursor)?;
    let page = session.client().timeline(run, &request).await?;
    session.output("run.timeline", &page)?;
    if should_follow {
        // Timeline-page cursors are bound to `timeline:<run>` and cannot authorize the
        // independently scoped `run:<run>` observation stream. Establishing the run stream
        // without a cross-feed cursor yields its current bounded observation before updates.
        follow(session, run).await?;
    }
    Ok(())
}

async fn submit(session: &CliSession, kind: &str, command: Command) -> Result<(), CliError> {
    let request = session.command_request(command)?;
    session.output(kind, &session.client().submit(&request).await?)
}

async fn follow(session: &CliSession, run: &str) -> Result<(), CliError> {
    crate::session::safe_identity(run)?;
    super::stream::follow(
        session,
        format!("v1/runs/{run}/stream"),
        None,
        "run.observation",
    )
    .await
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::signal_payload;

    #[test]
    fn signal_payload_shape_remains_json() -> Result<(), Box<dyn std::error::Error>> {
        let value = signal_payload(r#"{"answer":42}"#)?;
        assert_eq!(value, json!({"answer": 42}));
        assert!(signal_payload(r#"{"answer":1,"answer":2}"#).is_err());
        Ok(())
    }
}
