//! Bounded read-only inspection of task, artifact and independent host records.
use crate::{RedbStore, codec, error, json, schema::*};
use milkdrift_persistence::{
    ArtifactPublicationId, BoundedDetail, IntegrityScanCursor, IntegrityScanFamily,
    IntegrityScanRequest, IntegrityScanResult, PersistenceError, StorageAdmin,
    StorageComponentHealth, StorageHealth, StorageHealthStatus, StorageSchemaCompatibility,
    StorageSchemaInfo, TimestampMillis,
};
use milkdrift_workspace::{ArtifactMetadata, ArtifactReference};
use std::ops::Bound;
mod cursor;
mod integrity;
mod scan;
mod service;
