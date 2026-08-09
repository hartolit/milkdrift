use std::fs::File;
use std::io::{self, Read};
use std::path::Path;

#[derive(Debug)]
pub(crate) enum BoundedReadError {
    Io(io::Error),
    Limit,
}

pub(crate) fn read_bounded_file(
    path: &Path,
    maximum_bytes: u64,
) -> Result<Vec<u8>, BoundedReadError> {
    let file = File::open(path).map_err(BoundedReadError::Io)?;
    let known_byte_length = file.metadata().map_err(BoundedReadError::Io)?.len();
    read_bounded(file, known_byte_length, maximum_bytes)
}

pub(crate) fn read_bounded<R: Read>(
    reader: R,
    known_byte_length: u64,
    maximum_bytes: u64,
) -> Result<Vec<u8>, BoundedReadError> {
    if known_byte_length > maximum_bytes {
        return Err(BoundedReadError::Limit);
    }
    let read_limit = maximum_bytes
        .checked_add(1)
        .ok_or(BoundedReadError::Limit)?;
    let mut bytes = Vec::new();
    reader
        .take(read_limit)
        .read_to_end(&mut bytes)
        .map_err(BoundedReadError::Io)?;
    let observed_byte_length = u64::try_from(bytes.len()).map_err(|_| BoundedReadError::Limit)?;
    if observed_byte_length > maximum_bytes {
        return Err(BoundedReadError::Limit);
    }
    Ok(bytes)
}
