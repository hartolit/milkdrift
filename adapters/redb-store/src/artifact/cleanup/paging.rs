//! Bound cleanup traversal separately from each transactional reclamation step.
use super::{
    OrphanCleanupCursor, OrphanCleanupFamily, OrphanCleanupRequest, OrphanCleanupResult,
    PersistenceError, RedbStore, cleanup_content_files, cleanup_temporary_files,
    expire_writable_publications,
};
impl RedbStore {
    /// Reclaims interrupted public input uploads before this store admits new work.
    /// Workflow outputs and resumable peer transfers keep their writable publications.
    /// The caller must keep admission closed and follow the bounded cursor to exhaustion.
    pub fn recover_interrupted_client_inputs(
        &self,
        request: OrphanCleanupRequest,
    ) -> Result<OrphanCleanupResult, PersistenceError> {
        self.cleanup_orphans_scoped(request, true)
    }

    pub(in crate::artifact) fn cleanup_orphans_scoped(
        &self,
        request: OrphanCleanupRequest,
        only_client_inputs: bool,
    ) -> Result<OrphanCleanupResult, PersistenceError> {
        if request.created_before > request.observed_at {
            return Err(PersistenceError::InvalidDocument(
                "orphan cleanup threshold cannot be after observed_at".to_owned(),
            ));
        }
        if request
            .cursor
            .as_ref()
            .is_some_and(|cursor| cursor.created_before() != request.created_before)
        {
            return Err(PersistenceError::InvalidCursor(
                "orphan-cleanup cursor belongs to a different age threshold".to_owned(),
            ));
        }
        let _artifact_serialization = self.lock_artifact_publications()?;
        let mut result = OrphanCleanupResult::default();
        let mut examined = 0_u32;
        let start_family = request.cursor.as_ref().map_or(
            OrphanCleanupFamily::WritablePublications,
            OrphanCleanupCursor::family,
        );
        let mut last_cursor = None;

        if start_family <= OrphanCleanupFamily::WritablePublications {
            let after = request
                .cursor
                .as_ref()
                .filter(|cursor| cursor.family() == OrphanCleanupFamily::WritablePublications)
                .map(OrphanCleanupCursor::after_key);
            if expire_writable_publications(
                self,
                &request,
                only_client_inputs,
                after,
                &mut result,
                &mut examined,
                &mut last_cursor,
            )? {
                result.next_cursor = last_cursor;
                return Ok(result);
            }
        }

        if start_family <= OrphanCleanupFamily::TemporaryFiles {
            let after = request
                .cursor
                .as_ref()
                .filter(|cursor| cursor.family() == OrphanCleanupFamily::TemporaryFiles)
                .map(OrphanCleanupCursor::after_key);
            if cleanup_temporary_files(
                self,
                &request,
                after,
                &mut result,
                &mut examined,
                &mut last_cursor,
            )? {
                result.next_cursor = last_cursor;
                return Ok(result);
            }
        }
        if start_family <= OrphanCleanupFamily::ContentFiles
            && cleanup_content_files(
                self,
                &request,
                request
                    .cursor
                    .as_ref()
                    .filter(|cursor| cursor.family() == OrphanCleanupFamily::ContentFiles)
                    .map(OrphanCleanupCursor::after_key),
                &mut result,
                &mut examined,
                &mut last_cursor,
            )?
        {
            result.next_cursor = last_cursor;
        }
        Ok(result)
    }
}
