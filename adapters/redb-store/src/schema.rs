use redb::TableDefinition;

pub(crate) const STORAGE_SCHEMA_VERSION: u64 = 2;
pub(crate) const SCHEMA_VERSION_KEY: &str = "storage_schema_version";
pub(crate) const INTERNAL_DOCUMENT_FORMAT_VERSION: u64 = 5;
pub(crate) const INTERNAL_DOCUMENT_FORMAT_VERSION_KEY: &str = "internal_document_format_version";
pub(crate) const LEASE_SET_REVISION_KEY: &str = "lease_set_revision";

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
