#![forbid(unsafe_code)]
#![warn(missing_docs)]

//! Persist Milkdrift's accepted facts in a local redb store and owned artifact files.
//!
//! Open one [`RedbStore`] and supply it through the narrow `milkdrift-persistence` traits.
//! [`RedbStoreConfig`] selects storage/read bounds, retention, and the clock used for
//! fresh observations. Runtime recovery is a separate step after storage opens.
//!
//! Journal commands commit their receipt, events, workspace/account changes, and indexes
//! together. Artifacts publish verified content before metadata acceptance; replay and
//! cleanup handle interrupted publication. Application receipt archival and peer
//! tombstones preserve their respective replay contracts. Use the persistence admin port
//! for explicit historical verification beyond the ordinary bounded health sample.

mod admin;
mod application;
mod artifact;
mod clock;
mod codec;
mod controller_account;
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
pub use store::{RedbStore, RedbStoreConfig, StoreClock, SystemStoreClock};
