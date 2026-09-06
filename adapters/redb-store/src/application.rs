mod layouts;
pub(crate) use layouts::decode_layout;
use layouts::layout_key;
mod proposals;
use proposals::{PROPOSAL_FAMILY, proposal_key};
pub(crate) use proposals::{decode_proposal, validate_proposal_receipt};
mod audit;
pub(crate) use audit::decode_security_audit;

use std::ops::Bound;

use milkdrift_authority::ActorRef;
use milkdrift_persistence::{
    ApplicationCommandCommit, ApplicationCommandCommitOutcome, ApplicationCommandEffect,
    ApplicationCommandReceipt, ApplicationCommandStore, ApplicationCursor, ApplicationLayout,
    ApplicationPage, ApplicationPageQuery, ApplicationReceiptArchiveOutcome,
    ApplicationReceiptArchiveRequest, ApplicationReceiptStatus, CommandId, PersistenceError,
    TimestampMillis,
};
use redb::{ReadableTable, ReadableTableMetadata};

use crate::{
    RedbStore, clock::require_clock_in_transaction, codec, error, fault::FaultPoint, json,
    schema::APPLICATION_COLD_RECEIPT_COUNT_KEY, schema::APPLICATION_COMMAND_RECEIPTS_COLD,
    schema::APPLICATION_COMMAND_RECEIPTS_HOT, schema::APPLICATION_HOT_RECEIPT_COUNT_KEY,
    schema::APPLICATION_HOT_RECEIPTS_BY_COMPLETION, schema::APPLICATION_LAYOUTS,
    schema::APPLICATION_PROPOSALS, schema::APPLICATION_RECEIPT_ARCHIVE_GENERATION_KEY,
    schema::APPLICATION_RECEIPT_LAST_ARCHIVED_AT_KEY, schema::METADATA, schema::SECURITY_AUDIT,
    schema::SECURITY_AUDIT_COUNT_KEY,
};

const RECEIPT_FAMILY: &str = "application command receipt";

impl RedbStore {
    pub(crate) fn reestablish_application_retention_bounds(&self) -> Result<(), PersistenceError> {
        let write = self.database().begin_write().map_err(error::redb)?;
        let mut accounting = transaction_receipt_accounting(&write)?;
        let hot_bound = u64::from(self.hot_application_receipt_bound);
        if accounting.hot_count > hot_bound {
            let archived_at = self.clock.now()?;
            require_clock_in_transaction(&write, archived_at)?;
            while accounting.hot_count > hot_bound {
                let excess = accounting.hot_count - hot_bound;
                let maximum = u32::try_from(excess)
                    .unwrap_or(u32::MAX)
                    .min(self.application_receipt_archive_batch_size);
                let archived = move_receipts_with_verified_accounting(
                    &write,
                    self,
                    &mut accounting,
                    maximum,
                    archived_at,
                )?;
                if archived == 0 {
                    return Err(error::corruption(
                        "hot receipt accounting exceeds its bound without archive candidates",
                    ));
                }
            }
            write_receipt_accounting(&write, &accounting)?;
        }

        let retained_audits = {
            let mut audit = write.open_table(SECURITY_AUDIT).map_err(error::redb)?;
            let mut count = audit.len().map_err(error::redb)?;
            let bound = u64::from(self.max_security_audit_records);
            while count > bound {
                let oldest = audit
                    .first()
                    .map_err(error::redb)?
                    .map(|(key, _)| key.value())
                    .ok_or_else(|| error::corruption("security audit table length is invalid"))?;
                audit.remove(oldest).map_err(error::redb)?;
                count -= 1;
            }
            count
        };
        {
            let mut metadata = write.open_table(METADATA).map_err(error::redb)?;
            metadata
                .insert(SECURITY_AUDIT_COUNT_KEY, retained_audits)
                .map_err(error::redb)?;
        }
        write.commit().map_err(error::redb)
    }
}

impl ApplicationCommandStore for RedbStore {
    fn application_command_receipt(
        &self,
        actor: &ActorRef,
        command: &CommandId,
    ) -> Result<Option<ApplicationCommandReceipt>, PersistenceError> {
        let key = receipt_key(actor, command)?;
        let read = self.database().begin_read().map_err(error::redb)?;
        let hot = read
            .open_table(APPLICATION_COMMAND_RECEIPTS_HOT)
            .map_err(error::redb)?;
        let cold = read
            .open_table(APPLICATION_COMMAND_RECEIPTS_COLD)
            .map_err(error::redb)?;
        receipt_from_tiers(&hot, &cold, key.as_slice())
    }

    fn commit_application_command(
        &self,
        commit: &ApplicationCommandCommit,
    ) -> Result<ApplicationCommandCommitOutcome, PersistenceError> {
        commit.receipt.validate()?;
        let receipt_key = receipt_key(commit.receipt.actor(), commit.receipt.command())?;
        let write = self.database().begin_write().map_err(error::redb)?;
        let existing = {
            let hot = write
                .open_table(APPLICATION_COMMAND_RECEIPTS_HOT)
                .map_err(error::redb)?;
            let cold = write
                .open_table(APPLICATION_COMMAND_RECEIPTS_COLD)
                .map_err(error::redb)?;
            receipt_from_tiers(&hot, &cold, receipt_key.as_slice())?
        };
        if let Some(stored) = existing {
            if stored.command_digest() != commit.receipt.command_digest() {
                return Err(PersistenceError::ExternalCommandIdempotencyConflict {
                    actor: commit.receipt.actor().clone(),
                    command: commit.receipt.command().clone(),
                    existing: stored.command_digest().clone(),
                    supplied: commit.receipt.command_digest().clone(),
                });
            }
            return Ok(ApplicationCommandCommitOutcome::Replayed(Box::new(stored)));
        }
        commit.validate()?;
        let mut accounting = transaction_receipt_accounting(&write)?;
        while accounting.hot_count >= u64::from(self.hot_application_receipt_bound) {
            let excess_after_insert = accounting
                .hot_count
                .saturating_add(1)
                .saturating_sub(u64::from(self.hot_application_receipt_bound));
            let maximum = u32::try_from(excess_after_insert)
                .unwrap_or(u32::MAX)
                .min(self.application_receipt_archive_batch_size);
            let archived = move_receipts_with_verified_accounting(
                &write,
                self,
                &mut accounting,
                maximum,
                commit.receipt.completed_at(),
            )?;
            if archived == 0 {
                return Err(error::corruption(
                    "hot receipt accounting exceeds its bound without archive candidates",
                ));
            }
        }

        match &commit.effect {
            ApplicationCommandEffect::None => {}
            ApplicationCommandEffect::PutLayout(update) => {
                let key = layout_key(&update.workflow, &update.revision)?;
                let mut table = write.open_table(APPLICATION_LAYOUTS).map_err(error::redb)?;
                let existing = table
                    .get(key.as_slice())
                    .map_err(error::redb)?
                    .map(|bytes| decode_layout(key.as_slice(), bytes.value()))
                    .transpose()?;
                let created_at = if let Some(current) = &existing {
                    if current.digest() == &update.digest {
                        if current.generation() != update.generation
                            || current.document() != update.document
                        {
                            return Err(PersistenceError::Corruption(
                                "equal layout digest is bound to different generation/document"
                                    .to_owned(),
                            ));
                        }
                        current.created_at()
                    } else {
                        if update.generation != current.generation().saturating_add(1) {
                            return Err(PersistenceError::ImmutableConflict {
                                entity: "application layout generation",
                                identity: format!(
                                    "{}/{}",
                                    update.workflow.as_str(),
                                    update.revision.as_str()
                                ),
                            });
                        }
                        current.created_at()
                    }
                } else {
                    if update.generation != 1 {
                        return Err(PersistenceError::ImmutableConflict {
                            entity: "application layout generation",
                            identity: format!(
                                "{}/{}",
                                update.workflow.as_str(),
                                update.revision.as_str()
                            ),
                        });
                    }
                    update.updated_at
                };
                if existing
                    .as_ref()
                    .is_none_or(|current| current.digest() != &update.digest)
                {
                    let layout = ApplicationLayout::from_update(update.clone(), created_at)?;
                    let bytes = json::encode(&layout, LAYOUT_FAMILY)?;
                    table
                        .insert(key.as_slice(), bytes.as_slice())
                        .map_err(error::redb)?;
                }
            }
            ApplicationCommandEffect::IndexProposal(entry) => {
                entry.validate()?;
                let key = proposal_key(&entry.run, &entry.proposal, &entry.proposed_revision)?;
                let mut table = write
                    .open_table(APPLICATION_PROPOSALS)
                    .map_err(error::redb)?;
                if let Some(bytes) = table.get(key.as_slice()).map_err(error::redb)? {
                    let existing = decode_proposal(key.as_slice(), bytes.value())?;
                    if existing.proposed_revision != entry.proposed_revision {
                        return Err(PersistenceError::Corruption(
                            "proposal index key is bound to different proposal facts".to_owned(),
                        ));
                    }
                } else {
                    let bytes = json::encode(entry, PROPOSAL_FAMILY)?;
                    table
                        .insert(key.as_slice(), bytes.as_slice())
                        .map_err(error::redb)?;
                }
            }
        }

        {
            let bytes = json::encode(&commit.receipt, RECEIPT_FAMILY)?;
            let mut receipts = write
                .open_table(APPLICATION_COMMAND_RECEIPTS_HOT)
                .map_err(error::redb)?;
            receipts
                .insert(receipt_key.as_slice(), bytes.as_slice())
                .map_err(error::redb)?;
            let order_key = receipt_order_key(&commit.receipt)?;
            let mut ordered = write
                .open_table(APPLICATION_HOT_RECEIPTS_BY_COMPLETION)
                .map_err(error::redb)?;
            if ordered
                .insert(order_key.as_slice(), receipt_key.as_slice())
                .map_err(error::redb)?
                .is_some()
            {
                return Err(error::corruption(
                    "application receipt completion index rejected a unique identity",
                ));
            }
        }
        accounting.hot_count = accounting
            .hot_count
            .checked_add(1)
            .ok_or(PersistenceError::SequenceOverflow)?;
        write_receipt_accounting(&write, &accounting)?;
        self.faults.check(FaultPoint::BeforeApplicationCommit)?;
        write.commit().map_err(error::redb)?;
        self.faults.check(FaultPoint::AfterApplicationCommit)?;
        Ok(ApplicationCommandCommitOutcome::Committed)
    }

    fn application_command_receipts(
        &self,
        query: &ApplicationPageQuery,
    ) -> Result<ApplicationPage<ApplicationCommandReceipt>, PersistenceError> {
        let read = self.database().begin_read().map_err(error::redb)?;
        let hot = read
            .open_table(APPLICATION_COMMAND_RECEIPTS_HOT)
            .map_err(error::redb)?;
        let cold = read
            .open_table(APPLICATION_COMMAND_RECEIPTS_COLD)
            .map_err(error::redb)?;
        let lower: Bound<&[u8]> = query.after.as_ref().map_or(Bound::Unbounded, |cursor| {
            Bound::Excluded(cursor.as_bytes())
        });
        let mut hot_rows = hot
            .range::<&[u8]>((lower, Bound::Unbounded))
            .map_err(error::redb)?;
        let lower: Bound<&[u8]> = query.after.as_ref().map_or(Bound::Unbounded, |cursor| {
            Bound::Excluded(cursor.as_bytes())
        });
        let mut cold_rows = cold
            .range::<&[u8]>((lower, Bound::Unbounded))
            .map_err(error::redb)?;
        let limit = usize::try_from(query.limit.get()).map_err(|_| PersistenceError::Bounds {
            location: "application_receipt_page",
            reason: "page size exceeds platform".to_owned(),
        })?;
        let mut items = Vec::with_capacity(limit);
        let mut last_key = None;
        let mut hot_next = hot_rows.next().transpose().map_err(error::redb)?;
        let mut cold_next = cold_rows.next().transpose().map_err(error::redb)?;
        while items.len() < limit {
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
            let key = key.value().to_vec();
            items.push(decode_receipt(key.as_slice(), value.value())?);
            last_key = Some(key);
        }
        let more = hot_next.is_some() || cold_next.is_some();
        Ok(ApplicationPage {
            items,
            next: if more {
                last_key.map(ApplicationCursor::new).transpose()?
            } else {
                None
            },
        })
    }

    fn application_receipt_status(&self) -> Result<ApplicationReceiptStatus, PersistenceError> {
        let read = self.database().begin_read().map_err(error::redb)?;
        read_receipt_accounting(&read).map(|accounting| accounting.status(self))
    }

    fn archive_application_command_receipts(
        &self,
        request: ApplicationReceiptArchiveRequest,
    ) -> Result<ApplicationReceiptArchiveOutcome, PersistenceError> {
        let write = self.database().begin_write().map_err(error::redb)?;
        let mut accounting = transaction_receipt_accounting(&write)?;
        if accounting.archive_generation != request.expected_generation {
            return Err(
                PersistenceError::ApplicationReceiptArchiveGenerationConflict {
                    expected: request.expected_generation,
                    actual: accounting.archive_generation,
                },
            );
        }
        let archived = move_receipts_with_verified_accounting(
            &write,
            self,
            &mut accounting,
            self.application_receipt_archive_batch_size,
            request.archived_at,
        )?;
        write_receipt_accounting(&write, &accounting)?;
        self.faults
            .check(FaultPoint::BeforeApplicationReceiptArchiveCommit)?;
        write.commit().map_err(error::redb)?;
        self.faults
            .check(FaultPoint::AfterApplicationReceiptArchiveCommit)?;
        Ok(ApplicationReceiptArchiveOutcome {
            archived,
            status: accounting.status(self),
        })
    }
}

fn receipt_key(actor: &ActorRef, command: &CommandId) -> Result<Vec<u8>, PersistenceError> {
    codec::pair(actor.as_str(), command.as_str())
}

pub(crate) fn receipt_order_key(
    receipt: &ApplicationCommandReceipt,
) -> Result<Vec<u8>, PersistenceError> {
    let identity = receipt_key(receipt.actor(), receipt.command())?;
    let mut key = Vec::with_capacity(8_usize.saturating_add(identity.len()));
    key.extend_from_slice(&receipt.completed_at().get().to_be_bytes());
    key.extend_from_slice(&identity);
    Ok(key)
}

fn receipt_from_tiers(
    hot: &impl redb::ReadableTable<&'static [u8], &'static [u8]>,
    cold: &impl redb::ReadableTable<&'static [u8], &'static [u8]>,
    key: &[u8],
) -> Result<Option<ApplicationCommandReceipt>, PersistenceError> {
    let hot_value = hot.get(key).map_err(error::redb)?;
    let cold_value = cold.get(key).map_err(error::redb)?;
    match (hot_value, cold_value) {
        (Some(_), Some(_)) => Err(error::corruption(
            "application receipt has both hot and cold ownership",
        )),
        (Some(bytes), None) | (None, Some(bytes)) => decode_receipt(key, bytes.value()).map(Some),
        (None, None) => Ok(None),
    }
}

#[derive(Clone, Copy)]
struct ReceiptAccounting {
    hot_count: u64,
    cold_count: u64,
    archive_generation: u64,
    last_archived_at: u64,
}

impl ReceiptAccounting {
    fn status(self, store: &RedbStore) -> ApplicationReceiptStatus {
        ApplicationReceiptStatus {
            hot_count: self.hot_count,
            cold_count: self.cold_count,
            hot_bound: store.hot_application_receipt_bound,
            archive_batch_size: store.application_receipt_archive_batch_size,
            archive_generation: self.archive_generation,
            last_archived_at: (self.archive_generation != 0)
                .then_some(TimestampMillis::new(self.last_archived_at)),
        }
    }
}

fn receipt_accounting_values(
    metadata: &impl redb::ReadableTable<&'static str, u64>,
    hot: &impl redb::ReadableTable<&'static [u8], &'static [u8]>,
    cold: &impl redb::ReadableTable<&'static [u8], &'static [u8]>,
    ordered: &impl redb::ReadableTable<&'static [u8], &'static [u8]>,
) -> Result<ReceiptAccounting, PersistenceError> {
    let value = |key, missing| {
        metadata
            .get(key)
            .map_err(error::redb)?
            .map(|value| value.value())
            .ok_or_else(|| error::corruption(missing))
    };
    let accounting = ReceiptAccounting {
        hot_count: value(
            APPLICATION_HOT_RECEIPT_COUNT_KEY,
            "hot application receipt count is missing",
        )?,
        cold_count: value(
            APPLICATION_COLD_RECEIPT_COUNT_KEY,
            "cold application receipt count is missing",
        )?,
        archive_generation: value(
            APPLICATION_RECEIPT_ARCHIVE_GENERATION_KEY,
            "application receipt archive generation is missing",
        )?,
        last_archived_at: value(
            APPLICATION_RECEIPT_LAST_ARCHIVED_AT_KEY,
            "application receipt archive time is missing",
        )?,
    };
    if hot.len().map_err(error::redb)? != accounting.hot_count {
        return Err(error::corruption(
            "hot application receipt count disagrees with its authoritative table",
        ));
    }
    if cold.len().map_err(error::redb)? != accounting.cold_count {
        return Err(error::corruption(
            "cold application receipt count disagrees with its authoritative table",
        ));
    }
    if ordered.len().map_err(error::redb)? != accounting.hot_count {
        return Err(error::corruption(
            "hot application receipt count disagrees with its completion index",
        ));
    }
    Ok(accounting)
}

fn read_receipt_accounting(
    read: &redb::ReadTransaction,
) -> Result<ReceiptAccounting, PersistenceError> {
    let metadata = read.open_table(METADATA).map_err(error::redb)?;
    let hot = read
        .open_table(APPLICATION_COMMAND_RECEIPTS_HOT)
        .map_err(error::redb)?;
    let cold = read
        .open_table(APPLICATION_COMMAND_RECEIPTS_COLD)
        .map_err(error::redb)?;
    let ordered = read
        .open_table(APPLICATION_HOT_RECEIPTS_BY_COMPLETION)
        .map_err(error::redb)?;
    receipt_accounting_values(&metadata, &hot, &cold, &ordered)
}

fn transaction_receipt_accounting(
    write: &redb::WriteTransaction,
) -> Result<ReceiptAccounting, PersistenceError> {
    let metadata = write.open_table(METADATA).map_err(error::redb)?;
    let hot = write
        .open_table(APPLICATION_COMMAND_RECEIPTS_HOT)
        .map_err(error::redb)?;
    let cold = write
        .open_table(APPLICATION_COMMAND_RECEIPTS_COLD)
        .map_err(error::redb)?;
    let ordered = write
        .open_table(APPLICATION_HOT_RECEIPTS_BY_COMPLETION)
        .map_err(error::redb)?;
    receipt_accounting_values(&metadata, &hot, &cold, &ordered)
}

fn write_receipt_accounting(
    write: &redb::WriteTransaction,
    accounting: &ReceiptAccounting,
) -> Result<(), PersistenceError> {
    let mut metadata = write.open_table(METADATA).map_err(error::redb)?;
    metadata
        .insert(APPLICATION_HOT_RECEIPT_COUNT_KEY, accounting.hot_count)
        .map_err(error::redb)?;
    metadata
        .insert(APPLICATION_COLD_RECEIPT_COUNT_KEY, accounting.cold_count)
        .map_err(error::redb)?;
    metadata
        .insert(
            APPLICATION_RECEIPT_ARCHIVE_GENERATION_KEY,
            accounting.archive_generation,
        )
        .map_err(error::redb)?;
    metadata
        .insert(
            APPLICATION_RECEIPT_LAST_ARCHIVED_AT_KEY,
            accounting.last_archived_at,
        )
        .map_err(error::redb)?;
    Ok(())
}

fn archive_oldest_hot_receipts(
    write: &redb::WriteTransaction,
    store: &RedbStore,
    accounting: &mut ReceiptAccounting,
    maximum: u32,
    archived_at: TimestampMillis,
) -> Result<u32, PersistenceError> {
    let candidates = {
        let ordered = write
            .open_table(APPLICATION_HOT_RECEIPTS_BY_COMPLETION)
            .map_err(error::redb)?;
        ordered
            .iter()
            .map_err(error::redb)?
            .take(
                usize::try_from(maximum).map_err(|_| PersistenceError::Bounds {
                    location: "application_receipt_archive",
                    reason: "archive batch exceeds platform".to_owned(),
                })?,
            )
            .map(|row| {
                row.map(|(order, identity)| (order.value().to_vec(), identity.value().to_vec()))
                    .map_err(error::redb)
            })
            .collect::<Result<Vec<_>, _>>()?
    };
    if candidates.is_empty() {
        return Ok(0);
    }
    let mut hot = write
        .open_table(APPLICATION_COMMAND_RECEIPTS_HOT)
        .map_err(error::redb)?;
    let mut cold = write
        .open_table(APPLICATION_COMMAND_RECEIPTS_COLD)
        .map_err(error::redb)?;
    let mut ordered = write
        .open_table(APPLICATION_HOT_RECEIPTS_BY_COMPLETION)
        .map_err(error::redb)?;
    for (order_key, identity_key) in &candidates {
        let bytes = hot
            .get(identity_key.as_slice())
            .map_err(error::redb)?
            .ok_or_else(|| {
                error::corruption("hot receipt completion index has no authoritative receipt")
            })?
            .value()
            .to_vec();
        let receipt = decode_receipt(identity_key.as_slice(), bytes.as_slice())?;
        if receipt_order_key(&receipt)? != *order_key {
            return Err(error::corruption(
                "hot receipt completion index disagrees with its receipt",
            ));
        }
        if cold
            .get(identity_key.as_slice())
            .map_err(error::redb)?
            .is_some()
        {
            return Err(error::corruption(
                "application receipt has both hot and cold ownership",
            ));
        }
        store
            .faults
            .check(FaultPoint::BeforeApplicationReceiptColdInsert)?;
        cold.insert(identity_key.as_slice(), bytes.as_slice())
            .map_err(error::redb)?;
        store
            .faults
            .check(FaultPoint::AfterApplicationReceiptColdInsert)?;
        if hot
            .remove(identity_key.as_slice())
            .map_err(error::redb)?
            .is_none()
        {
            return Err(error::corruption(
                "hot application receipt disappeared during archival",
            ));
        }
        store
            .faults
            .check(FaultPoint::AfterApplicationReceiptHotRemove)?;
        if ordered
            .remove(order_key.as_slice())
            .map_err(error::redb)?
            .is_none()
        {
            return Err(error::corruption(
                "hot application receipt completion index disappeared during archival",
            ));
        }
    }
    let archived = u32::try_from(candidates.len()).map_err(|_| PersistenceError::Bounds {
        location: "application_receipt_archive",
        reason: "archive result exceeds u32".to_owned(),
    })?;
    accounting.hot_count = accounting
        .hot_count
        .checked_sub(u64::from(archived))
        .ok_or_else(|| error::corruption("hot receipt accounting underflow"))?;
    accounting.cold_count = accounting
        .cold_count
        .checked_add(u64::from(archived))
        .ok_or(PersistenceError::SequenceOverflow)?;
    accounting.archive_generation = accounting
        .archive_generation
        .checked_add(1)
        .ok_or(PersistenceError::SequenceOverflow)?;
    accounting.last_archived_at = archived_at.get();
    Ok(archived)
}

fn move_receipts_with_verified_accounting(
    write: &redb::WriteTransaction,
    store: &RedbStore,
    accounting: &mut ReceiptAccounting,
    maximum: u32,
    archived_at: TimestampMillis,
) -> Result<u32, PersistenceError> {
    let before = *accounting;
    let archived = archive_oldest_hot_receipts(write, store, accounting, maximum, archived_at)?;
    let archived_count = u64::from(archived);
    let expected_hot = before
        .hot_count
        .checked_sub(archived_count)
        .ok_or_else(|| error::corruption("receipt archival reported more rows than existed"))?;
    let expected_cold = before
        .cold_count
        .checked_add(archived_count)
        .ok_or(PersistenceError::SequenceOverflow)?;
    let expected_generation = if archived == 0 {
        before.archive_generation
    } else {
        before
            .archive_generation
            .checked_add(1)
            .ok_or(PersistenceError::SequenceOverflow)?
    };
    let expected_archived_at = if archived == 0 {
        before.last_archived_at
    } else {
        archived_at.get()
    };
    if accounting.hot_count != expected_hot
        || accounting.cold_count != expected_cold
        || accounting.archive_generation != expected_generation
        || accounting.last_archived_at != expected_archived_at
    {
        return Err(error::corruption(
            "receipt archival result disagrees with durable accounting progress",
        ));
    }
    Ok(archived)
}

pub(crate) fn decode_receipt(
    key: &[u8],
    bytes: &[u8],
) -> Result<ApplicationCommandReceipt, PersistenceError> {
    let receipt: ApplicationCommandReceipt = json::decode(bytes, RECEIPT_FAMILY)?;
    validate_stored(receipt.validate(), RECEIPT_FAMILY)?;
    let components = codec::decode_components(key, 2)?;
    if components[0] != receipt.actor().as_str() || components[1] != receipt.command().as_str() {
        return Err(PersistenceError::Corruption(
            "application receipt key does not match its document".to_owned(),
        ));
    }
    Ok(receipt)
}

fn validate_stored(
    result: Result<(), PersistenceError>,
    family: &str,
) -> Result<(), PersistenceError> {
    match result {
        Ok(()) => Ok(()),
        Err(error @ PersistenceError::UnsupportedVersion { .. }) => Err(error),
        Err(error) => Err(PersistenceError::Corruption(format!(
            "stored {family} violates its document contract: {error}"
        ))),
    }
}

use layouts::LAYOUT_FAMILY;
