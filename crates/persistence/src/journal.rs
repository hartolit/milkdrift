mod commit;
mod query;
mod receipt;

/// Maximum canonical runtime command bytes retained for exact idempotency evidence.
pub const MAX_COMMAND_DOCUMENT_BYTES: usize = 262_144;
/// Maximum canonical bytes in one retained command-result document.
pub const MAX_COMMAND_RESULT_DOCUMENT_BYTES: usize = 524_288;
/// Maximum index changes included in one aggregate commit.
pub const MAX_INDEX_MUTATIONS_PER_COMMIT: usize = 2_048;
/// Maximum scope/value mutations in one atomic aggregate commit.
pub const MAX_WORKSPACE_MUTATIONS_PER_COMMIT: usize = 2_048;
/// Maximum number of immutable origin hops verified for one workspace value.
///
/// This matches the atomic workspace-mutation ceiling so validation always has
/// a fixed adapter-neutral memory and lookup bound.
pub const MAX_VALUE_PROVENANCE_DEPTH: usize = MAX_WORKSPACE_MUTATIONS_PER_COMMIT;
/// Maximum distinct committed artifact references validated in one commit.
pub const MAX_REQUIRED_ARTIFACTS_PER_COMMIT: usize = 2_048;
/// Current opaque command-receipt/result document schema.
pub const COMMAND_RESULT_SCHEMA_VERSION_V1: u32 = 1;
/// Authorization-bearing command-result schema used by external commands.
pub const COMMAND_RESULT_SCHEMA_VERSION_V2: u32 = 2;

pub use commit::{
    ActiveLeaseSnapshot, AtomicRunCommitOutcome, AtomicRunCommitRequest, IndexedRunState,
    LeaseIndexEntry, LeaseIndexMutation, RunIndexUpdate, RunJournal, RunSummaryIndex,
    RunnableCursor, RunnableIndexEntry, RunnableIndexMutation, RunnablePage, TimerIndexEntry,
    TimerIndexMutation, WorkspaceAccounting, WorkspaceMutation,
};
pub use query::{
    EventCursor, EventPage, EventPageQuery, RunDiscoveryIntegrityStore, RunQueryStore,
    RunSummaryCursor, RunSummaryFilter, RunSummaryPage, RunSummaryPageQuery, WorkspaceStore,
};
pub use receipt::{CommandDisposition, CommandReceipt, CommandResultDocument};
