mod commit;
mod query;
mod receipt;

/// Byte limit for each canonical audit or intent JSON document retained by a command receipt.
pub const MAX_COMMAND_DOCUMENT_BYTES: usize = 262_144;
/// Byte limit for one encoded command-result document, including supplied whitespace on read.
pub const MAX_COMMAND_RESULT_DOCUMENT_BYTES: usize = 524_288;
/// Maximum combined runnable, timer, and lease mutations per commit, excluding its summary.
pub const MAX_INDEX_MUTATIONS_PER_COMMIT: usize = 2_048;
/// Maximum combined scope creations and value writes in one commit.
pub const MAX_WORKSPACE_MUTATIONS_PER_COMMIT: usize = 2_048;
/// Maximum number of immutable origin hops verified for one workspace value.
///
/// This matches the atomic workspace-mutation ceiling so validation always has
/// a fixed adapter-neutral memory and lookup bound.
pub const MAX_VALUE_PROVENANCE_DEPTH: usize = MAX_WORKSPACE_MUTATIONS_PER_COMMIT;
/// Maximum entries in each commit's required and newly referenced artifact lists.
pub const MAX_REQUIRED_ARTIFACTS_PER_COMMIT: usize = 2_048;
/// Result format without an authorization decision, used for closed internal commands.
/// [`CommandResultDocument::new`] emits this version; the reader retains support for it.
pub const COMMAND_RESULT_SCHEMA_VERSION_V1: u32 = 1;
/// Result format requiring the original authorization decision for an external command.
/// [`CommandResultDocument::new_authorized`] emits this version; the decision survives replay.
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
