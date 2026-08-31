#![forbid(unsafe_code)]
#![warn(missing_docs)]

//! Development-only repeatable performance and operational evidence for Milkdrift.
//!
//! This package deliberately depends outward on product crates. No semantic crate
//! depends on this harness, and its reports live under `target/evidence`.

mod adapters;
mod context;
mod daemon;
mod peer;
mod persistence;
mod report;

pub use adapters::{artifact_range_read, local_process_stream_drain, model_stream_parsers};
pub use context::{context_discovery_and_selection, context_materialization};
pub use daemon::{DaemonEvidence, daemon_owner_round_trip, measure_daemon_saturation};
pub use persistence::{
    StorageEvidence, application_receipt_paths, artifact_publication, journal_append_batch,
    journal_append_one, measure_storage_growth, peer_observation_paths, projection_rebuild,
    projection_snapshot_tail,
};
pub use report::{EvidenceError, EvidenceResult, LatencySummary, ScenarioMeasurement};

/// Default number of operations used by the release-mode operational lane.
pub const DEFAULT_OPERATION_COUNT: u32 = 256;
