#![forbid(unsafe_code)]
#![warn(missing_docs)]

//! Production local durable storage for Milkdrift.
//!
//! The adapter keeps redb and filesystem details behind the narrow persistence
//! ports. All command acceptance facts share one immediate-durability redb write
//! transaction; artifact content is synchronized before its metadata can commit.

mod admin;
mod application;
mod artifact;
mod codec;
mod error;
mod fault;
mod journal;
mod json;
mod peer;
mod revision;
mod schema;
mod snapshot;
mod store;

#[cfg(feature = "test-admin")]
pub mod testing;

#[cfg(feature = "test-admin")]
pub use fault::{FaultInjector, FaultPoint, injected_failure};
pub use store::{ArtifactClock, RedbStore, RedbStoreConfig, SystemArtifactClock};
