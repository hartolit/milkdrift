//! Shared resumable observation presentation for every exposed daemon feed.

use futures_util::StreamExt as _;
use milkdrift_control_protocol::{Cursor, Observation};

use crate::{error::CliError, session::CliSession};

pub(super) async fn follow(
    session: &CliSession,
    path: String,
    cursor: Option<Cursor>,
    output_kind: &str,
) -> Result<(), CliError> {
    let mut observations = session.client().subscribe(path.clone(), cursor);
    let mut reconnects = 0_u16;
    loop {
        match observations.next().await {
            Some(Ok(observation)) => {
                session.output(output_kind, &observation)?;
                match observation.observation {
                    Observation::ResyncRequired { .. } => {
                        if reconnects >= session.cli().max_reconnects {
                            return Err(CliError::Deadline);
                        }
                        reconnects += 1;
                        fresh_view(session, &path).await?;
                        observations = session.client().subscribe(path.clone(), None);
                    }
                    Observation::StreamClosing { reason } => {
                        return Err(milkdrift_control_client::ClientError::Stream(reason).into());
                    }
                    _ => {}
                }
            }
            Some(Err(error)) if error.retryable() => {
                if reconnects >= session.cli().max_reconnects {
                    return Err(CliError::Deadline);
                }
                reconnects += 1;
                session.stream_status(true, &error)?;
            }
            Some(Err(error)) => return Err(error.into()),
            None => {
                return Err(milkdrift_control_client::ClientError::Stream(
                    "observation feed ended".to_owned(),
                )
                .into());
            }
        }
    }
}

async fn fresh_view(session: &CliSession, path: &str) -> Result<(), CliError> {
    match path {
        "v1/stream/health" => {
            session.output("daemon.health.fresh", &session.client().health().await?)
        }
        "v1/stream/capabilities" => session.output(
            "capability.list.fresh",
            &session.client().capabilities().await?,
        ),
        _ => {
            let run = path
                .strip_prefix("v1/runs/")
                .and_then(|path| path.strip_suffix("/stream"))
                .ok_or_else(|| CliError::Invalid("unsupported observation feed".into()))?;
            session.output("run.result.fresh", &session.client().run_result(run).await?)
        }
    }
}
