use milkdrift_blueprint::WorkflowId;
use milkdrift_workspace::{
    RunId, ScopeId, ScopeReference, ValueKey, WorkspaceScope, WorkspaceUsage, WorkspaceValueEntry,
    WorkspaceValueReference,
};
use serde::{Deserialize, Serialize};

use super::commit::{
    ActiveLeaseSnapshot, IndexedRunState, LeaseIndexEntry, RunSummaryIndex, RunnableCursor,
    RunnableIndexEntry, RunnablePage, TimerIndexEntry,
};
use crate::{PageSize, PersistenceError, RunEventEnvelope, RunSequence, SignalId, TimestampMillis};

/// Stable event page cursor; the next sequence is inclusive.
///
/// For an existing non-empty run whose observed head is `N`, `N + 1` is the
/// one valid end-of-stream cursor. Reading that cursor returns an empty page,
/// no continuation, and the same observed head. A later cursor is invalid, as
/// is every cursor for an absent run. When `N` is the maximum sequence there is
/// no representable end-of-stream cursor and the final ordinary page has no
/// continuation.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct EventCursor {
    /// Owning aggregate prevents cross-run reuse.
    pub run: RunId,
    /// Inclusive next sequence.
    pub next_sequence: RunSequence,
}

/// Bounded page query over authoritative ordered events.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EventPageQuery {
    /// Aggregate to read.
    pub run: RunId,
    /// Cursor from a previous page; absent starts at sequence one.
    pub cursor: Option<EventCursor>,
    /// Maximum returned envelopes.
    pub limit: PageSize,
}

impl EventPageQuery {
    /// Constructs a query and rejects a cursor for another run or sequence zero.
    pub fn new(
        run: RunId,
        cursor: Option<EventCursor>,
        limit: PageSize,
    ) -> Result<Self, PersistenceError> {
        if let Some(cursor) = &cursor
            && (cursor.run != run || cursor.next_sequence == RunSequence::ZERO)
        {
            return Err(PersistenceError::InvalidCursor(
                "event cursor must belong to the query run and name a non-zero sequence".to_owned(),
            ));
        }
        Ok(Self { run, cursor, limit })
    }

    /// Validates this query against one atomically observed journal head and
    /// returns the first sequence to read.
    ///
    /// `Ok(None)` means the query is already at end of stream. This is returned
    /// for an absent run without a cursor and for the exact one-past-head cursor
    /// of an existing non-empty run. Implementations must use this method even
    /// when callers constructed the public query fields directly, so cursor
    /// ownership and the exact EOF rule cannot diverge between adapters.
    pub fn start_sequence(
        &self,
        observed_head: RunSequence,
    ) -> Result<Option<RunSequence>, PersistenceError> {
        let Some(cursor) = &self.cursor else {
            return Ok((observed_head != RunSequence::ZERO).then_some(RunSequence::FIRST));
        };
        if cursor.run != self.run || cursor.next_sequence == RunSequence::ZERO {
            return Err(PersistenceError::InvalidCursor(
                "event cursor must belong to the query run and name a non-zero sequence".to_owned(),
            ));
        }
        if observed_head == RunSequence::ZERO {
            return Err(PersistenceError::InvalidCursor(
                "event cursor names an absent run".to_owned(),
            ));
        }
        if cursor.next_sequence <= observed_head {
            return Ok(Some(cursor.next_sequence));
        }

        let exact_eof = observed_head.next().map_err(|_| {
            PersistenceError::InvalidCursor(
                "the observed journal head has no representable end-of-stream cursor".to_owned(),
            )
        })?;
        if cursor.next_sequence == exact_eof {
            Ok(None)
        } else {
            Err(PersistenceError::InvalidCursor(format!(
                "event cursor sequence {} is beyond exact end-of-stream position {exact_eof}",
                cursor.next_sequence
            )))
        }
    }
}

/// One ordered event page plus a resumable cursor.
#[derive(Clone, Debug, PartialEq)]
pub struct EventPage {
    /// Strictly contiguous verified envelopes.
    pub events: Vec<RunEventEnvelope>,
    /// Cursor for the next page, absent at the observed head. Reading an exact
    /// one-past-head cursor also returns no continuation.
    pub next: Option<EventCursor>,
    /// Journal head observed during this read transaction.
    pub observed_head: RunSequence,
}

/// Query filter for immutable run summaries.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct RunSummaryFilter {
    /// Optional exact discovery state.
    pub state: Option<IndexedRunState>,
    /// Optional workflow lineage.
    pub workflow: Option<WorkflowId>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum RunSummaryCursorScope {
    Query(RunSummaryFilter),
    Nonterminal,
}

/// Stable summary cursor based on the last physically scanned run identity.
///
/// The cursor is bound to the exact logical query that produced it. This lets an
/// adapter return an empty but advancing page when a bounded physical scan finds
/// no matching summaries, without allowing that continuation to be reused with a
/// different filter or with nonterminal recovery discovery.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RunSummaryCursor {
    after_run: RunId,
    scope: RunSummaryCursorScope,
}

impl RunSummaryCursor {
    /// Constructs a cursor for the exact immutable summary filter.
    #[must_use]
    pub fn for_query(after_run: RunId, filter: RunSummaryFilter) -> Self {
        Self {
            after_run,
            scope: RunSummaryCursorScope::Query(filter),
        }
    }

    /// Constructs a cursor for authoritative nonterminal discovery.
    #[must_use]
    pub fn for_nonterminal(after_run: RunId) -> Self {
        Self {
            after_run,
            scope: RunSummaryCursorScope::Nonterminal,
        }
    }

    /// Last physically scanned run (the exclusive resume point).
    #[must_use]
    pub fn after_run(&self) -> &RunId {
        &self.after_run
    }

    /// Whether this cursor belongs to the exact summary filter.
    #[must_use]
    pub fn matches_query(&self, filter: &RunSummaryFilter) -> bool {
        matches!(&self.scope, RunSummaryCursorScope::Query(bound) if bound == filter)
    }

    /// Whether this cursor belongs to nonterminal recovery discovery.
    #[must_use]
    pub fn is_nonterminal(&self) -> bool {
        self.scope == RunSummaryCursorScope::Nonterminal
    }
}

/// Bounded run-summary page query.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RunSummaryPageQuery {
    /// Immutable filters.
    pub filter: RunSummaryFilter,
    /// Last-scanned resume point bound to this exact filter.
    pub cursor: Option<RunSummaryCursor>,
    /// Maximum returned summaries.
    pub limit: PageSize,
}

/// One immutable run-summary page.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RunSummaryPage {
    /// Derived, verifiable summaries.
    pub runs: Vec<RunSummaryIndex>,
    /// Last-scanned resume point, absent when exhausted. `runs` may be empty
    /// while this cursor advances across a bounded range of nonmatching rows.
    pub next: Option<RunSummaryCursor>,
}

/// Read-only journal and discoverability queries for runtime/recovery/control APIs.
pub trait RunQueryStore: Send + Sync {
    /// Reads a verified contiguous event page. Malformed history is an error.
    ///
    /// Implementations must apply [`EventPageQuery::start_sequence`] to their
    /// atomically observed head. In particular, the exact one-past-head cursor
    /// of an existing non-empty run is valid EOF, later cursors are invalid,
    /// and a cursor for an absent run is invalid.
    fn events(&self, query: &EventPageQuery) -> Result<EventPage, PersistenceError>;

    /// Finds the authoritative receipt event for one stable signal identity.
    ///
    /// Implementations must use a bounded journal-derived identity index so
    /// command planning never scans a run's durable history.
    fn signal_receipt(
        &self,
        run: &RunId,
        signal: &SignalId,
    ) -> Result<Option<RunEventEnvelope>, PersistenceError>;

    /// Gets one run summary.
    fn run_summary(&self, run: &RunId) -> Result<Option<RunSummaryIndex>, PersistenceError>;

    /// Lists run summaries with stable identity-based pagination.
    fn run_summaries(
        &self,
        query: &RunSummaryPageQuery,
    ) -> Result<RunSummaryPage, PersistenceError>;

    /// Discovers one stable identity-ordered page of nonterminal runs.
    ///
    /// The cursor is exclusive. Callers performing bounded recurring maintenance
    /// retain the returned cursor and reset to the beginning only after `next` is
    /// absent, so an early run cannot permanently hide later owned work.
    fn nonterminal_run_page(
        &self,
        cursor: Option<&RunSummaryCursor>,
        limit: PageSize,
    ) -> Result<RunSummaryPage, PersistenceError>;

    /// Discovers eligible work with at most one deterministic candidate per run.
    ///
    /// The page bound applies directly to validated per-run heads, so a run
    /// with a saturated runnable set cannot hide another run behind its entries.
    /// Within one run the selected candidate is ordered by eligibility time
    /// ascending, priority descending only among equal eligibility timestamps,
    /// then execution identity ascending. Runtime owns fairness between returned
    /// runs and all dispatch decisions.
    /// A continuation retains the first page's `eligible_through` boundary and its
    /// exclusive key remains valid if a dispatched anchor row has been removed.
    fn runnable_page(
        &self,
        eligible_through: TimestampMillis,
        cursor: Option<&RunnableCursor>,
        limit: PageSize,
    ) -> Result<RunnablePage, PersistenceError>;

    /// Reads up to `limit` active durable leases in stable expiry/identity order.
    ///
    /// Callers that query with their global admission bound may reject immediately
    /// when the returned page reaches that bound. A shorter page is the complete
    /// active set and can be projected into exact run/branch/capability counts without
    /// scanning unrelated run summaries.
    fn active_leases(&self, limit: PageSize) -> Result<ActiveLeaseSnapshot, PersistenceError>;

    /// Discovers due timers; firing remains a runtime command/event decision.
    fn due_timers(
        &self,
        due_through: TimestampMillis,
        limit: PageSize,
    ) -> Result<Vec<TimerIndexEntry>, PersistenceError>;

    /// Discovers expired leases; recovery classification remains runtime-owned.
    fn expired_leases(
        &self,
        expired_through: TimestampMillis,
        limit: PageSize,
    ) -> Result<Vec<LeaseIndexEntry>, PersistenceError>;
}

/// Explicit logical validation port for derived per-run discovery state.
///
/// A caller that has replayed authoritative history supplies the complete projected
/// runnable, timer, and lease sets for one run. Adapters compare those expectations
/// with their derived indexes, including redundant physical pairs, so symmetric loss
/// of every row in an index cannot masquerade as an empty set. Runtime startup uses
/// this after authoritative replay for each bounded page of active runs; offline scrub
/// remains responsible for complete-store physical validation.
pub trait RunDiscoveryIntegrityStore: Send + Sync {
    /// Validates the complete derived discovery state at an authoritative run head.
    fn validate_run_discovery(
        &self,
        run: &RunId,
        through_sequence: RunSequence,
        runnable: &[RunnableIndexEntry],
        timers: &[TimerIndexEntry],
        leases: &[LeaseIndexEntry],
    ) -> Result<(), PersistenceError>;
}

/// Read-only access to durable workspace state. All mutations occur through
/// [`crate::RunJournal::commit_command`] to preserve crash atomicity with event history.
pub trait WorkspaceStore: Send + Sync {
    /// Reads the exact durable budget usage used as the next optimistic accounting guard.
    fn workspace_usage(&self, run: &RunId) -> Result<WorkspaceUsage, PersistenceError>;

    /// Gets one exact scope declaration.
    fn scope(
        &self,
        run: &RunId,
        scope: &ScopeId,
    ) -> Result<Option<WorkspaceScope>, PersistenceError>;

    /// Gets one exact immutable value version.
    fn value(
        &self,
        reference: &WorkspaceValueReference,
    ) -> Result<Option<WorkspaceValueEntry>, PersistenceError>;

    /// Gets the latest immutable version of one scope-local stream.
    fn latest_value(
        &self,
        scope: &ScopeReference,
        key: &ValueKey,
    ) -> Result<Option<WorkspaceValueEntry>, PersistenceError>;

    /// Lists a bounded root-to-leaf lineage after validating stored parent links.
    fn scope_lineage(&self, leaf: &ScopeReference)
    -> Result<Vec<WorkspaceScope>, PersistenceError>;
}
