use std::{
    fmt, fs,
    fs::File,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

use milkdrift_persistence::{PersistenceError, TimestampMillis};
use redb::Database;

use crate::{
    error,
    fault::{FaultInjector, no_faults},
    schema::{
        ARTIFACT_ACCOUNTING, ARTIFACT_DIGEST_RESERVATIONS, ARTIFACT_MANIFEST, ARTIFACT_METADATA,
        ARTIFACT_PUBLICATIONS, ARTIFACT_PUBLICATIONS_BY_AGE, ARTIFACT_REFERENCES,
        ARTIFACT_RESERVATIONS, ARTIFACT_TEMP_MANIFEST, ARTIFACT_TEMP_OWNERS, ARTIFACTS_BY_DIGEST,
        COMMAND_RESULTS, EVENT_CHECKSUMS, EVENT_HISTORY_DIGESTS, INTERNAL_DOCUMENT_FORMAT_VERSION,
        INTERNAL_DOCUMENT_FORMAT_VERSION_KEY, LEASE_ENTRIES, LEASE_INDEX, METADATA,
        NONTERMINAL_RUNS, REVISIONS, REVISIONS_BY_DIGEST, ROOT_SCOPES, RUN_ARTIFACT_OWNERSHIP,
        RUN_EVENTS, RUN_HEADS, RUN_HISTORY_ACCUMULATORS, RUN_SUMMARIES, RUNNABLE_ENTRIES,
        RUNNABLE_INDEX, RUNNABLE_RUN_HEADS, SCHEMA_VERSION_KEY, SCOPES, SNAPSHOT_LATEST, SNAPSHOTS,
        STORAGE_SCHEMA_VERSION, TIMER_ENTRIES, TIMER_INDEX, VALUES, WORKSPACE_BUDGETS,
        WORKSPACE_USAGE, WORKSPACE_VALUE_HEADS,
    },
};

mod config;
mod filesystem;
mod open;
mod schema;

pub(crate) use config::{ARTIFACT_DIRECTORY, DATABASE_FILENAME, TEMP_DIRECTORY};
pub use config::{ArtifactClock, RedbStore, RedbStoreConfig, SystemArtifactClock};
pub(crate) use filesystem::{
    ensure_regular_file_or_absent, prepare_owned_directory, sync_owned_directory,
};
pub(crate) use schema::{initialize_schema, validate_schema};
