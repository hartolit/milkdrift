use std::io::{self, Read};

#[derive(Debug)]
pub(crate) enum BoundedReadError {
    Io(io::Error),
    Limit,
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
