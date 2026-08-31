use super::super::*;
use super::{ScanContext, phase};
use redb::ReadableTableMetadata as _;

pub(super) fn scan(context: &mut ScanContext<'_, '_>) -> Result<(), PersistenceError> {
    let read = context.read;
    let hot = read
        .open_table(APPLICATION_COMMAND_RECEIPTS_HOT)
        .map_err(error::redb)?;
    let cold = read
        .open_table(APPLICATION_COMMAND_RECEIPTS_COLD)
        .map_err(error::redb)?;
    let ordered = read
        .open_table(APPLICATION_HOT_RECEIPTS_BY_COMPLETION)
        .map_err(error::redb)?;
    let layouts = read.open_table(APPLICATION_LAYOUTS).map_err(error::redb)?;
    let proposals = read
        .open_table(APPLICATION_PROPOSALS)
        .map_err(error::redb)?;
    let audit = read.open_table(SECURITY_AUDIT).map_err(error::redb)?;
    let metadata = read.open_table(METADATA).map_err(error::redb)?;
    let hot_count = metadata
        .get(APPLICATION_HOT_RECEIPT_COUNT_KEY)
        .map_err(error::redb)?
        .map(|value| value.value())
        .ok_or_else(|| error::corruption("hot application receipt count is missing"))?;
    if hot.len().map_err(error::redb)? != hot_count
        || ordered.len().map_err(error::redb)? != hot_count
    {
        return Err(error::corruption(
            "hot application receipt count disagrees with its table/index",
        ));
    }
    let cold_count = metadata
        .get(APPLICATION_COLD_RECEIPT_COUNT_KEY)
        .map_err(error::redb)?
        .map(|value| value.value())
        .ok_or_else(|| error::corruption("cold application receipt count is missing"))?;
    if cold.len().map_err(error::redb)? != cold_count {
        return Err(error::corruption(
            "cold application receipt count disagrees with its table",
        ));
    }
    let audit_count = metadata
        .get(SECURITY_AUDIT_COUNT_KEY)
        .map_err(error::redb)?
        .map(|value| value.value())
        .ok_or_else(|| error::corruption("security audit count is missing"))?;
    if audit.len().map_err(error::redb)? != audit_count {
        return Err(error::corruption(
            "security audit count disagrees with its authoritative table",
        ));
    }
    context.binary_bytes(
        phase::APPLICATION_HOT_RECEIPTS,
        &hot,
        "application_hot_receipts",
        |key, bytes| {
            if cold.get(key).map_err(error::redb)?.is_some() {
                return Err(error::corruption(
                    "application receipt has both hot and cold ownership",
                ));
            }
            let receipt = crate::application::decode_receipt(key, bytes)?;
            let order_key = crate::application::receipt_order_key(&receipt)?;
            let indexed = ordered
                .get(order_key.as_slice())
                .map_err(error::redb)?
                .ok_or_else(|| {
                    error::corruption("hot application receipt has no completion index")
                })?;
            if indexed.value() != key {
                return Err(error::corruption(
                    "hot application receipt completion index points elsewhere",
                ));
            }
            Ok(())
        },
    )?;
    context.binary_bytes(
        phase::APPLICATION_COLD_RECEIPTS,
        &cold,
        "application_cold_receipts",
        |key, bytes| {
            if hot.get(key).map_err(error::redb)?.is_some() {
                return Err(error::corruption(
                    "application receipt has both hot and cold ownership",
                ));
            }
            crate::application::decode_receipt(key, bytes).map(|_| ())
        },
    )?;
    context.binary_bytes(
        phase::APPLICATION_HOT_RECEIPT_ORDER,
        &ordered,
        "application_hot_receipt_order",
        |order_key, identity_key| {
            let bytes = hot
                .get(identity_key)
                .map_err(error::redb)?
                .ok_or_else(|| error::corruption("hot receipt completion index has no receipt"))?;
            let receipt = crate::application::decode_receipt(identity_key, bytes.value())?;
            if crate::application::receipt_order_key(&receipt)? != order_key {
                return Err(error::corruption(
                    "hot receipt completion index key disagrees with receipt",
                ));
            }
            Ok(())
        },
    )?;
    context.binary_bytes(
        phase::APPLICATION_LAYOUTS,
        &layouts,
        "application_layouts",
        |key, bytes| crate::application::decode_layout(key, bytes).map(|_| ()),
    )?;
    context.binary_bytes(
        phase::APPLICATION_PROPOSALS,
        &proposals,
        "application_proposals",
        |key, bytes| {
            let entry = crate::application::decode_proposal(key, bytes)?;
            crate::application::validate_proposal_receipt(&hot, &cold, &entry)
        },
    )?;
    context.u64_bytes(
        phase::SECURITY_AUDIT,
        &audit,
        "security_audit",
        |sequence, bytes| crate::application::decode_security_audit(sequence, bytes).map(|_| ()),
    )
}
