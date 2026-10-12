//! One administrative read transaction driving typed integrity phases in physical order.

use milkdrift_persistence::{
    IntegrityScanCursor, IntegrityScanFamily, IntegrityScanRequest, IntegrityScanResult,
    PersistenceError,
};
use milkdrift_workspace::ArtifactMetadata;
use std::ops::Bound;

use super::super::{ARTIFACT_MANIFEST, ARTIFACT_METADATA, RedbStore, TASK_WORKFLOWS, error, json};
use super::super::{
    cursor::{
        integrity_cursor_str, make_integrity_cursor, push_failure, storage_anchor,
        validate_integrity_cursor,
    },
    integrity::scan_index_integrity,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PhaseResult {
    Complete,
    LimitReached,
}

struct IntegrityDriver<'request> {
    request: &'request IntegrityScanRequest,
    start_family: IntegrityScanFamily,
    maximum: u64,
    anchor: [u8; 32],
    result: IntegrityScanResult,
    last_cursor: Option<IntegrityScanCursor>,
}

impl<'request> IntegrityDriver<'request> {
    fn new(request: &'request IntegrityScanRequest, anchor: [u8; 32]) -> Self {
        Self {
            request,
            start_family: request
                .cursor
                .as_ref()
                .map_or(IntegrityScanFamily::Tasks, IntegrityScanCursor::family),
            maximum: u64::from(request.limit.get()),
            anchor,
            result: IntegrityScanResult {
                documents_checked: 0,
                artifacts_checked: 0,
                failures: Vec::new(),
                next_cursor: None,
            },
            last_cursor: None,
        }
    }

    fn admit_document(&mut self) -> bool {
        if self.result.documents_checked == self.maximum {
            return false;
        }
        self.result.documents_checked += 1;
        true
    }

    fn finish(mut self, phase: PhaseResult) -> IntegrityScanResult {
        if phase == PhaseResult::LimitReached {
            self.result.next_cursor = self.last_cursor;
        }
        self.result
    }

    fn scan_tasks(
        &mut self,
        revisions: &impl redb::ReadableTable<&'static str, &'static [u8]>,
    ) -> Result<PhaseResult, PersistenceError> {
        if self.start_family > IntegrityScanFamily::Tasks {
            return Ok(PhaseResult::Complete);
        }
        let lower = if self.start_family == IntegrityScanFamily::Tasks {
            self.request
                .cursor
                .as_ref()
                .map(|cursor| integrity_cursor_str(cursor, "revision"))
                .transpose()?
                .map_or(Bound::Unbounded, Bound::Excluded)
        } else {
            Bound::Unbounded
        };
        for item in revisions
            .range::<&str>((lower, Bound::Unbounded))
            .map_err(error::redb)?
        {
            if !self.admit_document() {
                return Ok(PhaseResult::LimitReached);
            }
            let (key, bytes) = item.map_err(error::redb)?;
            self.last_cursor = Some(make_integrity_cursor(
                IntegrityScanFamily::Tasks,
                key.value().as_bytes(),
                self.request.verify_artifact_content,
                self.anchor,
            )?);
            if let Err(cause) = crate::task::validate_snapshot_row(key.value(), bytes.value()) {
                push_failure(&mut self.result, "task_workflow", &cause.to_string())?;
            }
        }
        Ok(PhaseResult::Complete)
    }

    fn scan_artifacts(
        &mut self,
        store: &RedbStore,
        artifacts: &impl redb::ReadableTable<&'static str, &'static [u8]>,
        artifact_manifest: &impl redb::ReadableTable<&'static str, &'static [u8]>,
    ) -> Result<PhaseResult, PersistenceError> {
        if self.start_family > IntegrityScanFamily::Artifacts {
            return Ok(PhaseResult::Complete);
        }
        let lower = if self.start_family == IntegrityScanFamily::Artifacts {
            self.request
                .cursor
                .as_ref()
                .map(|cursor| integrity_cursor_str(cursor, "artifact"))
                .transpose()?
                .map_or(Bound::Unbounded, Bound::Excluded)
        } else {
            Bound::Unbounded
        };
        for item in artifacts
            .range::<&str>((lower, Bound::Unbounded))
            .map_err(error::redb)?
        {
            if !self.admit_document() {
                return Ok(PhaseResult::LimitReached);
            }
            let (key, bytes) = item.map_err(error::redb)?;
            self.last_cursor = Some(make_integrity_cursor(
                IntegrityScanFamily::Artifacts,
                key.value().as_bytes(),
                self.request.verify_artifact_content,
                self.anchor,
            )?);
            let metadata: Result<ArtifactMetadata, _> =
                json::decode(bytes.value(), "artifact metadata");
            match metadata {
                Ok(metadata) if metadata.reference().artifact().as_str() == key.value() => {
                    self.validate_artifact(store, artifact_manifest, key.value(), &metadata)?;
                }
                Ok(_) => push_failure(
                    &mut self.result,
                    "artifact_metadata",
                    "artifact key does not match its document",
                )?,
                Err(cause) => {
                    push_failure(&mut self.result, "artifact_metadata", &cause.to_string())?
                }
            }
        }
        Ok(PhaseResult::Complete)
    }

    fn validate_artifact(
        &mut self,
        store: &RedbStore,
        artifact_manifest: &impl redb::ReadableTable<&'static str, &'static [u8]>,
        key: &str,
        metadata: &ArtifactMetadata,
    ) -> Result<(), PersistenceError> {
        let manifest = artifact_manifest
            .get(key)
            .map_err(error::redb)?
            .map(|bytes| json::decode(bytes.value(), "artifact manifest"))
            .transpose();
        if manifest.as_ref().ok() != Some(&Some(metadata.clone())) {
            push_failure(
                &mut self.result,
                "artifact_indexes",
                "artifact metadata is missing its exact authoritative manifest",
            )?;
        }
        if self.request.verify_artifact_content {
            self.result.artifacts_checked += 1;
            if let Err(cause) = crate::artifact::verify_blob(
                &store.content_path(metadata.reference().digest()),
                metadata.reference(),
                store.max_artifact_bytes,
            ) {
                push_failure(&mut self.result, "artifact_content", &cause.to_string())?;
            }
        }
        Ok(())
    }

    fn scan_indexes(
        &mut self,
        read: &redb::ReadTransaction,
    ) -> Result<PhaseResult, PersistenceError> {
        if self.start_family > IntegrityScanFamily::Indexes {
            return Ok(PhaseResult::Complete);
        }
        let mut more_remaining = false;
        scan_index_integrity(
            read,
            if self.start_family == IntegrityScanFamily::Indexes {
                self.request.cursor.as_ref()
            } else {
                None
            },
            self.maximum,
            self.request.verify_artifact_content,
            self.anchor,
            &mut self.result,
            &mut self.last_cursor,
            &mut more_remaining,
        )?;
        Ok(if more_remaining {
            PhaseResult::LimitReached
        } else {
            PhaseResult::Complete
        })
    }
}

pub(super) fn scan(
    store: &RedbStore,
    request: IntegrityScanRequest,
) -> Result<IntegrityScanResult, PersistenceError> {
    let read = store.database().begin_read().map_err(error::redb)?;
    let revisions = read.open_table(TASK_WORKFLOWS).map_err(error::redb)?;
    let artifacts = read.open_table(ARTIFACT_METADATA).map_err(error::redb)?;
    let artifact_manifest = read.open_table(ARTIFACT_MANIFEST).map_err(error::redb)?;
    let anchor = storage_anchor(&read)?;
    validate_integrity_cursor(&request, &read, &revisions, &artifacts)?;
    let mut driver = IntegrityDriver::new(&request, anchor);
    let mut phase = driver.scan_tasks(&revisions)?;
    if phase == PhaseResult::Complete {
        phase = driver.scan_artifacts(store, &artifacts, &artifact_manifest)?;
    }
    if phase == PhaseResult::Complete {
        phase = driver.scan_indexes(&read)?;
    }
    Ok(driver.finish(phase))
}
