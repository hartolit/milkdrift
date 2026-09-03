//! Bounded durable journal paging through the frozen discovery boundary.

use milkdrift_persistence::{EventPageQuery, PageSize};

use super::DiscoveryState;
use crate::context::source::{ContextBuildError, SOURCE_PAGE_SIZE, persistence};

impl DiscoveryState<'_, '_, '_> {
    pub(super) fn fold_journal(&mut self) -> Result<(), ContextBuildError> {
        'pages: loop {
            let remaining = self.maximum_records - self.scanned;
            if remaining == 0 {
                break;
            }
            let page_size = SOURCE_PAGE_SIZE.min(remaining);
            let page = self
                .source
                .store
                .events(
                    &EventPageQuery::new(
                        self.request.identity.run.clone(),
                        self.cursor.take(),
                        PageSize::new(page_size).map_err(persistence)?,
                    )
                    .map_err(persistence)?,
                )
                .map_err(persistence)?;
            self.scanned = self
                .scanned
                .checked_add(
                    u32::try_from(page.events.len())
                        .map_err(|_| ContextBuildError::AccountingOverflow)?,
                )
                .ok_or(ContextBuildError::AccountingOverflow)?;
            for event in &page.events {
                if event.sequence() > self.request.through_sequence {
                    break 'pages;
                }
                self.fold_event(event)?;
            }
            self.cursor = page.next;
            if self.cursor.is_none() {
                break;
            }
        }
        Ok(())
    }
}
