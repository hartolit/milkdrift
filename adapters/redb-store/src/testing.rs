//! Narrow logical fault operations for cross-crate storage integration tests.
//!
//! This module is available only with the `test-admin` feature. Callers must
//! close the store before using these operations and reopen it afterward.

use milkdrift_persistence::PersistenceError;

/// Creates a new private temporary directory for offline-storage tests. Unix uses
/// explicit mode 0700; Windows removes inherited access on this new empty directory.
/// Never changes permissions on an existing operator directory.
///
/// # Errors
/// Returns failures creating the temporary directory or establishing its private permissions.
pub fn private_offline_directory() -> Result<tempfile::TempDir, PersistenceError> {
    crate::offline::private_test_directory()
}
