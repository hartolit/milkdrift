use super::integrity::scan_index_integrity;
use super::*;
const INTEGRITY_CURSOR_VERSION: u8 = 1;
const INTEGRITY_CURSOR_PREFIX_BYTES: usize = 33;

pub(crate) fn make_integrity_cursor(
    family: IntegrityScanFamily,
    key: &[u8],
    verify_artifact_content: bool,
    anchor: [u8; 32],
) -> Result<IntegrityScanCursor, PersistenceError> {
    let mut opaque = Vec::with_capacity(INTEGRITY_CURSOR_PREFIX_BYTES.saturating_add(key.len()));
    opaque.push(INTEGRITY_CURSOR_VERSION);
    opaque.extend_from_slice(&anchor);
    opaque.extend_from_slice(key);
    IntegrityScanCursor::new(family, opaque, verify_artifact_content)
}

pub(crate) fn integrity_cursor_state(
    cursor: &IntegrityScanCursor,
) -> Result<([u8; 32], &[u8]), PersistenceError> {
    if cursor.after_key().len() <= INTEGRITY_CURSOR_PREFIX_BYTES
        || cursor.after_key()[0] != INTEGRITY_CURSOR_VERSION
    {
        return Err(PersistenceError::InvalidCursor(
            "integrity cursor has an invalid schema-anchor prefix".to_owned(),
        ));
    }
    let mut anchor = [0_u8; 32];
    anchor.copy_from_slice(&cursor.after_key()[1..INTEGRITY_CURSOR_PREFIX_BYTES]);
    Ok((anchor, &cursor.after_key()[INTEGRITY_CURSOR_PREFIX_BYTES..]))
}

pub(crate) fn integrity_cursor_anchor(
    cursor: Option<&IntegrityScanCursor>,
) -> Result<[u8; 32], PersistenceError> {
    let cursor =
        cursor.ok_or_else(|| error::corruption("integrity scan lost its schema anchor"))?;
    integrity_cursor_state(cursor).map(|(anchor, _)| anchor)
}

pub(crate) fn integrity_cursor_str<'a>(
    cursor: &'a IntegrityScanCursor,
    family: &str,
) -> Result<&'a str, PersistenceError> {
    let (_, state) = integrity_cursor_state(cursor)?;
    std::str::from_utf8(state).map_err(|_| {
        PersistenceError::InvalidCursor(format!("{family} integrity cursor is not valid UTF-8"))
    })
}

pub(crate) fn scan_index_sample(
    store: &RedbStore,
    maximum: u64,
) -> Result<IntegrityScanResult, PersistenceError> {
    let read = store.database().begin_read().map_err(error::redb)?;
    let mut result = IntegrityScanResult {
        documents_checked: 0,
        artifacts_checked: 0,
        failures: Vec::new(),
        next_cursor: None,
    };
    let mut last_cursor = None;
    let mut more_remaining = false;
    let anchor = storage_anchor(&read)?;
    scan_index_integrity(
        &read,
        None,
        maximum,
        false,
        anchor,
        &mut result,
        &mut last_cursor,
        &mut more_remaining,
    )?;
    if more_remaining {
        result.next_cursor = last_cursor;
    }
    Ok(result)
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn make_artifact_digest_cursor(
    phase: u8,
    physical_key: &[u8],
    total: u64,
    digest: Option<&str>,
    size: u64,
    verify_artifact_content: bool,
    prior: Option<&IntegrityScanCursor>,
) -> Result<IntegrityScanCursor, PersistenceError> {
    let digest = digest.unwrap_or("").as_bytes();
    let digest_length = u16::try_from(digest.len()).map_err(|_| PersistenceError::Bounds {
        location: "integrity_cursor",
        reason: "artifact digest cursor state exceeds u16".to_owned(),
    })?;
    let mut opaque = Vec::with_capacity(
        1_usize
            .saturating_add(8)
            .saturating_add(8)
            .saturating_add(2)
            .saturating_add(digest.len())
            .saturating_add(physical_key.len()),
    );
    opaque.push(phase);
    opaque.extend_from_slice(&total.to_be_bytes());
    opaque.extend_from_slice(&size.to_be_bytes());
    opaque.extend_from_slice(&digest_length.to_be_bytes());
    opaque.extend_from_slice(digest);
    opaque.extend_from_slice(physical_key);
    make_integrity_cursor(
        IntegrityScanFamily::Indexes,
        &opaque,
        verify_artifact_content,
        integrity_cursor_anchor(prior)?,
    )
}

pub(crate) type ArtifactDigestCursorState<'a> = (u64, Option<String>, u64, Option<&'a [u8]>);

pub(crate) fn parse_artifact_digest_cursor(
    state: &[u8],
) -> Result<ArtifactDigestCursorState<'_>, PersistenceError> {
    if state.len() < 18 {
        return Err(PersistenceError::InvalidCursor(
            "artifact digest integrity cursor state is truncated".to_owned(),
        ));
    }
    let total = u64::from_be_bytes(state[0..8].try_into().map_err(|_| {
        PersistenceError::InvalidCursor("artifact digest total is malformed".to_owned())
    })?);
    let size = u64::from_be_bytes(state[8..16].try_into().map_err(|_| {
        PersistenceError::InvalidCursor("artifact digest size is malformed".to_owned())
    })?);
    let digest_length = usize::from(u16::from_be_bytes(state[16..18].try_into().map_err(
        |_| PersistenceError::InvalidCursor("artifact digest length is malformed".to_owned()),
    )?));
    let digest_end = 18_usize.checked_add(digest_length).ok_or_else(|| {
        PersistenceError::InvalidCursor("artifact digest cursor length overflows".to_owned())
    })?;
    let digest_bytes = state.get(18..digest_end).ok_or_else(|| {
        PersistenceError::InvalidCursor("artifact digest cursor is truncated".to_owned())
    })?;
    let digest = if digest_bytes.is_empty() {
        None
    } else {
        Some(
            std::str::from_utf8(digest_bytes)
                .map_err(|_| {
                    PersistenceError::InvalidCursor(
                        "artifact digest cursor is not valid UTF-8".to_owned(),
                    )
                })?
                .to_owned(),
        )
    };
    let key = state
        .get(digest_end..)
        .filter(|key| !key.is_empty())
        .ok_or_else(|| {
            PersistenceError::InvalidCursor("artifact digest cursor has no physical key".to_owned())
        })?;
    Ok((total, digest, size, Some(key)))
}

pub(crate) fn index_cursor_position(
    cursor: Option<&IntegrityScanCursor>,
) -> Result<(u8, Option<&[u8]>), PersistenceError> {
    let Some(cursor) = cursor else {
        return Ok((0, None));
    };
    let (_, state) = integrity_cursor_state(cursor)?;
    let Some((&phase, key)) = state.split_first() else {
        return Err(PersistenceError::InvalidCursor(
            "index integrity cursor has no phase".to_owned(),
        ));
    };
    if phase > 22 || key.is_empty() {
        return Err(PersistenceError::InvalidCursor(
            "index integrity cursor has an unknown phase or empty key".to_owned(),
        ));
    }
    Ok((phase, Some(key)))
}

pub(crate) fn make_index_cursor(
    phase: u8,
    key: &[u8],
    verify_artifact_content: bool,
    prior: Option<&IntegrityScanCursor>,
) -> Result<IntegrityScanCursor, PersistenceError> {
    let mut opaque = Vec::with_capacity(key.len().saturating_add(1));
    opaque.push(phase);
    opaque.extend_from_slice(key);
    make_integrity_cursor(
        IntegrityScanFamily::Indexes,
        &opaque,
        verify_artifact_content,
        integrity_cursor_anchor(prior)?,
    )
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn scan_binary_bytes_phase(
    phase: u8,
    start_phase: u8,
    start_key: Option<&[u8]>,
    table: &impl redb::ReadableTable<&'static [u8], &'static [u8]>,
    maximum: u64,
    verify_artifact_content: bool,
    result: &mut IntegrityScanResult,
    last_cursor: &mut Option<IntegrityScanCursor>,
    more_remaining: &mut bool,
    component: &str,
    mut validate: impl FnMut(&[u8], &[u8]) -> Result<(), PersistenceError>,
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

#[allow(clippy::too_many_arguments)]
pub(crate) fn scan_string_bytes_phase(
    phase: u8,
    start_phase: u8,
    start_key: Option<&[u8]>,
    table: &impl redb::ReadableTable<&'static str, &'static [u8]>,
    maximum: u64,
    verify_artifact_content: bool,
    result: &mut IntegrityScanResult,
    last_cursor: &mut Option<IntegrityScanCursor>,
    more_remaining: &mut bool,
    component: &str,
    mut validate: impl FnMut(&str, &[u8]) -> Result<(), PersistenceError>,
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

#[allow(clippy::too_many_arguments)]
pub(crate) fn scan_string_string_phase(
    phase: u8,
    start_phase: u8,
    start_key: Option<&[u8]>,
    table: &impl redb::ReadableTable<&'static str, &'static str>,
    maximum: u64,
    verify_artifact_content: bool,
    result: &mut IntegrityScanResult,
    last_cursor: &mut Option<IntegrityScanCursor>,
    more_remaining: &mut bool,
    component: &str,
    mut validate: impl FnMut(&str, &str) -> Result<(), PersistenceError>,
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

#[allow(clippy::too_many_arguments)]
pub(crate) fn scan_string_u64_phase(
    phase: u8,
    start_phase: u8,
    start_key: Option<&[u8]>,
    table: &impl redb::ReadableTable<&'static str, u64>,
    maximum: u64,
    verify_artifact_content: bool,
    result: &mut IntegrityScanResult,
    last_cursor: &mut Option<IntegrityScanCursor>,
    more_remaining: &mut bool,
    component: &str,
    mut validate: impl FnMut(&str, u64) -> Result<(), PersistenceError>,
) -> Result<(), PersistenceError> {
    scan_string_scalar_phase(
        phase,
        start_phase,
        start_key,
        table,
        maximum,
        verify_artifact_content,
        result,
        last_cursor,
        more_remaining,
        component,
        |value| value,
        &mut validate,
    )
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn scan_string_u8_phase(
    phase: u8,
    start_phase: u8,
    start_key: Option<&[u8]>,
    table: &impl redb::ReadableTable<&'static str, u8>,
    maximum: u64,
    verify_artifact_content: bool,
    result: &mut IntegrityScanResult,
    last_cursor: &mut Option<IntegrityScanCursor>,
    more_remaining: &mut bool,
    component: &str,
    mut validate: impl FnMut(&str, u8) -> Result<(), PersistenceError>,
) -> Result<(), PersistenceError> {
    scan_string_scalar_phase(
        phase,
        start_phase,
        start_key,
        table,
        maximum,
        verify_artifact_content,
        result,
        last_cursor,
        more_remaining,
        component,
        |value| value,
        &mut validate,
    )
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn scan_string_scalar_phase<V: redb::Value + 'static, T: Copy>(
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
    scalar: impl for<'a> Fn(V::SelfType<'a>) -> T,
    validate: &mut impl FnMut(&str, T) -> Result<(), PersistenceError>,
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
        if let Err(cause) = validate(key.value(), scalar(value.value())) {
            push_failure(result, component, &cause.to_string())?;
        }
    }
    Ok(())
}

pub(crate) fn validate_integrity_cursor(
    request: &IntegrityScanRequest,
    read: &redb::ReadTransaction,
    revisions: &impl redb::ReadableTable<&'static str, &'static [u8]>,
    events: &impl redb::ReadableTable<&'static [u8], &'static [u8]>,
    artifacts: &impl redb::ReadableTable<&'static str, &'static [u8]>,
) -> Result<(), PersistenceError> {
    let Some(cursor) = request.cursor.as_ref() else {
        return Ok(());
    };
    if cursor.verify_artifact_content() != request.verify_artifact_content {
        return Err(PersistenceError::InvalidCursor(
            "integrity cursor belongs to a different artifact-verification mode".to_owned(),
        ));
    }
    let (cursor_anchor, cursor_key) = integrity_cursor_state(cursor)?;
    if cursor_anchor != storage_anchor(read)? {
        return Err(PersistenceError::InvalidCursor(
            "integrity cursor belongs to a different storage schema".to_owned(),
        ));
    }
    let exists = match cursor.family() {
        IntegrityScanFamily::Revisions => revisions
            .get(integrity_cursor_str(cursor, "revision")?)
            .map_err(error::redb)?
            .is_some(),
        IntegrityScanFamily::RunEvents => events.get(cursor_key).map_err(error::redb)?.is_some(),
        IntegrityScanFamily::Artifacts => artifacts
            .get(integrity_cursor_str(cursor, "artifact")?)
            .map_err(error::redb)?
            .is_some(),
        IntegrityScanFamily::Indexes => index_integrity_cursor_exists(read, cursor)?,
    };
    if !exists {
        return Err(PersistenceError::InvalidCursor(
            "integrity cursor does not name a durable record".to_owned(),
        ));
    }
    Ok(())
}

pub(crate) fn storage_anchor(read: &redb::ReadTransaction) -> Result<[u8; 32], PersistenceError> {
    let metadata = read.open_table(METADATA).map_err(error::redb)?;
    let schema = metadata
        .get(SCHEMA_VERSION_KEY)
        .map_err(error::redb)?
        .ok_or_else(|| error::corruption("storage schema version is missing"))?
        .value();
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"milkdrift.integrity-cursor.schema.v2\0");
    hasher.update(&schema.to_be_bytes());
    Ok(*hasher.finalize().as_bytes())
}

pub(crate) fn index_integrity_cursor_exists(
    read: &redb::ReadTransaction,
    cursor: &IntegrityScanCursor,
) -> Result<bool, PersistenceError> {
    let (phase, key) = index_cursor_position(Some(cursor))?;
    let key = key.ok_or_else(|| {
        PersistenceError::InvalidCursor("index integrity cursor is missing its key".to_owned())
    })?;
    let string_key = || {
        std::str::from_utf8(key).map_err(|_| {
            PersistenceError::InvalidCursor(
                "string index integrity cursor is not valid UTF-8".to_owned(),
            )
        })
    };
    match phase {
        0 => read
            .open_table(RUN_HEADS)
            .map_err(error::redb)?
            .get(string_key()?)
            .map_err(error::redb)
            .map(|row| row.is_some()),
        1 => read
            .open_table(RUN_SUMMARIES)
            .map_err(error::redb)?
            .get(string_key()?)
            .map_err(error::redb)
            .map(|row| row.is_some()),
        2 => read
            .open_table(NONTERMINAL_RUNS)
            .map_err(error::redb)?
            .get(string_key()?)
            .map_err(error::redb)
            .map(|row| row.is_some()),
        3 => binary_cursor_exists(read, COMMAND_RESULTS, key),
        4 => binary_cursor_exists(read, RUNNABLE_ENTRIES, key),
        5 => binary_cursor_exists(read, RUNNABLE_INDEX, key),
        6 => binary_cursor_exists(read, TIMER_ENTRIES, key),
        7 => binary_cursor_exists(read, TIMER_INDEX, key),
        8 => binary_cursor_exists(read, LEASE_ENTRIES, key),
        9 => binary_cursor_exists(read, LEASE_INDEX, key),
        10 => binary_cursor_exists(read, SCOPES, key),
        11 => binary_cursor_exists(read, VALUES, key),
        12 => read
            .open_table(WORKSPACE_USAGE)
            .map_err(error::redb)?
            .get(string_key()?)
            .map_err(error::redb)
            .map(|row| row.is_some()),
        13 => read
            .open_table(WORKSPACE_BUDGETS)
            .map_err(error::redb)?
            .get(string_key()?)
            .map_err(error::redb)
            .map(|row| row.is_some()),
        14 => read
            .open_table(ROOT_SCOPES)
            .map_err(error::redb)?
            .get(string_key()?)
            .map_err(error::redb)
            .map(|row| row.is_some()),
        15 => binary_cursor_exists(read, REVISIONS_BY_DIGEST, key),
        16 => read
            .open_table(ARTIFACT_MANIFEST)
            .map_err(error::redb)?
            .get(string_key()?)
            .map_err(error::redb)
            .map(|row| row.is_some()),
        17 => {
            let (_total, _digest, _size, physical_key) = parse_artifact_digest_cursor(key)?;
            binary_cursor_exists(
                read,
                ARTIFACTS_BY_DIGEST,
                physical_key.ok_or_else(|| {
                    PersistenceError::InvalidCursor(
                        "artifact digest cursor has no physical key".to_owned(),
                    )
                })?,
            )
        }
        18 => binary_cursor_exists(read, ARTIFACT_REFERENCES, key),
        19 => binary_cursor_exists(read, RUN_ARTIFACT_OWNERSHIP, key),
        20 => read
            .open_table(ARTIFACT_TEMP_MANIFEST)
            .map_err(error::redb)?
            .get(string_key()?)
            .map_err(error::redb)
            .map(|row| row.is_some()),
        21 => read
            .open_table(ARTIFACT_ACCOUNTING)
            .map_err(error::redb)?
            .get(string_key()?)
            .map_err(error::redb)
            .map(|row| row.is_some()),
        22 => read
            .open_table(ARTIFACT_TEMP_OWNERS)
            .map_err(error::redb)?
            .get(string_key()?)
            .map_err(error::redb)
            .map(|row| row.is_some()),
        _ => Err(PersistenceError::InvalidCursor(
            "index integrity cursor has an unknown phase".to_owned(),
        )),
    }
}

pub(crate) fn binary_cursor_exists(
    read: &redb::ReadTransaction,
    definition: redb::TableDefinition<'static, &'static [u8], &'static [u8]>,
    key: &[u8],
) -> Result<bool, PersistenceError> {
    read.open_table(definition)
        .map_err(error::redb)?
        .get(key)
        .map_err(error::redb)
        .map(|row| row.is_some())
}

pub(crate) fn push_failure(
    result: &mut IntegrityScanResult,
    component: &str,
    detail: &str,
) -> Result<(), PersistenceError> {
    result.failures.push(StorageComponentHealth {
        component: BoundedDetail::new(component)?,
        status: StorageHealthStatus::Degraded,
        detail: bounded_detail(detail)?,
    });
    Ok(())
}

pub(crate) fn bounded_detail(detail: &str) -> Result<BoundedDetail, PersistenceError> {
    let mut detail: String = detail
        .chars()
        .map(|character| {
            if character.is_control() {
                ' '
            } else {
                character
            }
        })
        .collect();
    if detail.len() > milkdrift_persistence::MAX_DETAIL_BYTES {
        let mut boundary = milkdrift_persistence::MAX_DETAIL_BYTES;
        while !detail.is_char_boundary(boundary) {
            boundary -= 1;
        }
        detail.truncate(boundary);
    }
    BoundedDetail::new(detail)
}
