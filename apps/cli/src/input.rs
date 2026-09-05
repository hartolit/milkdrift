//! Bounded explicit documents, with blocking OS reads outside the async deadline owner.
use crate::error::CliError;
use std::{
    fs,
    io::{self, Read as _},
    path::Path,
};

pub(crate) async fn read_bounded(
    path: &Path,
    maximum: usize,
    kind: &str,
) -> Result<Vec<u8>, CliError> {
    let path = path.to_owned();
    let kind = kind.to_owned();
    tokio::task::spawn_blocking(move || {
        let mut bytes = Vec::new();
        let limit = u64::try_from(maximum).unwrap_or(u64::MAX).saturating_add(1);
        if path == Path::new("-") {
            io::stdin()
                .lock()
                .take(limit)
                .read_to_end(&mut bytes)
                .map_err(|error| {
                    CliError::Invalid(format!("{kind} stdin unavailable: {:?}", error.kind()))
                })?;
        } else {
            let metadata = fs::metadata(&path)
                .map_err(|_| CliError::Invalid(format!("{kind} file unavailable")))?;
            if !metadata.is_file() || metadata.len() > limit.saturating_sub(1) {
                return Err(CliError::Invalid(format!(
                    "{kind} must be a bounded regular file"
                )));
            }
            fs::File::open(path)
                .and_then(|file| file.take(limit).read_to_end(&mut bytes))
                .map_err(|error| {
                    CliError::Invalid(format!("{kind} file unavailable: {:?}", error.kind()))
                })?;
        }
        if bytes.len() > maximum {
            return Err(CliError::Invalid(format!("{kind} exceeds {maximum} bytes")));
        }
        Ok(bytes)
    })
    .await
    .map_err(|error| CliError::Internal(error.to_string()))?
}
