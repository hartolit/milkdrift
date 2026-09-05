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
    let mut reconnects = 0_u16;
    loop {
        match observations.next().await {
            Some(Ok(observation)) => session.output(output_kind, &observation)?,
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
