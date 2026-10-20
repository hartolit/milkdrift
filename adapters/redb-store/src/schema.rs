use redb::TableDefinition;

pub(crate) const STORAGE_SCHEMA_VERSION: u64 = 21;
pub(crate) const SCHEMA_VERSION_KEY: &str = "storage_schema_version";
pub(crate) const INTERNAL_DOCUMENT_FORMAT_VERSION: u64 = 22;
pub(crate) const INTERNAL_DOCUMENT_FORMAT_VERSION_KEY: &str = "internal_document_format_version";
pub(crate) const CLOCK_WATERMARK_UNIX_MS_KEY: &str = "boundary_clock_high_water_unix_ms";
pub(crate) const SECURITY_AUDIT_NEXT_SEQUENCE_KEY: &str = "security_audit_next_sequence";
pub(crate) const SECURITY_AUDIT_COUNT_KEY: &str = "security_audit_count";
pub(crate) const MANAGED_ACTIVE_COUNT_KEY: &str = "managed_active_count";
pub(crate) const EVALUATION_PENDING_COUNT_KEY: &str = "evaluation_pending_count";
pub(crate) const PEER_EXECUTION_GLOBAL_ACCOUNTING_KEY: &str = "global";

// One declaration owns physical names, types, initialization and exact membership.
// Transaction creation, durable metadata and commit/fault boundaries stay in store::schema.
macro_rules! physical_tables {
    ($($table:ident: $key:ty, $value:ty = $name:literal;)*) => {
        $(pub(crate) const $table: TableDefinition<'static, $key, $value> =
            TableDefinition::new($name);)*

        pub(crate) fn initialize_tables(write: &redb::WriteTransaction) -> Result<(), milkdrift_persistence::PersistenceError> {
            $(drop(write.open_table($table).map_err(crate::error::redb)?);)*
            initialize_daemon_tables(write)?;
            initialize_project_tables(write)?;
            Ok(())
        }

        pub(crate) fn validate_tables(read: &redb::ReadTransaction) -> Result<(), milkdrift_persistence::PersistenceError> {
            validate_table_membership(read, 21)
        }

        pub(crate) fn validate_previous_tables(read: &redb::ReadTransaction) -> Result<(), milkdrift_persistence::PersistenceError> {
            validate_table_membership(read, 19)
        }

        pub(crate) fn validate_daemon_tables(read: &redb::ReadTransaction) -> Result<(), milkdrift_persistence::PersistenceError> {
            validate_table_membership(read, 20)
        }

        fn validate_table_membership(read: &redb::ReadTransaction, version: u64) -> Result<(), milkdrift_persistence::PersistenceError> {
            use redb::TableHandle as _;
            // Typed opens independently reject absent tables and wrong key/value encodings.
            $(drop(read.open_table($table).map_err(crate::error::redb)?);)*
            if version >= 20 {
                drop(read.open_table(DAEMON_METADATA).map_err(crate::error::redb)?);
                drop(read.open_table(DAEMON_RENAME_RECEIPTS).map_err(crate::error::redb)?);
            }
            if version >= 21 {
                drop(read.open_table(PROJECTS).map_err(crate::error::redb)?);
                drop(read.open_table(PROJECT_OPERATIONS).map_err(crate::error::redb)?);
                drop(read.open_table(WORKFLOW_PROJECTS).map_err(crate::error::redb)?);
            }
            for table in read.list_tables().map_err(crate::error::redb)? {
                if !matches!(table.name(), $($name)|*)
                    && !(version >= 20 && matches!(table.name(), DAEMON_METADATA_TABLE | DAEMON_RENAME_RECEIPTS_TABLE))
                    && !(version >= 21 && matches!(table.name(), PROJECTS_TABLE | PROJECT_OPERATIONS_TABLE | WORKFLOW_PROJECTS_TABLE))
                {
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

const PROJECTS_TABLE: &str = "milkdrift.v1.projects.registrations";
const PROJECT_OPERATIONS_TABLE: &str = "milkdrift.v1.projects.operations";
const WORKFLOW_PROJECTS_TABLE: &str = "milkdrift.v1.projects.workflow_associations";
pub(crate) const PROJECTS: TableDefinition<'static, &'static str, &'static [u8]> =
    TableDefinition::new(PROJECTS_TABLE);
pub(crate) const PROJECT_OPERATIONS: TableDefinition<'static, &'static str, &'static [u8]> =
    TableDefinition::new(PROJECT_OPERATIONS_TABLE);
pub(crate) const WORKFLOW_PROJECTS: TableDefinition<'static, &'static str, &'static str> =
    TableDefinition::new(WORKFLOW_PROJECTS_TABLE);

pub(crate) fn initialize_project_tables(
    write: &redb::WriteTransaction,
) -> Result<(), milkdrift_persistence::PersistenceError> {
    drop(write.open_table(PROJECTS).map_err(crate::error::redb)?);
    drop(
        write
            .open_table(PROJECT_OPERATIONS)
            .map_err(crate::error::redb)?,
    );
    drop(
        write
            .open_table(WORKFLOW_PROJECTS)
            .map_err(crate::error::redb)?,
    );
    Ok(())
}

const DAEMON_METADATA_TABLE: &str = "milkdrift.v1.daemon.metadata";
const DAEMON_RENAME_RECEIPTS_TABLE: &str = "milkdrift.v1.daemon.rename_receipts";
pub(crate) const DAEMON_METADATA: TableDefinition<'static, &'static str, &'static [u8]> =
    TableDefinition::new(DAEMON_METADATA_TABLE);
pub(crate) const DAEMON_RENAME_RECEIPTS: TableDefinition<'static, &'static str, &'static [u8]> =
    TableDefinition::new(DAEMON_RENAME_RECEIPTS_TABLE);

pub(crate) fn initialize_daemon_tables(
    write: &redb::WriteTransaction,
) -> Result<(), milkdrift_persistence::PersistenceError> {
    drop(
        write
            .open_table(DAEMON_METADATA)
            .map_err(crate::error::redb)?,
    );
    drop(
        write
            .open_table(DAEMON_RENAME_RECEIPTS)
            .map_err(crate::error::redb)?,
    );
    Ok(())
}

physical_tables! {
TASK_WORKFLOWS: &'static str, &'static [u8] = "milkdrift.v1.tasks.workflows";
TASK_RECEIPTS: &'static str, &'static [u8] = "milkdrift.v1.tasks.receipts";
TASK_ACCOUNTING: &'static str, &'static [u8] = "milkdrift.v1.tasks.accounting";
TASK_ACTION_OWNERS: &'static str, &'static str = "milkdrift.v1.tasks.action_owners";
TASK_OUTPUT_USAGE: &'static str, &'static [u8] = "milkdrift.v1.tasks.output_usage";
NATIVE_PATH_CLAIMS: &'static str, &'static [u8] = "milkdrift.v1.host.native_path_claims";
// Resource authority outlives serving observation compaction and invocation leases.
MANAGED_INSTALLATIONS: &'static str, &'static [u8] = "milkdrift.v1.managed.installations";
MANAGED_ACTIVE_INSTALLATIONS: &'static str, u64 = "milkdrift.v1.managed.active_installations";
MANAGED_RECEIPTS: &'static [u8], &'static [u8] = "milkdrift.v1.managed.receipts";
MANAGED_EVALUATIONS: &'static str, &'static [u8] = "milkdrift.v1.managed.evaluations";
MANAGED_PENDING_EVALUATIONS: &'static str, u64 = "milkdrift.v1.managed.pending_evaluations";
MANAGED_TRANSITIONS: &'static str, &'static [u8] = "milkdrift.v1.managed.transitions";
MANAGED_USES: &'static str, &'static str = "milkdrift.v1.managed.uses";
// Every durable family has a distinct, permanently named table. Keys that need
// ordering use the closed binary encodings in `codec`; documents are canonical
// JSON owned by the inward contracts.
// Physical-format markers and optimistic aggregate revisions.
METADATA: &'static str, u64 = "milkdrift.v1.metadata";
// Installation identity is independent of role, configured callers and process boot.
SERVING_HOST_IDENTITY: &'static str, &'static str = "milkdrift.v1.serving.host_identity";
// Independently retained protected-operation audit.
SECURITY_AUDIT: u64, &'static [u8] = "milkdrift.v1.application.security_audit";
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
ARTIFACT_OWNERSHIP: &'static [u8], &'static [u8] = "milkdrift.v1.artifacts.ownership_by_run";
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
