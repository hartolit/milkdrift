use std::{fmt, fs, fs::File, path::Path, path::PathBuf, sync::Arc, sync::Mutex};

use milkdrift_persistence::{PersistenceError, TimestampMillis};
use redb::Database;

use crate::{error, fault::FaultInjector, fault::no_faults};

mod config;
mod filesystem;
mod open;
mod schema;

pub(crate) use config::{ARTIFACT_DIRECTORY, DATABASE_FILENAME, TEMP_DIRECTORY};
pub use config::{RedbStore, RedbStoreConfig, StoreClock, SystemStoreClock};
pub(crate) use filesystem::{
    ensure_regular_file_or_absent, prepare_owned_directory, sync_owned_directory,
};
pub(crate) use schema::{initialize_schema, validate_schema};
