#![forbid(unsafe_code)]
#![warn(missing_docs)]

//! Production local durable storage for Milkdrift.
//!
//! The adapter keeps redb and filesystem details behind the narrow persistence
//! ports. All command acceptance facts share one immediate-durability redb write
//! transaction; artifact content is synchronized before its metadata can commit.

mod admin;
mod artifact;
mod codec;
mod error;
mod fault;
mod journal;
mod json;
mod revision;
mod schema;
mod snapshot;
mod store;

pub use fault::{FaultInjector, FaultPoint, injected_failure};
pub use store::{RedbStore, RedbStoreConfig};
