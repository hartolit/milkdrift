use super::super::*;
use super::{ScanContext, phase};
use redb::ReadableTableMetadata as _;

pub(super) fn scan(context: &mut ScanContext<'_, '_>) -> Result<(), PersistenceError> {
    let read = context.read;
    let receipts = read
        .open_table(APPLICATION_COMMAND_RECEIPTS)
        .map_err(error::redb)?;
    let layouts = read.open_table(APPLICATION_LAYOUTS).map_err(error::redb)?;
    let proposals = read
        .open_table(APPLICATION_PROPOSALS)
        .map_err(error::redb)?;
    let audit = read.open_table(SECURITY_AUDIT).map_err(error::redb)?;
    let metadata = read.open_table(METADATA).map_err(error::redb)?;
    let receipt_count = metadata
        .get(APPLICATION_RECEIPT_COUNT_KEY)
        .map_err(error::redb)?
        .map(|value| value.value())
        .ok_or_else(|| error::corruption("application receipt count is missing"))?;
    if receipts.len().map_err(error::redb)? != receipt_count {
        return Err(error::corruption(
            "application receipt count disagrees with its authoritative table",
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
        phase::APPLICATION_RECEIPTS,
        &receipts,
        "application_receipts",
        |key, bytes| crate::application::decode_receipt(key, bytes).map(|_| ()),
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
            crate::application::validate_proposal_receipt(&receipts, &entry)
        },
    )?;
    context.u64_bytes(
        phase::SECURITY_AUDIT,
        &audit,
        "security_audit",
        |sequence, bytes| crate::application::decode_security_audit(sequence, bytes).map(|_| ()),
    )
}
