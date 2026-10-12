use super::{
    IntegrityScanCursor, IntegrityScanFamily, IntegrityScanResult, PersistenceError,
    cursor::index_cursor_position, cursor::make_integrity_cursor,
};
use crate::{
    admin::scan::scan_binary_phase, admin::scan::scan_string_phase,
    admin::scan::scan_u64_bytes_phase,
};

mod artifacts;
mod managed;
mod tasks;

/// Stable physical diagnostic phases for integrity cursor schema v5.
/// Gaps retain artifact and managed-resource phase identities; retired graph phases are absent.
pub(super) mod phase {
    pub(super) const TASK_RECEIPTS: u8 = 0;
    pub(super) const ARTIFACT_MANIFEST: u8 = 17;
    pub(super) const ARTIFACT_DIGESTS: u8 = 18;
    pub(super) const ARTIFACT_REFERENCES: u8 = 19;
    pub(super) const ARTIFACT_OWNERSHIP: u8 = 20;
    pub(super) const ARTIFACT_TEMP_MANIFEST: u8 = 21;
    pub(super) const ARTIFACT_ACCOUNTING: u8 = 22;
    pub(super) const ARTIFACT_TEMP_OWNERS: u8 = 23;
    pub(super) const ARTIFACT_PUBLICATIONS: u8 = 30;
    pub(super) const ARTIFACT_PUBLICATION_AGE: u8 = 31;
    pub(super) const ARTIFACT_RESERVATIONS: u8 = 32;
    pub(super) const ARTIFACT_PATHS: u8 = 33;
    pub(super) const ARTIFACT_DELETE_GUARDS: u8 = 34;
    pub(super) const ARTIFACT_DIGEST_RESERVATIONS: u8 = 35;
    pub(super) const MANAGED_INSTALLATIONS: u8 = 47;
    pub(super) const MANAGED_USES: u8 = 48;
    pub(super) const MANAGED_RECEIPTS: u8 = 49;
    pub(super) const MANAGED_TRANSITIONS: u8 = 50;
    pub(super) const MANAGED_EVALUATIONS: u8 = 53;
    pub(super) const MANAGED_ACTIVE_INSTALLATIONS: u8 = 58;
    pub(super) const MANAGED_PENDING_EVALUATIONS: u8 = 59;
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
        scan_binary_phase(
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
        scan_binary_phase(
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
        scan_binary_phase(
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
        scan_string_phase(
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
        scan_string_phase(
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
        scan_string_phase(
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

#[expect(
    clippy::too_many_arguments,
    reason = "The scan resumes from one anchored cursor and updates one bounded result across all index tables."
)]
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
            &[phase::TASK_RECEIPTS],
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

    tasks::scan(&mut context)?;
    artifacts::scan_committed(&mut context)?;
    artifacts::scan_publications(&mut context)?;
    context.u64_bytes(
        41,
        &read
            .open_table(crate::schema::SECURITY_AUDIT)
            .map_err(crate::error::redb)?,
        "security_audit",
        |key, bytes| crate::security::decode_security_audit(key, bytes).map(|_| ()),
    )?;
    managed::scan(&mut context)?;
    managed::scan_active(&mut context)?;
    tasks::scan_peers(&mut context)
}
