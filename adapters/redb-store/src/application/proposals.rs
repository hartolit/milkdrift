//! Rebuildable proposal discovery and exact receipt cross-reference validation.
use super::{decode_receipt, receipt_from_tiers, receipt_key, validate_stored};
use crate::{
    RedbStore, codec, error, json, schema::APPLICATION_COMMAND_RECEIPTS_COLD,
    schema::APPLICATION_COMMAND_RECEIPTS_HOT, schema::APPLICATION_PROPOSALS,
};
use milkdrift_blueprint::RevisionId;
use milkdrift_persistence::{
    ApplicationCursor, ApplicationEffectReference, ApplicationPage, ApplicationPageQuery,
    PersistenceError, ProposalIndexEntry, ProposalIndexStore,
};
use milkdrift_workspace::RunId;
use std::ops::Bound;

pub(super) const PROPOSAL_FAMILY: &str = "application proposal index";

impl ProposalIndexStore for RedbStore {
    fn proposal_index(
        &self,
        run: &RunId,
        query: &ApplicationPageQuery,
    ) -> Result<ApplicationPage<ProposalIndexEntry>, PersistenceError> {
        let prefix = codec::component(run.as_str())?;
        let end = codec::prefix_end(prefix.clone())
            .ok_or_else(|| error::corruption("proposal prefix has no ordered end"))?;
        let after_key = query
            .after
            .as_ref()
            .map(|cursor| {
                let proposal = std::str::from_utf8(cursor.as_bytes()).map_err(|_| {
                    PersistenceError::InvalidCursor(
                        "proposal cursor is not a UTF-8 proposal identity".to_owned(),
                    )
                })?;
                codec::pair(run.as_str(), proposal)
            })
            .transpose()?;
        let lower: Bound<&[u8]> = after_key
            .as_ref()
            .map_or(Bound::Included(prefix.as_slice()), |key| {
                Bound::Excluded(key.as_slice())
            });
        let read = self.database().begin_read().map_err(error::redb)?;
        let proposals = read
            .open_table(APPLICATION_PROPOSALS)
            .map_err(error::redb)?;
        let hot_receipts = read
            .open_table(APPLICATION_COMMAND_RECEIPTS_HOT)
            .map_err(error::redb)?;
        let cold_receipts = read
            .open_table(APPLICATION_COMMAND_RECEIPTS_COLD)
            .map_err(error::redb)?;
        let rows = proposals
            .range::<&[u8]>((lower, Bound::Excluded(end.as_slice())))
            .map_err(error::redb)?;
        let limit = usize::try_from(query.limit.get()).map_err(|_| PersistenceError::Bounds {
            location: "proposal_index_page",
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
            let entry = decode_proposal(&key, value.value())?;
            validate_proposal_receipt(&hot_receipts, &cold_receipts, &entry)?;
            last_key = Some(entry.proposal.as_bytes().to_vec());
            items.push(entry);
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

    fn rebuild_proposal_index(&self) -> Result<u64, PersistenceError> {
        let write = self.database().begin_write().map_err(error::redb)?;
        write
            .delete_table(APPLICATION_PROPOSALS)
            .map_err(error::redb)?;
        let hot = write
            .open_table(APPLICATION_COMMAND_RECEIPTS_HOT)
            .map_err(error::redb)?;
        let cold = write
            .open_table(APPLICATION_COMMAND_RECEIPTS_COLD)
            .map_err(error::redb)?;
        let mut hot_rows = hot.iter().map_err(error::redb)?;
        let mut cold_rows = cold.iter().map_err(error::redb)?;
        let mut hot_next = hot_rows.next().transpose().map_err(error::redb)?;
        let mut cold_next = cold_rows.next().transpose().map_err(error::redb)?;
        let mut count = 0_u64;
        let mut proposals = write
            .open_table(APPLICATION_PROPOSALS)
            .map_err(error::redb)?;
        loop {
            let take_hot = match (&hot_next, &cold_next) {
                (Some((hot_key, _)), Some((cold_key, _))) => {
                    if hot_key.value() == cold_key.value() {
                        return Err(error::corruption(
                            "application receipt has both hot and cold ownership",
                        ));
                    }
                    hot_key.value() < cold_key.value()
                }
                (Some(_), None) => true,
                (None, Some(_)) => false,
                (None, None) => break,
            };
            let (key, value) = if take_hot {
                let row = hot_next.take().ok_or_else(|| {
                    error::corruption("hot application receipt iterator disappeared")
                })?;
                hot_next = hot_rows.next().transpose().map_err(error::redb)?;
                row
            } else {
                let row = cold_next.take().ok_or_else(|| {
                    error::corruption("cold application receipt iterator disappeared")
                })?;
                cold_next = cold_rows.next().transpose().map_err(error::redb)?;
                row
            };
            let receipt = decode_receipt(key.value(), value.value())?;
            let Some(ApplicationEffectReference::Proposal {
                run,
                proposal,
                proposed_revision,
            }) = receipt.result().effect()
            else {
                continue;
            };
            let entry = ProposalIndexEntry {
                run: run.clone(),
                proposal: proposal.clone(),
                proposed_revision: proposed_revision.clone(),
                receipt_actor: receipt.actor().clone(),
                receipt_command: receipt.command().clone(),
                created_at: receipt.completed_at(),
            };
            let key = proposal_key(run, proposal, proposed_revision)?;
            if let Some(bytes) = proposals.get(key.as_slice()).map_err(error::redb)? {
                let existing = decode_proposal(key.as_slice(), bytes.value())?;
                if existing.run != entry.run
                    || existing.proposal != entry.proposal
                    || existing.proposed_revision != entry.proposed_revision
                {
                    return Err(error::corruption(
                        "proposal rebuild found conflicting proposal facts",
                    ));
                }
            } else {
                let bytes = json::encode(&entry, PROPOSAL_FAMILY)?;
                proposals
                    .insert(key.as_slice(), bytes.as_slice())
                    .map_err(error::redb)?;
                count = count
                    .checked_add(1)
                    .ok_or(PersistenceError::SequenceOverflow)?;
            }
        }
        drop(proposals);
        drop(hot_next);
        drop(cold_next);
        drop(hot_rows);
        drop(cold_rows);
        drop(hot);
        drop(cold);
        write.commit().map_err(error::redb)?;
        Ok(count)
    }
}

pub(super) fn proposal_key(
    run: &RunId,
    proposal: &str,
    _revision: &RevisionId,
) -> Result<Vec<u8>, PersistenceError> {
    codec::pair(run.as_str(), proposal)
}

pub(crate) fn decode_proposal(
    key: &[u8],
    bytes: &[u8],
) -> Result<ProposalIndexEntry, PersistenceError> {
    let entry: ProposalIndexEntry = json::decode(bytes, PROPOSAL_FAMILY)?;
    validate_stored(entry.validate(), PROPOSAL_FAMILY)?;
    let components = codec::decode_components(key, 2)?;
    if components[0] != entry.run.as_str() || components[1] != entry.proposal {
        return Err(PersistenceError::Corruption(
            "proposal index key does not match its document".to_owned(),
        ));
    }
    Ok(entry)
}

pub(crate) fn validate_proposal_receipt(
    hot: &impl redb::ReadableTable<&'static [u8], &'static [u8]>,
    cold: &impl redb::ReadableTable<&'static [u8], &'static [u8]>,
    entry: &ProposalIndexEntry,
) -> Result<(), PersistenceError> {
    let key = receipt_key(&entry.receipt_actor, &entry.receipt_command)?;
    let receipt = receipt_from_tiers(hot, cold, key.as_slice())?.ok_or_else(|| {
        PersistenceError::Corruption(
            "proposal index has no authoritative application receipt".to_owned(),
        )
    })?;
    match receipt.result().effect() {
        Some(ApplicationEffectReference::Proposal {
            run,
            proposal,
            proposed_revision,
        }) if run == &entry.run
            && proposal == &entry.proposal
            && proposed_revision == &entry.proposed_revision =>
        {
            Ok(())
        }
        _ => Err(PersistenceError::Corruption(
            "proposal index disagrees with its authoritative receipt".to_owned(),
        )),
    }
}

use redb::ReadableTable as _;
