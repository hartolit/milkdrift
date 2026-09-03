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

// Every durable family has a distinct, permanently named table. Keys that need
// ordering use the closed binary encodings in `codec`; documents are canonical
// JSON owned by the inward contracts.
// Physical-format markers and optimistic aggregate revisions.
pub(crate) const METADATA: TableDefinition<'static, &'static str, u64> =
    TableDefinition::new("milkdrift.v1.metadata");
// Authoritative immutable revision documents plus a derived and verifiable digest index.
pub(crate) const REVISIONS: TableDefinition<'static, &'static str, &'static [u8]> =
    TableDefinition::new("milkdrift.v1.revisions.by_id");
pub(crate) const REVISIONS_BY_DIGEST: TableDefinition<'static, &'static [u8], &'static [u8]> =
    TableDefinition::new("milkdrift.v1.revisions.by_digest_and_id");
// Authoritative journal aggregates, immutable events, cumulative chain checkpoints,
// chain heads, and atomically accepted command results.
pub(crate) const RUN_HEADS: TableDefinition<'static, &'static str, u64> =
    TableDefinition::new("milkdrift.v1.runs.heads");
pub(crate) const RUN_EVENTS: TableDefinition<'static, &'static [u8], &'static [u8]> =
    TableDefinition::new("milkdrift.v1.runs.events");
pub(crate) const EVENT_HISTORY_DIGESTS: TableDefinition<'static, &'static [u8], &'static [u8]> =
    TableDefinition::new("milkdrift.v1.runs.event_history_digests");
pub(crate) const RUN_HISTORY_HEADS: TableDefinition<'static, &'static str, &'static [u8]> =
    TableDefinition::new("milkdrift.v2.runs.history_heads");
pub(crate) const COMMAND_RESULTS: TableDefinition<'static, &'static [u8], &'static [u8]> =
    TableDefinition::new("milkdrift.v1.commands.results");
// Exact-current continuous-controller accounts, immutable run bindings, and transition receipts.
pub(crate) const CONTROLLER_ACCOUNTS: TableDefinition<'static, &'static str, &'static [u8]> =
    TableDefinition::new("milkdrift.v1.controllers.accounts");
pub(crate) const CONTROLLER_ACCOUNT_REVISIONS: TableDefinition<
    'static,
    &'static str,
    &'static [u8],
> = TableDefinition::new("milkdrift.v1.controllers.account_revisions");
pub(crate) const CONTROLLER_RUN_BINDINGS: TableDefinition<'static, &'static str, &'static str> =
    TableDefinition::new("milkdrift.v1.controllers.run_bindings");
pub(crate) const CONTROLLER_TRANSITIONS: TableDefinition<'static, &'static str, &'static [u8]> =
    TableDefinition::new("milkdrift.v1.controllers.transitions");
pub(crate) const CONTROLLER_ARTIFACT_CHARGES: TableDefinition<
    'static,
    &'static str,
    &'static [u8],
> = TableDefinition::new("milkdrift.v1.controllers.artifact_charges");
// Daemon-owned application receipts have exactly one authoritative physical placement.
// The completion index is derived bounded operational state for the hot tier only.
pub(crate) const APPLICATION_COMMAND_RECEIPTS_HOT: TableDefinition<
    'static,
    &'static [u8],
    &'static [u8],
> = TableDefinition::new("milkdrift.v2.application.command_receipts.hot");
pub(crate) const APPLICATION_COMMAND_RECEIPTS_COLD: TableDefinition<
    'static,
    &'static [u8],
    &'static [u8],
> = TableDefinition::new("milkdrift.v2.application.command_receipts.cold");
pub(crate) const APPLICATION_HOT_RECEIPTS_BY_COMPLETION: TableDefinition<
    'static,
    &'static [u8],
    &'static [u8],
> = TableDefinition::new("milkdrift.v2.application.command_receipts.hot_by_completion");
// Presentation layout is authoritative application state but never semantic revision content.
pub(crate) const APPLICATION_LAYOUTS: TableDefinition<'static, &'static [u8], &'static [u8]> =
    TableDefinition::new("milkdrift.v1.application.layouts");
// Rebuildable proposal discovery projection. Exact state remains in control/runtime facts.
pub(crate) const APPLICATION_PROPOSALS: TableDefinition<'static, &'static [u8], &'static [u8]> =
    TableDefinition::new("milkdrift.v1.application.proposals");
// Independently retained protected-operation audit. Receipt retention is never affected.
pub(crate) const SECURITY_AUDIT: TableDefinition<'static, u64, &'static [u8]> =
    TableDefinition::new("milkdrift.v1.application.security_audit");
// Stable signal identities are indexed back to their authoritative receipt event.
pub(crate) const SIGNAL_RECEIPTS: TableDefinition<'static, &'static [u8], u64> =
    TableDefinition::new("milkdrift.v1.runs.signal_receipts");
// Derived and verifiable discoverability/index state. These rows never substitute
// for an absent authoritative event, head, or command result.
pub(crate) const RUN_SUMMARIES: TableDefinition<'static, &'static str, &'static [u8]> =
    TableDefinition::new("milkdrift.v1.discovery.run_summaries");
pub(crate) const NONTERMINAL_RUNS: TableDefinition<'static, &'static str, u8> =
    TableDefinition::new("milkdrift.v1.discovery.nonterminal_runs");
pub(crate) const RUNNABLE_ENTRIES: TableDefinition<'static, &'static [u8], &'static [u8]> =
    TableDefinition::new("milkdrift.v1.discovery.runnable_by_identity");
pub(crate) const RUNNABLE_INDEX: TableDefinition<'static, &'static [u8], &'static [u8]> =
    TableDefinition::new("milkdrift.v1.discovery.runnable");
pub(crate) const RUNNABLE_RUN_HEADS: TableDefinition<'static, &'static str, &'static [u8]> =
    TableDefinition::new("milkdrift.v1.discovery.runnable_run_heads");
pub(crate) const TIMER_ENTRIES: TableDefinition<'static, &'static [u8], &'static [u8]> =
    TableDefinition::new("milkdrift.v1.discovery.timers_by_identity");
pub(crate) const TIMER_INDEX: TableDefinition<'static, &'static [u8], &'static [u8]> =
    TableDefinition::new("milkdrift.v1.discovery.timers");
pub(crate) const LEASE_ENTRIES: TableDefinition<'static, &'static [u8], &'static [u8]> =
    TableDefinition::new("milkdrift.v1.discovery.leases_by_identity");
pub(crate) const LEASE_INDEX: TableDefinition<'static, &'static [u8], &'static [u8]> =
    TableDefinition::new("milkdrift.v1.discovery.leases");
// Optional snapshots and their derived latest pointer. Snapshots may be discarded;
// authoritative events remain sufficient for replay.
pub(crate) const SNAPSHOTS: TableDefinition<'static, &'static [u8], &'static [u8]> =
    TableDefinition::new("milkdrift.v1.snapshots.by_run_and_id");
pub(crate) const SNAPSHOT_LATEST: TableDefinition<'static, &'static str, &'static str> =
    TableDefinition::new("milkdrift.v1.snapshots.latest_by_run");
// Authoritative workspace scope/value documents and aggregate accounting, with
// derived root/value-head lookup indexes.
pub(crate) const SCOPES: TableDefinition<'static, &'static [u8], &'static [u8]> =
    TableDefinition::new("milkdrift.v1.workspace.scopes");
pub(crate) const ROOT_SCOPES: TableDefinition<'static, &'static str, &'static str> =
    TableDefinition::new("milkdrift.v1.workspace.root_scopes");
pub(crate) const VALUES: TableDefinition<'static, &'static [u8], &'static [u8]> =
    TableDefinition::new("milkdrift.v1.workspace.values");
pub(crate) const WORKSPACE_VALUE_HEADS: TableDefinition<'static, &'static [u8], &'static [u8]> =
    TableDefinition::new("milkdrift.v1.workspace.value_heads");
// Authoritative artifact metadata/publication coordination and derived/verifiable
// digest, age, ownership, reference, and temporary-path indexes.
pub(crate) const ARTIFACT_METADATA: TableDefinition<'static, &'static str, &'static [u8]> =
    TableDefinition::new("milkdrift.v1.artifacts.metadata_by_id");
pub(crate) const ARTIFACT_MANIFEST: TableDefinition<'static, &'static str, &'static [u8]> =
    TableDefinition::new("milkdrift.v1.artifacts.authoritative_manifest");
pub(crate) const ARTIFACT_PUBLICATIONS: TableDefinition<'static, &'static str, &'static [u8]> =
    TableDefinition::new("milkdrift.v1.artifacts.publications");
pub(crate) const ARTIFACT_PUBLICATIONS_BY_AGE: TableDefinition<
    'static,
    &'static [u8],
    &'static str,
> = TableDefinition::new("milkdrift.v1.artifacts.writable_by_age");
pub(crate) const ARTIFACT_RESERVATIONS: TableDefinition<'static, &'static str, &'static str> =
    TableDefinition::new("milkdrift.v1.artifacts.reservations_by_run");
pub(crate) const ARTIFACT_TEMP_OWNERS: TableDefinition<'static, &'static str, &'static str> =
    TableDefinition::new("milkdrift.v1.artifacts.temp_owners");
pub(crate) const ARTIFACT_TEMP_MANIFEST: TableDefinition<'static, &'static str, &'static [u8]> =
    TableDefinition::new("milkdrift.v1.artifacts.temporary_manifest");
pub(crate) const ARTIFACT_PATHS: TableDefinition<'static, &'static [u8], &'static [u8]> =
    TableDefinition::new("milkdrift.v2.artifacts.path_inventory");
pub(crate) const ARTIFACT_DELETE_GUARDS: TableDefinition<'static, &'static [u8], u8> =
    TableDefinition::new("milkdrift.v2.artifacts.delete_guards");
pub(crate) const ARTIFACT_DIGEST_RESERVATIONS: TableDefinition<'static, &'static [u8], u8> =
    TableDefinition::new("milkdrift.v1.artifacts.reservations_by_digest");
pub(crate) const ARTIFACTS_BY_DIGEST: TableDefinition<'static, &'static [u8], &'static [u8]> =
    TableDefinition::new("milkdrift.v1.artifacts.by_digest_and_id");
// Derived occurrence index plus authoritative per-run membership/accounting evidence.
pub(crate) const ARTIFACT_REFERENCES: TableDefinition<'static, &'static [u8], &'static [u8]> =
    TableDefinition::new("milkdrift.v1.artifacts.references");
pub(crate) const RUN_ARTIFACT_OWNERSHIP: TableDefinition<'static, &'static [u8], &'static [u8]> =
    TableDefinition::new("milkdrift.v1.artifacts.ownership_by_run");
pub(crate) const ARTIFACT_ACCOUNTING: TableDefinition<'static, &'static str, &'static [u8]> =
    TableDefinition::new("milkdrift.v1.artifacts.accounting");
pub(crate) const WORKSPACE_USAGE: TableDefinition<'static, &'static str, &'static [u8]> =
    TableDefinition::new("milkdrift.v1.workspace.usage");
pub(crate) const WORKSPACE_BUDGETS: TableDefinition<'static, &'static str, &'static [u8]> =
    TableDefinition::new("milkdrift.v1.workspace.budgets");

// Serving-peer durable acceptance, queue ownership, append-only observations and retention.
pub(crate) const PEER_RELATIONSHIPS: TableDefinition<'static, &'static str, &'static [u8]> =
    TableDefinition::new("milkdrift.v1.peers.relationships");
pub(crate) const PEER_CATALOGS: TableDefinition<'static, &'static str, &'static [u8]> =
    TableDefinition::new("milkdrift.v1.peers.catalogs");
pub(crate) const PEER_EXECUTIONS: TableDefinition<'static, &'static str, &'static [u8]> =
    TableDefinition::new("milkdrift.v2.peers.executions.hot");
pub(crate) const PEER_EXECUTION_TOMBSTONES: TableDefinition<'static, &'static str, &'static [u8]> =
    TableDefinition::new("milkdrift.v2.peers.executions.tombstones");
pub(crate) const PEER_EXECUTION_LOCATIONS: TableDefinition<'static, &'static str, u8> =
    TableDefinition::new("milkdrift.v2.peers.executions.locations");
pub(crate) const PEER_EXECUTIONS_BY_REQUEST: TableDefinition<'static, &'static [u8], &'static str> =
    TableDefinition::new("milkdrift.v2.peers.executions_by_request");
pub(crate) const PEER_OBSERVATIONS: TableDefinition<'static, &'static [u8], &'static [u8]> =
    TableDefinition::new("milkdrift.v2.peers.observations.hot");
pub(crate) const PEER_OBSERVATION_ARTIFACTS: TableDefinition<
    'static,
    &'static [u8],
    &'static [u8],
> = TableDefinition::new("milkdrift.v2.peers.observation_artifacts.hot");
pub(crate) const PEER_DISPATCH_AVAILABLE: TableDefinition<'static, &'static [u8], &'static str> =
    TableDefinition::new("milkdrift.v2.peers.dispatch_available");
pub(crate) const PEER_ACTIVE_CLAIMS: TableDefinition<'static, &'static [u8], &'static str> =
    TableDefinition::new("milkdrift.v2.peers.active_claims");
pub(crate) const PEER_TERMINAL_INDEX: TableDefinition<'static, &'static [u8], &'static str> =
    TableDefinition::new("milkdrift.v2.peers.hot_terminal_retention");
pub(crate) const PEER_EXECUTION_ACCOUNTING: TableDefinition<'static, &'static str, &'static [u8]> =
    TableDefinition::new("milkdrift.v2.peers.accounting");
