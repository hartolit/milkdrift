use std::collections::{BTreeMap, BTreeSet};

use milkdrift_blueprint::{RevisionId, WorkflowId};
use milkdrift_workspace::{
    ArtifactReference, RunId, WorkspaceBudget, WorkspaceScope, WorkspaceUsage, WorkspaceValueEntry,
};
use serde::{Deserialize, Serialize};

use super::{
    MAX_INDEX_MUTATIONS_PER_COMMIT, MAX_REQUIRED_ARTIFACTS_PER_COMMIT,
    MAX_WORKSPACE_MUTATIONS_PER_COMMIT,
    receipt::{CommandDisposition, CommandReceipt, CommandResultDocument},
};
use crate::{
    AttemptId, CommandId, IntegrityDigest, LeaseId, NodeExecutionId, PersistenceError,
    RunEventEnvelope, RunEventKind, RunSequence, TimerId, TimestampMillis, WorkerId,
    bounded::MAX_EVENTS_PER_COMMIT,
};

/// Materializes a scope or value introduced by this command's events.
/// [`AtomicRunCommitRequest::new`] checks these against the events; storage checks
/// saved ancestry and provenance and commits them together.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case", tag = "type", deny_unknown_fields)]
pub enum WorkspaceMutation {
    /// Create one validated structured scope.
    CreateScope {
        /// Immutable scope declaration.
        scope: WorkspaceScope,
    },
    /// Publish one exact immutable value version.
    PutValue {
        /// Immutable value record.
        entry: WorkspaceValueEntry,
    },
}

/// Charges workspace changes while checking the usage the runtime planned against.
///
/// Accepted commands require this even with unchanged usage; rejected commands omit it.
/// The constructor recomputes value and first-artifact-reference charges. Storage checks
/// saved usage and artifact membership atomically, preventing concurrent overspending.
/// A conflict requires rereading usage and replanning, not just replacing the guard.
/// Artifact-content publication remains a separate operation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorkspaceAccounting {
    /// Immutable limits for the run/workspace accounting domain.
    pub budget: WorkspaceBudget,
    /// Exact durable usage observed by the runtime.
    pub expected_usage: WorkspaceUsage,
    /// Exact usage after all `PutValue` mutations and first artifact references in this commit.
    pub resulting_usage: WorkspaceUsage,
}

impl WorkspaceMutation {
    fn run(&self) -> &RunId {
        match self {
            Self::CreateScope { scope } => scope.reference().run(),
            Self::PutValue { entry } => entry.reference().scope().run(),
        }
    }

    fn referenced_artifact(&self) -> Option<&ArtifactReference> {
        match self {
            Self::PutValue { entry } => entry.value().as_artifact(),
            Self::CreateScope { .. } => None,
        }
    }
}

/// Discoverability state derived from authoritative run events.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum IndexedRunState {
    /// Created but not started.
    Created,
    /// Contains eligible work.
    Runnable,
    /// Started and not paused/cancelling/terminal.
    Active,
    /// Admission and dispatch are paused.
    Paused,
    /// Cancellation intent is being drained.
    Cancelling,
    /// Waiting on a timer, signal, child, or authority.
    Waiting,
    /// Has unresolved uncertain/retained external work.
    Uncertain,
    /// Reached a terminal boundary.
    Terminal,
}

/// Derived, verifiable summary/discovery index for one run.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RunSummaryIndex {
    /// Run aggregate.
    pub run: RunId,
    /// Workflow lineage.
    pub workflow: WorkflowId,
    /// Current exact revision pin.
    pub revision: RevisionId,
    /// Derived discovery state.
    pub state: IndexedRunState,
    /// Exact authoritative journal sequence used to derive this entry.
    pub through_sequence: RunSequence,
    /// Boundary timestamp of the last contributing fact.
    pub updated_at: TimestampMillis,
}

/// Derived, verifiable runnable-discovery record.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RunnableIndexEntry {
    /// Owning aggregate.
    pub run: RunId,
    /// Eligible logical execution.
    pub execution: NodeExecutionId,
    /// Earliest boundary-clock admission time.
    pub eligible_at: TimestampMillis,
    /// Stable caller-derived priority; fairness is runtime-owned.
    pub priority: u16,
    /// Journal sequence used to derive the entry.
    pub through_sequence: RunSequence,
}

/// Stable runnable-discovery cursor based on the last validated stored run head.
///
/// The eligibility boundary is bound into the continuation. A completed cycle
/// therefore has one stable view of time even when the caller's boundary clock
/// advances between bounded pages.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RunnableCursor {
    after_run: RunId,
    eligible_through: TimestampMillis,
}

impl RunnableCursor {
    /// Constructs a validated exclusive runnable continuation.
    #[must_use]
    pub const fn new(after_run: RunId, eligible_through: TimestampMillis) -> Self {
        Self {
            after_run,
            eligible_through,
        }
    }

    /// Run component of the exclusive physical identity resume point.
    #[must_use]
    pub fn after_run(&self) -> &RunId {
        &self.after_run
    }

    /// Eligibility boundary owned by this continuation. A later page preserves
    /// this boundary even if its caller's wall clock has advanced; newly eligible
    /// work joins the next full cycle after the cursor is exhausted.
    #[must_use]
    pub const fn eligible_through(&self) -> TimestampMillis {
        self.eligible_through
    }
}

/// One bounded runnable page with no more than one candidate per run.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RunnablePage {
    /// Deterministically selected eligible candidate for each represented run.
    pub entries: Vec<RunnableIndexEntry>,
    /// Exclusive last-scanned cursor, absent when the physical index tail was reached.
    pub next: Option<RunnableCursor>,
}

/// Derived, verifiable timer-discovery record.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TimerIndexEntry {
    /// Owning aggregate.
    pub run: RunId,
    /// Durable timer.
    pub timer: TimerId,
    /// Exact deadline.
    pub fire_at: TimestampMillis,
    /// Journal sequence used to derive the entry.
    pub through_sequence: RunSequence,
}

/// Derived, verifiable active-lease discovery record.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct LeaseIndexEntry {
    /// Owning aggregate.
    pub run: RunId,
    /// Lease identity.
    pub lease: LeaseId,
    /// Owning attempt.
    pub attempt: AttemptId,
    /// Assigned worker.
    pub worker: WorkerId,
    /// Exact expiration.
    pub expires_at: TimestampMillis,
    /// Journal sequence used to derive the entry.
    pub through_sequence: RunSequence,
}

/// One bounded active-lease snapshot and its atomic-admission revision.
///
/// The revision is opaque to the runtime. Implementations must change it whenever
/// any active-lease row changes and compare an admission commit's expected revision
/// inside the same transaction that grants the new lease.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ActiveLeaseSnapshot {
    /// Active leases in stable expiry/identity order, bounded by the requested page size.
    pub entries: Vec<LeaseIndexEntry>,
    /// Revision of the complete active-lease set observed by this read.
    pub revision: IntegrityDigest,
}

/// Upsert/remove mutation for runnable discoverability.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case", tag = "type", deny_unknown_fields)]
pub enum RunnableIndexMutation {
    /// Insert or replace a derived record.
    Upsert {
        /// Complete replacement entry.
        entry: RunnableIndexEntry,
    },
    /// Remove an execution from discovery.
    Remove {
        /// Owning run.
        run: RunId,
        /// Execution identity.
        execution: NodeExecutionId,
    },
}

/// Upsert/remove mutation for timer discoverability.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case", tag = "type", deny_unknown_fields)]
pub enum TimerIndexMutation {
    /// Insert or replace a derived record.
    Upsert {
        /// Complete replacement entry.
        entry: TimerIndexEntry,
    },
    /// Remove a fired/cancelled timer.
    Remove {
        /// Owning run.
        run: RunId,
        /// Timer identity.
        timer: TimerId,
    },
}

/// Upsert/remove mutation for active-lease discoverability.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case", tag = "type", deny_unknown_fields)]
pub enum LeaseIndexMutation {
    /// Insert or replace a derived record.
    Upsert {
        /// Complete replacement entry.
        entry: LeaseIndexEntry,
    },
    /// Remove an expired/released lease.
    Remove {
        /// Owning run.
        run: RunId,
        /// Lease identity.
        lease: LeaseId,
    },
}

/// Updates the records used to discover a run's unfinished work after an append.
///
/// Runtime derives these from projected events; storage commits both together so
/// recovery finds newly eligible work and skips released leases. Accepted appends need
/// a summary at their resulting head; rejected commands use the empty [`Self::default`].
/// [`AtomicRunCommitRequest::new`] checks the update against the append and its bounds.
#[derive(Clone, Debug, Default, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RunIndexUpdate {
    /// Required summary for any accepted event append.
    summary: Option<RunSummaryIndex>,
    /// Runnable index changes.
    runnable: Vec<RunnableIndexMutation>,
    /// Timer index changes.
    timers: Vec<TimerIndexMutation>,
    /// Lease index changes.
    leases: Vec<LeaseIndexMutation>,
}

impl RunIndexUpdate {
    /// Collects the runtime's derived summary and index changes without validating them.
    /// Pass the update to [`AtomicRunCommitRequest::new`] to check it against the append.
    #[must_use]
    pub fn new(
        summary: Option<RunSummaryIndex>,
        runnable: Vec<RunnableIndexMutation>,
        timers: Vec<TimerIndexMutation>,
        leases: Vec<LeaseIndexMutation>,
    ) -> Self {
        Self {
            summary,
            runnable,
            timers,
            leases,
        }
    }

    /// Summary derived at the resulting journal head.
    #[must_use]
    pub const fn summary(&self) -> Option<&RunSummaryIndex> {
        self.summary.as_ref()
    }

    /// Runnable-index mutations.
    #[must_use]
    pub fn runnable(&self) -> &[RunnableIndexMutation] {
        &self.runnable
    }

    /// Timer-index mutations.
    #[must_use]
    pub fn timers(&self) -> &[TimerIndexMutation] {
        &self.timers
    }

    /// Lease-index mutations.
    #[must_use]
    pub fn leases(&self) -> &[LeaseIndexMutation] {
        &self.leases
    }

    /// Whether this update contains no summary or index mutation.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.summary.is_none()
            && self.runnable.is_empty()
            && self.timers.is_empty()
            && self.leases.is_empty()
    }
}

/// Everything storage must save together for one runtime command.
///
/// Runtime plans the events, receipt/result, workspace/accounting and discovery changes.
/// Construction checks their agreement; [`RunJournal::commit_command`] checks stored
/// state and saves them. For example, `RunCreated` requires its root scope, accounting
/// even with no inputs, and a resulting-head summary; missing the scope refuses the request.
/// Rejections have no events, workspace/artifact changes, accounting, lease guard, or
/// index update, and retain a rejected result at the expected head.
/// Attach controller changes or a projection commitment before committing. [`RunJournal`]
/// owns replay and lost-response recovery; constructing this value neither authorizes nor saves it.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct AtomicRunCommitRequest {
    /// Exact audit/intent bytes and idempotency fingerprint.
    receipt: CommandReceipt,
    /// Contiguous checksummed facts, empty only for a rejected command.
    events: Vec<RunEventEnvelope>,
    /// Workspace scopes/values committed atomically with history.
    workspace: Vec<WorkspaceMutation>,
    /// Optimistic budget/accounting transition; required for accepted commits.
    workspace_accounting: Option<WorkspaceAccounting>,
    /// Every artifact referenced by events/workspace; append must prove each is committed.
    required_artifacts: Vec<ArtifactReference>,
    /// Required artifacts first admitted into this run's accounting domain by this commit.
    newly_referenced_artifacts: Vec<ArtifactReference>,
    /// Exact active-lease revision observed while admitting a new durable lease.
    /// Required for `LeaseGranted`/`NodeReLeased`; the adapter compares it atomically
    /// so concurrent services cannot admit against the same pre-lease global usage.
    expected_lease_revision: Option<IntegrityDigest>,
    /// Optional controller-account mutation committed beside the event facts.
    controller_account: Option<crate::ControllerAccountTransaction>,
    /// Optional projection payload commitment derived from the accepted resulting state.
    projection_checkpoint: Option<crate::ProjectionCheckpoint>,
    /// Fully durable result returned on redelivery.
    result: CommandResultDocument,
    /// Derived, verifiable discovery/index changes committed atomically.
    indexes: RunIndexUpdate,
}

impl AtomicRunCommitRequest {
    /// Checks that the receipt, events, result, workspace, and indexes describe one append.
    ///
    /// Events must belong to the receipt's run and be contiguous immediately after its
    /// expected sequence. The result must bind the same command fingerprint, event IDs
    /// in order, and final sequence. Workspace mutations must exactly supply the scopes
    /// and value identities those events introduce; hidden extra mutations also fail.
    ///
    /// `required_artifacts` must name every direct event/workspace artifact reference
    /// exactly once. `newly_referenced_artifacts` is a distinct subset being charged to
    /// this run for the first time. The constructor verifies the lists and accounting
    /// arithmetic; storage later verifies committed content, membership, and usage.
    /// `expected_lease_revision` is required exactly when an event grants or regrants
    /// a lease. Obtain it from the active-lease read used to plan admission.
    ///
    /// # Errors
    ///
    /// Returns [`PersistenceError::Bounds`] for oversized event, workspace, artifact,
    /// or combined index lists; [`PersistenceError::SequenceOverflow`] if numbering
    /// the append exhausts the sequence; and [`PersistenceError::InvalidDocument`] for
    /// inconsistent documents, duplicates, or invalid budget/accounting calculations.
    /// No storage is read or changed, so these errors cannot indicate a partial commit.
    #[allow(clippy::too_many_arguments)] // One atomic commit binds receipt/events to workspace accounting, artifact references, lease guard, result, and indexes.
    pub fn new(
        receipt: CommandReceipt,
        events: Vec<RunEventEnvelope>,
        workspace: Vec<WorkspaceMutation>,
        workspace_accounting: Option<WorkspaceAccounting>,
        required_artifacts: Vec<ArtifactReference>,
        newly_referenced_artifacts: Vec<ArtifactReference>,
        expected_lease_revision: Option<IntegrityDigest>,
        result: CommandResultDocument,
        indexes: RunIndexUpdate,
    ) -> Result<Self, PersistenceError> {
        if events.len() > MAX_EVENTS_PER_COMMIT {
            return Err(PersistenceError::Bounds {
                location: "commit.events",
                reason: format!("at most {MAX_EVENTS_PER_COMMIT} events are allowed"),
            });
        }
        if workspace.len() > MAX_WORKSPACE_MUTATIONS_PER_COMMIT {
            return Err(PersistenceError::Bounds {
                location: "commit.workspace",
                reason: format!(
                    "at most {MAX_WORKSPACE_MUTATIONS_PER_COMMIT} workspace mutations are allowed"
                ),
            });
        }
        if required_artifacts.len() > MAX_REQUIRED_ARTIFACTS_PER_COMMIT {
            return Err(PersistenceError::Bounds {
                location: "commit.required_artifacts",
                reason: format!(
                    "at most {MAX_REQUIRED_ARTIFACTS_PER_COMMIT} artifact references are allowed"
                ),
            });
        }
        if newly_referenced_artifacts.len() > MAX_REQUIRED_ARTIFACTS_PER_COMMIT {
            return Err(PersistenceError::Bounds {
                location: "commit.newly_referenced_artifacts",
                reason: format!(
                    "at most {MAX_REQUIRED_ARTIFACTS_PER_COMMIT} artifact references are allowed"
                ),
            });
        }
        let grants_lease = events.iter().any(|event| {
            matches!(
                event.kind(),
                RunEventKind::LeaseGranted { .. } | RunEventKind::NodeReLeased { .. }
            )
        });
        if grants_lease != expected_lease_revision.is_some() {
            return Err(PersistenceError::InvalidDocument(
                "expected_lease_revision must be present exactly for a lease-creating commit"
                    .to_owned(),
            ));
        }
        let index_count = indexes
            .runnable
            .len()
            .saturating_add(indexes.timers.len())
            .saturating_add(indexes.leases.len());
        if index_count > MAX_INDEX_MUTATIONS_PER_COMMIT {
            return Err(PersistenceError::Bounds {
                location: "commit.indexes",
                reason: format!(
                    "at most {MAX_INDEX_MUTATIONS_PER_COMMIT} index mutations are allowed"
                ),
            });
        }
        if receipt.command() != result.command()
            || receipt.run() != result.run()
            || receipt.fingerprint() != result.command_fingerprint()
        {
            return Err(PersistenceError::InvalidDocument(
                "command receipt and result identities/fingerprint differ".to_owned(),
            ));
        }
        if events.is_empty() != matches!(result.disposition(), CommandDisposition::Rejected) {
            return Err(PersistenceError::InvalidDocument(
                "only a rejected command may commit no events".to_owned(),
            ));
        }

        let mut expected = receipt.expected_sequence();
        let mut event_ids = Vec::with_capacity(events.len());
        for event in &events {
            expected = expected.next()?;
            if event.run_id() != receipt.run() || event.sequence() != expected {
                return Err(PersistenceError::InvalidDocument(format!(
                    "events must belong to run {} and be contiguous after sequence {}",
                    receipt.run(),
                    receipt.expected_sequence()
                )));
            }
            if matches!(
                event.kind(),
                RunEventKind::SignalDeduplicated {
                    duplicate_command,
                    ..
                } if duplicate_command != receipt.command()
            ) {
                return Err(PersistenceError::InvalidDocument(
                    "signal deduplication must name the command atomically recording it".to_owned(),
                ));
            }
            event_ids.push(event.event_id().clone());
        }
        let resulting_sequence = if events.is_empty() {
            receipt.expected_sequence()
        } else {
            expected
        };
        if result.resulting_sequence() != resulting_sequence || result.event_ids() != event_ids {
            return Err(PersistenceError::InvalidDocument(
                "command result sequence/event identities do not describe the append".to_owned(),
            ));
        }
        if workspace
            .iter()
            .any(|mutation| mutation.run() != receipt.run())
        {
            return Err(PersistenceError::InvalidDocument(
                "workspace mutations must belong to the command run".to_owned(),
            ));
        }
        let mut declared_scopes = BTreeMap::new();
        let mut introduced_values = BTreeSet::new();
        for event in &events {
            let scope = match event.kind() {
                RunEventKind::RunCreated { root_scope, .. } => Some(root_scope),
                RunEventKind::BranchScopeCreated { scope, .. }
                | RunEventKind::RepeatIterationCreated { scope, .. }
                | RunEventKind::SubworkflowCreated { scope, .. } => Some(scope),
                _ => None,
            };
            if scope.is_some_and(|scope| {
                declared_scopes
                    .insert(scope.reference().clone(), scope.clone())
                    .is_some()
            }) {
                return Err(PersistenceError::InvalidDocument(
                    "one atomic commit cannot declare the same workspace scope twice".to_owned(),
                ));
            }
            match event.kind() {
                RunEventKind::RunCreated { inputs, .. } => {
                    for value in inputs {
                        if !introduced_values.insert(value.clone()) {
                            return Err(PersistenceError::InvalidDocument(
                                "one atomic commit cannot introduce the same workspace value twice"
                                    .to_owned(),
                            ));
                        }
                    }
                }
                RunEventKind::NodeOutputPublished { value, .. }
                | RunEventKind::DeterministicOutputPublished { value, .. }
                    if !introduced_values.insert(value.clone()) =>
                {
                    return Err(PersistenceError::InvalidDocument(
                        "one atomic commit cannot introduce the same workspace value twice"
                            .to_owned(),
                    ));
                }
                RunEventKind::SubworkflowCreated { scope, inputs, .. } => {
                    for value in inputs
                        .iter()
                        .filter(|value| value.scope() == scope.reference())
                    {
                        if !introduced_values.insert(value.clone()) {
                            return Err(PersistenceError::InvalidDocument(
                                "one atomic commit cannot introduce the same workspace value twice"
                                    .to_owned(),
                            ));
                        }
                    }
                }
                RunEventKind::SubworkflowOutputImported { parent_value, .. }
                    if !introduced_values.insert(parent_value.clone()) =>
                {
                    return Err(PersistenceError::InvalidDocument(
                        "one atomic commit cannot introduce the same workspace value twice"
                            .to_owned(),
                    ));
                }
                _ => {}
            }
        }
        let mut mutated_scopes = BTreeMap::new();
        let mut mutated_values = BTreeSet::new();
        for mutation in &workspace {
            match mutation {
                WorkspaceMutation::CreateScope { scope } => {
                    if mutated_scopes
                        .insert(scope.reference().clone(), scope.clone())
                        .is_some()
                    {
                        return Err(PersistenceError::InvalidDocument(
                            "one atomic commit cannot create the same workspace scope twice"
                                .to_owned(),
                        ));
                    }
                }
                WorkspaceMutation::PutValue { entry } => {
                    if !mutated_values.insert(entry.reference().clone()) {
                        return Err(PersistenceError::InvalidDocument(
                            "one atomic commit cannot put the same workspace value twice"
                                .to_owned(),
                        ));
                    }
                }
            }
        }
        // Equality also rejects hidden writes: checking only that event references
        // are present would permit workspace changes with no explaining event.
        if mutated_scopes != declared_scopes || mutated_values != introduced_values {
            return Err(PersistenceError::InvalidDocument(
                "workspace mutations must exactly materialize scope/value references introduced by the command's events"
                    .to_owned(),
            ));
        }
        match (&workspace_accounting, events.is_empty()) {
            (None, false) => {
                return Err(PersistenceError::InvalidDocument(
                    "an accepted command requires workspace budget/accounting".to_owned(),
                ));
            }
            (Some(_), true) => {
                return Err(PersistenceError::InvalidDocument(
                    "a rejected command cannot change workspace accounting".to_owned(),
                ));
            }
            _ => {}
        }
        let required: BTreeSet<_> = required_artifacts.iter().cloned().collect();
        let newly_referenced: BTreeSet<_> = newly_referenced_artifacts.iter().cloned().collect();
        if required.len() != required_artifacts.len()
            || newly_referenced.len() != newly_referenced_artifacts.len()
            || !newly_referenced.is_subset(&required)
        {
            return Err(PersistenceError::InvalidDocument(
                "required/newly-referenced artifact lists must be distinct and newly-referenced artifacts must be a subset"
                    .to_owned(),
            ));
        }
        let mut referenced: BTreeSet<_> = workspace
            .iter()
            .filter_map(WorkspaceMutation::referenced_artifact)
            .cloned()
            .collect();
        for event in &events {
            referenced.extend(event.kind().required_artifacts()?);
        }
        if referenced != required {
            return Err(PersistenceError::InvalidDocument(
                "required_artifacts must exactly equal all direct event/workspace artifact references"
                    .to_owned(),
            ));
        }
        if let Some(accounting) = &workspace_accounting {
            accounting
                .budget
                .validate_usage(&accounting.expected_usage)
                .map_err(|error| PersistenceError::InvalidDocument(error.to_string()))?;
            let mut calculated = accounting.expected_usage;
            for mutation in &workspace {
                if let WorkspaceMutation::PutValue { entry } = mutation {
                    calculated = accounting
                        .budget
                        .admit_value(&calculated, entry.value())
                        .map_err(|error| PersistenceError::InvalidDocument(error.to_string()))?;
                }
            }
            for artifact in &newly_referenced_artifacts {
                calculated = accounting
                    .budget
                    .admit_artifact_reference(&calculated, artifact)
                    .map_err(|error| PersistenceError::InvalidDocument(error.to_string()))?;
            }
            if calculated != accounting.resulting_usage {
                return Err(PersistenceError::InvalidDocument(
                    "workspace resulting usage must exactly charge committed value mutations and newly referenced artifacts"
                        .to_owned(),
                ));
            }
        }

        if events.is_empty() {
            if indexes != RunIndexUpdate::default() || !workspace.is_empty() {
                return Err(PersistenceError::InvalidDocument(
                    "a rejected command cannot mutate indexes or workspace".to_owned(),
                ));
            }
        } else {
            let summary = indexes.summary.as_ref().ok_or_else(|| {
                PersistenceError::InvalidDocument(
                    "an accepted event append requires a discoverability summary".to_owned(),
                )
            })?;
            if &summary.run != receipt.run() || summary.through_sequence != resulting_sequence {
                return Err(PersistenceError::InvalidDocument(
                    "summary run/sequence must match the resulting journal head".to_owned(),
                ));
            }
            validate_index_sequences(&indexes, receipt.run(), resulting_sequence)?;
        }

        Ok(Self {
            receipt,
            events,
            workspace,
            workspace_accounting,
            required_artifacts,
            newly_referenced_artifacts,
            expected_lease_revision,
            controller_account: None,
            projection_checkpoint: None,
            result,
            indexes,
        })
    }

    /// Binds a future optional snapshot's payload to this accepted append.
    ///
    /// Storage commits this evidence with the events. A later [`crate::SnapshotStore`]
    /// write can fail without undoing the command; the commitment is not the payload.
    /// Returns [`PersistenceError::InvalidDocument`] for rejection or a second attachment.
    pub fn with_projection_checkpoint(
        mut self,
        checkpoint: crate::ProjectionCheckpoint,
    ) -> Result<Self, PersistenceError> {
        if self.events.is_empty() {
            return Err(PersistenceError::InvalidDocument(
                "a rejected command cannot carry a projection checkpoint".to_owned(),
            ));
        }
        if self.projection_checkpoint.is_some() {
            return Err(PersistenceError::InvalidDocument(
                "an atomic command cannot replace its projection checkpoint".to_owned(),
            ));
        }
        self.projection_checkpoint = Some(checkpoint);
        Ok(self)
    }

    /// Includes a controller-account transition in the same commit as its run events.
    ///
    /// Storage checks current account state and events before committing both, so entry
    /// intent cannot become visible without its reservation. This builder only attaches
    /// the transaction; account-specific validation remains with its constructor and store.
    /// Returns [`PersistenceError::InvalidDocument`] for rejection or a second attachment.
    pub fn with_controller_account_transaction(
        mut self,
        transaction: crate::ControllerAccountTransaction,
    ) -> Result<Self, PersistenceError> {
        if self.events.is_empty() {
            return Err(PersistenceError::InvalidDocument(
                "a rejected command cannot mutate a controller account".to_owned(),
            ));
        }
        if self.controller_account.is_some() {
            return Err(PersistenceError::InvalidDocument(
                "an atomic command cannot replace its controller-account transaction".to_owned(),
            ));
        }
        self.controller_account = Some(transaction);
        Ok(self)
    }

    /// Validated command receipt.
    #[must_use]
    pub const fn receipt(&self) -> &CommandReceipt {
        &self.receipt
    }

    /// Contiguous event append.
    #[must_use]
    pub fn events(&self) -> &[RunEventEnvelope] {
        &self.events
    }

    /// Workspace mutations committed with the event append.
    #[must_use]
    pub fn workspace(&self) -> &[WorkspaceMutation] {
        &self.workspace
    }

    /// Validated workspace accounting transition, when applicable.
    #[must_use]
    pub const fn workspace_accounting(&self) -> Option<&WorkspaceAccounting> {
        self.workspace_accounting.as_ref()
    }

    /// Every artifact referenced by this commit.
    #[must_use]
    pub fn required_artifacts(&self) -> &[ArtifactReference] {
        &self.required_artifacts
    }

    /// Artifacts newly admitted to this run's accounting domain.
    #[must_use]
    pub fn newly_referenced_artifacts(&self) -> &[ArtifactReference] {
        &self.newly_referenced_artifacts
    }

    /// Active-lease revision required for a lease-creating commit.
    #[must_use]
    pub const fn expected_lease_revision(&self) -> Option<&IntegrityDigest> {
        self.expected_lease_revision.as_ref()
    }

    /// Controller-account mutation committed with this command, when required.
    #[must_use]
    pub const fn controller_account_transaction(
        &self,
    ) -> Option<&crate::ControllerAccountTransaction> {
        self.controller_account.as_ref()
    }

    /// Projection payload commitment recorded at the resulting journal head, when requested.
    #[must_use]
    pub const fn projection_checkpoint(&self) -> Option<&crate::ProjectionCheckpoint> {
        self.projection_checkpoint.as_ref()
    }

    /// Durable command result returned on redelivery.
    #[must_use]
    pub const fn result(&self) -> &CommandResultDocument {
        &self.result
    }

    /// Derived, verifiable index transition.
    #[must_use]
    pub const fn indexes(&self) -> &RunIndexUpdate {
        &self.indexes
    }
}

fn validate_index_sequences(
    indexes: &RunIndexUpdate,
    run: &RunId,
    through: RunSequence,
) -> Result<(), PersistenceError> {
    let runnable_unique = indexes
        .runnable
        .iter()
        .map(|mutation| match mutation {
            RunnableIndexMutation::Upsert { entry } => (&entry.run, &entry.execution),
            RunnableIndexMutation::Remove {
                run: entry_run,
                execution,
            } => (entry_run, execution),
        })
        .collect::<BTreeSet<_>>()
        .len()
        == indexes.runnable.len();
    let timers_unique = indexes
        .timers
        .iter()
        .map(|mutation| match mutation {
            TimerIndexMutation::Upsert { entry } => (&entry.run, &entry.timer),
            TimerIndexMutation::Remove {
                run: entry_run,
                timer,
            } => (entry_run, timer),
        })
        .collect::<BTreeSet<_>>()
        .len()
        == indexes.timers.len();
    let leases_unique = indexes
        .leases
        .iter()
        .map(|mutation| match mutation {
            LeaseIndexMutation::Upsert { entry } => (&entry.run, &entry.lease),
            LeaseIndexMutation::Remove {
                run: entry_run,
                lease,
            } => (entry_run, lease),
        })
        .collect::<BTreeSet<_>>()
        .len()
        == indexes.leases.len();
    if !(runnable_unique && timers_unique && leases_unique) {
        return Err(PersistenceError::InvalidDocument(
            "one atomic commit cannot mutate the same derived index identity more than once"
                .to_owned(),
        ));
    }
    let runnable_valid = indexes.runnable.iter().all(|mutation| match mutation {
        RunnableIndexMutation::Upsert { entry } => {
            &entry.run == run && entry.through_sequence == through
        }
        RunnableIndexMutation::Remove { run: entry_run, .. } => entry_run == run,
    });
    let timers_valid = indexes.timers.iter().all(|mutation| match mutation {
        TimerIndexMutation::Upsert { entry } => {
            &entry.run == run && entry.through_sequence == through
        }
        TimerIndexMutation::Remove { run: entry_run, .. } => entry_run == run,
    });
    let leases_valid = indexes.leases.iter().all(|mutation| match mutation {
        LeaseIndexMutation::Upsert { entry } => {
            &entry.run == run && entry.through_sequence == through
        }
        LeaseIndexMutation::Remove { run: entry_run, .. } => entry_run == run,
    });
    if !(runnable_valid && timers_valid && leases_valid) {
        return Err(PersistenceError::InvalidDocument(
            "every index mutation must belong to the run and resulting sequence".to_owned(),
        ));
    }
    Ok(())
}

/// Whether storage saved a new command result or returned an existing one.
///
/// Both variants contain the original result, which may be accepted or rejected.
/// Neither reports completion of external work. In particular, `Committed` with a
/// rejected [`CommandDisposition`] saved a refusal without appending run events.
#[derive(Clone, Debug, PartialEq)]
pub enum AtomicRunCommitOutcome {
    /// A previously unseen command's receipt and result were saved with all its changes.
    Committed(CommandResultDocument),
    /// The receipt matched a saved command; its original result is returned without writes.
    /// The returned sequence may be older than the run's current head.
    Replayed(CommandResultDocument),
}

impl AtomicRunCommitOutcome {
    /// Returns the original durable result for either first delivery or replay.
    #[must_use]
    pub const fn result(&self) -> &CommandResultDocument {
        match self {
            Self::Committed(result) | Self::Replayed(result) => result,
        }
    }
}

/// Saves runtime command decisions and retrieves their durable results.
///
/// Runtime command handling calls this synchronous port after planning a transition.
/// Implementers own storage transactions and integrity checks; they do not authorize
/// commands, choose runtime transitions, or invoke capabilities. Sharing a port across
/// threads must preserve the same atomicity and optimistic guards.
///
/// # Implementation obligations
///
/// For `(run, command)`, validate any saved receipt/result and its supporting history,
/// then compare the incoming fingerprint. An exact match returns the saved result
/// without writes or testing the now-stale expected sequence; a mismatch returns
/// [`PersistenceError::IdempotencyConflict`]. Replay must not bypass integrity checks.
///
/// For a new command, compare the run head and applicable workspace, lease, and account
/// guards inside the write transaction. Validate committed artifact references and
/// saved scope/value provenance. Commit the receipt/result, contiguous events and
/// history integrity evidence, run head, workspace/accounting, artifact-reference
/// membership, and summary/runnable/timer/lease updates together. Include attached
/// account transitions and projection commitments. Readers must never observe only
/// part of that change. A runtime rejection saves only its receipt/result.
///
/// # Recovery after an error
///
/// A storage failure can be reported after a transaction commits. Preserve the command
/// identity and intent, then query [`Self::command_result`] or redeliver through
/// [`Self::commit_command`]. Do not infer absence from a failed read or generate a fresh
/// command identity to escape an unknown outcome. This resolves the saved command
/// result; retrying external work still requires the runtime's attempt/recovery rules.
pub trait RunJournal: Send + Sync {
    /// Persists the runtime's decision according to this trait's atomicity/replay contract.
    ///
    /// Sequence, usage, lease, or account conflicts require rereading state and replanning.
    /// Idempotency conflicts preserve the saved result. Missing artifacts, invalid stored
    /// relationships, corruption, and storage failures also refuse the operation.
    /// Follow the trait's recovery guidance: an error can arrive after commit.
    fn commit_command(
        &self,
        request: &AtomicRunCommitRequest,
    ) -> Result<AtomicRunCommitOutcome, PersistenceError>;

    /// Returns the verified journal head, or zero when the run has no events.
    ///
    /// A rejected command can have a saved result at sequence zero, so this is not a
    /// command-result lookup. Integrity or storage failure must return an error, not
    /// zero. The observed head can become stale before a subsequent commit.
    fn head(&self, run: &RunId) -> Result<RunSequence, PersistenceError>;

    /// Reads the saved result for `(run, command)`, including durable rejections.
    ///
    /// Compare its fingerprint with the intended receipt before treating it as that
    /// request's answer: this lookup takes no intent to compare. `None` means no result
    /// was found at this read, not that external work could not have happened. Storage
    /// must validate the record and its history links; corrupt or unreadable evidence
    /// returns an error rather than `None`.
    fn command_result(
        &self,
        run: &RunId,
        command: &CommandId,
    ) -> Result<Option<CommandResultDocument>, PersistenceError>;
}
