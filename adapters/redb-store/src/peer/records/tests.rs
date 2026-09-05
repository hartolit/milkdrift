//! Independent physical corruption fixtures for both transaction kinds.
use milkdrift_peer_protocol::PeerExecutionId;
use milkdrift_persistence::{PersistenceError, StorageFailureClass};
use redb::Database;

use crate::schema::{PEER_EXECUTION_LOCATIONS, PEER_EXECUTION_TOMBSTONES, PEER_EXECUTIONS};

#[test]
fn hot_and_archived_reads_refuse_broken_location_authority()
-> Result<(), Box<dyn std::error::Error>> {
    for (location, hot, archived, message) in [
        (None, false, false, None),
        (None, true, false, Some("without a location index")),
        (None, false, true, Some("without a location index")),
        (Some(1), false, false, Some("missing record")),
        (Some(2), false, false, Some("missing tombstone")),
        (Some(255), false, false, Some("unknown value")),
        (Some(1), true, false, Some("")),
        (Some(2), false, true, Some("")),
    ] {
        let root = tempfile::tempdir()?;
        let db = Database::create(root.path().join("peer.redb"))?;
        let write = db.begin_write()?;
        crate::schema::initialize_tables(&write)?;
        let execution = PeerExecutionId::new("execution")?;
        if let Some(location) = location {
            write
                .open_table(PEER_EXECUTION_LOCATIONS)?
                .insert(execution.as_str(), location)?;
        }
        // Invalid documents must not become absence or be trusted because the index exists.
        for (present, table) in [
            (hot, PEER_EXECUTIONS),
            (archived, PEER_EXECUTION_TOMBSTONES),
        ] {
            if present {
                write
                    .open_table(table)?
                    .insert(execution.as_str(), b"{}".as_slice())?;
            }
        }
        let assert_result = |result: Result<_, PersistenceError>| {
            let matches_expected = match (message, &result) {
                (None, Ok(None)) => true,
                (Some(""), Err(PersistenceError::Corruption(message))) => {
                    message.contains("envelope failed decoding")
                }
                (
                    Some(expected),
                    Err(PersistenceError::Storage {
                        class: StorageFailureClass::Corruption,
                        message,
                    }),
                ) => !expected.is_empty() && message.contains(expected),
                _ => false,
            };
            assert!(
                matches_expected,
                "unexpected peer authority result: {result:?}"
            );
        };
        assert_result(super::snapshot_optional_in_transaction(&write, &execution));
        write.commit()?;
        assert_result(super::snapshot_optional_in_read_transaction(
            &db.begin_read()?,
            &execution,
        ));
    }
    Ok(())
}
