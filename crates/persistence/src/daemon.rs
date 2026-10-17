//! The installation's shared name and exact recovery of accepted rename commands.

use milkdrift_authority::ActorRef;
use milkdrift_capability::PeerId;
use serde::{Deserialize, Serialize};

use crate::PersistenceError;

/// Accepted rename receipts are never evicted to make room for another request.
pub const MAX_DAEMON_RENAME_RECEIPTS: u64 = 1_024;
/// Combined bytes of each canonical rename request and its result.
pub const MAX_DAEMON_RENAME_RECEIPT_BYTES: usize = 16 * 1024;

/// Human-readable installation name. It is presentation, never identity or authority.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(transparent)]
pub struct DaemonDisplayName(String);

impl DaemonDisplayName {
    /// Trim surrounding whitespace and validate the shared name.
    ///
    /// # Errors
    /// Refuses control characters anywhere in the supplied value, or a trimmed name
    /// outside 1–80 Unicode scalar values.
    pub fn new(value: impl Into<String>) -> Result<Self, PersistenceError> {
        let value = value.into();
        let trimmed = value.trim();
        if value.chars().any(char::is_control) || trimmed.is_empty() || trimmed.chars().count() > 80
        {
            return Err(PersistenceError::Bounds {
                location: "daemon_display_name",
                reason:
                    "name must contain 1–80 characters after trimming, without control characters"
                        .to_owned(),
            });
        }
        Ok(Self(trimmed.to_owned()))
    }

    /// The normalized name shown by every connected client.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Default for DaemonDisplayName {
    fn default() -> Self {
        Self("Milkdrift".to_owned())
    }
}

milkdrift_contracts::deserialize_via!(DaemonDisplayName, String, |value| Self::new(value));

/// Current daemon-wide presentation metadata; generations start at one.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DaemonMetadata {
    /// Stable installation identity, independent of its name.
    pub installation: PeerId,
    /// Accepted shared name.
    pub display_name: DaemonDisplayName,
    /// Optimistic guard for the next rename.
    pub generation: u64,
}

/// An already-authorized rename, retaining the original canonical request before name trimming.
#[derive(Clone, Debug)]
pub struct DaemonRename {
    /// Installation to which the authenticated request was bound.
    pub installation: PeerId,
    /// Authenticated receipt owner.
    pub actor: ActorRef,
    /// Actor-local request key.
    pub key: String,
    /// Metadata generation observed by the caller.
    pub expected_generation: u64,
    /// Validated name to accept.
    pub display_name: DaemonDisplayName,
    /// Exact canonical public request bytes, including the untrimmed proposed name.
    pub canonical_request: Vec<u8>,
}

/// Saved acceptance returned after a lost reply, even when the current name has changed again.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DaemonRenameReceipt {
    /// Installation accepting the rename.
    pub installation: PeerId,
    /// Actor whose request key owns this receipt.
    pub actor: ActorRef,
    /// Original actor-local key.
    pub key: String,
    /// Original canonical request for exact conflict detection.
    pub canonical_request: Vec<u8>,
    /// Metadata accepted by the original request, not the current live state.
    pub result: DaemonMetadata,
}

/// Distinguishes a new acceptance from recovery without altering the saved result.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DaemonRenameOutcome {
    /// Metadata and receipt committed together.
    Applied(DaemonRenameReceipt),
    /// Exact original response; the current name and generation were not changed.
    Replay(DaemonRenameReceipt),
}

/// One daemon-owned name with actor-scoped exact mutation recovery.
///
/// Initialization binds the existing installation identity and accepts a seed only when no
/// metadata exists. Renames atomically change the name and save their result. Exact replay
/// precedes generation and capacity checks; an error after commit requires receipt lookup or
/// the same canonical request, never an automatic retry under a new key.
pub trait DaemonMetadataStore: Send + Sync {
    /// Initialize absent metadata or return the already accepted name unchanged.
    ///
    /// # Errors
    /// Refuses a different installation identity and returns integrity/storage failures.
    fn initialize_daemon_metadata(
        &self,
        installation: &PeerId,
        seed: &DaemonDisplayName,
    ) -> Result<DaemonMetadata, PersistenceError>;

    /// Read this installation's metadata, if it has been initialized.
    ///
    /// # Errors
    /// Refuses identity mismatch and returns integrity/storage failures.
    fn daemon_metadata(
        &self,
        installation: &PeerId,
    ) -> Result<Option<DaemonMetadata>, PersistenceError>;

    /// Recover only the authenticated actor's request in this installation.
    ///
    /// # Errors
    /// Refuses identity mismatch or invalid keys and returns integrity/storage failures.
    fn daemon_rename_receipt(
        &self,
        installation: &PeerId,
        actor: &ActorRef,
        key: &str,
    ) -> Result<Option<DaemonRenameReceipt>, PersistenceError>;

    /// Apply a guarded rename and save its exact acceptance in the same transaction.
    ///
    /// # Errors
    /// Refuses conflicting request reuse, stale generations, invalid canonical documents,
    /// exhausted receipt capacity, identity mismatch and integrity/storage failures.
    fn rename_daemon(&self, rename: DaemonRename) -> Result<DaemonRenameOutcome, PersistenceError>;
}
