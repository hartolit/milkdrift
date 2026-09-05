use redb::TableDefinition;

pub(crate) const STORAGE_SCHEMA_VERSION: u64 = 11;
pub(crate) const SCHEMA_VERSION_KEY: &str = "storage_schema_version";
pub(crate) const INTERNAL_DOCUMENT_FORMAT_VERSION: u64 = 14;
pub(crate) const INTERNAL_DOCUMENT_FORMAT_VERSION_KEY: &str = "internal_document_format_version";
pub(crate) const CLOCK_WATERMARK_UNIX_MS_KEY: &str = "boundary_clock_high_water_unix_ms";
pub(crate) const LEASE_SET_REVISION_KEY: &str = "lease_set_revision";
pub(crate) const NONTERMINAL_SET_COUNT_KEY: &str = "nonterminal_set_count";
pub(crate) const APPLICATION_HOT_RECEIPT_COUNT_KEY: &str = "application_hot_receipt_count";
pub(crate) const APPLICATION_COLD_RECEIPT_COUNT_KEY: &str = "application_cold_receipt_count";
pub(crate) const APPLICATION_RECEIPT_ARCHIVE_GENERATION_KEY: &str =
    "application_receipt_archive_generation";
pub(crate) const APPLICATION_RECEIPT_LAST_ARCHIVED_AT_KEY: &str =
    "application_receipt_last_archived_at";
pub(crate) const SECURITY_AUDIT_NEXT_SEQUENCE_KEY: &str = "security_audit_next_sequence";
pub(crate) const SECURITY_AUDIT_COUNT_KEY: &str = "security_audit_count";
pub(crate) const PEER_EXECUTION_GLOBAL_ACCOUNTING_KEY: &str = "global";

// One declaration owns physical names, types, initialization and exact membership.
// Transaction creation, durable metadata and commit/fault boundaries stay in store::schema.
macro_rules! physical_tables {
    ($($table:ident: $key:ty, $value:ty = $name:literal;)*) => {
        $(pub(crate) const $table: TableDefinition<'static, $key, $value> =
            TableDefinition::new($name);)*

        pub(crate) fn initialize_tables(write: &redb::WriteTransaction) -> Result<(), milkdrift_persistence::PersistenceError> {
            $(drop(write.open_table($table).map_err(crate::error::redb)?);)*
            Ok(())
        }

        pub(crate) fn validate_tables(read: &redb::ReadTransaction) -> Result<(), milkdrift_persistence::PersistenceError> {
            use redb::TableHandle as _;
            // Typed opens independently reject absent tables and wrong key/value encodings.
            $(drop(read.open_table($table).map_err(crate::error::redb)?);)*
            for table in read.list_tables().map_err(crate::error::redb)? {
                if !matches!(table.name(), $($name)|*) {
                    return Err(crate::error::corruption("unexpected physical table"));
                }
            }
            if read.list_multimap_tables().map_err(crate::error::redb)?.next().is_some() {
                return Err(crate::error::corruption("unexpected physical multimap table"));
            }
            Ok(())
        }
    };
}

physical_tables! {
// Every durable family has a distinct, permanently named table. Keys that need
// ordering use the closed binary encodings in `codec`; documents are canonical
// JSON owned by the inward contracts.
// Physical-format markers and optimistic aggregate revisions.
METADATA: &'static str, u64 = "milkdrift.v1.metadata";
// Authoritative immutable revision documents plus a derived and verifiable digest index.
REVISIONS: &'static str, &'static [u8] = "milkdrift.v1.revisions.by_id";
REVISIONS_BY_DIGEST: &'static [u8], &'static [u8] = "milkdrift.v1.revisions.by_digest_and_id";
// Authoritative journal aggregates, immutable events, cumulative chain checkpoints,
// chain heads, and atomically accepted command results.
RUN_HEADS: &'static str, u64 = "milkdrift.v1.runs.heads";
RUN_EVENTS: &'static [u8], &'static [u8] = "milkdrift.v1.runs.events";
EVENT_HISTORY_DIGESTS: &'static [u8], &'static [u8] = "milkdrift.v1.runs.event_history_digests";
RUN_HISTORY_HEADS: &'static str, &'static [u8] = "milkdrift.v2.runs.history_heads";
COMMAND_RESULTS: &'static [u8], &'static [u8] = "milkdrift.v1.commands.results";
// Exact-current continuous-controller accounts, immutable run bindings, and transition receipts.
CONTROLLER_ACCOUNTS: &'static str, &'static [u8] = "milkdrift.v1.controllers.accounts";
CONTROLLER_ACCOUNT_REVISIONS: &'static str, &'static [u8] = "milkdrift.v1.controllers.account_revisions";
CONTROLLER_RUN_BINDINGS: &'static str, &'static str = "milkdrift.v1.controllers.run_bindings";
CONTROLLER_TRANSITIONS: &'static str, &'static [u8] = "milkdrift.v1.controllers.transitions";
CONTROLLER_ARTIFACT_CHARGES: &'static str, &'static [u8] = "milkdrift.v1.controllers.artifact_charges";
// Daemon-owned application receipts have exactly one authoritative physical placement.
// The completion index is derived bounded operational state for the hot tier only.
APPLICATION_COMMAND_RECEIPTS_HOT: &'static [u8], &'static [u8] = "milkdrift.v2.application.command_receipts.hot";
APPLICATION_COMMAND_RECEIPTS_COLD: &'static [u8], &'static [u8] = "milkdrift.v2.application.command_receipts.cold";
APPLICATION_HOT_RECEIPTS_BY_COMPLETION: &'static [u8], &'static [u8] = "milkdrift.v2.application.command_receipts.hot_by_completion";
// Presentation layout is authoritative application state but never semantic revision content.
APPLICATION_LAYOUTS: &'static [u8], &'static [u8] = "milkdrift.v1.application.layouts";
// Rebuildable proposal discovery projection. Exact state remains in control/runtime facts.
APPLICATION_PROPOSALS: &'static [u8], &'static [u8] = "milkdrift.v1.application.proposals";
// Independently retained protected-operation audit. Receipt retention is never affected.
SECURITY_AUDIT: u64, &'static [u8] = "milkdrift.v1.application.security_audit";
// Stable signal identities are indexed back to their authoritative receipt event.
SIGNAL_RECEIPTS: &'static [u8], u64 = "milkdrift.v1.runs.signal_receipts";
// Derived and verifiable discoverability/index state. These rows never substitute
// for an absent authoritative event, head, or command result.
RUN_SUMMARIES: &'static str, &'static [u8] = "milkdrift.v1.discovery.run_summaries";
NONTERMINAL_RUNS: &'static str, u8 = "milkdrift.v1.discovery.nonterminal_runs";
RUNNABLE_ENTRIES: &'static [u8], &'static [u8] = "milkdrift.v1.discovery.runnable_by_identity";
RUNNABLE_INDEX: &'static [u8], &'static [u8] = "milkdrift.v1.discovery.runnable";
RUNNABLE_RUN_HEADS: &'static str, &'static [u8] = "milkdrift.v1.discovery.runnable_run_heads";
TIMER_ENTRIES: &'static [u8], &'static [u8] = "milkdrift.v1.discovery.timers_by_identity";
TIMER_INDEX: &'static [u8], &'static [u8] = "milkdrift.v1.discovery.timers";
LEASE_ENTRIES: &'static [u8], &'static [u8] = "milkdrift.v1.discovery.leases_by_identity";
LEASE_INDEX: &'static [u8], &'static [u8] = "milkdrift.v1.discovery.leases";
// Optional snapshots and their derived latest pointer. Snapshots may be discarded;
// authoritative events remain sufficient for replay.
SNAPSHOTS: &'static [u8], &'static [u8] = "milkdrift.v1.snapshots.by_run_and_id";
SNAPSHOT_LATEST: &'static str, &'static str = "milkdrift.v1.snapshots.latest_by_run";
// Authoritative workspace scope/value documents and aggregate accounting, with
// derived root/value-head lookup indexes.
SCOPES: &'static [u8], &'static [u8] = "milkdrift.v1.workspace.scopes";
ROOT_SCOPES: &'static str, &'static str = "milkdrift.v1.workspace.root_scopes";
VALUES: &'static [u8], &'static [u8] = "milkdrift.v1.workspace.values";
WORKSPACE_VALUE_HEADS: &'static [u8], &'static [u8] = "milkdrift.v1.workspace.value_heads";
// Authoritative artifact metadata/publication coordination and derived/verifiable
// digest, age, ownership, reference, and temporary-path indexes.
ARTIFACT_METADATA: &'static str, &'static [u8] = "milkdrift.v1.artifacts.metadata_by_id";
ARTIFACT_MANIFEST: &'static str, &'static [u8] = "milkdrift.v1.artifacts.authoritative_manifest";
ARTIFACT_PUBLICATIONS: &'static str, &'static [u8] = "milkdrift.v1.artifacts.publications";
ARTIFACT_PUBLICATIONS_BY_AGE: &'static [u8], &'static str = "milkdrift.v1.artifacts.writable_by_age";
ARTIFACT_RESERVATIONS: &'static str, &'static str = "milkdrift.v1.artifacts.reservations_by_run";
ARTIFACT_TEMP_OWNERS: &'static str, &'static str = "milkdrift.v1.artifacts.temp_owners";
ARTIFACT_TEMP_MANIFEST: &'static str, &'static [u8] = "milkdrift.v1.artifacts.temporary_manifest";
ARTIFACT_PATHS: &'static [u8], &'static [u8] = "milkdrift.v2.artifacts.path_inventory";
ARTIFACT_DELETE_GUARDS: &'static [u8], u8 = "milkdrift.v2.artifacts.delete_guards";
ARTIFACT_DIGEST_RESERVATIONS: &'static [u8], u8 = "milkdrift.v1.artifacts.reservations_by_digest";
ARTIFACTS_BY_DIGEST: &'static [u8], &'static [u8] = "milkdrift.v1.artifacts.by_digest_and_id";
// Derived occurrence index plus authoritative per-run membership/accounting evidence.
ARTIFACT_REFERENCES: &'static [u8], &'static [u8] = "milkdrift.v1.artifacts.references";
RUN_ARTIFACT_OWNERSHIP: &'static [u8], &'static [u8] = "milkdrift.v1.artifacts.ownership_by_run";
ARTIFACT_ACCOUNTING: &'static str, &'static [u8] = "milkdrift.v1.artifacts.accounting";
WORKSPACE_USAGE: &'static str, &'static [u8] = "milkdrift.v1.workspace.usage";
WORKSPACE_BUDGETS: &'static str, &'static [u8] = "milkdrift.v1.workspace.budgets";

// Serving-peer durable acceptance, queue ownership, append-only observations and retention.
PEER_RELATIONSHIPS: &'static str, &'static [u8] = "milkdrift.v1.peers.relationships";
PEER_CATALOGS: &'static str, &'static [u8] = "milkdrift.v1.peers.catalogs";
PEER_EXECUTIONS: &'static str, &'static [u8] = "milkdrift.v2.peers.executions.hot";
PEER_EXECUTION_TOMBSTONES: &'static str, &'static [u8] = "milkdrift.v2.peers.executions.tombstones";
PEER_EXECUTION_LOCATIONS: &'static str, u8 = "milkdrift.v2.peers.executions.locations";
PEER_EXECUTIONS_BY_REQUEST: &'static [u8], &'static str = "milkdrift.v2.peers.executions_by_request";
PEER_OBSERVATIONS: &'static [u8], &'static [u8] = "milkdrift.v2.peers.observations.hot";
PEER_OBSERVATION_ARTIFACTS: &'static [u8], &'static [u8] = "milkdrift.v2.peers.observation_artifacts.hot";
PEER_DISPATCH_AVAILABLE: &'static [u8], &'static str = "milkdrift.v2.peers.dispatch_available";
PEER_ACTIVE_CLAIMS: &'static [u8], &'static str = "milkdrift.v2.peers.active_claims";
PEER_TERMINAL_INDEX: &'static [u8], &'static str = "milkdrift.v2.peers.hot_terminal_retention";
PEER_EXECUTION_ACCOUNTING: &'static str, &'static [u8] = "milkdrift.v2.peers.accounting";
}

#[cfg(test)]
mod tests;
