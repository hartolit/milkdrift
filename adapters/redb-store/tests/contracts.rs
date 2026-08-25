//! End-to-end durability, conflict, corruption, and artifact contract tests.

#[path = "contracts/artifact.rs"]
mod artifact;
#[path = "contracts/discovery_snapshot_revision.rs"]
mod discovery_snapshot_revision;
#[path = "contracts/event_page.rs"]
mod event_page;
#[path = "contracts/fault_reopen.rs"]
mod fault_reopen;
#[path = "contracts/journal_workspace.rs"]
mod journal_workspace;

use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use milkdrift_blueprint::{
    BlueprintRevisionDocument, ContentDigest as BlueprintContentDigest, NodeId, PortId, RevisionId,
    WorkflowId,
};
use milkdrift_capability::{BoundedJson, CapabilityId, InvocationRequest, OperationId};
use milkdrift_persistence::{
    ActorRef, ArtifactPublicationId, ArtifactReadAuthority, ArtifactReadRequest, ArtifactStore,
    AtomicRunCommitOutcome, AtomicRunCommitRequest, AttemptId, BeginArtifactOutcome,
    BeginArtifactPublication, CommandDisposition, CommandId, CommandReceipt, CommandResultDocument,
    EventId, EventPageQuery, ImmutableRevisionPut, IndexedRunState, IntegrityScanRequest, LeaseId,
    LeaseIndexEntry, LeaseIndexMutation, NodeExecutionId, OrphanCleanupRequest, PageSize,
    PersistenceError, RevisionStore, RunDiscoveryIntegrityStore, RunEventEnvelope, RunEventKind,
    RunIndexUpdate, RunJournal, RunQueryStore, RunSequence, RunSummaryFilter, RunSummaryIndex,
    RunSummaryPageQuery, RunnableIndexEntry, RunnableIndexMutation, SignalDeliveryMode, SignalId,
    SignalTypeId, SnapshotDocument, SnapshotId, SnapshotLoad, SnapshotStore, StorageAdmin,
    StorageFailureClass, StorageHealthStatus, TimerId, TimerIndexEntry, TimerIndexMutation,
    TimestampMillis, WorkerId, WorkspaceAccounting, WorkspaceMutation, WorkspaceStore,
    history_digest,
};
use milkdrift_redb_store::{
    ArtifactClock, FaultInjector, FaultPoint, RedbStore, RedbStoreConfig, injected_failure,
};
use milkdrift_workspace::{
    ArtifactId, ArtifactMetadata, ArtifactProvenance, ArtifactRetention, ArtifactSensitivity,
    BranchId, CausalId, CausalReference, ContentDigest, MediaType, RunId, ScopeId, ValueKey,
    WorkspaceBudget, WorkspaceScope, WorkspaceUsage, WorkspaceValue, WorkspaceValueEntry,
    WorkspaceValueReference,
};
use redb::{Database, ReadableTable, TableDefinition};
use serde_json::json;
use tempfile::TempDir;

fn revision_id() -> Result<RevisionId, PersistenceError> {
    serde_json::from_value(json!(format!("rev_{}", "0".repeat(64)))).map_err(PersistenceError::Json)
}

fn revision_digest() -> Result<BlueprintContentDigest, PersistenceError> {
    serde_json::from_value(json!(format!("b3_{}", "0".repeat(64)))).map_err(PersistenceError::Json)
}

fn stored_workspace_value_key(
    reference: &WorkspaceValueReference,
) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    let mut encoded = Vec::new();
    for component in [
        reference.scope().run().as_str(),
        reference.scope().scope().as_str(),
        reference.key().as_str(),
    ] {
        encoded.extend_from_slice(&u32::try_from(component.len())?.to_be_bytes());
        encoded.extend_from_slice(component.as_bytes());
    }
    encoded.extend_from_slice(&reference.version().get().to_be_bytes());
    Ok(encoded)
}

fn stored_workspace_scope_key(
    scope: &milkdrift_workspace::ScopeReference,
) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    let mut encoded = Vec::new();
    for component in [scope.run().as_str(), scope.scope().as_str()] {
        encoded.extend_from_slice(&u32::try_from(component.len())?.to_be_bytes());
        encoded.extend_from_slice(component.as_bytes());
    }
    Ok(encoded)
}

fn assert_storage_corruption<T: std::fmt::Debug>(result: Result<T, PersistenceError>) {
    assert!(
        matches!(
            &result,
            Err(PersistenceError::Storage {
                class: StorageFailureClass::Corruption,
                ..
            }) | Err(PersistenceError::Corruption(_))
        ),
        "expected storage corruption, got {result:?}"
    );
}

fn accepted_request(
    run: &str,
    command: &str,
    event: &str,
    command_type: &str,
) -> Result<AtomicRunCommitRequest, Box<dyn std::error::Error>> {
    let run = RunId::new(run)?;
    let command = CommandId::new(command)?;
    let document = format!(r#"{{"schema_version":1,"type":"{command_type}"}}"#).into_bytes();
    let receipt = CommandReceipt::new(
        command.clone(),
        run.clone(),
        ActorRef::new("actor-test")?,
        RunSequence::ZERO,
        TimestampMillis::new(10),
        document,
    )?;
    let event = RunEventEnvelope::new(
        EventId::new(event)?,
        run.clone(),
        RunSequence::FIRST,
        TimestampMillis::new(10),
        RunEventKind::RunStarted,
    )?;
    let result = CommandResultDocument::new(
        command,
        run.clone(),
        receipt.fingerprint().clone(),
        CommandDisposition::Accepted,
        RunSequence::FIRST,
        vec![event.event_id().clone()],
        BoundedJson::new(json!({"accepted": true}))?,
    )?;
    let accounting = WorkspaceAccounting {
        budget: WorkspaceBudget::new(0, 0, 0, 0, 0, 0)?,
        expected_usage: WorkspaceUsage::EMPTY,
        resulting_usage: WorkspaceUsage::EMPTY,
    };
    Ok(AtomicRunCommitRequest::new(
        receipt,
        vec![event],
        Vec::new(),
        Some(accounting),
        Vec::new(),
        Vec::new(),
        None,
        result,
        RunIndexUpdate::new(
            Some(RunSummaryIndex {
                run,
                workflow: WorkflowId::new("workflow-test")?,
                revision: revision_id()?,
                state: IndexedRunState::Active,
                through_sequence: RunSequence::FIRST,
                updated_at: TimestampMillis::new(10),
            }),
            Vec::new(),
            Vec::new(),
            Vec::new(),
        ),
    )?)
}

fn accepted_followup_request(
    run: RunId,
    command: &str,
    event: &str,
) -> Result<AtomicRunCommitRequest, Box<dyn std::error::Error>> {
    accepted_sequenced_followup_request(
        run,
        command,
        event,
        RunSequence::FIRST,
        RunSequence::new(2),
        TimestampMillis::new(11),
    )
}

fn accepted_sequenced_followup_request(
    run: RunId,
    command: &str,
    event: &str,
    expected_sequence: RunSequence,
    sequence: RunSequence,
    timestamp: TimestampMillis,
) -> Result<AtomicRunCommitRequest, Box<dyn std::error::Error>> {
    if expected_sequence.next()? != sequence {
        return Err(PersistenceError::InvalidDocument(
            "test follow-up request must append exactly one contiguous event".to_owned(),
        )
        .into());
    }
    let command = CommandId::new(command)?;
    let document = br#"{"schema_version":1,"type":"followup"}"#.to_vec();
    let receipt = CommandReceipt::new(
        command.clone(),
        run.clone(),
        ActorRef::new("actor-test")?,
        expected_sequence,
        timestamp,
        document,
    )?;
    let event = RunEventEnvelope::new(
        EventId::new(event)?,
        run.clone(),
        sequence,
        timestamp,
        RunEventKind::RunStarted,
    )?;
    let result = CommandResultDocument::new(
        command,
        run.clone(),
        receipt.fingerprint().clone(),
        CommandDisposition::Accepted,
        sequence,
        vec![event.event_id().clone()],
        BoundedJson::new(json!({"accepted": true}))?,
    )?;
    Ok(AtomicRunCommitRequest::new(
        receipt,
        vec![event],
        Vec::new(),
        Some(WorkspaceAccounting {
            budget: WorkspaceBudget::new(0, 0, 0, 0, 0, 0)?,
            expected_usage: WorkspaceUsage::EMPTY,
            resulting_usage: WorkspaceUsage::EMPTY,
        }),
        Vec::new(),
        Vec::new(),
        None,
        result,
        RunIndexUpdate::new(
            Some(RunSummaryIndex {
                run,
                workflow: WorkflowId::new("workflow-test")?,
                revision: revision_id()?,
                state: IndexedRunState::Active,
                through_sequence: sequence,
                updated_at: timestamp,
            }),
            Vec::new(),
            Vec::new(),
            Vec::new(),
        ),
    )?)
}

fn accepted_workspace_request(
    run: RunId,
    command: &str,
    event_prefix: &str,
    kinds: Vec<RunEventKind>,
    workspace: Vec<WorkspaceMutation>,
    accounting: WorkspaceAccounting,
) -> Result<AtomicRunCommitRequest, Box<dyn std::error::Error>> {
    let command = CommandId::new(command)?;
    let receipt = CommandReceipt::new(
        command.clone(),
        run.clone(),
        ActorRef::new("actor-test")?,
        RunSequence::ZERO,
        TimestampMillis::new(10),
        br#"{"schema_version":1,"type":"workspace-test"}"#.to_vec(),
    )?;
    let events = kinds
        .into_iter()
        .enumerate()
        .map(|(index, kind)| {
            RunEventEnvelope::new(
                EventId::new(format!("{event_prefix}-{index}"))?,
                run.clone(),
                RunSequence::new(index as u64 + 1),
                TimestampMillis::new(10),
                kind,
            )
        })
        .collect::<Result<Vec<_>, PersistenceError>>()?;
    let resulting_sequence = RunSequence::new(events.len() as u64);
    let result = CommandResultDocument::new(
        command,
        run.clone(),
        receipt.fingerprint().clone(),
        CommandDisposition::Accepted,
        resulting_sequence,
        events
            .iter()
            .map(|event| event.event_id().clone())
            .collect(),
        BoundedJson::new(json!({"accepted": true}))?,
    )?;
    Ok(AtomicRunCommitRequest::new(
        receipt,
        events,
        workspace,
        Some(accounting),
        Vec::new(),
        Vec::new(),
        None,
        result,
        RunIndexUpdate::new(
            Some(RunSummaryIndex {
                run,
                workflow: WorkflowId::new("workflow-test")?,
                revision: revision_id()?,
                state: IndexedRunState::Active,
                through_sequence: resulting_sequence,
                updated_at: TimestampMillis::new(10),
            }),
            Vec::new(),
            Vec::new(),
            Vec::new(),
        ),
    )?)
}

fn accepted_workspace_followup_request(
    run: RunId,
    expected_sequence: RunSequence,
    command: &str,
    event_prefix: &str,
    kinds: Vec<RunEventKind>,
    workspace: Vec<WorkspaceMutation>,
    accounting: WorkspaceAccounting,
) -> Result<AtomicRunCommitRequest, Box<dyn std::error::Error>> {
    let command = CommandId::new(command)?;
    let receipt = CommandReceipt::new(
        command.clone(),
        run.clone(),
        ActorRef::new("actor-test")?,
        expected_sequence,
        TimestampMillis::new(11),
        br#"{"schema_version":1,"type":"workspace-followup-test"}"#.to_vec(),
    )?;
    let events = kinds
        .into_iter()
        .enumerate()
        .map(|(index, kind)| {
            let offset = u64::try_from(index)?
                .checked_add(1)
                .ok_or("event offset overflow")?;
            let sequence = expected_sequence
                .get()
                .checked_add(offset)
                .ok_or("event sequence overflow")?;
            Ok(RunEventEnvelope::new(
                EventId::new(format!("{event_prefix}-{index}"))?,
                run.clone(),
                RunSequence::new(sequence),
                TimestampMillis::new(11),
                kind,
            )?)
        })
        .collect::<Result<Vec<_>, Box<dyn std::error::Error>>>()?;
    let resulting_sequence = events
        .last()
        .map_or(expected_sequence, RunEventEnvelope::sequence);
    let result = CommandResultDocument::new(
        command,
        run.clone(),
        receipt.fingerprint().clone(),
        CommandDisposition::Accepted,
        resulting_sequence,
        events
            .iter()
            .map(|event| event.event_id().clone())
            .collect(),
        BoundedJson::new(json!({"accepted": true}))?,
    )?;
    Ok(AtomicRunCommitRequest::new(
        receipt,
        events,
        workspace,
        Some(accounting),
        Vec::new(),
        Vec::new(),
        None,
        result,
        RunIndexUpdate::new(
            Some(RunSummaryIndex {
                run,
                workflow: WorkflowId::new("workflow-test")?,
                revision: revision_id()?,
                state: IndexedRunState::Active,
                through_sequence: resulting_sequence,
                updated_at: TimestampMillis::new(11),
            }),
            Vec::new(),
            Vec::new(),
            Vec::new(),
        ),
    )?)
}

fn run_created_kind(
    root_scope: WorkspaceScope,
    workspace_budget: WorkspaceBudget,
    inputs: Vec<milkdrift_workspace::WorkspaceValueReference>,
) -> Result<RunEventKind, Box<dyn std::error::Error>> {
    Ok(RunEventKind::RunCreated {
        workflow: WorkflowId::new("workflow-test")?,
        revision: revision_id()?,
        revision_digest: revision_digest()?,
        root_scope,
        workspace_budget,
        inputs,
    })
}

fn rebuild_request_with_indexes(
    request: &AtomicRunCommitRequest,
    indexes: RunIndexUpdate,
) -> Result<AtomicRunCommitRequest, PersistenceError> {
    AtomicRunCommitRequest::new(
        request.receipt().clone(),
        request.events().to_vec(),
        request.workspace().to_vec(),
        request.workspace_accounting().cloned(),
        request.required_artifacts().to_vec(),
        request.newly_referenced_artifacts().to_vec(),
        request.expected_lease_revision().cloned(),
        request.result().clone(),
        indexes,
    )
}

fn accepted_request_with_runnable(
    run: &str,
    command: &str,
    event: &str,
    entries: Vec<RunnableIndexEntry>,
) -> Result<AtomicRunCommitRequest, Box<dyn std::error::Error>> {
    let request = accepted_request(run, command, event, "start")?;
    let indexes = RunIndexUpdate::new(
        request.indexes().summary().cloned(),
        entries
            .into_iter()
            .map(|entry| RunnableIndexMutation::Upsert { entry })
            .collect(),
        request.indexes().timers().to_vec(),
        request.indexes().leases().to_vec(),
    );
    Ok(rebuild_request_with_indexes(&request, indexes)?)
}

fn accepted_request_with_lease(
    run: &str,
    command: &str,
    event: &str,
    entry: LeaseIndexEntry,
) -> Result<AtomicRunCommitRequest, Box<dyn std::error::Error>> {
    let request = accepted_request(run, command, event, "start")?;
    let indexes = RunIndexUpdate::new(
        request.indexes().summary().cloned(),
        request.indexes().runnable().to_vec(),
        request.indexes().timers().to_vec(),
        vec![LeaseIndexMutation::Upsert { entry }],
    );
    Ok(rebuild_request_with_indexes(&request, indexes)?)
}

#[derive(Clone, Copy)]
enum DiscoveryIndexKind {
    Runnable,
    Timer,
    Lease,
}

fn accepted_request_with_discovery_index(
    kind: DiscoveryIndexKind,
    suffix: &str,
) -> Result<AtomicRunCommitRequest, Box<dyn std::error::Error>> {
    let run_text = format!("run-discovery-{suffix}");
    let command = format!("command-discovery-{suffix}");
    let event = format!("event-discovery-{suffix}");
    let request = accepted_request(&run_text, &command, &event, "start")?;
    let run = request.receipt().run().clone();
    let mut runnable = request.indexes().runnable().to_vec();
    let mut timers = request.indexes().timers().to_vec();
    let mut leases = request.indexes().leases().to_vec();
    match kind {
        DiscoveryIndexKind::Runnable => runnable.push(RunnableIndexMutation::Upsert {
            entry: RunnableIndexEntry {
                run,
                execution: NodeExecutionId::new(format!("execution-{suffix}"))?,
                eligible_at: TimestampMillis::new(10),
                priority: 1,
                through_sequence: RunSequence::FIRST,
            },
        }),
        DiscoveryIndexKind::Timer => timers.push(TimerIndexMutation::Upsert {
            entry: TimerIndexEntry {
                run,
                timer: TimerId::new(format!("timer-{suffix}"))?,
                fire_at: TimestampMillis::new(10),
                through_sequence: RunSequence::FIRST,
            },
        }),
        DiscoveryIndexKind::Lease => leases.push(LeaseIndexMutation::Upsert {
            entry: LeaseIndexEntry {
                run,
                lease: LeaseId::new(format!("lease-{suffix}"))?,
                attempt: AttemptId::new(format!("attempt-{suffix}"))?,
                worker: WorkerId::new("worker-discovery")?,
                expires_at: TimestampMillis::new(10),
                through_sequence: RunSequence::FIRST,
            },
        }),
    }
    let indexes = RunIndexUpdate::new(
        request.indexes().summary().cloned(),
        runnable,
        timers,
        leases,
    );
    Ok(rebuild_request_with_indexes(&request, indexes)?)
}

struct FailOnce {
    point: FaultPoint,
    remaining: AtomicUsize,
}

impl FailOnce {
    fn new(point: FaultPoint) -> Self {
        Self {
            point,
            remaining: AtomicUsize::new(1),
        }
    }
}

impl FaultInjector for FailOnce {
    fn check(&self, point: FaultPoint) -> Result<(), PersistenceError> {
        if point == self.point && self.remaining.swap(0, Ordering::SeqCst) == 1 {
            Err(injected_failure(point))
        } else {
            Ok(())
        }
    }
}

fn artifact_metadata(
    id: &str,
    bytes: &[u8],
    sensitivity: ArtifactSensitivity,
) -> Result<ArtifactMetadata, Box<dyn std::error::Error>> {
    let reference = milkdrift_workspace::ArtifactReference::new(
        ArtifactId::new(id)?,
        ContentDigest::for_bytes(bytes),
        MediaType::new("application/octet-stream")?,
        bytes.len() as u64,
    );
    Ok(ArtifactMetadata::new(
        reference,
        sensitivity,
        ArtifactRetention::WhileReferenced,
        ArtifactProvenance::new(
            CausalReference::External {
                source: CausalId::new("test-source")?,
            },
            Vec::new(),
        )?,
    )?)
}
