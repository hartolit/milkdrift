//! Narrow logical fault operations for cross-crate storage integration tests.
//!
//! This module is available only with the `test-admin` feature. Callers must
//! close the store before using these operations and reopen it afterward.

use std::path::Path;

use milkdrift_persistence::PersistenceError;
use milkdrift_workspace::{ScopeReference, WorkspaceValueEntry, WorkspaceValueReference};
use redb::Database;

use crate::{
    codec, error, json,
    schema::{SCOPES, VALUES},
    store::DATABASE_FILENAME,
};

/// Removes one authoritative workspace scope while leaving its indexes intact.
///
/// This deliberately creates corrupt storage for an integration test. The
/// target is expressed as a logical scope reference; physical table names and
/// key encodings remain adapter-owned.
pub fn remove_workspace_scope(
    root: &Path,
    reference: &ScopeReference,
) -> Result<(), PersistenceError> {
    let key = codec::pair(reference.run().as_str(), reference.scope().as_str())?;
    remove_required_workspace_row(root, SCOPES, &key, "workspace_scope")
}

/// Removes one authoritative workspace value while leaving its indexes intact.
///
/// This deliberately creates corrupt storage for an integration test. The
/// target is expressed as a logical value reference; physical table names and
/// key encodings remain adapter-owned.
pub fn remove_workspace_value(
    root: &Path,
    reference: &WorkspaceValueReference,
) -> Result<(), PersistenceError> {
    let key = workspace_value_key(reference)?;
    remove_required_workspace_row(root, VALUES, &key, "workspace_value")
}

/// Inserts an authoritative workspace value without updating any derived index.
///
/// This deliberately creates an orphan record for an integration test. The
/// adapter still owns the durable envelope, checksum, table, and key encoding.
pub fn insert_orphan_workspace_value(
    root: &Path,
    entry: &WorkspaceValueEntry,
) -> Result<(), PersistenceError> {
    let key = workspace_value_key(entry.reference())?;
    let encoded = json::encode(entry, "workspace value")?;
    let database = open_database(root)?;
    let write = database.begin_write().map_err(error::redb)?;
    let replaced = {
        let mut table = write.open_table(VALUES).map_err(error::redb)?;
        table
            .insert(key.as_slice(), encoded.as_slice())
            .map_err(error::redb)?
            .is_some()
    };
    if replaced {
        return Err(PersistenceError::ImmutableConflict {
            entity: "workspace_value",
            identity: format!("{:?}", entry.reference()),
        });
    }
    write.commit().map_err(error::redb)
}

fn remove_required_workspace_row(
    root: &Path,
    definition: redb::TableDefinition<'static, &'static [u8], &'static [u8]>,
    key: &[u8],
    entity: &'static str,
) -> Result<(), PersistenceError> {
    let database = open_database(root)?;
    let write = database.begin_write().map_err(error::redb)?;
    let removed = {
        let mut table = write.open_table(definition).map_err(error::redb)?;
        table.remove(key).map_err(error::redb)?.is_some()
    };
    if !removed {
        return Err(PersistenceError::NotFound {
            entity,
            identity: "requested logical corruption target".to_owned(),
        });
    }
    write.commit().map_err(error::redb)
}

fn open_database(root: &Path) -> Result<Database, PersistenceError> {
    Database::create(root.join(DATABASE_FILENAME)).map_err(error::database)
}

fn workspace_value_key(reference: &WorkspaceValueReference) -> Result<Vec<u8>, PersistenceError> {
    codec::value(
        reference.scope().run().as_str(),
        reference.scope().scope().as_str(),
        reference.key().as_str(),
        reference.version().get(),
    )
}
