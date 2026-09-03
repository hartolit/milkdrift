use futures_util::StreamExt as _;
use milkdrift_control_protocol::{Command, Cursor};
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
            session.output("run.show", &run)?;
            if run.terminal.as_deref() == Some("failed") {
                return Err(CliError::FailedTask(
                    "run reached a failed terminal outcome".to_owned(),
                ));
            }
            Ok(())
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
    serde_json::from_str(payload).map_err(|error| CliError::Invalid(error.to_string()))
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
        follow(session, run, page.observed_cursor.or(request.cursor)).await?;
    }
    Ok(())
}

async fn submit(session: &CliSession, kind: &str, command: Command) -> Result<(), CliError> {
    let request = session.command_request(command);
    session.output(kind, &session.client().submit(&request).await?)
}

async fn follow(session: &CliSession, run: &str, cursor: Option<Cursor>) -> Result<(), CliError> {
    crate::session::safe_identity(run)?;
    let mut observations = session
        .client()
        .subscribe(format!("v1/runs/{run}/stream"), cursor);
    loop {
        tokio::select! {
            signal = tokio::signal::ctrl_c() => {
                signal.map_err(|error| CliError::Internal(error.to_string()))?;
                return Ok(());
            }
            item = observations.next() => match item {
                Some(Ok(observation)) => session.output("run.observation", &observation)?,
                Some(Err(error)) => {
                    session.stream_status(error.retryable(), &error)?;
                    if !error.retryable() {
                        return Err(error.into());
                    }
                }
                None => return Ok(()),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    #[test]
    fn signal_payload_shape_remains_json() -> Result<(), Box<dyn std::error::Error>> {
        let value: serde_json::Value = serde_json::from_str(r#"{"answer":42}"#)?;
        assert_eq!(value, json!({"answer": 42}));
        Ok(())
    }
}
