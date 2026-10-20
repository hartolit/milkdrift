//! Durable native-directory claims, retained while an accepted writer is uncertain.

use crate::PersistenceError;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// One physical directory selected and verified by capability-host.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NativePathRoot {
    /// Original configured path, checked again before final entry.
    pub requested: PathBuf,
    /// Canonical physical directory path used for overlap checks.
    pub physical: PathBuf,
    /// Host-observed physical directory identity, not a path spelling.
    pub identity: String,
    /// Write claims conflict with either read or write overlap.
    pub write: bool,
}

/// Exact directory selection bound to one durable attempt.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NativePathClaim {
    /// Stable attempt identity; retry cannot silently replace its selection.
    pub owner: String,
    /// Bounded canonical directory claims.
    pub roots: Vec<NativePathRoot>,
}

/// Atomic conflict owner for claims captured by capability-host. Claims have no lease or
/// time-based expiry: absence of a live process does not establish safe release.
pub trait NativePathClaimStore: Send + Sync {
    /// Accept exact replay or insert the complete set, refusing conflicting roots atomically.
    ///
    /// # Errors
    /// Refuses changed replay, overlapping writers, exhausted capacity and storage failures.
    fn claim_native_paths(&self, claim: &NativePathClaim) -> Result<(), PersistenceError>;
    /// Claim a serving invocation's configured directories while its exact pre-entry
    /// dispatch claim is still current. Cancellation cannot recreate a released claim.
    ///
    /// # Errors
    /// Refuses stale dispatch ownership, changed selection, overlapping writers and storage failures.
    fn claim_serving_native_paths(
        &self,
        record: &crate::PeerExecutionRecord,
        claim: &NativePathClaim,
    ) -> Result<(), PersistenceError>;
    /// Return the claim used to recheck physical identities before entry or after restart.
    ///
    /// # Errors
    /// Returns storage failures or malformed identities.
    fn native_path_claim(&self, owner: &str) -> Result<Option<NativePathClaim>, PersistenceError>;
    /// Release only after the owning runtime durably settles or authorizes reconciliation.
    /// This trusted internal operation is not exposed as an ordinary client deletion.
    ///
    /// # Errors
    /// Returns storage failures. Releasing an already released owner is exact replay.
    fn release_native_paths(&self, owner: &str) -> Result<(), PersistenceError>;
}
