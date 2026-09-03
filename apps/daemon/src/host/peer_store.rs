//! Daemon-owned peer persistence adapters.
//!
//! `PeerService` is intentionally usable outside the daemon with any implementation of its
//! narrow persistence ports. Inside the daemon, however, redb and core artifact transfer state
//! belong to the runtime owner thread. These adapters preserve the existing ports while making
//! every off-owner call enter the same bounded owner queue as the local control plane.

use std::sync::Weak;

use milkdrift_capability::{ArtifactReference, PeerId};
use milkdrift_peer_http::{
    CorePeerArtifactStore, PeerArtifactError, PeerArtifactStore, PeerArtifactTransferFacts,
};
use milkdrift_peer_protocol::{
    ArtifactChunk, ArtifactMetadataOffer, ArtifactTransferDecision,
    PeerCancellationAcknowledgement, PeerCancellationRequest, PeerExecutionId, PeerObservation,
    PeerRequestId, TransferId,
};
use milkdrift_persistence::{
    PageSize, PeerAdmission, PeerAdmissionOutcome, PeerCatalogState, PeerClaimOutcome,
    PeerDispatchClaimRequest, PeerEntryOutcome, PeerEntryRequest, PeerExecutionRecord,
    PeerExecutionSnapshot, PeerExecutionStatus, PeerExecutionStore, PeerObservationAppend,
    PeerObservationPage, PeerRecoveryResult, PeerRelationshipState, PeerRetentionPage,
    PeerRetentionRequest, PersistenceError, StorageFailureClass, WorkerId,
};
use milkdrift_redb_store::RedbStore;

use super::{OwnerCallFailure, OwnerQueue};

fn execution_failure(failure: OwnerCallFailure) -> PersistenceError {
    PersistenceError::Storage {
        class: match failure {
            OwnerCallFailure::QueueFull => StorageFailureClass::ResourceExhausted,
            OwnerCallFailure::Disconnected | OwnerCallFailure::ResponseTimeout => {
                StorageFailureClass::Unavailable
            }
        },
        message: failure.message().to_owned(),
    }
}

fn artifact_failure(failure: OwnerCallFailure) -> PeerArtifactError {
    match failure {
        OwnerCallFailure::QueueFull => PeerArtifactError::Overloaded(failure.message().to_owned()),
        OwnerCallFailure::Disconnected | OwnerCallFailure::ResponseTimeout => {
            PeerArtifactError::Persistence(failure.message().to_owned())
        }
    }
}

pub(super) struct OwnerPeerExecutionStore {
    queue: OwnerQueue,
    direct: Weak<RedbStore>,
}

impl OwnerPeerExecutionStore {
    pub(super) fn new(queue: OwnerQueue, direct: Weak<RedbStore>) -> Self {
        Self { queue, direct }
    }

    fn call<T>(
        &self,
        operation: impl FnOnce(&RedbStore) -> Result<T, PersistenceError> + Send + 'static,
    ) -> Result<T, PersistenceError>
    where
        T: Send + 'static,
    {
        let direct = self.direct.clone();
        self.queue.call(
            move || {
                let direct = direct
                    .upgrade()
                    .ok_or_else(|| execution_failure(OwnerCallFailure::Disconnected))?;
                operation(&direct)
            },
            execution_failure,
        )
    }
}

impl PeerExecutionStore for OwnerPeerExecutionStore {
    fn set_peer_admission_open(&self, open: bool) -> Result<(), PersistenceError> {
        self.call(move |direct| direct.set_peer_admission_open(open))
    }

    fn configure_peer_relationship(
        &self,
        relationship: &PeerRelationshipState,
    ) -> Result<(), PersistenceError> {
        let relationship = relationship.clone();
        self.call(move |direct| direct.configure_peer_relationship(&relationship))
    }

    fn publish_peer_catalog(&self, catalog: &PeerCatalogState) -> Result<(), PersistenceError> {
        let catalog = catalog.clone();
        self.call(move |direct| direct.publish_peer_catalog(&catalog))
    }

    fn peer_catalog(&self, peer: &PeerId) -> Result<Option<PeerCatalogState>, PersistenceError> {
        let peer = peer.clone();
        self.call(move |direct| direct.peer_catalog(&peer))
    }

    fn admit_peer_execution(
        &self,
        admission: &PeerAdmission<'_>,
    ) -> Result<PeerAdmissionOutcome, PersistenceError> {
        let owner_peer = admission.owner_peer.clone();
        let request = admission.request.clone();
        let authority = admission.authority.clone();
        let execution = admission.execution.clone();
        let relationship_generation = admission.relationship_generation;
        let accepted_at_unix_ms = admission.accepted_at_unix_ms;
        let maximum_global_active = admission.maximum_global_active;
        let maximum_dispatch_queue = admission.maximum_dispatch_queue;
        let maximum_hot_terminal_records = admission.maximum_hot_terminal_records;
        let archive_batch_size = admission.archive_batch_size;
        let archive_terminal_before_or_at_unix_ms = admission.archive_terminal_before_or_at_unix_ms;
        self.call(move |direct| {
            direct.admit_peer_execution(&PeerAdmission {
                owner_peer: &owner_peer,
                request: &request,
                authority: &authority,
                execution: &execution,
                relationship_generation,
                accepted_at_unix_ms,
                maximum_global_active,
                maximum_dispatch_queue,
                maximum_hot_terminal_records,
                archive_batch_size,
                archive_terminal_before_or_at_unix_ms,
            })
        })
    }

    fn peer_execution_by_request(
        &self,
        owner: &PeerId,
        request: &PeerRequestId,
    ) -> Result<Option<PeerExecutionSnapshot>, PersistenceError> {
        let owner = owner.clone();
        let request = request.clone();
        self.call(move |direct| direct.peer_execution_by_request(&owner, &request))
    }

    fn peer_execution(
        &self,
        owner: &PeerId,
        execution: &PeerExecutionId,
    ) -> Result<Option<PeerExecutionSnapshot>, PersistenceError> {
        let owner = owner.clone();
        let execution = execution.clone();
        self.call(move |direct| direct.peer_execution(&owner, &execution))
    }

    fn peer_execution_status(&self) -> Result<PeerExecutionStatus, PersistenceError> {
        self.call(move |direct| direct.peer_execution_status())
    }

    fn verify_peer_execution_integrity(&self) -> Result<(), PersistenceError> {
        self.call(move |direct| direct.verify_peer_execution_integrity())
    }

    fn peer_observations(
        &self,
        owner: &PeerId,
        execution: &PeerExecutionId,
        after_sequence: u64,
        limit: PageSize,
    ) -> Result<PeerObservationPage, PersistenceError> {
        let owner = owner.clone();
        let execution = execution.clone();
        self.call(move |direct| direct.peer_observations(&owner, &execution, after_sequence, limit))
    }

    fn claim_peer_dispatch(
        &self,
        request: &PeerDispatchClaimRequest<'_>,
    ) -> Result<PeerClaimOutcome, PersistenceError> {
        let worker = request.worker.clone();
        let claimed_at_unix_ms = request.claimed_at_unix_ms;
        let lease_expires_at_unix_ms = request.lease_expires_at_unix_ms;
        self.call(move |direct| {
            direct.claim_peer_dispatch(&PeerDispatchClaimRequest {
                worker: &worker,
                claimed_at_unix_ms,
                lease_expires_at_unix_ms,
            })
        })
    }

    fn mark_peer_entered(
        &self,
        request: &PeerEntryRequest<'_>,
    ) -> Result<PeerEntryOutcome, PersistenceError> {
        let owner = request.owner.clone();
        let execution = request.execution.clone();
        let worker = request.worker.clone();
        let claim_generation = request.claim_generation;
        let relationship_generation = request.relationship_generation;
        let entered_at_unix_ms = request.entered_at_unix_ms;
        let authority = request.authority.clone();
        self.call(move |direct| {
            direct.mark_peer_entered(&PeerEntryRequest {
                owner: &owner,
                execution: &execution,
                worker: &worker,
                claim_generation,
                relationship_generation,
                entered_at_unix_ms,
                authority: &authority,
            })
        })
    }

    fn release_peer_claim(
        &self,
        owner: &PeerId,
        execution: &PeerExecutionId,
        worker: &WorkerId,
        claim_generation: u64,
        available_at_unix_ms: u64,
    ) -> Result<PeerExecutionRecord, PersistenceError> {
        let owner = owner.clone();
        let execution = execution.clone();
        let worker = worker.clone();
        self.call(move |direct| {
            direct.release_peer_claim(
                &owner,
                &execution,
                &worker,
                claim_generation,
                available_at_unix_ms,
            )
        })
    }

    fn extend_peer_claim(
        &self,
        owner: &PeerId,
        execution: &PeerExecutionId,
        worker: &WorkerId,
        claim_generation: u64,
        lease_expires_at_unix_ms: u64,
    ) -> Result<(), PersistenceError> {
        let owner = owner.clone();
        let execution = execution.clone();
        let worker = worker.clone();
        self.call(move |direct| {
            direct.extend_peer_claim(
                &owner,
                &execution,
                &worker,
                claim_generation,
                lease_expires_at_unix_ms,
            )
        })
    }

    fn mark_peer_uncertain(
        &self,
        owner: &PeerId,
        execution: &PeerExecutionId,
        worker: &WorkerId,
        claim_generation: u64,
        uncertain_at_unix_ms: u64,
        reason: &str,
    ) -> Result<PeerExecutionRecord, PersistenceError> {
        let owner = owner.clone();
        let execution = execution.clone();
        let worker = worker.clone();
        let reason = reason.to_owned();
        self.call(move |direct| {
            direct.mark_peer_uncertain(
                &owner,
                &execution,
                &worker,
                claim_generation,
                uncertain_at_unix_ms,
                &reason,
            )
        })
    }

    fn append_peer_observation(
        &self,
        owner: &PeerId,
        execution: &PeerExecutionId,
        observation: &PeerObservation,
    ) -> Result<PeerObservationAppend, PersistenceError> {
        let owner = owner.clone();
        let execution = execution.clone();
        let observation = observation.clone();
        self.call(move |direct| direct.append_peer_observation(&owner, &execution, &observation))
    }

    fn request_peer_cancellation(
        &self,
        owner: &PeerId,
        request: &PeerCancellationRequest,
        requested_at_unix_ms: u64,
    ) -> Result<PeerExecutionRecord, PersistenceError> {
        let owner = owner.clone();
        let request = request.clone();
        self.call(move |direct| {
            direct.request_peer_cancellation(&owner, &request, requested_at_unix_ms)
        })
    }

    fn acknowledge_peer_cancellation(
        &self,
        owner: &PeerId,
        acknowledgement: &PeerCancellationAcknowledgement,
        acknowledged_at_unix_ms: u64,
    ) -> Result<PeerExecutionRecord, PersistenceError> {
        let owner = owner.clone();
        let acknowledgement = acknowledgement.clone();
        self.call(move |direct| {
            direct.acknowledge_peer_cancellation(&owner, &acknowledgement, acknowledged_at_unix_ms)
        })
    }

    fn recover_peer_claims(
        &self,
        recovered_at_unix_ms: u64,
        limit: PageSize,
    ) -> Result<PeerRecoveryResult, PersistenceError> {
        self.call(move |direct| direct.recover_peer_claims(recovered_at_unix_ms, limit))
    }

    fn archive_peer_executions(
        &self,
        request: &PeerRetentionRequest,
    ) -> Result<PeerRetentionPage, PersistenceError> {
        let terminal_before_or_at = request.terminal_before_or_at;
        let archived_at = request.archived_at;
        let limit = request.limit;
        self.call(move |direct| {
            direct.archive_peer_executions(&PeerRetentionRequest {
                terminal_before_or_at,
                archived_at,
                limit,
            })
        })
    }

    fn peer_observation_artifact(
        &self,
        execution: &PeerExecutionId,
        sequence: u64,
    ) -> Result<Option<ArtifactReference>, PersistenceError> {
        let execution = execution.clone();
        self.call(move |direct| direct.peer_observation_artifact(&execution, sequence))
    }
}

pub(super) struct OwnerPeerArtifactStore {
    queue: OwnerQueue,
    direct: Weak<CorePeerArtifactStore>,
}

impl OwnerPeerArtifactStore {
    pub(super) fn new(queue: OwnerQueue, direct: Weak<CorePeerArtifactStore>) -> Self {
        Self { queue, direct }
    }

    fn call<T>(
        &self,
        operation: impl FnOnce(&CorePeerArtifactStore) -> Result<T, PeerArtifactError> + Send + 'static,
    ) -> Result<T, PeerArtifactError>
    where
        T: Send + 'static,
    {
        let direct = self.direct.clone();
        self.queue.call(
            move || {
                let direct = direct
                    .upgrade()
                    .ok_or_else(|| artifact_failure(OwnerCallFailure::Disconnected))?;
                operation(&direct)
            },
            artifact_failure,
        )
    }
}

impl PeerArtifactStore for OwnerPeerArtifactStore {
    fn transfer_facts(
        &self,
        owner_peer: &PeerId,
        transfer: &TransferId,
    ) -> Result<PeerArtifactTransferFacts, PeerArtifactError> {
        let owner_peer = owner_peer.clone();
        let transfer = transfer.clone();
        self.call(move |direct| direct.transfer_facts(&owner_peer, &transfer))
    }

    fn negotiate(
        &self,
        owner_peer: &PeerId,
        offer: &ArtifactMetadataOffer,
        maximum_artifact_bytes: u64,
    ) -> Result<ArtifactTransferDecision, PeerArtifactError> {
        let owner_peer = owner_peer.clone();
        let offer = offer.clone();
        self.call(move |direct| direct.negotiate(&owner_peer, &offer, maximum_artifact_bytes))
    }

    fn write_chunk(
        &self,
        owner_peer: &PeerId,
        chunk: &ArtifactChunk,
        maximum_chunk_bytes: u32,
    ) -> Result<ArtifactTransferDecision, PeerArtifactError> {
        let owner_peer = owner_peer.clone();
        let chunk = chunk.clone();
        self.call(move |direct| direct.write_chunk(&owner_peer, &chunk, maximum_chunk_bytes))
    }

    fn read_chunk(
        &self,
        owner_peer: &PeerId,
        transfer: &TransferId,
        offset: u64,
        maximum_bytes: u32,
    ) -> Result<ArtifactChunk, PeerArtifactError> {
        let owner_peer = owner_peer.clone();
        let transfer = transfer.clone();
        self.call(move |direct| direct.read_chunk(&owner_peer, &transfer, offset, maximum_bytes))
    }

    fn abort(&self, owner_peer: &PeerId, transfer: &TransferId) -> Result<(), PeerArtifactError> {
        let owner_peer = owner_peer.clone();
        let transfer = transfer.clone();
        self.call(move |direct| direct.abort(&owner_peer, &transfer))
    }
}
