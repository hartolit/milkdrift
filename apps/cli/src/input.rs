//! Bounded explicit documents, with blocking OS reads outside the async deadline owner.
use crate::error::CliError;
use std::{
    fs,
    io::{self, Read as _},
    path::Path,
};

/// Open an operator-owned record without following a substituted link. Validate
/// the handle before reading or locking it; a prior pathname check is insufficient.
pub(crate) fn open_regular(path: &Path, options: &mut fs::OpenOptions) -> io::Result<fs::File> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600).custom_flags(
            (rustix::fs::OFlags::NOFOLLOW | rustix::fs::OFlags::NONBLOCK)
                .bits()
                .cast_signed(),
        );
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt as _;
        const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
        options.custom_flags(FILE_FLAG_OPEN_REPARSE_POINT);
    }
    let file = options.open(path)?;
    let opened = file.metadata()?;
    let named = fs::symlink_metadata(path)?;
    if !opened.is_file() || !named.is_file() || named.file_type().is_symlink() {
        return Err(io::Error::other(
            "record must be a regular file, not a link",
        ));
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt as _;
        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
        if opened.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
            || named.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
        {
            return Err(io::Error::other("record must not be a reparse point"));
        }
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt as _;
        if opened.dev() != named.dev() || opened.ino() != named.ino() {
            return Err(io::Error::other("record changed while opening"));
        }
    }
    Ok(file)
}

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
