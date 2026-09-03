//! Shared resumable observation presentation for every exposed daemon feed.

use futures_util::StreamExt as _;
use milkdrift_control_protocol::Cursor;

use crate::{error::CliError, session::CliSession};

pub(super) async fn follow(
    session: &CliSession,
    path: String,
    cursor: Option<Cursor>,
    output_kind: &str,
) -> Result<(), CliError> {
    let mut observations = session.client().subscribe(path, cursor);
    loop {
        tokio::select! {
            signal = tokio::signal::ctrl_c() => {
                signal.map_err(|error| CliError::Internal(error.to_string()))?;
                return Ok(());
            }
            item = observations.next() => match item {
                Some(Ok(observation)) => session.output(output_kind, &observation)?,
                Some(Err(error)) if error.retryable() => {
                    session.stream_status(true, &error)?;
                }
                Some(Err(error)) => return Err(error.into()),
                None => return Ok(()),
            }
        }
    }
}
