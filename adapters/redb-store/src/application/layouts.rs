//! Presentation layout reads; mutations remain in the atomic receipt transaction.
use super::validate_stored;
use crate::{RedbStore, codec, error, json, schema::APPLICATION_LAYOUTS};
use milkdrift_blueprint::{RevisionId, WorkflowId};
use milkdrift_persistence::{
    ApplicationCursor, ApplicationLayout, ApplicationLayoutStore, ApplicationPage,
    ApplicationPageQuery, PersistenceError,
};
use std::ops::Bound;

pub(super) const LAYOUT_FAMILY: &str = "application layout";

impl ApplicationLayoutStore for RedbStore {
    fn application_layout(
        &self,
        workflow: &WorkflowId,
        revision: &RevisionId,
    ) -> Result<Option<ApplicationLayout>, PersistenceError> {
        let key = layout_key(workflow, revision)?;
        let read = self.database().begin_read().map_err(error::redb)?;
        let table = read.open_table(APPLICATION_LAYOUTS).map_err(error::redb)?;
        let value = table.get(key.as_slice()).map_err(error::redb)?;
        value
            .map(|bytes| decode_layout(key.as_slice(), bytes.value()))
            .transpose()
    }

    fn application_layouts(
        &self,
        query: &ApplicationPageQuery,
    ) -> Result<ApplicationPage<ApplicationLayout>, PersistenceError> {
        let read = self.database().begin_read().map_err(error::redb)?;
        let table = read.open_table(APPLICATION_LAYOUTS).map_err(error::redb)?;
        let lower: Bound<&[u8]> = query.after.as_ref().map_or(Bound::Unbounded, |cursor| {
            Bound::Excluded(cursor.as_bytes())
        });
        let rows = table
            .range::<&[u8]>((lower, Bound::Unbounded))
            .map_err(error::redb)?;
        let limit = usize::try_from(query.limit.get()).map_err(|_| PersistenceError::Bounds {
            location: "application_layout_page",
            reason: "page size exceeds platform".to_owned(),
        })?;
        let mut items = Vec::with_capacity(limit);
        let mut last_key = None;
        let mut more = false;
        for (index, row) in rows.enumerate() {
            let (key, value) = row.map_err(error::redb)?;
            if index == limit {
                more = true;
                break;
            }
            let key = key.value().to_vec();
            items.push(decode_layout(&key, value.value())?);
            last_key = Some(key);
        }
        Ok(ApplicationPage {
            items,
            next: if more {
                last_key.map(ApplicationCursor::new).transpose()?
            } else {
                None
            },
        })
    }
}

pub(super) fn layout_key(
    workflow: &WorkflowId,
    revision: &RevisionId,
) -> Result<Vec<u8>, PersistenceError> {
    codec::pair(workflow.as_str(), revision.as_str())
}

pub(crate) fn decode_layout(
    key: &[u8],
    bytes: &[u8],
) -> Result<ApplicationLayout, PersistenceError> {
    let layout: ApplicationLayout = json::decode(bytes, LAYOUT_FAMILY)?;
    validate_stored(layout.validate(), LAYOUT_FAMILY)?;
    let components = codec::decode_components(key, 2)?;
    if components[0] != layout.workflow().as_str() || components[1] != layout.revision().as_str() {
        return Err(PersistenceError::Corruption(
            "application layout key does not match its document".to_owned(),
        ));
    }
    Ok(layout)
}
