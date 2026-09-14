use super::Result;
use milkdrift_persistence::{
    IntegrityScanCursor, IntegrityScanFamily, IntegrityScanRequest, PageSize,
};
use milkdrift_redb_store::offline::OfflineStore;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Cursor {
    source: String,
    family: u8,
    after: Vec<u8>,
    hash_artifacts: bool,
}

pub(super) fn page(
    store: &OfflineStore,
    limit: u32,
    after: Option<&str>,
    hash_artifacts: bool,
) -> Result<Value> {
    let cursor = after
        .map(|text| -> Result<_> {
            if text.len() > 8192 {
                return Err("integrity cursor exceeds 8192 bytes".into());
            }
            let cursor: Cursor = serde_json::from_str(text)?;
            if cursor.source != store.database_digest() || cursor.hash_artifacts != hash_artifacts {
                return Err("integrity cursor belongs to another source or hash mode".into());
            }
            let family = match cursor.family {
                0 => IntegrityScanFamily::Revisions,
                1 => IntegrityScanFamily::RunEvents,
                2 => IntegrityScanFamily::Artifacts,
                3 => IntegrityScanFamily::Indexes,
                _ => return Err("invalid integrity family".into()),
            };
            Ok(IntegrityScanCursor::new(
                family,
                cursor.after,
                hash_artifacts,
            )?)
        })
        .transpose()?;
    let page = store.scan_integrity(IntegrityScanRequest {
        limit: PageSize::new(limit)?,
        cursor,
        verify_artifact_content: hash_artifacts,
    })?;
    let next = page
        .next_cursor
        .map(|cursor| {
            serde_json::to_string(&Cursor {
                source: store.database_digest().into(),
                family: match cursor.family() {
                    IntegrityScanFamily::Revisions => 0,
                    IntegrityScanFamily::RunEvents => 1,
                    IntegrityScanFamily::Artifacts => 2,
                    IntegrityScanFamily::Indexes => 3,
                },
                after: cursor.after_key().to_vec(),
                hash_artifacts,
            })
        })
        .transpose()?;
    Ok(
        json!({"documents_checked": page.documents_checked, "artifacts_checked": page.artifacts_checked,
        "failures": page.failures.iter().map(|f| json!({"component": f.component.as_str(), "classification": "integrity_failure", "detail": "redacted; inspect the affected record family"})).collect::<Vec<_>>(),
        "next": next, "scope": "this page only; complete all pages in the same hash mode for a full scan"}),
    )
}
