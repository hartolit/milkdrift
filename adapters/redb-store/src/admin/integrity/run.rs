use super::super::{
    COMMAND_RESULTS, IndexedRunState, METADATA, NONTERMINAL_RUNS, PersistenceError, RUN_EVENTS,
    RUN_HEADS, RUN_SUMMARIES, RunId, SIGNAL_RECEIPTS, SignalId, WORKSPACE_BUDGETS, WORKSPACE_USAGE,
    WorkspaceBudget, WorkspaceUsage, codec, error, json,
};
use super::{ScanContext, phase};

pub(super) fn scan_core(context: &mut ScanContext<'_, '_>) -> Result<(), PersistenceError> {
    let read = context.read;
    let heads = read.open_table(RUN_HEADS).map_err(error::redb)?;
    let events = read.open_table(RUN_EVENTS).map_err(error::redb)?;
    let summaries = read.open_table(RUN_SUMMARIES).map_err(error::redb)?;
    let nonterminal = read.open_table(NONTERMINAL_RUNS).map_err(error::redb)?;
    let commands = read.open_table(COMMAND_RESULTS).map_err(error::redb)?;
    let usage = read.open_table(WORKSPACE_USAGE).map_err(error::redb)?;
    let budgets = read.open_table(WORKSPACE_BUDGETS).map_err(error::redb)?;

    context.string_u64(phase::HEADS, &heads, "run_indexes", |key, stored_head| {
        let run = RunId::new(key)
            .map_err(|cause| error::corruption(format!("invalid run-head identity: {cause}")))?;
        let head = crate::journal::validated_run_head(&heads, &events, &run)?;
        crate::snapshot::validate_history_head(read, &run, head)?;
        if head.get() != stored_head {
            return Err(error::corruption(
                "run-head key changed within one read transaction",
            ));
        }
        let summary_bytes = summaries
            .get(key)
            .map_err(error::redb)?
            .ok_or_else(|| error::corruption("run head is missing its summary"))?;
        let summary: milkdrift_persistence::RunSummaryIndex =
            json::decode(summary_bytes.value(), "run summary")?;
        if summary.run != run || summary.through_sequence != head {
            return Err(error::corruption(
                "run summary does not match its authoritative head",
            ));
        }
        validate_nonterminal_marker(&nonterminal, &summary)?;
        let usage_bytes = usage
            .get(key)
            .map_err(error::redb)?
            .ok_or_else(|| error::corruption("run head is missing workspace usage"))?;
        let budget_bytes = budgets
            .get(key)
            .map_err(error::redb)?
            .ok_or_else(|| error::corruption("run head is missing its workspace budget"))?;
        let usage: WorkspaceUsage = json::decode(usage_bytes.value(), "workspace usage")?;
        let _budget: WorkspaceBudget = json::decode(budget_bytes.value(), "workspace budget")?;
        let stored_usage = crate::journal::validated_workspace_domain(read, &run)?
            .ok_or_else(|| error::corruption("run head has no workspace domain"))?;
        if stored_usage != usage {
            return Err(error::corruption(
                "run head workspace usage disagrees with its durable domain",
            ));
        }
        Ok(())
    })?;
    context.string_bytes(phase::SUMMARIES, &summaries, "run_indexes", |key, bytes| {
        let summary: milkdrift_persistence::RunSummaryIndex = json::decode(bytes, "run summary")?;
        if summary.run.as_str() != key {
            return Err(error::corruption(
                "run-summary key does not match its document",
            ));
        }
        let head = crate::journal::validated_run_head(&heads, &events, &summary.run)?;
        if heads.get(key).map_err(error::redb)?.is_none() || head != summary.through_sequence {
            return Err(error::corruption(
                "run summary does not match an authoritative head",
            ));
        }
        validate_nonterminal_marker(&nonterminal, &summary)
    })?;
    context.string_u8(
        phase::NONTERMINAL,
        &nonterminal,
        "run_indexes",
        |key, marker| {
            if marker != 1 {
                return Err(error::corruption(
                    "nonterminal index contains an invalid marker",
                ));
            }
            let run = RunId::new(key).map_err(|cause| {
                error::corruption(format!("invalid nonterminal run identity: {cause}"))
            })?;
            let _head = crate::journal::validated_run_head(&heads, &events, &run)?;
            if heads.get(key).map_err(error::redb)?.is_none() {
                return Err(error::corruption(
                    "nonterminal index names a run without an authoritative head",
                ));
            }
            let bytes = summaries
                .get(key)
                .map_err(error::redb)?
                .ok_or_else(|| error::corruption("nonterminal index is missing its summary"))?;
            let summary: milkdrift_persistence::RunSummaryIndex =
                json::decode(bytes.value(), "run summary")?;
            if summary.run != run || summary.state == IndexedRunState::Terminal {
                return Err(error::corruption(
                    "nonterminal index disagrees with its run summary",
                ));
            }
            Ok(())
        },
    )?;
    context.binary_bytes(
        phase::COMMANDS,
        &commands,
        "command_indexes",
        |key, bytes| crate::journal::validate_stored_command_record(key, bytes, &heads, &events),
    )
}

pub(super) fn scan_signal_receipts(
    context: &mut ScanContext<'_, '_>,
) -> Result<(), PersistenceError> {
    let read = context.read;
    let signal_receipts = read.open_table(SIGNAL_RECEIPTS).map_err(error::redb)?;
    context.binary_u64(
        phase::SIGNAL_RECEIPTS,
        &signal_receipts,
        "signal_indexes",
        |key, sequence| {
            let components = codec::decode_components(key, 2)?;
            let run = RunId::new(components[0]).map_err(|cause| {
                error::corruption(format!("invalid signal-receipt run identity: {cause}"))
            })?;
            let signal = SignalId::new(components[1]).map_err(|cause| {
                error::corruption(format!("invalid signal-receipt identity: {cause}"))
            })?;
            crate::journal::validate_signal_receipt_row(read, &run, &signal, sequence).map(|_| ())
        },
    )
}

pub(super) fn scan_invocation_facts(
    context: &mut ScanContext<'_, '_>,
) -> Result<(), PersistenceError> {
    let read = context.read;
    let metadata = read.open_table(METADATA).map_err(error::redb)?;
    metadata
        .get(crate::schema::CLOCK_WATERMARK_UNIX_MS_KEY)
        .map_err(error::redb)?
        .ok_or_else(|| error::corruption("boundary-clock high-water evidence is missing"))?;
    context.string_u64(
        phase::INVOCATION_FACTS,
        &metadata,
        "invocation_indexes",
        |key, sequence| {
            if matches!(
                key,
                crate::schema::SCHEMA_VERSION_KEY
                    | crate::schema::INTERNAL_DOCUMENT_FORMAT_VERSION_KEY
                    | crate::schema::CLOCK_WATERMARK_UNIX_MS_KEY
                    | crate::schema::LEASE_SET_REVISION_KEY
                    | crate::schema::NONTERMINAL_SET_COUNT_KEY
                    | crate::schema::APPLICATION_HOT_RECEIPT_COUNT_KEY
                    | crate::schema::APPLICATION_COLD_RECEIPT_COUNT_KEY
                    | crate::schema::APPLICATION_RECEIPT_ARCHIVE_GENERATION_KEY
                    | crate::schema::APPLICATION_RECEIPT_LAST_ARCHIVED_AT_KEY
                    | crate::schema::SECURITY_AUDIT_NEXT_SEQUENCE_KEY
                    | crate::schema::SECURITY_AUDIT_COUNT_KEY
            ) {
                Ok(())
            } else {
                crate::journal::validate_invocation_fact_row(read, key, sequence)
            }
        },
    )
}

fn validate_nonterminal_marker(
    index: &impl redb::ReadableTable<&'static str, u8>,
    summary: &milkdrift_persistence::RunSummaryIndex,
) -> Result<(), PersistenceError> {
    let marker = index
        .get(summary.run.as_str())
        .map_err(error::redb)?
        .map(|value| value.value());
    match (summary.state, marker) {
        (IndexedRunState::Terminal, None) | (_, Some(1)) => Ok(()),
        (IndexedRunState::Terminal, Some(_)) => Err(error::corruption(
            "terminal run is present in the nonterminal index",
        )),
        (_, None) => Err(error::corruption(
            "nonterminal run is missing from its discovery index",
        )),
        (_, Some(_)) => Err(error::corruption(
            "nonterminal index contains an invalid marker",
        )),
    }
}
