#![forbid(unsafe_code)]
#![warn(missing_docs)]

//! Reproduce product behavior and measure selected paths with controlled evidence scenarios.
//!
//! The binaries exercise actual daemon/CLI use, local or external model/process scenarios,
//! mutation campaigns, and operational load. Library functions supply the shared measurements
//! and [`application`] child lifecycle. Product crates remain the owners of configuration,
//! scheduling, storage, and authority; none depends on this development package.
//!
//! [`ScenarioMeasurement`] records work and a result checksum, while [`StorageEvidence`] and
//! [`DaemonEvidence`] report the observations needed to interpret turnover and recovery. Timing
//! is not a correctness budget. Deterministic fixtures and a model-only smoke run cannot qualify
//! combined real-agent/model interoperability; that requires the external runner's strict report.
//! Keep reports and sensitive scenario state in ignored `target/` or private external directories.

mod adapters;
pub mod application;
mod context;
mod daemon;
/// Controlled loopback endpoint framing for development evidence.
pub mod http_fixture;
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
