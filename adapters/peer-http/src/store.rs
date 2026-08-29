use std::{
    collections::BTreeMap,
    fs::{self, File, OpenOptions},
    io::Write as _,
    path::PathBuf,
    sync::{Arc, Mutex},
};

use milkdrift_authority::{AuthorityDecisionSnapshot, PeerId};
use milkdrift_peer_protocol::{
    InvocationAcceptance, InvocationLookup, PeerCancellationAcknowledgement, PeerExecutionId,
    PeerInvocationRequest, PeerObservation, PeerRequestId, RemoteExecutionStatus,
};
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Durable accepted-execution persistence failure.
#[derive(Debug, Error)]
pub enum PeerStoreError {
    /// Store serialization or record validation failed.
    #[error("invalid peer execution record: {0}")]
    Invalid(String),
    /// Store I/O failed without exposing a host path.
    #[error("peer execution store I/O failed: {0}")]
    Io(String),
    /// Store lock is unavailable after a panic.
    #[error("peer execution store is unavailable")]
    Unavailable,
    /// Observation did not append contiguously.
    #[error("peer execution observation is not contiguous")]
    Sequence,
}

/// Exact fault boundaries used by deterministic durability tests.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PeerStoreFaultPoint {
    /// Before a new acceptance record is durably replaced.
    AcceptanceBeforeCommit,
    /// After acceptance durability but before returning to the service.
    AcceptanceAfterCommit,
    /// Before an observation record is durably replaced.
    ObservationBeforeCommit,
}

/// Optional deterministic persistence fault injector.
pub trait PeerStoreFaultInjector: Send + Sync {
    /// Returns an injected failure for the exact boundary, when configured.
    fn check(&self, point: PeerStoreFaultPoint) -> Result<(), PeerStoreError>;
}

#[derive(Default)]
struct NoFaults;

impl PeerStoreFaultInjector for NoFaults {
    fn check(&self, _point: PeerStoreFaultPoint) -> Result<(), PeerStoreError> {
        Ok(())
    }
}

/// Complete durable accepted-execution record owned by the peer adapter.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct StoredExecution {
    /// Authenticated submitting peer.
    pub owner_peer: PeerId,
    /// Exact canonical accepted request.
    pub request: PeerInvocationRequest,
    /// Exact allow decision that admitted this consequential peer invocation. Legacy records
    /// created before the unified authority boundary may omit it.
    #[serde(default)]
    pub authority: Option<AuthorityDecisionSnapshot>,
    /// Stable remote execution identity.
    pub execution: PeerExecutionId,
    /// Durable acceptance boundary.
    pub accepted_at_unix_ms: u64,
    /// Current accepted execution lease expiry.
    pub lease_expires_at_unix_ms: u64,
    /// Current durable status.
    pub status: RemoteExecutionStatus,
    /// Contiguous durable semantic observations.
    pub observations: Vec<PeerObservation>,
    /// Latest durable cancellation acknowledgement, when any.
    pub cancellation: Option<PeerCancellationAcknowledgement>,
}

impl StoredExecution {
    /// Converts this record to the public idempotency lookup shape.
    #[must_use]
    pub fn lookup(&self) -> InvocationLookup {
        InvocationLookup::Known {
            execution: self.execution.clone(),
            request_digest: self.request.request_digest.clone(),
            status: self.status,
            last_sequence: self
                .observations
                .last()
                .map_or(0, |observation| observation.sequence),
        }
    }
}

/// Result of atomic idempotent acceptance.
#[derive(Clone, Debug, PartialEq)]
pub enum StoreAcceptance {
    /// This call committed the first durable acceptance.
    New(StoredExecution),
    /// Exact request bytes replayed the prior durable acceptance.
    Replay(StoredExecution),
    /// The same idempotency key was reused with a different canonical request.
    Conflict(StoredExecution),
}

/// Narrow persistence port for durable acceptance and resumable observations.
pub trait PeerExecutionStore: Send + Sync {
    /// Atomically accepts exactly one request identity or returns its prior record.
    fn accept(
        &self,
        owner_peer: &PeerId,
        request: &PeerInvocationRequest,
        authority: Option<&AuthorityDecisionSnapshot>,
        execution: &PeerExecutionId,
        accepted_at_unix_ms: u64,
        lease_expires_at_unix_ms: u64,
    ) -> Result<StoreAcceptance, PeerStoreError>;

    /// Looks up exact durable knowledge by authenticated owner and request key.
    fn by_request(
        &self,
        owner_peer: &PeerId,
        request: &PeerRequestId,
    ) -> Result<Option<StoredExecution>, PeerStoreError>;

    /// Looks up an accepted execution and checks its authenticated owner.
    fn by_execution(
        &self,
        owner_peer: &PeerId,
        execution: &PeerExecutionId,
    ) -> Result<Option<StoredExecution>, PeerStoreError>;

    /// Appends exactly one next semantic observation durably.
    fn append_observation(
        &self,
        owner_peer: &PeerId,
        execution: &PeerExecutionId,
        observation: PeerObservation,
    ) -> Result<StoredExecution, PeerStoreError>;

    /// Durably records adapter-entry intent before entering the external boundary.
    fn mark_running(
        &self,
        owner_peer: &PeerId,
        execution: &PeerExecutionId,
    ) -> Result<(), PeerStoreError>;

    /// Persists a cancellation acknowledgement separately from connection state.
    fn record_cancellation(
        &self,
        owner_peer: &PeerId,
        execution: &PeerExecutionId,
        acknowledgement: PeerCancellationAcknowledgement,
    ) -> Result<StoredExecution, PeerStoreError>;

    /// Extends an accepted execution lease after a semantic-independent heartbeat.
    fn extend_lease(
        &self,
        owner_peer: &PeerId,
        execution: &PeerExecutionId,
        lease_expires_at_unix_ms: u64,
    ) -> Result<(), PeerStoreError>;

    /// Returns bounded records needing restart recovery.
    fn recoverable(&self, maximum: usize) -> Result<Vec<StoredExecution>, PeerStoreError>;
}

/// Crash-durable one-file-per-execution store used by the daemon peer adapter.
pub struct FilePeerExecutionStore {
    root: PathBuf,
    state: Mutex<BTreeMap<String, StoredExecution>>,
    faults: Arc<dyn PeerStoreFaultInjector>,
}

impl std::fmt::Debug for FilePeerExecutionStore {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("FilePeerExecutionStore")
            .field("root", &"[owned peer execution directory]")
            .finish_non_exhaustive()
    }
}

impl FilePeerExecutionStore {
    /// Opens or creates an owned directory and validates every retained record.
    pub fn open(root: impl Into<PathBuf>) -> Result<Self, PeerStoreError> {
        Self::open_with_faults(root, Arc::new(NoFaults))
    }

    /// Opens with deterministic test fault injection.
    pub fn open_with_faults(
        root: impl Into<PathBuf>,
        faults: Arc<dyn PeerStoreFaultInjector>,
    ) -> Result<Self, PeerStoreError> {
        let root = root.into();
        fs::create_dir_all(&root).map_err(io_error)?;
        let mut state = BTreeMap::new();
        for entry in fs::read_dir(&root).map_err(io_error)? {
            let entry = entry.map_err(io_error)?;
            let path = entry.path();
            if path.extension().and_then(|value| value.to_str()) == Some("tmp") {
                fs::remove_file(&path).map_err(io_error)?;
                continue;
            }
            if path.extension().and_then(|value| value.to_str()) != Some("json") {
                continue;
            }
            let bytes = fs::read(&path).map_err(io_error)?;
            if bytes.len() > milkdrift_peer_protocol::MAX_PEER_DOCUMENT_BYTES * 16 {
                return Err(PeerStoreError::Invalid(
                    "stored execution exceeds the recovery bound".to_owned(),
                ));
            }
            let record: StoredExecution = serde_json::from_slice(&bytes)
                .map_err(|error| PeerStoreError::Invalid(error.to_string()))?;
            validate_record(&record)?;
            state.insert(
                record_key(&record.owner_peer, &record.request.request_id),
                record,
            );
        }
        Ok(Self {
            root,
            state: Mutex::new(state),
            faults,
        })
    }

    fn persist(&self, key: &str, record: &StoredExecution) -> Result<(), PeerStoreError> {
        let digest = blake3::hash(key.as_bytes()).to_hex().to_string();
        let destination = self.root.join(format!("{digest}.json"));
        let temporary = self.root.join(format!(".{digest}.tmp"));
        let bytes = serde_json::to_vec(record)
            .map_err(|error| PeerStoreError::Invalid(error.to_string()))?;
        let mut file = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(&temporary)
            .map_err(io_error)?;
        file.write_all(&bytes).map_err(io_error)?;
        file.sync_all().map_err(io_error)?;
        fs::rename(&temporary, &destination).map_err(io_error)?;
        File::open(&self.root)
            .and_then(|directory| directory.sync_all())
            .map_err(io_error)?;
        Ok(())
    }

    fn mutate(
        &self,
        owner_peer: &PeerId,
        execution: &PeerExecutionId,
        change: impl FnOnce(&mut StoredExecution) -> Result<(), PeerStoreError>,
    ) -> Result<StoredExecution, PeerStoreError> {
        let mut state = self.state.lock().map_err(|_| PeerStoreError::Unavailable)?;
        let (key, current) = state
            .iter()
            .find(|(_key, record)| {
                &record.owner_peer == owner_peer && &record.execution == execution
            })
            .map(|(key, record)| (key.clone(), record.clone()))
            .ok_or_else(|| PeerStoreError::Invalid("execution was not found".to_owned()))?;
        let mut updated = current;
        change(&mut updated)?;
        validate_record(&updated)?;
        self.persist(&key, &updated)?;
        state.insert(key, updated.clone());
        Ok(updated)
    }
}

impl PeerExecutionStore for FilePeerExecutionStore {
    fn accept(
        &self,
        owner_peer: &PeerId,
        request: &PeerInvocationRequest,
        authority: Option<&AuthorityDecisionSnapshot>,
        execution: &PeerExecutionId,
        accepted_at_unix_ms: u64,
        lease_expires_at_unix_ms: u64,
    ) -> Result<StoreAcceptance, PeerStoreError> {
        let key = record_key(owner_peer, &request.request_id);
        let mut state = self.state.lock().map_err(|_| PeerStoreError::Unavailable)?;
        if let Some(existing) = state.get(&key) {
            return Ok(
                if existing.request.request_digest == request.request_digest {
                    StoreAcceptance::Replay(existing.clone())
                } else {
                    StoreAcceptance::Conflict(existing.clone())
                },
            );
        }
        self.faults
            .check(PeerStoreFaultPoint::AcceptanceBeforeCommit)?;
        let record = StoredExecution {
            owner_peer: owner_peer.clone(),
            request: request.clone(),
            authority: authority.cloned(),
            execution: execution.clone(),
            accepted_at_unix_ms,
            lease_expires_at_unix_ms,
            status: RemoteExecutionStatus::Accepted,
            observations: Vec::new(),
            cancellation: None,
        };
        validate_record(&record)?;
        self.persist(&key, &record)?;
        state.insert(key, record.clone());
        self.faults
            .check(PeerStoreFaultPoint::AcceptanceAfterCommit)?;
        Ok(StoreAcceptance::New(record))
    }

    fn by_request(
        &self,
        owner_peer: &PeerId,
        request: &PeerRequestId,
    ) -> Result<Option<StoredExecution>, PeerStoreError> {
        Ok(self
            .state
            .lock()
            .map_err(|_| PeerStoreError::Unavailable)?
            .get(&record_key(owner_peer, request))
            .cloned())
    }

    fn by_execution(
        &self,
        owner_peer: &PeerId,
        execution: &PeerExecutionId,
    ) -> Result<Option<StoredExecution>, PeerStoreError> {
        Ok(self
            .state
            .lock()
            .map_err(|_| PeerStoreError::Unavailable)?
            .values()
            .find(|record| &record.owner_peer == owner_peer && &record.execution == execution)
            .cloned())
    }

    fn append_observation(
        &self,
        owner_peer: &PeerId,
        execution: &PeerExecutionId,
        observation: PeerObservation,
    ) -> Result<StoredExecution, PeerStoreError> {
        self.faults
            .check(PeerStoreFaultPoint::ObservationBeforeCommit)?;
        self.mutate(owner_peer, execution, |record| {
            let expected = record
                .observations
                .last()
                .map_or(1, |item| item.sequence.saturating_add(1));
            if observation.execution != record.execution || observation.sequence != expected {
                return Err(PeerStoreError::Sequence);
            }
            observation
                .validate()
                .map_err(|error| PeerStoreError::Invalid(error.to_string()))?;
            let maximum = usize::try_from(record.request.limits.observations).unwrap_or(usize::MAX);
            if record.observations.len() >= maximum
                || (record.observations.len().saturating_add(1) == maximum
                    && observation.event.kind().terminal().is_none())
            {
                return Err(PeerStoreError::Invalid(
                    "execution observation quota is exhausted".to_owned(),
                ));
            }
            record.status = if observation.event.kind().terminal().is_some() {
                RemoteExecutionStatus::Terminal
            } else {
                RemoteExecutionStatus::Running
            };
            record.observations.push(observation);
            Ok(())
        })
    }

    fn mark_running(
        &self,
        owner_peer: &PeerId,
        execution: &PeerExecutionId,
    ) -> Result<(), PeerStoreError> {
        self.mutate(owner_peer, execution, |record| {
            if record.status != RemoteExecutionStatus::Accepted {
                return Err(PeerStoreError::Invalid(
                    "only an accepted execution can enter running state".to_owned(),
                ));
            }
            record.status = RemoteExecutionStatus::Running;
            Ok(())
        })?;
        Ok(())
    }

    fn record_cancellation(
        &self,
        owner_peer: &PeerId,
        execution: &PeerExecutionId,
        acknowledgement: PeerCancellationAcknowledgement,
    ) -> Result<StoredExecution, PeerStoreError> {
        self.mutate(owner_peer, execution, |record| {
            acknowledgement
                .validate()
                .map_err(|error| PeerStoreError::Invalid(error.to_string()))?;
            record.cancellation = Some(acknowledgement);
            Ok(())
        })
    }

    fn extend_lease(
        &self,
        owner_peer: &PeerId,
        execution: &PeerExecutionId,
        lease_expires_at_unix_ms: u64,
    ) -> Result<(), PeerStoreError> {
        self.mutate(owner_peer, execution, |record| {
            if lease_expires_at_unix_ms <= record.lease_expires_at_unix_ms {
                return Err(PeerStoreError::Invalid(
                    "execution lease did not move forward".to_owned(),
                ));
            }
            record.lease_expires_at_unix_ms = lease_expires_at_unix_ms;
            Ok(())
        })?;
        Ok(())
    }

    fn recoverable(&self, maximum: usize) -> Result<Vec<StoredExecution>, PeerStoreError> {
        Ok(self
            .state
            .lock()
            .map_err(|_| PeerStoreError::Unavailable)?
            .values()
            .filter(|record| record.status != RemoteExecutionStatus::Terminal)
            .take(maximum)
            .cloned()
            .collect())
    }
}

fn validate_record(record: &StoredExecution) -> Result<(), PeerStoreError> {
    record
        .request
        .validate()
        .map_err(|error| PeerStoreError::Invalid(error.to_string()))?;
    let maximum = usize::try_from(record.request.limits.observations).unwrap_or(usize::MAX);
    if record.observations.len() > maximum
        || (record.observations.len() == maximum
            && record
                .observations
                .last()
                .is_some_and(|observation| observation.event.kind().terminal().is_none()))
    {
        return Err(PeerStoreError::Invalid(
            "stored execution exceeds its observation quota".to_owned(),
        ));
    }
    let mut expected = 1;
    for observation in &record.observations {
        observation
            .validate()
            .map_err(|error| PeerStoreError::Invalid(error.to_string()))?;
        if observation.execution != record.execution || observation.sequence != expected {
            return Err(PeerStoreError::Sequence);
        }
        expected = expected.saturating_add(1);
    }
    if record.status == RemoteExecutionStatus::Terminal
        && record
            .observations
            .last()
            .is_none_or(|item| item.event.kind().terminal().is_none())
    {
        return Err(PeerStoreError::Invalid(
            "terminal record lacks terminal evidence".to_owned(),
        ));
    }
    Ok(())
}

fn record_key(peer: &PeerId, request: &PeerRequestId) -> String {
    format!("{}|{}", peer.as_str(), request.as_str())
}

fn io_error(error: std::io::Error) -> PeerStoreError {
    PeerStoreError::Io(format!("{:?}", error.kind()))
}

pub(crate) fn acceptance(record: &StoredExecution, replayed: bool) -> InvocationAcceptance {
    InvocationAcceptance::Accepted {
        request_id: record.request.request_id.clone(),
        execution: record.execution.clone(),
        request_digest: record.request.request_digest.clone(),
        accepted_at_unix_ms: record.accepted_at_unix_ms,
        lease_expires_at_unix_ms: record.lease_expires_at_unix_ms,
        replayed,
    }
}
