use super::cursor::{
    index_cursor_position, make_integrity_cursor, scan_binary_bytes_phase,
    scan_binary_string_phase, scan_binary_u8_phase, scan_binary_u64_phase, scan_string_bytes_phase,
    scan_string_string_phase, scan_string_u8_phase, scan_string_u64_phase, scan_u64_bytes_phase,
};
use super::{IntegrityScanCursor, IntegrityScanFamily, IntegrityScanResult, PersistenceError};

mod application;
mod artifacts;
mod controller;
mod revisions;
mod run;
mod scheduler;
mod snapshots;
mod workspace;

/// Persisted phase tags. Their values and ordering are integrity-cursor schema v2.
/// Moving a phase requires an explicit cursor-version decision.
pub(super) mod phase {
    pub(super) const HEADS: u8 = 0;
    pub(super) const SUMMARIES: u8 = 1;
    pub(super) const NONTERMINAL: u8 = 2;
    pub(super) const COMMANDS: u8 = 3;
    pub(super) const RUNNABLE_IDENTITIES: u8 = 4;
    pub(super) const RUNNABLE_ORDERED: u8 = 5;
    pub(super) const TIMER_IDENTITIES: u8 = 6;
    pub(super) const TIMER_ORDERED: u8 = 7;
    pub(super) const LEASE_IDENTITIES: u8 = 8;
    pub(super) const LEASE_ORDERED: u8 = 9;
    pub(super) const SCOPES: u8 = 10;
    pub(super) const VALUES: u8 = 11;
    pub(super) const USAGE: u8 = 12;
    pub(super) const BUDGET: u8 = 13;
    pub(super) const ROOT_SCOPES: u8 = 14;
    pub(super) const REVISION_PRIMARY: u8 = 15;
    pub(super) const REVISION_DIGESTS: u8 = 16;
    pub(super) const ARTIFACT_MANIFEST: u8 = 17;
    pub(super) const ARTIFACT_DIGESTS: u8 = 18;
    pub(super) const ARTIFACT_REFERENCES: u8 = 19;
    pub(super) const ARTIFACT_OWNERSHIP: u8 = 20;
    pub(super) const ARTIFACT_TEMP_MANIFEST: u8 = 21;
    pub(super) const ARTIFACT_ACCOUNTING: u8 = 22;
    pub(super) const ARTIFACT_TEMP_OWNERS: u8 = 23;
    pub(super) const SIGNAL_RECEIPTS: u8 = 24;
    pub(super) const RUNNABLE_RUN_HEADS: u8 = 25;
    pub(super) const WORKSPACE_VALUE_HEADS: u8 = 26;
    pub(super) const INVOCATION_FACTS: u8 = 27;
    pub(super) const SNAPSHOTS: u8 = 28;
    pub(super) const SNAPSHOT_LATEST: u8 = 29;
    pub(super) const ARTIFACT_PUBLICATIONS: u8 = 30;
    pub(super) const ARTIFACT_PUBLICATION_AGE: u8 = 31;
    pub(super) const ARTIFACT_RESERVATIONS: u8 = 32;
    pub(super) const ARTIFACT_PATHS: u8 = 33;
    pub(super) const ARTIFACT_DELETE_GUARDS: u8 = 34;
    pub(super) const ARTIFACT_DIGEST_RESERVATIONS: u8 = 35;
    pub(super) const APPLICATION_HOT_RECEIPTS: u8 = 36;
    pub(super) const APPLICATION_COLD_RECEIPTS: u8 = 37;
    pub(super) const APPLICATION_HOT_RECEIPT_ORDER: u8 = 38;
    pub(super) const APPLICATION_LAYOUTS: u8 = 39;
    pub(super) const APPLICATION_PROPOSALS: u8 = 40;
    pub(super) const SECURITY_AUDIT: u8 = 41;
    pub(super) const CONTROLLER_ACCOUNTS: u8 = 42;
    pub(super) const CONTROLLER_RUN_BINDINGS: u8 = 43;
    pub(super) const CONTROLLER_TRANSITIONS: u8 = 44;
    pub(super) const CONTROLLER_ARTIFACT_CHARGES: u8 = 45;
}

/// Shared state for one ordered scan page. Domain modules own tables and validation.
pub(super) struct ScanContext<'read, 'state> {
    pub(super) read: &'read redb::ReadTransaction,
    pub(super) start_phase: u8,
    pub(super) start_key: Option<Vec<u8>>,
    pub(super) maximum: u64,
    pub(super) verify_artifact_content: bool,
    pub(super) result: &'state mut IntegrityScanResult,
    pub(super) last_cursor: &'state mut Option<IntegrityScanCursor>,
    pub(super) more_remaining: &'state mut bool,
}

impl ScanContext<'_, '_> {
    pub(super) fn binary_bytes(
        &mut self,
        phase: u8,
        table: &impl redb::ReadableTable<&'static [u8], &'static [u8]>,
        component: &str,
        validate: impl FnMut(&[u8], &[u8]) -> Result<(), PersistenceError>,
    ) -> Result<(), PersistenceError> {
        scan_binary_bytes_phase(
            phase,
            self.start_phase,
            self.start_key.as_deref(),
            table,
            self.maximum,
            self.verify_artifact_content,
            self.result,
            self.last_cursor,
            self.more_remaining,
            component,
            validate,
        )
    }

    pub(super) fn binary_string(
        &mut self,
        phase: u8,
        table: &impl redb::ReadableTable<&'static [u8], &'static str>,
        component: &str,
        validate: impl FnMut(&[u8], &str) -> Result<(), PersistenceError>,
    ) -> Result<(), PersistenceError> {
        scan_binary_string_phase(
            phase,
            self.start_phase,
            self.start_key.as_deref(),
            table,
            self.maximum,
            self.verify_artifact_content,
            self.result,
            self.last_cursor,
            self.more_remaining,
            component,
            validate,
        )
    }

    pub(super) fn binary_u8(
        &mut self,
        phase: u8,
        table: &impl redb::ReadableTable<&'static [u8], u8>,
        component: &str,
        validate: impl FnMut(&[u8], u8) -> Result<(), PersistenceError>,
    ) -> Result<(), PersistenceError> {
        scan_binary_u8_phase(
            phase,
            self.start_phase,
            self.start_key.as_deref(),
            table,
            self.maximum,
            self.verify_artifact_content,
            self.result,
            self.last_cursor,
            self.more_remaining,
            component,
            validate,
        )
    }

    pub(super) fn binary_u64(
        &mut self,
        phase: u8,
        table: &impl redb::ReadableTable<&'static [u8], u64>,
        component: &str,
        validate: impl FnMut(&[u8], u64) -> Result<(), PersistenceError>,
    ) -> Result<(), PersistenceError> {
        scan_binary_u64_phase(
            phase,
            self.start_phase,
            self.start_key.as_deref(),
            table,
            self.maximum,
            self.verify_artifact_content,
            self.result,
            self.last_cursor,
            self.more_remaining,
            component,
            validate,
        )
    }

    pub(super) fn u64_bytes(
        &mut self,
        phase: u8,
        table: &impl redb::ReadableTable<u64, &'static [u8]>,
        component: &str,
        validate: impl FnMut(u64, &[u8]) -> Result<(), PersistenceError>,
    ) -> Result<(), PersistenceError> {
        scan_u64_bytes_phase(
            phase,
            self.start_phase,
            self.start_key.as_deref(),
            table,
            self.maximum,
            self.verify_artifact_content,
            self.result,
            self.last_cursor,
            self.more_remaining,
            component,
            validate,
        )
    }

    pub(super) fn string_bytes(
        &mut self,
        phase: u8,
        table: &impl redb::ReadableTable<&'static str, &'static [u8]>,
        component: &str,
        validate: impl FnMut(&str, &[u8]) -> Result<(), PersistenceError>,
    ) -> Result<(), PersistenceError> {
        scan_string_bytes_phase(
            phase,
            self.start_phase,
            self.start_key.as_deref(),
            table,
            self.maximum,
            self.verify_artifact_content,
            self.result,
            self.last_cursor,
            self.more_remaining,
            component,
            validate,
        )
    }

    pub(super) fn string_string(
        &mut self,
        phase: u8,
        table: &impl redb::ReadableTable<&'static str, &'static str>,
        component: &str,
        validate: impl FnMut(&str, &str) -> Result<(), PersistenceError>,
    ) -> Result<(), PersistenceError> {
        scan_string_string_phase(
            phase,
            self.start_phase,
            self.start_key.as_deref(),
            table,
            self.maximum,
            self.verify_artifact_content,
            self.result,
            self.last_cursor,
            self.more_remaining,
            component,
            validate,
        )
    }

    pub(super) fn string_u8(
        &mut self,
        phase: u8,
        table: &impl redb::ReadableTable<&'static str, u8>,
        component: &str,
        validate: impl FnMut(&str, u8) -> Result<(), PersistenceError>,
    ) -> Result<(), PersistenceError> {
        scan_string_u8_phase(
            phase,
            self.start_phase,
            self.start_key.as_deref(),
            table,
            self.maximum,
            self.verify_artifact_content,
            self.result,
            self.last_cursor,
            self.more_remaining,
            component,
            validate,
        )
    }

    pub(super) fn string_u64(
        &mut self,
        phase: u8,
        table: &impl redb::ReadableTable<&'static str, u64>,
        component: &str,
        validate: impl FnMut(&str, u64) -> Result<(), PersistenceError>,
    ) -> Result<(), PersistenceError> {
        scan_string_u64_phase(
            phase,
            self.start_phase,
            self.start_key.as_deref(),
            table,
            self.maximum,
            self.verify_artifact_content,
            self.result,
            self.last_cursor,
            self.more_remaining,
            component,
            validate,
        )
    }
}

#[allow(clippy::too_many_arguments)] // Integrity verification keeps every cross-table evidence source explicit.
pub(crate) fn scan_index_integrity(
    read: &redb::ReadTransaction,
    cursor: Option<&IntegrityScanCursor>,
    maximum: u64,
    verify_artifact_content: bool,
    anchor: [u8; 32],
    result: &mut IntegrityScanResult,
    last_cursor: &mut Option<IntegrityScanCursor>,
    more_remaining: &mut bool,
) -> Result<(), PersistenceError> {
    let (start_phase, start_key) = index_cursor_position(cursor)?;
    if last_cursor.is_none() {
        *last_cursor = Some(make_integrity_cursor(
            IntegrityScanFamily::Indexes,
            &[phase::HEADS],
            verify_artifact_content,
            anchor,
        )?);
    }
    let mut context = ScanContext {
        read,
        start_phase,
        start_key: start_key.map(<[u8]>::to_vec),
        maximum,
        verify_artifact_content,
        result,
        last_cursor,
        more_remaining,
    };

    // This call order is the persisted phase order; families with separated legacy
    // phase ranges intentionally re-enter without silently renumbering cursors.
    run::scan_core(&mut context)?;
    scheduler::scan_ordered(&mut context)?;
    workspace::scan_core(&mut context)?;
    revisions::scan(&mut context)?;
    artifacts::scan_committed(&mut context)?;
    run::scan_signal_receipts(&mut context)?;
    scheduler::scan_run_heads(&mut context)?;
    workspace::scan_value_heads(&mut context)?;
    run::scan_invocation_facts(&mut context)?;
    snapshots::scan(&mut context)?;
    artifacts::scan_publications(&mut context)?;
    application::scan(&mut context)?;
    controller::scan(&mut context)
}
