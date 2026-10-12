//! Atomic storage of the runtime-owned task document and its externally recoverable receipt.

use milkdrift_blueprint::{TaskId, WorkflowId};
use milkdrift_capability::{InvocationId, ResolvedCapabilitySnapshot};
use serde::{Deserialize, Serialize};

use crate::{PersistenceError, TimestampMillis};

/// Largest runtime document accepted by the task storage boundary.
pub const MAX_WORKFLOW_DOCUMENT_BYTES: usize = 256 * 1024 * 1024;
/// Largest canonical client request retained for exact conflict detection.
pub const MAX_WORKFLOW_REQUEST_BYTES: usize = 1024 * 1024;

/// Saved runtime document plus the fields needed for authorized discovery and accounting.
/// Runtime owns the document's state machine; storage never interprets or schedules tasks.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkflowSnapshot {
    /// Daemon-issued workflow identity.
    pub workflow: WorkflowId,
    /// Storage compare-and-swap generation; starts at one.
    pub generation: u64,
    /// Searchable workflow title.
    pub title: String,
    /// Searchable brief or description.
    pub description: String,
    /// Authenticated owner used by the public read authorization boundary.
    pub owner_actor: String,
    /// Last accepted runtime transition time.
    pub updated_at: TimestampMillis,
    /// Canonical runtime-owned document, including frozen starts and immutable results.
    pub document: serde_json::Value,
    /// Runtime-certified external entries, retained monotonically for artifact provenance.
    #[serde(default)]
    pub entered_invocations: Vec<InvocationId>,
    /// Frozen managed selections and monotonic entry/settlement facts certified by runtime.
    pub managed_executions: Vec<WorkflowManagedExecution>,
    /// Aggregate accepted output ceiling while obligations remain unfinished.
    /// Storage credits already retained and pending outputs against this reservation.
    pub artifact_capacity: u64,
    /// Remaining bytes protected for accepted tasks' entry, result and settlement records.
    pub reserved_bytes: u64,
    /// Remaining mandatory control receipt slots, inaccessible to ordinary admission.
    pub reserved_receipts: u32,
}

/// The runtime's exact accepted managed operation, consumed by same-transaction host guards.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkflowManagedExecution {
    /// Task whose immutable frozen start owns this selection.
    pub task: TaskId,
    /// Exact adapter-facing invocation identity.
    pub invocation: InvocationId,
    /// Workflow generation at the first accepted frozen start.
    pub accepted_generation: u64,
    /// Exact capability generation and its managed binding.
    pub selection: ResolvedCapabilitySnapshot,
    /// Final entry intent has committed; false permits no-entry release after settlement.
    pub entered: bool,
    /// Runtime has accepted terminal evidence; physical stop is a separate managed guard.
    pub settled: bool,
}

/// Exact actor-scoped request and result accepted with one workflow transition.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkflowCommandAcceptance {
    /// Authenticated actor, never an actor asserted in model output.
    pub actor: String,
    /// Bounded actor-local idempotency identity.
    pub key: String,
    /// Runtime-issued mandatory action identity; only its first actor can consume reserved capacity.
    pub reserved_action: Option<String>,
    /// Exact canonical request bytes, retained even after display detail expires.
    pub canonical_request: Vec<u8>,
    /// Exact compact result, including daemon-issued IDs and alias mapping.
    pub result: serde_json::Value,
}

/// Recoverable response to an accepted request.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkflowCommandReceipt {
    /// Workflow changed by the original request.
    pub workflow: WorkflowId,
    /// Storage generation at the original acceptance.
    pub generation: u64,
    /// Original request and compact result.
    pub acceptance: WorkflowCommandAcceptance,
}

/// All durable facts of a task transition, committed atomically.
#[derive(Clone, Debug)]
pub struct WorkflowCommit {
    /// None means a new identity must be absent; Some guards an existing state.
    pub expected_generation: Option<u64>,
    /// Next complete runtime state and its discovery fields.
    pub snapshot: WorkflowSnapshot,
    /// External requests retain a receipt; internal runtime transitions do not allocate one.
    pub command: Option<WorkflowCommandAcceptance>,
}

/// Exact replay precedes generation and capacity checks.
#[derive(Clone, Debug, PartialEq)]
pub enum WorkflowCommitOutcome {
    /// The new state and optional external receipt committed together.
    Applied(Option<WorkflowCommandReceipt>),
    /// The original response is returned without changing any workflow state.
    Replay(WorkflowCommandReceipt),
}

/// Stable identity-ordered bounded storage page. Public cursors additionally bind authority.
#[derive(Clone, Debug)]
pub struct WorkflowPage {
    /// At most the requested number of records.
    pub items: Vec<WorkflowSnapshot>,
    /// Last returned identity when additional records remain.
    pub next: Option<String>,
}

/// Independent bounds for retained task facts and exact command recovery content.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct WorkflowStorageLimits {
    /// Retained snapshots, promised state/output growth, pending writes and committed content.
    pub history_bytes: u64,
    /// Exact requests and compact results, including promised mandatory action space.
    pub receipt_bytes: u64,
    /// Accepted receipts plus promised mandatory action slots.
    pub receipt_entries: u32,
}

impl Default for WorkflowStorageLimits {
    fn default() -> Self {
        Self {
            history_bytes: 1024 * 1024 * 1024,
            receipt_bytes: 256 * 1024 * 1024,
            receipt_entries: 100_000,
        }
    }
}

/// Current retained usage and capacity promised to accepted work.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct WorkflowStorageUsage {
    /// Encoded runtime snapshot bytes.
    pub history_bytes: u64,
    /// Exact retained command receipt bytes.
    pub receipt_bytes: u64,
    /// Exact retained command receipt count.
    pub receipt_entries: u64,
    /// Bytes protected for remaining runtime records.
    pub reserved_bytes: u64,
    /// Receipt slots protected for mandatory controls.
    pub reserved_receipts: u64,
    /// Remaining accepted output space not already written.
    pub artifact_reserved_bytes: u64,
    /// Artifact bytes with durable writable publication intent.
    pub pending_artifact_bytes: u64,
    /// Verified committed content bytes, deduplicated physically.
    pub artifact_content_bytes: u64,
}

/// Atomic task state/receipt port. Its implementation preserves exact requests, reserves
/// settlement capacity, and refuses conflicting reuse before considering the supplied state.
pub trait WorkflowStore: Send + Sync {
    /// Read current retained usage without changing state or observing the clock.
    ///
    /// # Errors
    /// Returns unavailable or corrupt accounting failures.
    fn workflow_storage_usage(&self) -> Result<WorkflowStorageUsage, PersistenceError>;

    /// Read the authoritative workflow snapshot.
    ///
    /// # Errors
    /// Returns storage corruption or unavailable-store failures.
    fn workflow(&self, workflow: &WorkflowId)
    -> Result<Option<WorkflowSnapshot>, PersistenceError>;
    /// Read an identity-ordered page, with a maximum of 200 rows.
    ///
    /// # Errors
    /// Refuses invalid limits/cursors and returns storage failures.
    fn workflows(&self, after: Option<&str>, limit: u16) -> Result<WorkflowPage, PersistenceError>;
    /// Recover an actor-scoped request result without knowing its workflow identity.
    ///
    /// # Errors
    /// Refuses malformed identities and returns storage failures.
    fn workflow_receipt(
        &self,
        actor: &str,
        key: &str,
    ) -> Result<Option<WorkflowCommandReceipt>, PersistenceError>;
    /// Commit state, discovery fields, capacity accounting and exact response together.
    ///
    /// # Errors
    /// Refuses stale generations, conflicting keys, invalid documents and exhausted capacity.
    fn commit_workflow(
        &self,
        commit: WorkflowCommit,
    ) -> Result<WorkflowCommitOutcome, PersistenceError>;
}
