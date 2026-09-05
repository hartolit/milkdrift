//! Bounded physical iteration; cursor encoding and semantic validators have separate owners.
use super::{
    Bound, IntegrityScanCursor, IntegrityScanResult, PersistenceError, cursor::make_index_cursor,
    cursor::push_failure, error,
};

#[allow(clippy::too_many_arguments)] // One bounded phase shares cursor and result state with the integrity driver.
pub(crate) fn scan_binary_phase<V: redb::Value + 'static>(
    phase: u8,
    start_phase: u8,
    start_key: Option<&[u8]>,
    table: &impl redb::ReadableTable<&'static [u8], V>,
    maximum: u64,
    verify_artifact_content: bool,
    result: &mut IntegrityScanResult,
    last_cursor: &mut Option<IntegrityScanCursor>,
    more_remaining: &mut bool,
    component: &str,
    mut validate: impl for<'a> FnMut(&[u8], V::SelfType<'a>) -> Result<(), PersistenceError>,
) -> Result<(), PersistenceError> {
    if *more_remaining || phase < start_phase {
        return Ok(());
    }
    let lower = if phase == start_phase {
        start_key.map_or(Bound::Unbounded, Bound::Excluded)
    } else {
        Bound::Unbounded
    };
    for item in table
        .range::<&[u8]>((lower, Bound::Unbounded))
        .map_err(error::redb)?
    {
        if result.documents_checked == maximum {
            *more_remaining = true;
            break;
        }
        let (key, value) = item.map_err(error::redb)?;
        result.documents_checked += 1;
        *last_cursor = Some(make_index_cursor(
            phase,
            key.value(),
            verify_artifact_content,
            last_cursor.as_ref(),
        )?);
        if let Err(cause) = validate(key.value(), value.value()) {
            push_failure(result, component, &cause.to_string())?;
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)] // One bounded phase shares cursor and result state with the integrity driver.
pub(crate) fn scan_string_phase<V: redb::Value + 'static>(
    phase: u8,
    start_phase: u8,
    start_key: Option<&[u8]>,
    table: &impl redb::ReadableTable<&'static str, V>,
    maximum: u64,
    verify_artifact_content: bool,
    result: &mut IntegrityScanResult,
    last_cursor: &mut Option<IntegrityScanCursor>,
    more_remaining: &mut bool,
    component: &str,
    mut validate: impl for<'a> FnMut(&str, V::SelfType<'a>) -> Result<(), PersistenceError>,
) -> Result<(), PersistenceError> {
    if *more_remaining || phase < start_phase {
        return Ok(());
    }
    let lower = if phase == start_phase {
        start_key
            .map(|key| std::str::from_utf8(key))
            .transpose()
            .map_err(|_| {
                PersistenceError::InvalidCursor(
                    "string index integrity cursor is not valid UTF-8".to_owned(),
                )
            })?
            .map_or(Bound::Unbounded, Bound::Excluded)
    } else {
        Bound::Unbounded
    };
    for item in table
        .range::<&str>((lower, Bound::Unbounded))
        .map_err(error::redb)?
    {
        if result.documents_checked == maximum {
            *more_remaining = true;
            break;
        }
        let (key, value) = item.map_err(error::redb)?;
        result.documents_checked += 1;
        *last_cursor = Some(make_index_cursor(
            phase,
            key.value().as_bytes(),
            verify_artifact_content,
            last_cursor.as_ref(),
        )?);
        if let Err(cause) = validate(key.value(), value.value()) {
            push_failure(result, component, &cause.to_string())?;
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)] // One bounded phase shares cursor and result state with the integrity driver.
pub(crate) fn scan_u64_bytes_phase(
    phase: u8,
    start_phase: u8,
    start_key: Option<&[u8]>,
    table: &impl redb::ReadableTable<u64, &'static [u8]>,
    maximum: u64,
    verify_artifact_content: bool,
    result: &mut IntegrityScanResult,
    last_cursor: &mut Option<IntegrityScanCursor>,
    more_remaining: &mut bool,
    component: &str,
    mut validate: impl FnMut(u64, &[u8]) -> Result<(), PersistenceError>,
) -> Result<(), PersistenceError> {
    if *more_remaining || phase < start_phase {
        return Ok(());
    }
    let lower = if phase == start_phase {
        start_key
            .map(|key| {
                let bytes: [u8; 8] = key.try_into().map_err(|_| {
                    PersistenceError::InvalidCursor(
                        "u64 index integrity cursor must contain eight bytes".to_owned(),
                    )
                })?;
                Ok::<u64, PersistenceError>(u64::from_be_bytes(bytes))
            })
            .transpose()?
            .map_or(Bound::Unbounded, Bound::Excluded)
    } else {
        Bound::Unbounded
    };
    for item in table
        .range::<u64>((lower, Bound::Unbounded))
        .map_err(error::redb)?
    {
        if result.documents_checked == maximum {
            *more_remaining = true;
            break;
        }
        let (key, value) = item.map_err(error::redb)?;
        let sequence = key.value();
        result.documents_checked += 1;
        *last_cursor = Some(make_index_cursor(
            phase,
            &sequence.to_be_bytes(),
            verify_artifact_content,
            last_cursor.as_ref(),
        )?);
        if let Err(cause) = validate(sequence, value.value()) {
            push_failure(result, component, &cause.to_string())?;
        }
    }
    Ok(())
}
