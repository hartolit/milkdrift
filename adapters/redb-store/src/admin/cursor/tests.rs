use super::*;

#[test]
fn cursor_headers_require_complete_payloads() -> Result<(), PersistenceError> {
    let anchor = [7; 32];
    let cursor = make_integrity_cursor(IntegrityScanFamily::Indexes, b"key", false, anchor)?;
    assert_eq!(
        integrity_cursor_state(&cursor)?,
        (anchor, b"key".as_slice())
    );
    for length in 1..=INTEGRITY_CURSOR_PREFIX_BYTES {
        let bytes = cursor
            .after_key()
            .get(..length)
            .ok_or_else(|| error::corruption("test prefix"))?;
        let truncated =
            IntegrityScanCursor::new(IntegrityScanFamily::Indexes, bytes.to_vec(), false)?;
        assert!(matches!(
            integrity_cursor_state(&truncated),
            Err(PersistenceError::InvalidCursor(_))
        ));
    }
    let wrong_version = IntegrityScanCursor::new(IntegrityScanFamily::Indexes, vec![0; 34], false)?;
    assert!(matches!(
        integrity_cursor_state(&wrong_version),
        Err(PersistenceError::InvalidCursor(_))
    ));
    Ok(())
}

#[test]
fn artifact_cursor_payloads_preserve_lengths_and_phases() -> Result<(), PersistenceError> {
    let prior = make_integrity_cursor(IntegrityScanFamily::Indexes, b"key", false, [7; 32])?;
    for digest in [None, Some("digest")] {
        let cursor =
            make_artifact_digest_cursor(18, b"key", u64::MAX, digest, 42, false, Some(&prior))?;
        let (_, state) = index_cursor_position(Some(&cursor))?;
        let state = state.ok_or_else(|| error::corruption("test cursor state"))?;
        assert_eq!(
            parse_artifact_digest_cursor(state)?,
            (
                u64::MAX,
                digest.map(str::to_owned),
                42,
                Some(b"key".as_slice())
            )
        );
        for length in 0..=18_usize.saturating_add(digest.map_or(0, str::len)) {
            let prefix = state
                .get(..length)
                .ok_or_else(|| error::corruption("test prefix"))?;
            assert!(matches!(
                parse_artifact_digest_cursor(prefix),
                Err(PersistenceError::InvalidCursor(_))
            ));
        }
    }
    for (in_progress, path) in [
        (false, None),
        (true, None),
        (true, Some(b"path".as_slice())),
    ] {
        let cursor = make_delete_guard_cursor(b"guard", in_progress, path, false, Some(&prior))?;
        let (_, state) = index_cursor_position(Some(&cursor))?;
        let state = state.ok_or_else(|| error::corruption("test cursor state"))?;
        assert_eq!(
            parse_delete_guard_cursor(state)?,
            (b"guard".as_slice(), in_progress, path)
        );
    }
    for state in [
        b"".as_slice(),
        &[1, 0, 1],
        &[0, 0, 1, 0, 7],
        &[1, 0, 0, 0],
        &[1, 0, 2, 0, 7],
        &[1, 0, 1, 2, 7],
        &[1, 0, 1, 3, 7],
        &[1, 0, 1, 0, 7, 8],
    ] {
        assert!(matches!(
            parse_delete_guard_cursor(state),
            Err(PersistenceError::InvalidCursor(_))
        ));
    }
    Ok(())
}
