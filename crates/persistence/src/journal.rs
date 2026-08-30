use std::collections::{BTreeMap, BTreeSet};

use milkdrift_authority::{ActorRef, AuthorityDecisionSnapshot};
use milkdrift_blueprint::{RevisionId, WorkflowId};
use milkdrift_capability::BoundedJson;
use milkdrift_workspace::{
    ArtifactReference, RunId, ScopeId, ScopeReference, ValueKey, WorkspaceBudget, WorkspaceScope,
    WorkspaceUsage, WorkspaceValueEntry, WorkspaceValueReference,
};
use serde::{Deserialize, Serialize};

use crate::{
    AttemptId, CommandId, IntegrityDigest, LeaseId, NodeExecutionId, PageSize, PersistenceError,
    RunEventEnvelope, RunEventKind, RunSequence, SignalId, TimerId, TimestampMillis, WorkerId,
    bounded::MAX_EVENTS_PER_COMMIT,
};

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

/// Exact canonical runtime command receipt used for durable idempotency.
///
/// Persistence does not interpret runtime transitions. It retains the complete audit
/// document, the canonical semantic intent, and their derived fingerprint so a repeated
/// [`CommandId`] can be proven identical or conflicting across delivery retries.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CommandReceipt {
    command: CommandId,
    run: RunId,
    actor: ActorRef,
    expected_sequence: RunSequence,
    submitted_at: TimestampMillis,
    canonical_document: Vec<u8>,
    canonical_intent: Vec<u8>,
    fingerprint: IntegrityDigest,
}

fn validate_canonical_command_bytes(
    location: &'static str,
    bytes: &[u8],
) -> Result<(), PersistenceError> {
    if bytes.is_empty() || bytes.len() > MAX_COMMAND_DOCUMENT_BYTES {
        return Err(PersistenceError::Bounds {
            location,
            reason: format!(
                "must contain 1..={MAX_COMMAND_DOCUMENT_BYTES} canonical document bytes"
            ),
        });
    }
    let value: serde_json::Value = serde_json::from_slice(bytes)?;
    let canonical = crate::document::canonical_json_bytes(&value, MAX_COMMAND_DOCUMENT_BYTES)?;
    if canonical != bytes {
        return Err(PersistenceError::InvalidDocument(format!(
            "{location} bytes must be canonical compact key-sorted JSON"
        )));
    }
    Ok(())
}

impl CommandReceipt {
    /// Constructs a receipt from a runtime-owned canonical command document.
    pub fn new(
        command: CommandId,
        run: RunId,
        actor: ActorRef,
        expected_sequence: RunSequence,
        submitted_at: TimestampMillis,
        canonical_document: Vec<u8>,
    ) -> Result<Self, PersistenceError> {
        Self::new_idempotent(
            command,
            run,
            actor,
            expected_sequence,
            submitted_at,
            canonical_document.clone(),
            canonical_document,
        )
    }

    /// Constructs a receipt whose idempotency fingerprint is bound to a canonical semantic
    /// intent document rather than retry-local delivery metadata.
    ///
    /// The complete command document is still retained for audit. Runtime callers use this
    /// constructor so an exact command can be retried at a newer optimistic sequence or
    /// timestamp without turning the same [`CommandId`] into a false conflict.
    #[allow(clippy::too_many_arguments)] // One validated durable document keeps its complete storage facts explicit.
    pub fn new_idempotent(
        command: CommandId,
        run: RunId,
        actor: ActorRef,
        expected_sequence: RunSequence,
        submitted_at: TimestampMillis,
        canonical_document: Vec<u8>,
        canonical_intent: Vec<u8>,
    ) -> Result<Self, PersistenceError> {
        validate_canonical_command_bytes("command.document", &canonical_document)?;
        validate_canonical_command_bytes("command.intent", &canonical_intent)?;

        let mut hasher = blake3::Hasher::new();
        hasher.update(b"milkdrift.command-receipt.semantic.v1\0");
        for component in [
            command.as_str().as_bytes(),
            run.as_str().as_bytes(),
            actor.as_str().as_bytes(),
            canonical_intent.as_slice(),
        ] {
            let length = u32::try_from(component.len()).map_err(|_| PersistenceError::Bounds {
                location: "command.receipt_component",
                reason: "component length does not fit u32".to_owned(),
            })?;
            hasher.update(&length.to_be_bytes());
            hasher.update(component);
        }
        let fingerprint = IntegrityDigest::new(format!("b3_{}", hasher.finalize()))?;
        Ok(Self {
            command,
            run,
            actor,
            expected_sequence,
            submitted_at,
            canonical_document,
            canonical_intent,
            fingerprint,
        })
    }

    /// Command/idempotency identity.
    #[must_use]
    pub const fn command(&self) -> &CommandId {
        &self.command
    }

    /// Owning aggregate.
    #[must_use]
    pub const fn run(&self) -> &RunId {
        &self.run
    }

    /// Issuer reference retained with the command.
    #[must_use]
    pub const fn actor(&self) -> &ActorRef {
        &self.actor
    }

    /// Optimistic guard supplied by the runtime.
    #[must_use]
    pub const fn expected_sequence(&self) -> RunSequence {
        self.expected_sequence
    }

    /// Boundary-clock receipt timestamp.
    #[must_use]
    pub const fn submitted_at(&self) -> TimestampMillis {
        self.submitted_at
    }

    /// Runtime-owned canonical command bytes retained as exact audit evidence.
    #[must_use]
    pub fn canonical_document(&self) -> &[u8] {
        &self.canonical_document
    }

    /// Canonical semantic intent bytes that own idempotency across delivery retries.
    ///
    /// These bytes are persisted separately from the complete audit document so storage
    /// can reconstruct and validate the semantic fingerprint without interpreting runtime
    /// fields such as the optimistic sequence or delivery timestamp.
    #[must_use]
    pub fn canonical_intent(&self) -> &[u8] {
        &self.canonical_intent
    }

    /// Domain-separated command fingerprint.
    #[must_use]
    pub const fn fingerprint(&self) -> &IntegrityDigest {
        &self.fingerprint
    }
}

/// Whether runtime transition validation accepted or rejected a command.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CommandDisposition {
    /// The command emitted one or more semantic events.
    Accepted,
    /// Runtime validation rejected it without semantic events.
    Rejected,
}

/// Fully durable typed result returned for exact command redelivery.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CommandResultDocument {
    schema_version: u32,
    command: CommandId,
    run: RunId,
    command_fingerprint: IntegrityDigest,
    disposition: CommandDisposition,
    resulting_sequence: RunSequence,
    event_ids: Vec<crate::EventId>,
    result: BoundedJson,
    #[serde(skip_serializing_if = "Option::is_none")]
    authorization: Option<AuthorityDecisionSnapshot>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CommandResultWire {
    schema_version: u32,
    command: CommandId,
    run: RunId,
    command_fingerprint: IntegrityDigest,
    disposition: CommandDisposition,
    resulting_sequence: RunSequence,
    event_ids: Vec<crate::EventId>,
    result: BoundedJson,
    #[serde(default)]
    authorization: Option<AuthorityDecisionSnapshot>,
}

impl<'de> Deserialize<'de> for CommandResultDocument {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let wire = CommandResultWire::deserialize(deserializer)?;
        if !matches!(
            wire.schema_version,
            COMMAND_RESULT_SCHEMA_VERSION_V1 | COMMAND_RESULT_SCHEMA_VERSION_V2
        ) {
            return Err(serde::de::Error::custom(format!(
                "unsupported command_result schema version {}; supported version is {}",
                wire.schema_version, COMMAND_RESULT_SCHEMA_VERSION_V2
            )));
        }
        Self::build(
            wire.schema_version,
            wire.command,
            wire.run,
            wire.command_fingerprint,
            wire.disposition,
            wire.resulting_sequence,
            wire.event_ids,
            wire.result,
            wire.authorization,
        )
        .map_err(serde::de::Error::custom)
    }
}

impl CommandResultDocument {
    /// Constructs the exact result retained by the idempotency record.
    #[allow(clippy::too_many_arguments)] // One validated durable document keeps its complete storage facts explicit.
    pub fn new(
        command: CommandId,
        run: RunId,
        command_fingerprint: IntegrityDigest,
        disposition: CommandDisposition,
        resulting_sequence: RunSequence,
        event_ids: Vec<crate::EventId>,
        result: BoundedJson,
    ) -> Result<Self, PersistenceError> {
        Self::build(
            COMMAND_RESULT_SCHEMA_VERSION_V1,
            command,
            run,
            command_fingerprint,
            disposition,
            resulting_sequence,
            event_ids,
            result,
            None,
        )
    }

    /// Constructs an authorization-bearing external command result.
    #[allow(clippy::too_many_arguments)] // One validated durable document keeps its complete storage facts explicit.
    pub fn new_authorized(
        command: CommandId,
        run: RunId,
        command_fingerprint: IntegrityDigest,
        disposition: CommandDisposition,
        resulting_sequence: RunSequence,
        event_ids: Vec<crate::EventId>,
        result: BoundedJson,
        authorization: AuthorityDecisionSnapshot,
    ) -> Result<Self, PersistenceError> {
        Self::build(
            COMMAND_RESULT_SCHEMA_VERSION_V2,
            command,
            run,
            command_fingerprint,
            disposition,
            resulting_sequence,
            event_ids,
            result,
            Some(authorization),
        )
    }

    #[allow(clippy::too_many_arguments)] // One validated durable document keeps its complete storage facts explicit.
    fn build(
        schema_version: u32,
        command: CommandId,
        run: RunId,
        command_fingerprint: IntegrityDigest,
        disposition: CommandDisposition,
        resulting_sequence: RunSequence,
        event_ids: Vec<crate::EventId>,
        result: BoundedJson,
        authorization: Option<AuthorityDecisionSnapshot>,
    ) -> Result<Self, PersistenceError> {
        if (schema_version == COMMAND_RESULT_SCHEMA_VERSION_V2) != authorization.is_some() {
            return Err(PersistenceError::InvalidDocument(
                "command-result schema v2 requires authorization and v1 forbids it".to_owned(),
            ));
        }
        if event_ids.len() > MAX_EVENTS_PER_COMMIT {
            return Err(PersistenceError::Bounds {
                location: "command_result.event_ids",
                reason: format!("at most {MAX_EVENTS_PER_COMMIT} event identities are allowed"),
            });
        }
        if matches!(disposition, CommandDisposition::Accepted) == event_ids.is_empty() {
            return Err(PersistenceError::InvalidDocument(
                "accepted command results require events and rejected results require none"
                    .to_owned(),
            ));
        }
        let mut unique = BTreeSet::new();
        if !event_ids.iter().all(|event| unique.insert(event)) {
            return Err(PersistenceError::InvalidDocument(
                "a command result cannot contain duplicate event identities".to_owned(),
            ));
        }
        Ok(Self {
            schema_version,
            command,
            run,
            command_fingerprint,
            disposition,
            resulting_sequence,
            event_ids,
            result,
            authorization,
        })
    }

    /// Document schema.
    #[must_use]
    pub const fn schema_version(&self) -> u32 {
        self.schema_version
    }

    /// Command identity.
    #[must_use]
    pub const fn command(&self) -> &CommandId {
        &self.command
    }

    /// Aggregate identity.
    #[must_use]
    pub const fn run(&self) -> &RunId {
        &self.run
    }

    /// Fingerprint that must match a redelivered receipt.
    #[must_use]
    pub const fn command_fingerprint(&self) -> &IntegrityDigest {
        &self.command_fingerprint
    }

    /// Accepted/rejected disposition.
    #[must_use]
    pub const fn disposition(&self) -> CommandDisposition {
        self.disposition
    }

    /// Authoritative journal sequence after the original result.
    #[must_use]
    pub const fn resulting_sequence(&self) -> RunSequence {
        self.resulting_sequence
    }

    /// Event identities emitted by the command, in sequence order.
    #[must_use]
    pub fn event_ids(&self) -> &[crate::EventId] {
        &self.event_ids
    }

    /// Bounded runtime-owned typed result payload.
    #[must_use]
    pub const fn result(&self) -> &BoundedJson {
        &self.result
    }

    /// Exact immutable external authorization decision, absent only for internal/v1 results.
    #[must_use]
    pub const fn authorization(&self) -> Option<&AuthorityDecisionSnapshot> {
        self.authorization.as_ref()
    }

    /// Serializes recursively key-sorted canonical compact JSON.
    pub fn to_canonical_json(&self) -> Result<Vec<u8>, PersistenceError> {
        crate::document::canonical_json_bytes(self, MAX_COMMAND_RESULT_DOCUMENT_BYTES)
    }

    /// Decodes, version-checks, and revalidates a persisted command result.
    pub fn from_json(bytes: &[u8]) -> Result<Self, PersistenceError> {
        if bytes.len() > MAX_COMMAND_RESULT_DOCUMENT_BYTES {
            return Err(PersistenceError::Bounds {
                location: "command_result.document",
                reason: format!("exceeds {MAX_COMMAND_RESULT_DOCUMENT_BYTES} bytes"),
            });
        }
        let value = crate::document::parse_json_without_duplicates(bytes)?;
        let version = value
            .get("schema_version")
            .and_then(serde_json::Value::as_u64)
            .and_then(|value| u32::try_from(value).ok())
            .ok_or_else(|| {
                PersistenceError::InvalidDocument(
                    "command result requires a numeric u32 schema_version".to_owned(),
                )
            })?;
        if !matches!(
            version,
            COMMAND_RESULT_SCHEMA_VERSION_V1 | COMMAND_RESULT_SCHEMA_VERSION_V2
        ) {
            return Err(PersistenceError::UnsupportedVersion {
                document: "command_result",
                found: version,
                supported: COMMAND_RESULT_SCHEMA_VERSION_V2,
            });
        }
        Ok(serde_json::from_value(value)?)
    }
}

/// Workspace mutation applied in the same transaction as accepted event history.
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

/// Optimistic workspace budget/accounting guard coordinated with a journal commit.
///
/// Artifact content publication and aggregate reference accounting are separate.
/// This guard atomically charges inline immutable value versions and the exact first
/// references admitted by this journal commit; the adapter verifies those references
/// against `AtomicRunCommitRequest::newly_referenced_artifacts`.
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

/// Complete derived index update coordinated with one journal append.
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
    /// Creates one immutable derived-index transition.
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

/// One all-or-nothing command receipt, event, workspace, result, and index commit.
///
/// Implementations must first look up `(run, command)`. An equal fingerprint returns
/// the original result without checking the now-stale optimistic guard or writing;
/// a different fingerprint returns [`PersistenceError::IdempotencyConflict`]. Only a
/// previously unseen command checks `expected_sequence` and performs the transaction.
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
    ///
    /// Present exactly when this commit contains a `LeaseGranted` or `NodeReLeased`
    /// fact. The adapter compares it atomically before the lease/index mutation so
    /// concurrent runtime services cannot both admit against the same pre-lease
    /// global usage.
    expected_lease_revision: Option<IntegrityDigest>,
    /// Optional projection payload commitment derived from the accepted resulting state.
    projection_checkpoint: Option<crate::ProjectionCheckpoint>,
    /// Fully durable result returned on redelivery.
    result: CommandResultDocument,
    /// Derived, verifiable discovery/index changes committed atomically.
    indexes: RunIndexUpdate,
}

impl AtomicRunCommitRequest {
    /// Validates cross-document atomicity and sequence invariants.
    #[allow(clippy::too_many_arguments)] // One validated durable document keeps its complete storage facts explicit.
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
            projection_checkpoint: None,
            result,
            indexes,
        })
    }

    /// Attaches a projection commitment to the accepted resulting journal head.
    ///
    /// Storage must persist this in the same transaction as the event append. Rejected
    /// commands cannot carry a projection commitment.
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

/// Result of the one atomic durable command operation.
#[derive(Clone, Debug, PartialEq)]
pub enum AtomicRunCommitOutcome {
    /// A previously unseen command was committed.
    Committed(CommandResultDocument),
    /// An exactly matching command was redelivered; no bytes were changed.
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

/// Narrow synchronous object-safe append/idempotency port.
pub trait RunJournal: Send + Sync {
    /// Atomically accepts/rejects one command and coordinates every durable consequence.
    ///
    /// Receipt, contiguous event append, command result, workspace mutations,
    /// required-artifact validation, run summary, and runnable/timer/lease indexes are
    /// one crash-atomic call. Implementations must never expose an accepted event that
    /// references absent workspace state or uncommitted artifact content.
    fn commit_command(
        &self,
        request: &AtomicRunCommitRequest,
    ) -> Result<AtomicRunCommitOutcome, PersistenceError>;

    /// Returns the sole authoritative aggregate sequence, or zero when absent.
    fn head(&self, run: &RunId) -> Result<RunSequence, PersistenceError>;

    /// Returns a prior exact idempotency result when present.
    fn command_result(
        &self,
        run: &RunId,
        command: &CommandId,
    ) -> Result<Option<CommandResultDocument>, PersistenceError>;
}

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

    /// Compatibility shorthand for the first nonterminal page.
    fn nonterminal_runs(&self, limit: PageSize) -> Result<Vec<RunSummaryIndex>, PersistenceError> {
        Ok(self.nonterminal_run_page(None, limit)?.runs)
    }

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

    /// Compatibility shorthand for the first fair runnable page.
    fn runnable(
        &self,
        eligible_through: TimestampMillis,
        limit: PageSize,
    ) -> Result<Vec<RunnableIndexEntry>, PersistenceError> {
        Ok(self.runnable_page(eligible_through, None, limit)?.entries)
    }

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
/// [`RunJournal::commit_command`] to preserve crash atomicity with event history.
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
