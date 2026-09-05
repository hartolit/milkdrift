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
            session.output("run.show", &run)
        }
        RunCommand::Wait {
            run,
            terminal,
            poll_ms,
            max_polls,
        } => {
            for poll in 0..*max_polls {
                let state = session.client().run(run).await?;
                if let Some(outcome) = state.terminal.as_deref() {
                    if !terminal.matches(outcome) || outcome != "succeeded" {
                        return Err(CliError::FailedTask(Box::new(state)));
                    }
                    return session.output("run.wait", &state);
                }
                if poll + 1 < *max_polls {
                    tokio::time::sleep(std::time::Duration::from_millis(*poll_ms)).await;
                }
            }
            Err(CliError::Deadline)
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
            session.confirm("request durable run cancellation").await?;
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
    milkdrift_control_protocol::decode_json(payload.as_bytes())
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
