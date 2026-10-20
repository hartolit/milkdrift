#![forbid(unsafe_code)]
#![warn(missing_docs)]

//! Persist Milkdrift's accepted facts in a local redb store and owned artifact files.
//!
//! Open one [`RedbStore`] and supply it through the narrow `milkdrift-persistence` traits.
//! [`RedbStoreConfig`] selects storage/read bounds, retention, and the clock used for
//! fresh observations. Runtime recovery is a separate step after storage opens.
//!
//! Task generations commit snapshots, exact receipts, resource holds and capacity together.
//! Artifacts publish verified content before metadata acceptance; replay and cleanup recover
//! interrupted publication. Peer tombstones preserve durable remote acceptance and outcomes.
//! Use the persistence admin port for historical verification beyond bounded health samples.

mod admin;
mod artifact;
mod clock;
mod codec;
mod daemon;
mod error;
mod fault;
mod json;
mod managed;
mod native_paths;
pub mod offline;
mod peer;
mod project;
mod schema;
mod security;
mod store;
mod task;

#[cfg(feature = "test-admin")]
pub mod testing;

#[cfg(feature = "test-admin")]
pub use fault::{FaultInjector, FaultPoint, injected_failure};
pub use store::{RedbStore, RedbStoreConfig, StoreClock, SystemStoreClock};
