use milkdrift_authority::{AuthorityOperation, PeerId};
use milkdrift_peer_protocol::{
    PeerCancellationAcknowledgement, PeerCancellationRequest, PeerExecutionId, PeerObservation,
    PeerRequestId,
};
use milkdrift_persistence::{
    PEER_EXECUTION_RECORD_SCHEMA_VERSION_V2, PEER_EXECUTION_RECORD_SCHEMA_VERSION_V3,
    PEER_EXECUTION_TOMBSTONE_SCHEMA_VERSION_V1, PeerAcceptedAuthoritySummary, PeerAdmission,
    PeerAdmissionOutcome, PeerAdmissionRejection, PeerArchivedDisposition, PeerCancellationRecord,
    PeerCatalogState, PeerClaimOutcome, PeerDispatchClaim, PeerDispatchClaimRequest,
    PeerEntryEvidence, PeerEntryOutcome, PeerEntryRequest, PeerExecutionAccounting,
    PeerExecutionPhase, PeerExecutionRecord, PeerExecutionSnapshot, PeerExecutionStatus,
    PeerExecutionStore, PeerExecutionTombstone, PeerObservationAppend, PeerObservationPage,
    PeerRecoveryResult, PeerRelationshipState, PeerRetentionPage, PeerRetentionRequest,
    PersistenceError, StorageFailureClass, WorkerId,
};
use redb::{ReadableTable, ReadableTableMetadata};
use serde::{Deserialize, Serialize};

use crate::{
    RedbStore, codec, error,
    fault::FaultPoint,
    json,
    schema::{
        PEER_ACTIVE_CLAIMS, PEER_CATALOGS, PEER_DISPATCH_AVAILABLE, PEER_EXECUTION_ACCOUNTING,
        PEER_EXECUTION_GLOBAL_ACCOUNTING_KEY, PEER_EXECUTION_LOCATIONS, PEER_EXECUTION_TOMBSTONES,
        PEER_EXECUTIONS, PEER_EXECUTIONS_BY_REQUEST, PEER_OBSERVATION_ARTIFACTS, PEER_OBSERVATIONS,
        PEER_RELATIONSHIPS, PEER_TERMINAL_INDEX,
    },
};

const PEER_ACCOUNTING_SCHEMA_VERSION: u32 = 2;
const MAX_UNCERTAINTY_REASON_BYTES: usize = 2_048;
const LOCATION_HOT: u8 = 1;
const LOCATION_ARCHIVED: u8 = 2;
const OBSERVATION_DIGEST_DOMAIN: &[u8] = b"milkdrift.peer.observation-history.v1\0";

#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct GlobalPeerAccounting {
    schema_version: u32,
    next_acceptance_sequence: u64,
    active: u32,
    dispatch_queued: u32,
    hot_terminal: u64,
    tombstones: u64,
    archive_generation: u64,
    last_archived_at_unix_ms: Option<u64>,
    admission_open: bool,
}

impl GlobalPeerAccounting {
    pub(crate) const EMPTY: Self = Self {
        schema_version: PEER_ACCOUNTING_SCHEMA_VERSION,
        next_acceptance_sequence: 1,
        active: 0,
        dispatch_queued: 0,
        hot_terminal: 0,
        tombstones: 0,
        archive_generation: 0,
        last_archived_at_unix_ms: None,
        admission_open: false,
    };
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct PerPeerAccounting {
    schema_version: u32,
    peer: PeerId,
    active: u32,
    revision: u64,
}

impl PerPeerAccounting {
    fn empty(peer: &PeerId) -> Self {
        Self {
            schema_version: PEER_ACCOUNTING_SCHEMA_VERSION,
            peer: peer.clone(),
            active: 0,
            revision: 0,
        }
    }
}

impl PeerExecutionStore for RedbStore {
    fn set_peer_admission_open(&self, open: bool) -> Result<(), PersistenceError> {
        let write = self.database().begin_write().map_err(error::redb)?;
        let mut global = global_accounting(&write)?;
        global.admission_open = open;
        put_global_accounting(&write, global)?;
        write.commit().map_err(error::redb)
    }

    fn configure_peer_relationship(
        &self,
        relationship: &PeerRelationshipState,
    ) -> Result<(), PersistenceError> {
        validate_relationship(relationship)?;
        let write = self.database().begin_write().map_err(error::redb)?;
        let existing: Option<PeerRelationshipState> = {
            let table = write.open_table(PEER_RELATIONSHIPS).map_err(error::redb)?;
            table
                .get(relationship.peer.as_str())
                .map_err(error::redb)?
                .map(|bytes| json::decode(bytes.value(), "peer relationship"))
                .transpose()?
        };
        if let Some(existing) = existing {
            if existing == *relationship {
                return Ok(());
            }
            if relationship.generation <= existing.generation {
                return Err(PersistenceError::ImmutableConflict {
                    entity: "peer_relationship_generation",
                    identity: relationship.peer.to_string(),
                });
            }
        }
        let bytes = json::encode(relationship, "peer relationship")?;
        write
            .open_table(PEER_RELATIONSHIPS)
            .map_err(error::redb)?
            .insert(relationship.peer.as_str(), bytes.as_slice())
            .map_err(error::redb)?;
        write.commit().map_err(error::redb)
    }

    fn publish_peer_catalog(&self, catalog: &PeerCatalogState) -> Result<(), PersistenceError> {
        validate_catalog(catalog)?;
        let write = self.database().begin_write().map_err(error::redb)?;
        let relationship = relationship_in_transaction(&write, &catalog.peer)?
            .ok_or_else(|| missing("peer_relationship", catalog.peer.as_str()))?;
        if !relationship.enabled || relationship.generation != catalog.relationship_generation {
            return Err(PersistenceError::ImmutableConflict {
                entity: "peer_catalog_relationship_generation",
                identity: catalog.peer.to_string(),
            });
        }
        let existing = catalog_in_transaction(&write, &catalog.peer)?;
        if let Some(existing) = existing {
            if existing == *catalog {
                return Ok(());
            }
            if catalog.generation <= existing.generation {
                return Err(PersistenceError::ImmutableConflict {
                    entity: "peer_catalog_generation",
                    identity: catalog.peer.to_string(),
                });
            }
        }
        let bytes = json::encode(catalog, "peer catalog")?;
        write
            .open_table(PEER_CATALOGS)
            .map_err(error::redb)?
            .insert(catalog.peer.as_str(), bytes.as_slice())
            .map_err(error::redb)?;
        write.commit().map_err(error::redb)
    }

    fn peer_catalog(&self, peer: &PeerId) -> Result<Option<PeerCatalogState>, PersistenceError> {
        let read = self.database().begin_read().map_err(error::redb)?;
        read.open_table(PEER_CATALOGS)
            .map_err(error::redb)?
            .get(peer.as_str())
            .map_err(error::redb)?
            .map(|bytes| json::decode(bytes.value(), "peer catalog"))
            .transpose()
    }

    fn admit_peer_execution(
        &self,
        admission: &PeerAdmission<'_>,
    ) -> Result<PeerAdmissionOutcome, PersistenceError> {
        admission
            .request
            .validate()
            .map_err(|cause| invalid(&cause.to_string()))?;
        let write = self.database().begin_write().map_err(error::redb)?;
        let request_key = request_key(admission.owner_peer, &admission.request.request_id)?;
        let existing_execution = {
            let by_request = write
                .open_table(PEER_EXECUTIONS_BY_REQUEST)
                .map_err(error::redb)?;
            by_request
                .get(request_key.as_slice())
                .map_err(error::redb)?
                .map(|value| value.value().to_owned())
        };
        if let Some(execution) = existing_execution {
            let existing = snapshot_in_transaction_text(&write, &execution)?;
            if existing.owner_peer() != admission.owner_peer
                || existing.request_id() != &admission.request.request_id
            {
                return Err(corruption(
                    "peer request index disagrees with its execution",
                ));
            }
            return Ok(
                if existing.request_digest() == admission.request.request_digest {
                    PeerAdmissionOutcome::Replayed(existing)
                } else {
                    PeerAdmissionOutcome::Conflict(existing)
                },
            );
        }

        validate_admission(admission)?;

        let mut global = global_accounting(&write)?;
        if !global.admission_open {
            return Ok(PeerAdmissionOutcome::Rejected(
                PeerAdmissionRejection::AdmissionClosed,
            ));
        }

        let Some(relationship) = relationship_in_transaction(&write, admission.owner_peer)? else {
            return Ok(PeerAdmissionOutcome::Rejected(
                PeerAdmissionRejection::RelationshipUnavailable,
            ));
        };
        if !relationship.enabled
            || relationship.generation != admission.relationship_generation
            || admission.accepted_at_unix_ms > relationship.expires_at_unix_ms
        {
            return Ok(PeerAdmissionOutcome::Rejected(
                PeerAdmissionRejection::RelationshipUnavailable,
            ));
        }
        let catalog = catalog_in_transaction(&write, admission.owner_peer)?;
        if catalog.as_ref().is_none_or(|state| {
            state.relationship_generation != admission.relationship_generation
                || state.generation != admission.request.catalog_generation
                || state.digest != admission.request.catalog_digest.as_str()
                || admission.accepted_at_unix_ms > state.expires_at_unix_ms
        }) {
            return Ok(PeerAdmissionOutcome::Rejected(
                PeerAdmissionRejection::CatalogUnavailable,
            ));
        }

        let mut peer = peer_accounting(&write, admission.owner_peer)?;
        if peer.active >= relationship.maximum_active {
            return Ok(PeerAdmissionOutcome::Rejected(
                PeerAdmissionRejection::PeerCapacity,
            ));
        }
        if global.active >= admission.maximum_global_active {
            return Ok(PeerAdmissionOutcome::Rejected(
                PeerAdmissionRejection::GlobalCapacity,
            ));
        }
        if global.dispatch_queued >= admission.maximum_dispatch_queue {
            return Ok(PeerAdmissionOutcome::Rejected(
                PeerAdmissionRejection::DispatchCapacity,
            ));
        }
        let reserved_hot = global
            .hot_terminal
            .checked_add(u64::from(global.active))
            .ok_or_else(|| corruption("peer reserved hot capacity overflowed"))?;
        if reserved_hot >= admission.maximum_hot_terminal_records {
            archive_eligible_in_transaction(
                self,
                &write,
                admission.archive_terminal_before_or_at_unix_ms,
                admission.accepted_at_unix_ms,
                admission.archive_batch_size,
            )?;
            global = global_accounting(&write)?;
        }
        if global
            .hot_terminal
            .checked_add(u64::from(global.active))
            .ok_or_else(|| corruption("peer reserved hot capacity overflowed"))?
            >= admission.maximum_hot_terminal_records
        {
            return Ok(PeerAdmissionOutcome::Rejected(
                PeerAdmissionRejection::RetentionCapacity,
            ));
        }
        if snapshot_optional_in_transaction(&write, admission.execution)?.is_some() {
            return Err(PersistenceError::ImmutableConflict {
                entity: "peer_execution",
                identity: admission.execution.to_string(),
            });
        }
        let sequence = global.next_acceptance_sequence;
        global.next_acceptance_sequence = sequence
            .checked_add(1)
            .ok_or_else(|| corruption("peer acceptance sequence overflowed"))?;
        global.active = global
            .active
            .checked_add(1)
            .ok_or_else(|| corruption("peer active count overflowed"))?;
        global.dispatch_queued = global
            .dispatch_queued
            .checked_add(1)
            .ok_or_else(|| corruption("peer dispatch count overflowed"))?;
        peer.active = peer
            .active
            .checked_add(1)
            .ok_or_else(|| corruption("per-peer active count overflowed"))?;
        peer.revision = peer
            .revision
            .checked_add(1)
            .ok_or_else(|| corruption("per-peer accounting revision overflowed"))?;
        let record = PeerExecutionRecord {
            schema_version: PEER_EXECUTION_RECORD_SCHEMA_VERSION_V3,
            owner_peer: admission.owner_peer.clone(),
            relationship_generation: admission.relationship_generation,
            request: admission.request.clone(),
            authority: admission.authority.clone(),
            execution: admission.execution.clone(),
            acceptance_sequence: sequence,
            accepted_at_unix_ms: admission.accepted_at_unix_ms,
            phase: PeerExecutionPhase::DispatchAvailable {
                available_at_unix_ms: admission.accepted_at_unix_ms,
            },
            cancellation: None,
            last_observation_sequence: 0,
            accounting: PeerExecutionAccounting {
                artifact_bytes: admission
                    .request
                    .input_artifact_bytes()
                    .map_err(|cause| invalid(&cause.to_string()))?,
                ..PeerExecutionAccounting::default()
            },
            observation_digest: observation_genesis_digest(),
            revision: 1,
        };
        validate_record(&record)?;
        put_execution(&write, &record)?;
        write
            .open_table(PEER_EXECUTIONS_BY_REQUEST)
            .map_err(error::redb)?
            .insert(request_key.as_slice(), record.execution.as_str())
            .map_err(error::redb)?;
        write
            .open_table(PEER_EXECUTION_LOCATIONS)
            .map_err(error::redb)?
            .insert(record.execution.as_str(), LOCATION_HOT)
            .map_err(error::redb)?;
        let available_key = available_key(&record)?;
        write
            .open_table(PEER_DISPATCH_AVAILABLE)
            .map_err(error::redb)?
            .insert(available_key.as_slice(), record.execution.as_str())
            .map_err(error::redb)?;
        put_global_accounting(&write, global)?;
        put_peer_accounting(&write, &peer)?;
        self.faults.check(FaultPoint::BeforePeerAdmissionCommit)?;
        write.commit().map_err(error::redb)?;
        self.faults.check(FaultPoint::AfterPeerAdmissionCommit)?;
        Ok(PeerAdmissionOutcome::Accepted(Box::new(record)))
    }

    fn peer_execution_by_request(
        &self,
        owner: &PeerId,
        request: &PeerRequestId,
    ) -> Result<Option<PeerExecutionSnapshot>, PersistenceError> {
        let read = self.database().begin_read().map_err(error::redb)?;
        let key = request_key(owner, request)?;
        let execution = read
            .open_table(PEER_EXECUTIONS_BY_REQUEST)
            .map_err(error::redb)?
            .get(key.as_slice())
            .map_err(error::redb)?
            .map(|value| value.value().to_owned());
        let Some(execution) = execution else {
            return Ok(None);
        };
        let record = snapshot_in_read_transaction_text(&read, &execution)?;
        if record.owner_peer() != owner || record.request_id() != request {
            return Err(corruption(
                "peer request lookup returned mismatched primary record",
            ));
        }
        Ok(Some(record))
    }

    fn peer_execution(
        &self,
        owner: &PeerId,
        execution: &PeerExecutionId,
    ) -> Result<Option<PeerExecutionSnapshot>, PersistenceError> {
        let read = self.database().begin_read().map_err(error::redb)?;
        let record = snapshot_optional_in_read_transaction(&read, execution)?;
        Ok(record.filter(|record| record.owner_peer() == owner))
    }

    fn peer_execution_status(&self) -> Result<PeerExecutionStatus, PersistenceError> {
        let read = self.database().begin_read().map_err(error::redb)?;
        let value = global_accounting_read(&read)?;
        Ok(PeerExecutionStatus {
            active: value.active,
            dispatch_queued: value.dispatch_queued,
            hot_terminal: value.hot_terminal,
            tombstones: value.tombstones,
            archive_generation: value.archive_generation,
            last_archived_at_unix_ms: value.last_archived_at_unix_ms,
        })
    }

    fn verify_peer_execution_integrity(&self) -> Result<(), PersistenceError> {
        verify_integrity(self)
    }

    fn peer_observations(
        &self,
        owner: &PeerId,
        execution: &PeerExecutionId,
        after_sequence: u64,
        limit: milkdrift_persistence::PageSize,
    ) -> Result<PeerObservationPage, PersistenceError> {
        let read = self.database().begin_read().map_err(error::redb)?;
        let snapshot = snapshot_optional_in_read_transaction(&read, execution)?
            .filter(|record| record.owner_peer() == owner)
            .ok_or_else(|| missing("peer_execution", execution.as_str()))?;
        if after_sequence > snapshot.last_observation_sequence() {
            return Err(PersistenceError::InvalidCursor(
                "peer observation cursor is beyond the durable head".to_owned(),
            ));
        }
        let PeerExecutionSnapshot::Hot(record) = &snapshot else {
            return Ok(PeerObservationPage {
                execution: snapshot,
                observations: Vec::new(),
            });
        };
        let observations_table = read.open_table(PEER_OBSERVATIONS).map_err(error::redb)?;
        let mut observations = Vec::with_capacity(limit.get() as usize);
        let mut sequence = after_sequence.saturating_add(1);
        while sequence <= record.last_observation_sequence
            && observations.len() < limit.get() as usize
        {
            let key = observation_key(execution, sequence)?;
            let bytes = observations_table
                .get(key.as_slice())
                .map_err(error::redb)?
                .ok_or_else(|| corruption("peer observation head has a missing row"))?;
            let observation: PeerObservation = json::decode(bytes.value(), "peer observation")?;
            if observation.execution != *execution || observation.sequence != sequence {
                return Err(corruption(
                    "peer observation key disagrees with its document",
                ));
            }
            observations.push(observation);
            sequence = sequence.saturating_add(1);
        }
        Ok(PeerObservationPage {
            execution: snapshot,
            observations,
        })
    }

    fn claim_peer_dispatch(
        &self,
        request: &PeerDispatchClaimRequest<'_>,
    ) -> Result<PeerClaimOutcome, PersistenceError> {
        if request.claimed_at_unix_ms == 0
            || request.lease_expires_at_unix_ms <= request.claimed_at_unix_ms
        {
            return Err(invalid("peer dispatch claim has an invalid lease boundary"));
        }
        let write = self.database().begin_write().map_err(error::redb)?;
        let candidate = {
            let available = write
                .open_table(PEER_DISPATCH_AVAILABLE)
                .map_err(error::redb)?;
            available
                .iter()
                .map_err(error::redb)?
                .next()
                .transpose()
                .map_err(error::redb)?
                .map(|(key, execution)| (key.value().to_vec(), execution.value().to_owned()))
        };
        let Some((available_index_key, execution)) = candidate else {
            return Ok(PeerClaimOutcome::Empty);
        };
        let mut record = execution_in_transaction_text(&write, &execution)?;
        if available_key(&record)? != available_index_key {
            return Err(corruption(
                "peer dispatch index disagrees with its primary record",
            ));
        }
        let cancellation_only = matches!(
            record.phase,
            PeerExecutionPhase::CancellationRequested {
                claim: None,
                evidence: None
            }
        );
        if !matches!(record.phase, PeerExecutionPhase::DispatchAvailable { .. })
            && !cancellation_only
        {
            return Err(corruption(
                "peer dispatch index points at a nondispatchable phase",
            ));
        }
        let generation = record
            .revision
            .checked_add(1)
            .ok_or_else(|| corruption("peer claim generation overflowed"))?;
        let claim = PeerDispatchClaim {
            worker: request.worker.clone(),
            generation,
            claimed_at_unix_ms: request.claimed_at_unix_ms,
            lease_expires_at_unix_ms: request.lease_expires_at_unix_ms,
        };
        record.phase = if cancellation_only {
            PeerExecutionPhase::CancellationRequested {
                claim: Some(claim.clone()),
                evidence: None,
            }
        } else {
            PeerExecutionPhase::DispatchClaimed {
                claim: claim.clone(),
            }
        };
        bump_record(&mut record)?;
        validate_record(&record)?;
        write
            .open_table(PEER_DISPATCH_AVAILABLE)
            .map_err(error::redb)?
            .remove(available_index_key.as_slice())
            .map_err(error::redb)?
            .ok_or_else(|| corruption("peer dispatch row disappeared during claim"))?;
        let claim_key = claim_key(&record.execution, &claim)?;
        write
            .open_table(PEER_ACTIVE_CLAIMS)
            .map_err(error::redb)?
            .insert(claim_key.as_slice(), record.execution.as_str())
            .map_err(error::redb)?;
        put_execution(&write, &record)?;
        self.faults.check(FaultPoint::BeforePeerClaimCommit)?;
        write.commit().map_err(error::redb)?;
        self.faults.check(FaultPoint::AfterPeerClaimCommit)?;
        Ok(if cancellation_only {
            PeerClaimOutcome::CancellationRequested(record)
        } else {
            PeerClaimOutcome::Claimed(record)
        })
    }

    fn mark_peer_entered(
        &self,
        request: &PeerEntryRequest<'_>,
    ) -> Result<PeerEntryOutcome, PersistenceError> {
        if request.entered_at_unix_ms == 0 {
            return Err(invalid("peer adapter entry time must be nonzero"));
        }
        let write = self.database().begin_write().map_err(error::redb)?;
        let mut record = owned_execution_in_transaction(&write, request.owner, request.execution)?;
        validate_entry_authority(&record, request.authority)?;
        let claim =
            exact_pre_entry_claim(&record, request.worker, request.claim_generation)?.clone();
        let mut global = global_accounting(&write)?;
        if !global.admission_open {
            return Ok(PeerEntryOutcome::AdmissionClosed);
        }
        let relationship = relationship_in_transaction(&write, request.owner)?;
        if relationship.as_ref().is_none_or(|relationship| {
            !relationship.enabled
                || relationship.generation != request.relationship_generation
                || request.entered_at_unix_ms > relationship.expires_at_unix_ms
        }) {
            return Ok(PeerEntryOutcome::RelationshipUnavailable);
        }
        let evidence = PeerEntryEvidence {
            worker: request.worker.clone(),
            claim_generation: request.claim_generation,
            entered_at_unix_ms: request.entered_at_unix_ms,
            authority: request.authority.clone(),
        };
        record.phase = PeerExecutionPhase::Entered { claim, evidence };
        bump_record(&mut record)?;
        global.dispatch_queued = global
            .dispatch_queued
            .checked_sub(1)
            .ok_or_else(|| corruption("peer dispatch count underflowed at entry"))?;
        put_execution(&write, &record)?;
        put_global_accounting(&write, global)?;
        write.commit().map_err(error::redb)?;
        Ok(PeerEntryOutcome::Entered(Box::new(record)))
    }

    fn release_peer_claim(
        &self,
        owner: &PeerId,
        execution: &PeerExecutionId,
        worker: &WorkerId,
        claim_generation: u64,
        available_at_unix_ms: u64,
    ) -> Result<PeerExecutionRecord, PersistenceError> {
        if available_at_unix_ms == 0 {
            return Err(invalid("peer dispatch availability time must be nonzero"));
        }
        let write = self.database().begin_write().map_err(error::redb)?;
        let mut record = owned_execution_in_transaction(&write, owner, execution)?;
        let claim = exact_claim(&record, worker, claim_generation)?.clone();
        let cancellation_only = matches!(
            &record.phase,
            PeerExecutionPhase::CancellationRequested { evidence: None, .. }
        );
        if !matches!(&record.phase, PeerExecutionPhase::DispatchClaimed { .. })
            && !cancellation_only
        {
            return Err(PersistenceError::ImmutableConflict {
                entity: "peer_dispatch_claim",
                identity: execution.to_string(),
            });
        }
        remove_claim_index(&write, execution, &claim)?;
        record.phase = if cancellation_only {
            PeerExecutionPhase::CancellationRequested {
                claim: None,
                evidence: None,
            }
        } else {
            PeerExecutionPhase::DispatchAvailable {
                available_at_unix_ms,
            }
        };
        bump_record(&mut record)?;
        let key = available_key(&record)?;
        write
            .open_table(PEER_DISPATCH_AVAILABLE)
            .map_err(error::redb)?
            .insert(key.as_slice(), execution.as_str())
            .map_err(error::redb)?;
        put_execution(&write, &record)?;
        write.commit().map_err(error::redb)?;
        Ok(record)
    }

    fn extend_peer_claim(
        &self,
        owner: &PeerId,
        execution: &PeerExecutionId,
        worker: &WorkerId,
        claim_generation: u64,
        lease_expires_at_unix_ms: u64,
    ) -> Result<(), PersistenceError> {
        let write = self.database().begin_write().map_err(error::redb)?;
        let mut record = owned_execution_in_transaction(&write, owner, execution)?;
        let old = exact_claim(&record, worker, claim_generation)?.clone();
        if lease_expires_at_unix_ms <= old.lease_expires_at_unix_ms {
            return Err(invalid("peer claim lease did not move forward"));
        }
        remove_claim_index(&write, execution, &old)?;
        let mut updated = old;
        updated.lease_expires_at_unix_ms = lease_expires_at_unix_ms;
        match &mut record.phase {
            PeerExecutionPhase::DispatchClaimed { claim }
            | PeerExecutionPhase::Entered { claim, .. } => *claim = updated.clone(),
            PeerExecutionPhase::CancellationRequested { claim, .. } => {
                *claim = Some(updated.clone());
            }
            _ => {
                return Err(corruption(
                    "peer claim phase changed during lease extension",
                ));
            }
        }
        bump_record(&mut record)?;
        let key = claim_key(execution, &updated)?;
        write
            .open_table(PEER_ACTIVE_CLAIMS)
            .map_err(error::redb)?
            .insert(key.as_slice(), execution.as_str())
            .map_err(error::redb)?;
        put_execution(&write, &record)?;
        write.commit().map_err(error::redb)
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
        if uncertain_at_unix_ms == 0
            || reason.is_empty()
            || reason.len() > MAX_UNCERTAINTY_REASON_BYTES
        {
            return Err(invalid("peer uncertainty reason or boundary is invalid"));
        }
        let write = self.database().begin_write().map_err(error::redb)?;
        let mut record = owned_execution_in_transaction(&write, owner, execution)?;
        if record.phase.entry_evidence().is_none() {
            return Err(PersistenceError::ImmutableConflict {
                entity: "peer_execution_entry",
                identity: execution.to_string(),
            });
        }
        let claim = exact_claim(&record, worker, claim_generation)?.clone();
        remove_claim_index(&write, execution, &claim)?;
        record.phase = PeerExecutionPhase::Uncertain {
            uncertain_at_unix_ms,
            reason: reason.to_owned(),
        };
        bump_record(&mut record)?;
        release_active_accounting(&write, &record.owner_peer, false)?;
        insert_terminal_index(&write, &record, uncertain_at_unix_ms)?;
        put_execution(&write, &record)?;
        write.commit().map_err(error::redb)?;
        Ok(record)
    }

    fn append_peer_observation(
        &self,
        owner: &PeerId,
        execution: &PeerExecutionId,
        observation: &PeerObservation,
    ) -> Result<PeerObservationAppend, PersistenceError> {
        observation
            .validate()
            .map_err(|cause| invalid(&cause.to_string()))?;
        if observation.execution != *execution {
            return Err(invalid("peer observation targets a different execution"));
        }
        let write = self.database().begin_write().map_err(error::redb)?;
        let mut record = owned_execution_in_transaction(&write, owner, execution)?;
        if observation.sequence <= record.last_observation_sequence {
            let key = observation_key(execution, observation.sequence)?;
            let observations = write.open_table(PEER_OBSERVATIONS).map_err(error::redb)?;
            let existing = observations
                .get(key.as_slice())
                .map_err(error::redb)?
                .ok_or_else(|| corruption("peer replay sequence is missing its row"))?;
            let existing: PeerObservation = json::decode(existing.value(), "peer observation")?;
            return if existing == *observation {
                Ok(PeerObservationAppend::Replayed(record))
            } else {
                Err(PersistenceError::ImmutableConflict {
                    entity: "peer_observation",
                    identity: format!("{}:{}", execution.as_str(), observation.sequence),
                })
            };
        }
        if observation.sequence != record.last_observation_sequence.saturating_add(1) {
            return Err(PersistenceError::InvalidCursor(
                "peer observation sequence is not contiguous".to_owned(),
            ));
        }
        let maximum = u64::from(record.request.limits.observations);
        if observation.sequence > maximum
            || (observation.sequence == maximum && observation.event.kind().terminal().is_none())
        {
            return Err(PersistenceError::Bounds {
                location: "peer_observations",
                reason: "accepted observation quota is exhausted".to_owned(),
            });
        }
        let is_terminal = observation.event.kind().terminal().is_some();
        let phase_allows = match &record.phase {
            PeerExecutionPhase::Entered { .. } => true,
            PeerExecutionPhase::CancellationRequested { evidence, .. } => {
                evidence.is_some() || is_terminal
            }
            PeerExecutionPhase::DispatchClaimed { .. } => is_terminal,
            PeerExecutionPhase::Uncertain { .. } => true,
            PeerExecutionPhase::DispatchAvailable { .. } | PeerExecutionPhase::Terminal { .. } => {
                false
            }
        };
        if !phase_allows {
            return Err(PersistenceError::ImmutableConflict {
                entity: "peer_execution_phase",
                identity: execution.to_string(),
            });
        }
        if record.schema_version == PEER_EXECUTION_RECORD_SCHEMA_VERSION_V2 {
            record.accounting.artifact_bytes = record
                .accounting
                .artifact_bytes
                .checked_add(record.request.input_artifact_bytes().map_err(|cause| {
                    corruption(format!("stored peer input is invalid: {cause}"))
                })?)
                .ok_or_else(|| corruption("peer artifact accounting overflowed"))?;
            record.schema_version = PEER_EXECUTION_RECORD_SCHEMA_VERSION_V3;
        }
        let output_size = observation
            .event
            .kind()
            .output()
            .map(|(_name, artifact)| {
                artifact
                    .size_bytes()
                    .ok_or_else(|| PersistenceError::Bounds {
                        location: "peer_observation_artifact",
                        reason: "peer output artifact requires an exact byte size".to_owned(),
                    })
            })
            .transpose()?;
        if output_size.is_some_and(|size| {
            record
                .accounting
                .artifact_bytes
                .checked_add(size)
                .is_none_or(|total| total > record.request.limits.artifact_bytes)
        }) {
            return Err(PersistenceError::Bounds {
                location: "peer_execution_artifact_bytes",
                reason: "cumulative input and output artifacts exceed the accepted quota"
                    .to_owned(),
            });
        }
        let key = observation_key(execution, observation.sequence)?;
        let bytes = json::encode(observation, "peer observation")?;
        let replaced = {
            let mut observations = write.open_table(PEER_OBSERVATIONS).map_err(error::redb)?;
            observations
                .insert(key.as_slice(), bytes.as_slice())
                .map_err(error::redb)?
                .is_some()
        };
        if replaced {
            return Err(corruption("peer observation append replaced a prior row"));
        }
        if let Some((_name, artifact)) = observation.event.kind().output() {
            let output_size = output_size.ok_or_else(|| {
                corruption("peer output artifact size disappeared after validation")
            })?;
            let bytes = json::encode(artifact, "peer observation artifact")?;
            write
                .open_table(PEER_OBSERVATION_ARTIFACTS)
                .map_err(error::redb)?
                .insert(key.as_slice(), bytes.as_slice())
                .map_err(error::redb)?;
            record.accounting.artifact_bytes = record
                .accounting
                .artifact_bytes
                .checked_add(output_size)
                .ok_or_else(|| corruption("peer artifact accounting overflowed"))?;
        }
        if let Some(terminal) = observation.event.kind().terminal()
            && let Some(usage) = terminal.usage()
        {
            record.accounting.duration_ms = usage.duration_ms();
            record.accounting.cost_micros = usage.cost_micros();
        }
        record.last_observation_sequence = observation.sequence;
        record.observation_digest = observation_link_digest(&record.observation_digest, &bytes)?;
        record.accounting.observations = record
            .accounting
            .observations
            .checked_add(1)
            .ok_or_else(|| corruption("peer observation accounting overflowed"))?;
        if is_terminal {
            let was_active = record.phase.is_active();
            let was_pre_entry = record.phase.entry_evidence().is_none()
                && !matches!(record.phase, PeerExecutionPhase::Uncertain { .. });
            let was_uncertain_at = match record.phase {
                PeerExecutionPhase::Uncertain {
                    uncertain_at_unix_ms,
                    ..
                } => Some(uncertain_at_unix_ms),
                _ => None,
            };
            if let Some(claim) = record.phase.claim().cloned() {
                remove_claim_index(&write, execution, &claim)?;
            } else if was_pre_entry {
                let available = available_key(&record)?;
                let removed = write
                    .open_table(PEER_DISPATCH_AVAILABLE)
                    .map_err(error::redb)?
                    .remove(available.as_slice())
                    .map_err(error::redb)?
                    .map(|value| value.value().to_owned());
                if removed.as_deref() != Some(execution.as_str()) {
                    return Err(corruption(
                        "pre-entry peer terminalization lacks its dispatch index",
                    ));
                }
            }
            if let Some(uncertain_at) = was_uncertain_at {
                remove_terminal_index(&write, execution, uncertain_at)?;
            }
            record.phase = PeerExecutionPhase::Terminal {
                sequence: observation.sequence,
                terminal_at_unix_ms: observation.observed_at_unix_ms,
            };
            if was_active {
                release_active_accounting(&write, &record.owner_peer, was_pre_entry)?;
            }
            insert_terminal_index(&write, &record, observation.observed_at_unix_ms)?;
        }
        bump_record(&mut record)?;
        put_execution(&write, &record)?;
        self.faults.check(FaultPoint::BeforePeerObservationCommit)?;
        write.commit().map_err(error::redb)?;
        self.faults.check(FaultPoint::AfterPeerObservationCommit)?;
        Ok(PeerObservationAppend::Appended(record))
    }

    fn request_peer_cancellation(
        &self,
        owner: &PeerId,
        request: &PeerCancellationRequest,
        requested_at_unix_ms: u64,
    ) -> Result<PeerExecutionRecord, PersistenceError> {
        if request.sequence == 0
            || request.reason.is_empty()
            || request.reason.len() > 512
            || requested_at_unix_ms == 0
        {
            return Err(invalid("peer cancellation request is invalid"));
        }
        let write = self.database().begin_write().map_err(error::redb)?;
        let mut record = owned_execution_in_transaction(&write, owner, &request.execution)?;
        if let Some(existing) = &record.cancellation {
            return if existing.request == *request {
                Ok(record)
            } else {
                Err(PersistenceError::ImmutableConflict {
                    entity: "peer_cancellation",
                    identity: request.execution.to_string(),
                })
            };
        }
        let next_phase = match &record.phase {
            PeerExecutionPhase::DispatchAvailable { .. } => {
                Some(PeerExecutionPhase::CancellationRequested {
                    claim: None,
                    evidence: None,
                })
            }
            PeerExecutionPhase::DispatchClaimed { claim } => {
                remove_claim_index(&write, &record.execution, claim)?;
                let key = available_key(&record)?;
                write
                    .open_table(PEER_DISPATCH_AVAILABLE)
                    .map_err(error::redb)?
                    .insert(key.as_slice(), record.execution.as_str())
                    .map_err(error::redb)?;
                Some(PeerExecutionPhase::CancellationRequested {
                    claim: None,
                    evidence: None,
                })
            }
            PeerExecutionPhase::Entered { claim, evidence } => {
                Some(PeerExecutionPhase::CancellationRequested {
                    claim: Some(claim.clone()),
                    evidence: Some(evidence.clone()),
                })
            }
            PeerExecutionPhase::CancellationRequested { .. } => {
                return Err(corruption(
                    "peer cancellation phase lacks cancellation facts",
                ));
            }
            PeerExecutionPhase::Terminal { .. } | PeerExecutionPhase::Uncertain { .. } => None,
        };
        if let Some(phase) = next_phase {
            record.phase = phase;
        }
        record.cancellation = Some(PeerCancellationRecord {
            request: request.clone(),
            requested_at_unix_ms,
            acknowledgement: None,
            acknowledged_at_unix_ms: None,
        });
        bump_record(&mut record)?;
        put_execution(&write, &record)?;
        write.commit().map_err(error::redb)?;
        Ok(record)
    }

    fn acknowledge_peer_cancellation(
        &self,
        owner: &PeerId,
        acknowledgement: &PeerCancellationAcknowledgement,
        acknowledged_at_unix_ms: u64,
    ) -> Result<PeerExecutionRecord, PersistenceError> {
        acknowledgement
            .validate()
            .map_err(|cause| invalid(&cause.to_string()))?;
        if acknowledged_at_unix_ms == 0 {
            return Err(invalid(
                "peer cancellation acknowledgement time must be nonzero",
            ));
        }
        let write = self.database().begin_write().map_err(error::redb)?;
        let mut record = owned_execution_in_transaction(&write, owner, &acknowledgement.execution)?;
        let cancellation =
            record
                .cancellation
                .as_mut()
                .ok_or_else(|| PersistenceError::ImmutableConflict {
                    entity: "peer_cancellation_request",
                    identity: acknowledgement.execution.to_string(),
                })?;
        if cancellation.request.request_id != acknowledgement.request_id {
            return Err(PersistenceError::ImmutableConflict {
                entity: "peer_cancellation_request",
                identity: acknowledgement.execution.to_string(),
            });
        }
        if let Some(existing) = &cancellation.acknowledgement {
            return if existing == acknowledgement {
                Ok(record)
            } else {
                Err(PersistenceError::ImmutableConflict {
                    entity: "peer_cancellation_acknowledgement",
                    identity: acknowledgement.execution.to_string(),
                })
            };
        }
        cancellation.acknowledgement = Some(acknowledgement.clone());
        cancellation.acknowledged_at_unix_ms = Some(acknowledged_at_unix_ms);
        bump_record(&mut record)?;
        put_execution(&write, &record)?;
        write.commit().map_err(error::redb)?;
        Ok(record)
    }

    fn recover_peer_claims(
        &self,
        recovered_at_unix_ms: u64,
        limit: milkdrift_persistence::PageSize,
    ) -> Result<PeerRecoveryResult, PersistenceError> {
        if recovered_at_unix_ms == 0 {
            return Err(invalid("peer recovery boundary must be nonzero"));
        }
        let write = self.database().begin_write().map_err(error::redb)?;
        let candidates = {
            let claims = write.open_table(PEER_ACTIVE_CLAIMS).map_err(error::redb)?;
            claims
                .iter()
                .map_err(error::redb)?
                .take(limit.get() as usize)
                .map(|row| {
                    row.map(|(key, execution)| (key.value().to_vec(), execution.value().to_owned()))
                        .map_err(error::redb)
                })
                .collect::<Result<Vec<_>, _>>()?
        };
        let mut result = PeerRecoveryResult::default();
        for (stored_claim_key, execution) in candidates {
            let mut record = execution_in_transaction_text(&write, &execution)?;
            let claim = record.phase.claim().cloned().ok_or_else(|| {
                corruption("active peer claim index points at an unclaimed phase")
            })?;
            if claim_key(&record.execution, &claim)? != stored_claim_key {
                return Err(corruption(
                    "active peer claim key disagrees with primary record",
                ));
            }
            remove_claim_index(&write, &record.execution, &claim)?;
            match record.phase.entry_evidence() {
                None if matches!(record.phase, PeerExecutionPhase::DispatchClaimed { .. }) => {
                    record.phase = PeerExecutionPhase::DispatchAvailable {
                        available_at_unix_ms: recovered_at_unix_ms,
                    };
                    let key = available_key(&record)?;
                    write
                        .open_table(PEER_DISPATCH_AVAILABLE)
                        .map_err(error::redb)?
                        .insert(key.as_slice(), record.execution.as_str())
                        .map_err(error::redb)?;
                    result.requeued = result.requeued.saturating_add(1);
                }
                None => {
                    record.phase = PeerExecutionPhase::CancellationRequested {
                        claim: None,
                        evidence: None,
                    };
                    let key = available_key(&record)?;
                    write
                        .open_table(PEER_DISPATCH_AVAILABLE)
                        .map_err(error::redb)?
                        .insert(key.as_slice(), record.execution.as_str())
                        .map_err(error::redb)?;
                    result.requeued = result.requeued.saturating_add(1);
                }
                Some(_) => {
                    record.phase = PeerExecutionPhase::Uncertain {
                        uncertain_at_unix_ms: recovered_at_unix_ms,
                        reason: "serving daemon restarted after durable adapter entry".to_owned(),
                    };
                    release_active_accounting(&write, &record.owner_peer, false)?;
                    insert_terminal_index(&write, &record, recovered_at_unix_ms)?;
                    result.uncertain = result.uncertain.saturating_add(1);
                }
            }
            bump_record(&mut record)?;
            put_execution(&write, &record)?;
        }
        result.more = write
            .open_table(PEER_ACTIVE_CLAIMS)
            .map_err(error::redb)?
            .len()
            .map_err(error::redb)?
            > 0;
        write.commit().map_err(error::redb)?;
        Ok(result)
    }

    fn archive_peer_executions(
        &self,
        request: &PeerRetentionRequest,
    ) -> Result<PeerRetentionPage, PersistenceError> {
        if request.archived_at.get() == 0 {
            return Err(invalid("peer archive boundary must be nonzero"));
        }
        let write = self.database().begin_write().map_err(error::redb)?;
        let page = archive_eligible_in_transaction(
            self,
            &write,
            request.terminal_before_or_at.get(),
            request.archived_at.get(),
            request.limit.get(),
        )?;
        self.faults.check(FaultPoint::BeforePeerArchiveCommit)?;
        write.commit().map_err(error::redb)?;
        self.faults.check(FaultPoint::AfterPeerArchiveCommit)?;
        Ok(page)
    }

    fn peer_observation_artifact(
        &self,
        execution: &PeerExecutionId,
        sequence: u64,
    ) -> Result<Option<milkdrift_capability::ArtifactReference>, PersistenceError> {
        let read = self.database().begin_read().map_err(error::redb)?;
        let key = observation_key(execution, sequence)?;
        read.open_table(PEER_OBSERVATION_ARTIFACTS)
            .map_err(error::redb)?
            .get(key.as_slice())
            .map_err(error::redb)?
            .map(|bytes| json::decode(bytes.value(), "peer observation artifact"))
            .transpose()
    }
}

fn relationship_in_transaction(
    write: &redb::WriteTransaction,
    peer: &PeerId,
) -> Result<Option<PeerRelationshipState>, PersistenceError> {
    write
        .open_table(PEER_RELATIONSHIPS)
        .map_err(error::redb)?
        .get(peer.as_str())
        .map_err(error::redb)?
        .map(|bytes| json::decode(bytes.value(), "peer relationship"))
        .transpose()
}

fn catalog_in_transaction(
    write: &redb::WriteTransaction,
    peer: &PeerId,
) -> Result<Option<PeerCatalogState>, PersistenceError> {
    write
        .open_table(PEER_CATALOGS)
        .map_err(error::redb)?
        .get(peer.as_str())
        .map_err(error::redb)?
        .map(|bytes| json::decode(bytes.value(), "peer catalog"))
        .transpose()
}

fn execution_optional_in_transaction(
    write: &redb::WriteTransaction,
    execution: &PeerExecutionId,
) -> Result<Option<PeerExecutionRecord>, PersistenceError> {
    let record = write
        .open_table(PEER_EXECUTIONS)
        .map_err(error::redb)?
        .get(execution.as_str())
        .map_err(error::redb)?
        .map(|bytes| decode_record(bytes.value()))
        .transpose()?;
    if record.is_some()
        && execution_location_in_transaction(write, execution)? != Some(LOCATION_HOT)
    {
        return Err(corruption(
            "hot peer execution disagrees with its location index",
        ));
    }
    Ok(record)
}

fn execution_optional_in_read_transaction(
    read: &redb::ReadTransaction,
    execution: &PeerExecutionId,
) -> Result<Option<PeerExecutionRecord>, PersistenceError> {
    let record = read
        .open_table(PEER_EXECUTIONS)
        .map_err(error::redb)?
        .get(execution.as_str())
        .map_err(error::redb)?
        .map(|bytes| decode_record(bytes.value()))
        .transpose()?;
    if record.is_some()
        && execution_location_in_read_transaction(read, execution)? != Some(LOCATION_HOT)
    {
        return Err(corruption(
            "hot peer execution disagrees with its location index",
        ));
    }
    Ok(record)
}

fn snapshot_optional_in_transaction(
    write: &redb::WriteTransaction,
    execution: &PeerExecutionId,
) -> Result<Option<PeerExecutionSnapshot>, PersistenceError> {
    match execution_location_in_transaction(write, execution)? {
        None => {
            let hot = write
                .open_table(PEER_EXECUTIONS)
                .map_err(error::redb)?
                .get(execution.as_str())
                .map_err(error::redb)?
                .is_some();
            let archived = write
                .open_table(PEER_EXECUTION_TOMBSTONES)
                .map_err(error::redb)?
                .get(execution.as_str())
                .map_err(error::redb)?
                .is_some();
            if hot || archived {
                return Err(corruption("peer execution exists without a location index"));
            }
            Ok(None)
        }
        Some(LOCATION_HOT) => execution_optional_in_transaction(write, execution)?
            .map(|record| PeerExecutionSnapshot::Hot(Box::new(record)))
            .ok_or_else(|| corruption("peer hot location points at a missing record"))
            .map(Some),
        Some(LOCATION_ARCHIVED) => tombstone_optional_in_transaction(write, execution)?
            .map(|tombstone| PeerExecutionSnapshot::Archived(Box::new(tombstone)))
            .ok_or_else(|| corruption("peer archived location points at a missing tombstone"))
            .map(Some),
        Some(_) => Err(corruption("peer execution location has an unknown value")),
    }
}

fn snapshot_optional_in_read_transaction(
    read: &redb::ReadTransaction,
    execution: &PeerExecutionId,
) -> Result<Option<PeerExecutionSnapshot>, PersistenceError> {
    match execution_location_in_read_transaction(read, execution)? {
        None => {
            let hot = read
                .open_table(PEER_EXECUTIONS)
                .map_err(error::redb)?
                .get(execution.as_str())
                .map_err(error::redb)?
                .is_some();
            let archived = read
                .open_table(PEER_EXECUTION_TOMBSTONES)
                .map_err(error::redb)?
                .get(execution.as_str())
                .map_err(error::redb)?
                .is_some();
            if hot || archived {
                return Err(corruption("peer execution exists without a location index"));
            }
            Ok(None)
        }
        Some(LOCATION_HOT) => execution_optional_in_read_transaction(read, execution)?
            .map(|record| PeerExecutionSnapshot::Hot(Box::new(record)))
            .ok_or_else(|| corruption("peer hot location points at a missing record"))
            .map(Some),
        Some(LOCATION_ARCHIVED) => tombstone_optional_in_read_transaction(read, execution)?
            .map(|tombstone| PeerExecutionSnapshot::Archived(Box::new(tombstone)))
            .ok_or_else(|| corruption("peer archived location points at a missing tombstone"))
            .map(Some),
        Some(_) => Err(corruption("peer execution location has an unknown value")),
    }
}

fn execution_location_in_transaction(
    write: &redb::WriteTransaction,
    execution: &PeerExecutionId,
) -> Result<Option<u8>, PersistenceError> {
    write
        .open_table(PEER_EXECUTION_LOCATIONS)
        .map_err(error::redb)?
        .get(execution.as_str())
        .map_err(error::redb)
        .map(|value| value.map(|value| value.value()))
}

fn execution_location_in_read_transaction(
    read: &redb::ReadTransaction,
    execution: &PeerExecutionId,
) -> Result<Option<u8>, PersistenceError> {
    read.open_table(PEER_EXECUTION_LOCATIONS)
        .map_err(error::redb)?
        .get(execution.as_str())
        .map_err(error::redb)
        .map(|value| value.map(|value| value.value()))
}

fn tombstone_optional_in_transaction(
    write: &redb::WriteTransaction,
    execution: &PeerExecutionId,
) -> Result<Option<PeerExecutionTombstone>, PersistenceError> {
    write
        .open_table(PEER_EXECUTION_TOMBSTONES)
        .map_err(error::redb)?
        .get(execution.as_str())
        .map_err(error::redb)?
        .map(|bytes| decode_tombstone(bytes.value()))
        .transpose()
}

fn tombstone_optional_in_read_transaction(
    read: &redb::ReadTransaction,
    execution: &PeerExecutionId,
) -> Result<Option<PeerExecutionTombstone>, PersistenceError> {
    read.open_table(PEER_EXECUTION_TOMBSTONES)
        .map_err(error::redb)?
        .get(execution.as_str())
        .map_err(error::redb)?
        .map(|bytes| decode_tombstone(bytes.value()))
        .transpose()
}

fn execution_in_transaction_text(
    write: &redb::WriteTransaction,
    execution: &str,
) -> Result<PeerExecutionRecord, PersistenceError> {
    let execution_id = PeerExecutionId::new(execution.to_owned()).map_err(|cause| {
        corruption(format!(
            "stored peer execution identity is invalid: {cause}"
        ))
    })?;
    execution_optional_in_transaction(write, &execution_id)?
        .ok_or_else(|| corruption("peer index points at a missing primary record"))
}

fn snapshot_in_transaction_text(
    write: &redb::WriteTransaction,
    execution: &str,
) -> Result<PeerExecutionSnapshot, PersistenceError> {
    let execution_id = parse_execution_id(execution)?;
    snapshot_optional_in_transaction(write, &execution_id)?
        .ok_or_else(|| corruption("peer index points at a missing execution authority"))
}

fn snapshot_in_read_transaction_text(
    read: &redb::ReadTransaction,
    execution: &str,
) -> Result<PeerExecutionSnapshot, PersistenceError> {
    let execution_id = parse_execution_id(execution)?;
    snapshot_optional_in_read_transaction(read, &execution_id)?
        .ok_or_else(|| corruption("peer index points at a missing execution authority"))
}

fn parse_execution_id(execution: &str) -> Result<PeerExecutionId, PersistenceError> {
    PeerExecutionId::new(execution.to_owned()).map_err(|cause| {
        corruption(format!(
            "stored peer execution identity is invalid: {cause}"
        ))
    })
}

fn owned_execution_in_transaction(
    write: &redb::WriteTransaction,
    owner: &PeerId,
    execution: &PeerExecutionId,
) -> Result<PeerExecutionRecord, PersistenceError> {
    execution_optional_in_transaction(write, execution)?
        .filter(|record| record.owner_peer == *owner)
        .ok_or_else(|| missing("peer_execution", execution.as_str()))
}

fn put_execution(
    write: &redb::WriteTransaction,
    record: &PeerExecutionRecord,
) -> Result<(), PersistenceError> {
    validate_record(record)?;
    let bytes = json::encode(record, "peer execution")?;
    write
        .open_table(PEER_EXECUTIONS)
        .map_err(error::redb)?
        .insert(record.execution.as_str(), bytes.as_slice())
        .map_err(error::redb)?;
    Ok(())
}

fn decode_record(bytes: &[u8]) -> Result<PeerExecutionRecord, PersistenceError> {
    let record = json::decode(bytes, "peer execution")?;
    validate_record(&record)?;
    Ok(record)
}

fn put_tombstone(
    write: &redb::WriteTransaction,
    tombstone: &PeerExecutionTombstone,
) -> Result<(), PersistenceError> {
    validate_tombstone(tombstone)?;
    let bytes = json::encode(tombstone, "peer execution tombstone")?;
    if write
        .open_table(PEER_EXECUTION_TOMBSTONES)
        .map_err(error::redb)?
        .insert(tombstone.execution.as_str(), bytes.as_slice())
        .map_err(error::redb)?
        .is_some()
    {
        return Err(corruption(
            "peer tombstone insertion replaced an existing authority",
        ));
    }
    Ok(())
}

fn decode_tombstone(bytes: &[u8]) -> Result<PeerExecutionTombstone, PersistenceError> {
    let tombstone = json::decode(bytes, "peer execution tombstone")?;
    validate_tombstone(&tombstone)?;
    Ok(tombstone)
}

fn global_accounting(
    write: &redb::WriteTransaction,
) -> Result<GlobalPeerAccounting, PersistenceError> {
    let table = write
        .open_table(PEER_EXECUTION_ACCOUNTING)
        .map_err(error::redb)?;
    let bytes = table
        .get(PEER_EXECUTION_GLOBAL_ACCOUNTING_KEY)
        .map_err(error::redb)?
        .ok_or_else(|| corruption("peer global accounting is missing"))?;
    let value: GlobalPeerAccounting = json::decode(bytes.value(), "peer global accounting")?;
    validate_global_accounting(value)
}

fn global_accounting_read(
    read: &redb::ReadTransaction,
) -> Result<GlobalPeerAccounting, PersistenceError> {
    let table = read
        .open_table(PEER_EXECUTION_ACCOUNTING)
        .map_err(error::redb)?;
    let bytes = table
        .get(PEER_EXECUTION_GLOBAL_ACCOUNTING_KEY)
        .map_err(error::redb)?
        .ok_or_else(|| corruption("peer global accounting is missing"))?;
    let value: GlobalPeerAccounting = json::decode(bytes.value(), "peer global accounting")?;
    validate_global_accounting(value)
}

fn validate_global_accounting(
    value: GlobalPeerAccounting,
) -> Result<GlobalPeerAccounting, PersistenceError> {
    if value.schema_version != PEER_ACCOUNTING_SCHEMA_VERSION
        || value.next_acceptance_sequence == 0
        || (value.archive_generation == 0) != value.last_archived_at_unix_ms.is_none()
    {
        return Err(corruption("peer global accounting schema is invalid"));
    }
    Ok(value)
}

fn put_global_accounting(
    write: &redb::WriteTransaction,
    value: GlobalPeerAccounting,
) -> Result<(), PersistenceError> {
    let bytes = json::encode(&value, "peer global accounting")?;
    write
        .open_table(PEER_EXECUTION_ACCOUNTING)
        .map_err(error::redb)?
        .insert(PEER_EXECUTION_GLOBAL_ACCOUNTING_KEY, bytes.as_slice())
        .map_err(error::redb)?;
    Ok(())
}

fn peer_accounting(
    write: &redb::WriteTransaction,
    peer: &PeerId,
) -> Result<PerPeerAccounting, PersistenceError> {
    let value = write
        .open_table(PEER_EXECUTION_ACCOUNTING)
        .map_err(error::redb)?
        .get(peer.as_str())
        .map_err(error::redb)?
        .map(|bytes| json::decode(bytes.value(), "peer relationship accounting"))
        .transpose()?
        .unwrap_or_else(|| PerPeerAccounting::empty(peer));
    if value.schema_version != PEER_ACCOUNTING_SCHEMA_VERSION || value.peer != *peer {
        return Err(corruption("per-peer accounting schema or key is invalid"));
    }
    Ok(value)
}

fn put_peer_accounting(
    write: &redb::WriteTransaction,
    value: &PerPeerAccounting,
) -> Result<(), PersistenceError> {
    let bytes = json::encode(value, "peer relationship accounting")?;
    write
        .open_table(PEER_EXECUTION_ACCOUNTING)
        .map_err(error::redb)?
        .insert(value.peer.as_str(), bytes.as_slice())
        .map_err(error::redb)?;
    Ok(())
}

fn release_active_accounting(
    write: &redb::WriteTransaction,
    owner: &PeerId,
    decrement_dispatch: bool,
) -> Result<(), PersistenceError> {
    let mut global = global_accounting(write)?;
    let mut peer = peer_accounting(write, owner)?;
    global.active = global
        .active
        .checked_sub(1)
        .ok_or_else(|| corruption("peer global active count underflowed"))?;
    if decrement_dispatch {
        global.dispatch_queued = global
            .dispatch_queued
            .checked_sub(1)
            .ok_or_else(|| corruption("peer dispatch count underflowed"))?;
    }
    global.hot_terminal = global
        .hot_terminal
        .checked_add(1)
        .ok_or_else(|| corruption("peer hot terminal count overflowed"))?;
    peer.active = peer
        .active
        .checked_sub(1)
        .ok_or_else(|| corruption("per-peer active count underflowed"))?;
    peer.revision = peer
        .revision
        .checked_add(1)
        .ok_or_else(|| corruption("per-peer accounting revision overflowed"))?;
    put_global_accounting(write, global)?;
    put_peer_accounting(write, &peer)
}

fn exact_pre_entry_claim<'a>(
    record: &'a PeerExecutionRecord,
    worker: &WorkerId,
    generation: u64,
) -> Result<&'a PeerDispatchClaim, PersistenceError> {
    match &record.phase {
        PeerExecutionPhase::DispatchClaimed { claim }
            if claim.worker == *worker && claim.generation == generation =>
        {
            Ok(claim)
        }
        _ => Err(PersistenceError::ImmutableConflict {
            entity: "peer_dispatch_claim",
            identity: record.execution.to_string(),
        }),
    }
}

fn exact_claim<'a>(
    record: &'a PeerExecutionRecord,
    worker: &WorkerId,
    generation: u64,
) -> Result<&'a PeerDispatchClaim, PersistenceError> {
    record
        .phase
        .claim()
        .filter(|claim| claim.worker == *worker && claim.generation == generation)
        .ok_or_else(|| PersistenceError::ImmutableConflict {
            entity: "peer_dispatch_claim",
            identity: record.execution.to_string(),
        })
}

fn remove_claim_index(
    write: &redb::WriteTransaction,
    execution: &PeerExecutionId,
    claim: &PeerDispatchClaim,
) -> Result<(), PersistenceError> {
    let key = claim_key(execution, claim)?;
    let removed = write
        .open_table(PEER_ACTIVE_CLAIMS)
        .map_err(error::redb)?
        .remove(key.as_slice())
        .map_err(error::redb)?
        .map(|value| value.value().to_owned());
    if removed.as_deref() != Some(execution.as_str()) {
        return Err(corruption("peer claim index is missing or mismatched"));
    }
    Ok(())
}

fn insert_terminal_index(
    write: &redb::WriteTransaction,
    record: &PeerExecutionRecord,
    terminal_at: u64,
) -> Result<(), PersistenceError> {
    let key = ordered_key(terminal_at, record.execution.as_str())?;
    write
        .open_table(PEER_TERMINAL_INDEX)
        .map_err(error::redb)?
        .insert(key.as_slice(), record.execution.as_str())
        .map_err(error::redb)?;
    Ok(())
}

fn remove_terminal_index(
    write: &redb::WriteTransaction,
    execution: &PeerExecutionId,
    terminal_at: u64,
) -> Result<(), PersistenceError> {
    let key = ordered_key(terminal_at, execution.as_str())?;
    let removed = write
        .open_table(PEER_TERMINAL_INDEX)
        .map_err(error::redb)?
        .remove(key.as_slice())
        .map_err(error::redb)?
        .map(|value| value.value().to_owned());
    if removed.as_deref() != Some(execution.as_str()) {
        return Err(corruption("peer terminal index is missing or mismatched"));
    }
    Ok(())
}

fn archive_eligible_in_transaction(
    store: &RedbStore,
    write: &redb::WriteTransaction,
    terminal_before_or_at: u64,
    archived_at: u64,
    limit: u32,
) -> Result<PeerRetentionPage, PersistenceError> {
    if terminal_before_or_at == 0 || archived_at == 0 || limit == 0 {
        return Err(invalid(
            "peer archival boundaries and limit must be nonzero",
        ));
    }
    let candidates = {
        let terminal = write.open_table(PEER_TERMINAL_INDEX).map_err(error::redb)?;
        terminal
            .iter()
            .map_err(error::redb)?
            .take(limit as usize + 1)
            .map(|row| {
                row.map(|(key, execution)| (key.value().to_vec(), execution.value().to_owned()))
                    .map_err(error::redb)
            })
            .collect::<Result<Vec<_>, _>>()?
    };
    let mut archived = 0_u32;
    let mut more = false;
    for (terminal_key, execution_text) in candidates {
        let terminal_at = decode_ordered_time(&terminal_key)?;
        if terminal_at > terminal_before_or_at {
            break;
        }
        if archived >= limit {
            more = true;
            break;
        }
        let execution = parse_execution_id(&execution_text)?;
        if execution_location_in_transaction(write, &execution)? != Some(LOCATION_HOT) {
            return Err(corruption(
                "peer terminal index does not point at singular hot ownership",
            ));
        }
        if tombstone_optional_in_transaction(write, &execution)?.is_some() {
            return Err(corruption(
                "peer execution has hot and tombstone dual ownership",
            ));
        }
        let record = execution_optional_in_transaction(write, &execution)?
            .ok_or_else(|| corruption("peer terminal index points at a missing hot record"))?;
        if !matches!(
            record.phase,
            PeerExecutionPhase::Terminal { .. } | PeerExecutionPhase::Uncertain { .. }
        ) {
            return Err(corruption("peer terminal index points at an active record"));
        }
        let request_key = request_key(&record.owner_peer, &record.request.request_id)?;
        let indexed_execution = write
            .open_table(PEER_EXECUTIONS_BY_REQUEST)
            .map_err(error::redb)?
            .get(request_key.as_slice())
            .map_err(error::redb)?
            .map(|value| value.value().to_owned());
        if indexed_execution.as_deref() != Some(record.execution.as_str()) {
            return Err(corruption(
                "peer request index disagrees before tombstone creation",
            ));
        }
        let tombstone = tombstone_from_record(write, &record, archived_at)?;
        put_tombstone(write, &tombstone)?;
        store.faults.check(FaultPoint::AfterPeerTombstoneInsert)?;
        let previous_location = write
            .open_table(PEER_EXECUTION_LOCATIONS)
            .map_err(error::redb)?
            .insert(execution.as_str(), LOCATION_ARCHIVED)
            .map_err(error::redb)?
            .map(|value| value.value());
        if previous_location != Some(LOCATION_HOT) {
            return Err(corruption(
                "peer archive location transition lost hot ownership",
            ));
        }
        compact_observation_rows(write, &record)?;
        store
            .faults
            .check(FaultPoint::AfterPeerObservationCleanup)?;
        remove_terminal_index(write, &record.execution, terminal_at)?;
        let mut hot = write.open_table(PEER_EXECUTIONS).map_err(error::redb)?;
        let removed = hot.remove(execution.as_str()).map_err(error::redb)?;
        if removed.is_none() {
            return Err(corruption("peer hot record disappeared during archival"));
        }
        store.faults.check(FaultPoint::AfterPeerHotRemove)?;
        let mut global = global_accounting(write)?;
        global.hot_terminal = global
            .hot_terminal
            .checked_sub(1)
            .ok_or_else(|| corruption("peer hot terminal count underflowed during archival"))?;
        global.tombstones = global
            .tombstones
            .checked_add(1)
            .ok_or_else(|| corruption("peer tombstone count overflowed"))?;
        put_global_accounting(write, global)?;
        store.faults.check(FaultPoint::AfterPeerArchiveAccounting)?;
        archived = archived.saturating_add(1);
    }
    if archived > 0 {
        let mut global = global_accounting(write)?;
        global.archive_generation = global
            .archive_generation
            .checked_add(1)
            .ok_or_else(|| corruption("peer archive generation overflowed"))?;
        global.last_archived_at_unix_ms = Some(archived_at);
        put_global_accounting(write, global)?;
    }
    Ok(PeerRetentionPage { archived, more })
}

fn tombstone_from_record(
    write: &redb::WriteTransaction,
    record: &PeerExecutionRecord,
    archived_at_unix_ms: u64,
) -> Result<PeerExecutionTombstone, PersistenceError> {
    let disposition = match &record.phase {
        PeerExecutionPhase::Terminal { sequence, .. } => {
            let key = observation_key(&record.execution, *sequence)?;
            let observations = write.open_table(PEER_OBSERVATIONS).map_err(error::redb)?;
            let observation = observations
                .get(key.as_slice())
                .map_err(error::redb)?
                .ok_or_else(|| corruption("terminal peer record lacks its final observation"))?;
            let observation: PeerObservation =
                json::decode(observation.value(), "peer observation")?;
            PeerArchivedDisposition::Terminal {
                observation: Box::new(observation),
            }
        }
        PeerExecutionPhase::Uncertain {
            uncertain_at_unix_ms,
            reason,
        } => PeerArchivedDisposition::Uncertain {
            uncertain_at_unix_ms: *uncertain_at_unix_ms,
            reason: reason.clone(),
        },
        _ => return Err(corruption("active peer record cannot become a tombstone")),
    };
    let operation = record.request.selection.operation_contract();
    let tombstone = PeerExecutionTombstone {
        schema_version: PEER_EXECUTION_TOMBSTONE_SCHEMA_VERSION_V1,
        owner_peer: record.owner_peer.clone(),
        target_peer: record.request.delegation.target_peer.clone(),
        delegation_ref: record.request.delegation.reference.clone(),
        relationship_generation: record.relationship_generation,
        request_id: record.request.request_id.clone(),
        request_digest: record.request.request_digest.clone(),
        execution: record.execution.clone(),
        acceptance_sequence: record.acceptance_sequence,
        accepted_at_unix_ms: record.accepted_at_unix_ms,
        catalog_generation: record.request.catalog_generation,
        catalog_digest: record.request.catalog_digest.as_str().to_owned(),
        capability: record.request.selection.capability().clone(),
        capability_generation: record.request.selection.descriptor_revision(),
        capability_digest: record.request.selection.digest().to_owned(),
        operation: record.request.selection.operation().clone(),
        side_effect: operation.side_effect(),
        idempotency: operation.idempotency(),
        authority: PeerAcceptedAuthoritySummary {
            decision: record.authority.request().decision.clone(),
            actor: record.authority.request().actor.clone(),
            grant: record.authority.request().grant.clone(),
            grant_revision: record.authority.request().grant_revision,
            grant_digest: record.authority.request().grant_digest.clone(),
            revocation_generation: record.authority.request().revocation_generation,
            policy: record.authority.policy().clone(),
            policy_version: record.authority.policy_version(),
            decision_digest: record.authority.digest().to_owned(),
        },
        provenance: record.request.delegation.provenance.clone(),
        disposition,
        cancellation: record.cancellation.clone(),
        last_observation_sequence: record.last_observation_sequence,
        observation_digest: record.observation_digest.clone(),
        accounting: record.accounting,
        compacted_through_sequence: record.last_observation_sequence,
        archived_at_unix_ms,
    };
    validate_tombstone(&tombstone)?;
    Ok(tombstone)
}

fn compact_observation_rows(
    write: &redb::WriteTransaction,
    record: &PeerExecutionRecord,
) -> Result<(), PersistenceError> {
    let mut observations = write.open_table(PEER_OBSERVATIONS).map_err(error::redb)?;
    let mut artifacts = write
        .open_table(PEER_OBSERVATION_ARTIFACTS)
        .map_err(error::redb)?;
    for sequence in 1..=record.last_observation_sequence {
        let key = observation_key(&record.execution, sequence)?;
        if observations
            .remove(key.as_slice())
            .map_err(error::redb)?
            .is_none()
        {
            return Err(corruption(
                "peer archival encountered a missing observation row",
            ));
        }
        artifacts.remove(key.as_slice()).map_err(error::redb)?;
    }
    Ok(())
}

fn request_key(owner: &PeerId, request: &PeerRequestId) -> Result<Vec<u8>, PersistenceError> {
    codec::pair(owner.as_str(), request.as_str())
}

fn observation_key(
    execution: &PeerExecutionId,
    sequence: u64,
) -> Result<Vec<u8>, PersistenceError> {
    if sequence == 0 {
        return Err(invalid("peer observation sequence must be nonzero"));
    }
    ordered_key(sequence, execution.as_str())
}

fn available_key(record: &PeerExecutionRecord) -> Result<Vec<u8>, PersistenceError> {
    ordered_key(record.acceptance_sequence, record.execution.as_str())
}

fn claim_key(
    execution: &PeerExecutionId,
    claim: &PeerDispatchClaim,
) -> Result<Vec<u8>, PersistenceError> {
    ordered_key(claim.lease_expires_at_unix_ms, execution.as_str())
}

fn ordered_key(number: u64, identity: &str) -> Result<Vec<u8>, PersistenceError> {
    let mut key = number.to_be_bytes().to_vec();
    key.extend_from_slice(&codec::component(identity)?);
    Ok(key)
}

fn decode_ordered_time(key: &[u8]) -> Result<u64, PersistenceError> {
    let bytes: [u8; 8] = key
        .get(..8)
        .ok_or_else(|| corruption("ordered peer index key is truncated"))?
        .try_into()
        .map_err(|_| corruption("ordered peer index key is malformed"))?;
    Ok(u64::from_be_bytes(bytes))
}

fn bump_record(record: &mut PeerExecutionRecord) -> Result<(), PersistenceError> {
    record.revision = record
        .revision
        .checked_add(1)
        .ok_or_else(|| corruption("peer execution revision overflowed"))?;
    Ok(())
}

fn validate_relationship(value: &PeerRelationshipState) -> Result<(), PersistenceError> {
    if value.generation == 0 || value.expires_at_unix_ms == 0 || value.maximum_active == 0 {
        return Err(invalid("peer relationship persistence facts are invalid"));
    }
    Ok(())
}

fn validate_catalog(value: &PeerCatalogState) -> Result<(), PersistenceError> {
    if value.relationship_generation == 0
        || value.generation == 0
        || value.expires_at_unix_ms == 0
        || !valid_prefixed_blake3(&value.digest)
    {
        return Err(invalid("peer catalog persistence facts are invalid"));
    }
    Ok(())
}

fn validate_admission(value: &PeerAdmission<'_>) -> Result<(), PersistenceError> {
    value
        .request
        .validate()
        .map_err(|cause| invalid(&cause.to_string()))?;
    let decision_request = value.authority.request();
    let resources = &decision_request.resources;
    let provenance = &decision_request.provenance;
    let delegated = &value.request.delegation.provenance;
    if !value.authority.is_allowed()
        || decision_request.operation != AuthorityOperation::InvokePeerCapability
        || decision_request.actor != value.request.delegation.actor
        || resources.peer.as_ref() != Some(value.owner_peer)
        || resources.capability.as_ref() != Some(value.request.selection.capability())
        || resources.capability_operation.as_ref() != Some(value.request.selection.operation())
        || provenance
            .revision
            .as_ref()
            .map(ToString::to_string)
            .as_deref()
            != Some(delegated.revision.as_str())
        || provenance.node.as_ref().map(ToString::to_string).as_deref()
            != Some(delegated.node.as_str())
        || provenance.execution.as_deref() != Some(delegated.execution.as_str())
        || provenance.attempt.as_deref() != Some(delegated.attempt.as_str())
        || provenance.descriptor_revision != Some(value.request.selection.descriptor_revision())
        || value.relationship_generation == 0
        || value.accepted_at_unix_ms == 0
        || value.maximum_global_active == 0
        || value.maximum_dispatch_queue == 0
        || value.maximum_hot_terminal_records == 0
        || value.archive_batch_size == 0
        || value.archive_terminal_before_or_at_unix_ms == 0
        || value.maximum_hot_terminal_records < u64::from(value.maximum_global_active)
    {
        return Err(invalid("peer admission persistence facts are invalid"));
    }
    Ok(())
}

fn validate_entry_authority(
    record: &PeerExecutionRecord,
    authority: &milkdrift_authority::AuthorityDecisionSnapshot,
) -> Result<(), PersistenceError> {
    let accepted = record.authority.request();
    let entry = authority.request();
    if !authority.is_allowed()
        || entry.operation != AuthorityOperation::InvokePeerCapability
        || entry.actor != record.request.delegation.actor
        || entry.resources != accepted.resources
        || entry.budget != accepted.budget
        || entry.provenance != accepted.provenance
    {
        return Err(invalid(
            "peer adapter-entry authority does not match the accepted execution envelope",
        ));
    }
    Ok(())
}

fn validate_record(record: &PeerExecutionRecord) -> Result<(), PersistenceError> {
    record
        .request
        .validate()
        .map_err(|cause| corruption(format!("stored peer request is invalid: {cause}")))?;
    if !matches!(
        record.schema_version,
        PEER_EXECUTION_RECORD_SCHEMA_VERSION_V2 | PEER_EXECUTION_RECORD_SCHEMA_VERSION_V3
    ) || record.relationship_generation == 0
        || record.acceptance_sequence == 0
        || record.accepted_at_unix_ms == 0
        || record.revision == 0
        || u64::from(record.accounting.observations) != record.last_observation_sequence
        || record.last_observation_sequence > u64::from(record.request.limits.observations)
        || record.accounting.artifact_bytes > record.request.limits.artifact_bytes
        || (record.schema_version == PEER_EXECUTION_RECORD_SCHEMA_VERSION_V3
            && record.accounting.artifact_bytes
                < record.request.input_artifact_bytes().map_err(|cause| {
                    corruption(format!("stored peer input is invalid: {cause}"))
                })?)
        || !valid_prefixed_blake3(&record.observation_digest)
    {
        return Err(corruption(
            "stored peer execution primary facts are invalid",
        ));
    }
    if let PeerExecutionPhase::Terminal { sequence, .. } = record.phase
        && sequence != record.last_observation_sequence
    {
        return Err(corruption(
            "stored peer terminal sequence disagrees with its head",
        ));
    }
    if matches!(
        record.phase,
        PeerExecutionPhase::CancellationRequested { .. }
    ) != record.cancellation.is_some()
        && record.phase.is_active()
    {
        return Err(corruption(
            "stored peer cancellation phase disagrees with its facts",
        ));
    }
    Ok(())
}

fn validate_tombstone(tombstone: &PeerExecutionTombstone) -> Result<(), PersistenceError> {
    if tombstone.schema_version != PEER_EXECUTION_TOMBSTONE_SCHEMA_VERSION_V1
        || tombstone.relationship_generation == 0
        || tombstone.acceptance_sequence == 0
        || tombstone.accepted_at_unix_ms == 0
        || tombstone.catalog_generation == 0
        || tombstone.capability_generation == 0
        || tombstone.authority.grant_revision == 0
        || tombstone.authority.policy_version == 0
        || tombstone.archived_at_unix_ms == 0
        || tombstone.compacted_through_sequence != tombstone.last_observation_sequence
        || u64::from(tombstone.accounting.observations) != tombstone.last_observation_sequence
        || !valid_prefixed_blake3(&tombstone.request_digest)
        || !valid_prefixed_blake3(&tombstone.catalog_digest)
        || !valid_capability_digest(&tombstone.capability_digest)
        || !valid_prefixed_blake3(&tombstone.authority.decision_digest)
        || !valid_prefixed_blake3(&tombstone.observation_digest)
    {
        return Err(corruption(
            "stored peer execution tombstone facts are invalid",
        ));
    }
    match &tombstone.disposition {
        PeerArchivedDisposition::Terminal { observation } => {
            observation.validate().map_err(|cause| {
                corruption(format!("archived terminal summary is invalid: {cause}"))
            })?;
            if observation.execution != tombstone.execution
                || observation.sequence != tombstone.last_observation_sequence
                || observation.event.kind().terminal().is_none()
                || observation.observed_at_unix_ms > tombstone.archived_at_unix_ms
            {
                return Err(corruption(
                    "archived terminal summary disagrees with its tombstone",
                ));
            }
        }
        PeerArchivedDisposition::Uncertain {
            uncertain_at_unix_ms,
            reason,
        } => {
            if *uncertain_at_unix_ms == 0
                || *uncertain_at_unix_ms > tombstone.archived_at_unix_ms
                || reason.is_empty()
                || reason.len() > MAX_UNCERTAINTY_REASON_BYTES
            {
                return Err(corruption("archived uncertainty summary is invalid"));
            }
        }
    }
    if tombstone.cancellation.as_ref().is_some_and(|cancellation| {
        cancellation.request.execution != tombstone.execution
            || cancellation
                .acknowledgement
                .as_ref()
                .is_some_and(|acknowledgement| acknowledgement.execution != tombstone.execution)
    }) {
        return Err(corruption(
            "archived cancellation facts target another execution",
        ));
    }
    Ok(())
}

fn observation_genesis_digest() -> String {
    let mut hasher = blake3::Hasher::new();
    hasher.update(OBSERVATION_DIGEST_DOMAIN);
    hasher.update(b"genesis");
    format!("b3_{}", hasher.finalize().to_hex())
}

fn observation_link_digest(
    previous: &str,
    observation_document: &[u8],
) -> Result<String, PersistenceError> {
    if !valid_prefixed_blake3(previous) {
        return Err(corruption("peer observation history digest is invalid"));
    }
    let mut hasher = blake3::Hasher::new();
    hasher.update(OBSERVATION_DIGEST_DOMAIN);
    hasher.update(previous.as_bytes());
    hasher.update(observation_document);
    Ok(format!("b3_{}", hasher.finalize().to_hex()))
}

fn verify_integrity(store: &RedbStore) -> Result<(), PersistenceError> {
    let read = store.database().begin_read().map_err(error::redb)?;
    let global = global_accounting_read(&read)?;
    let hot = read.open_table(PEER_EXECUTIONS).map_err(error::redb)?;
    let tombstones = read
        .open_table(PEER_EXECUTION_TOMBSTONES)
        .map_err(error::redb)?;
    let locations = read
        .open_table(PEER_EXECUTION_LOCATIONS)
        .map_err(error::redb)?;
    let requests = read
        .open_table(PEER_EXECUTIONS_BY_REQUEST)
        .map_err(error::redb)?;
    let available = read
        .open_table(PEER_DISPATCH_AVAILABLE)
        .map_err(error::redb)?;
    let claims = read.open_table(PEER_ACTIVE_CLAIMS).map_err(error::redb)?;
    let terminal = read.open_table(PEER_TERMINAL_INDEX).map_err(error::redb)?;
    let observations = read.open_table(PEER_OBSERVATIONS).map_err(error::redb)?;
    let observation_artifacts = read
        .open_table(PEER_OBSERVATION_ARTIFACTS)
        .map_err(error::redb)?;

    let mut active_count = 0_u32;
    let mut dispatch_count = 0_u32;
    let mut hot_terminal_count = 0_u64;
    let mut available_index_count = 0_u64;
    let mut claim_index_count = 0_u64;
    let mut terminal_index_count = 0_u64;
    let mut observation_count = 0_u64;
    let mut observation_artifact_count = 0_u64;
    for row in hot.iter().map_err(error::redb)? {
        let (key, bytes) = row.map_err(error::redb)?;
        let record = decode_record(bytes.value())?;
        if key.value() != record.execution.as_str()
            || locations
                .get(record.execution.as_str())
                .map_err(error::redb)?
                .map(|value| value.value())
                != Some(LOCATION_HOT)
            || tombstones
                .get(record.execution.as_str())
                .map_err(error::redb)?
                .is_some()
        {
            return Err(corruption(
                "peer hot record key/location/tombstone ownership is inconsistent",
            ));
        }
        let request_index_key = request_key(&record.owner_peer, &record.request.request_id)?;
        if requests
            .get(request_index_key.as_slice())
            .map_err(error::redb)?
            .map(|value| value.value().to_owned())
            .as_deref()
            != Some(record.execution.as_str())
        {
            return Err(corruption("peer hot record request index is inconsistent"));
        }
        let mut recomputed_digest = observation_genesis_digest();
        observation_count = observation_count
            .checked_add(record.last_observation_sequence)
            .ok_or_else(|| corruption("peer integrity observation count overflowed"))?;
        for sequence in 1..=record.last_observation_sequence {
            let observation_key = observation_key(&record.execution, sequence)?;
            let stored = observations
                .get(observation_key.as_slice())
                .map_err(error::redb)?
                .ok_or_else(|| corruption("peer hot observation history has a missing row"))?;
            let observation: PeerObservation = json::decode(stored.value(), "peer observation")?;
            if observation.execution != record.execution || observation.sequence != sequence {
                return Err(corruption(
                    "peer observation row key/document integrity is inconsistent",
                ));
            }
            let stored_artifact = observation_artifacts
                .get(observation_key.as_slice())
                .map_err(error::redb)?;
            match (observation.event.kind().output(), stored_artifact) {
                (Some((_name, expected)), Some(stored)) => {
                    let stored: milkdrift_capability::ArtifactReference =
                        json::decode(stored.value(), "peer observation artifact")?;
                    if stored != *expected {
                        return Err(corruption(
                            "peer observation artifact mapping is inconsistent",
                        ));
                    }
                    observation_artifact_count =
                        observation_artifact_count.checked_add(1).ok_or_else(|| {
                            corruption("peer integrity artifact mapping count overflowed")
                        })?;
                }
                (None, None) => {}
                (Some(_), None) | (None, Some(_)) => {
                    return Err(corruption(
                        "peer observation artifact mapping presence is inconsistent",
                    ));
                }
            }
            recomputed_digest = observation_link_digest(&recomputed_digest, stored.value())?;
        }
        if recomputed_digest != record.observation_digest {
            return Err(corruption(
                "peer observation history digest is inconsistent",
            ));
        }
        if record.phase.is_active() {
            active_count = active_count
                .checked_add(1)
                .ok_or_else(|| corruption("peer integrity active count overflowed"))?;
            let accounting = peer_accounting_read(&read, &record.owner_peer)?;
            if accounting.active == 0 {
                return Err(corruption("active peer record has no per-peer accounting"));
            }
        }
        match &record.phase {
            PeerExecutionPhase::DispatchAvailable { .. }
            | PeerExecutionPhase::CancellationRequested {
                claim: None,
                evidence: None,
            } => {
                dispatch_count = dispatch_count.saturating_add(1);
                available_index_count = available_index_count.saturating_add(1);
                let key = available_key(&record)?;
                if available
                    .get(key.as_slice())
                    .map_err(error::redb)?
                    .map(|value| value.value().to_owned())
                    .as_deref()
                    != Some(record.execution.as_str())
                {
                    return Err(corruption("peer dispatch-available index is inconsistent"));
                }
            }
            PeerExecutionPhase::DispatchClaimed { claim } => {
                dispatch_count = dispatch_count.saturating_add(1);
                claim_index_count = claim_index_count.saturating_add(1);
                verify_claim_index(&claims, &record, claim)?;
            }
            PeerExecutionPhase::Entered { claim, .. }
            | PeerExecutionPhase::CancellationRequested {
                claim: Some(claim), ..
            } => {
                claim_index_count = claim_index_count.saturating_add(1);
                verify_claim_index(&claims, &record, claim)?;
            }
            PeerExecutionPhase::Terminal {
                terminal_at_unix_ms,
                ..
            } => {
                hot_terminal_count = hot_terminal_count.saturating_add(1);
                terminal_index_count = terminal_index_count.saturating_add(1);
                verify_terminal_index(&terminal, &record, *terminal_at_unix_ms)?;
            }
            PeerExecutionPhase::Uncertain {
                uncertain_at_unix_ms,
                ..
            } => {
                hot_terminal_count = hot_terminal_count.saturating_add(1);
                terminal_index_count = terminal_index_count.saturating_add(1);
                verify_terminal_index(&terminal, &record, *uncertain_at_unix_ms)?;
            }
            PeerExecutionPhase::CancellationRequested { .. } => {
                return Err(corruption(
                    "peer cancellation phase has inconsistent claim evidence",
                ));
            }
        }
    }

    let mut tombstone_count = 0_u64;
    for row in tombstones.iter().map_err(error::redb)? {
        let (key, bytes) = row.map_err(error::redb)?;
        let tombstone = decode_tombstone(bytes.value())?;
        tombstone_count = tombstone_count.saturating_add(1);
        if key.value() != tombstone.execution.as_str()
            || locations
                .get(tombstone.execution.as_str())
                .map_err(error::redb)?
                .map(|value| value.value())
                != Some(LOCATION_ARCHIVED)
            || hot
                .get(tombstone.execution.as_str())
                .map_err(error::redb)?
                .is_some()
        {
            return Err(corruption(
                "peer tombstone key/location/hot ownership is inconsistent",
            ));
        }
        let request_index_key = request_key(&tombstone.owner_peer, &tombstone.request_id)?;
        if requests
            .get(request_index_key.as_slice())
            .map_err(error::redb)?
            .map(|value| value.value().to_owned())
            .as_deref()
            != Some(tombstone.execution.as_str())
        {
            return Err(corruption("peer tombstone request index is inconsistent"));
        }
        if tombstone.last_observation_sequence > 0 {
            let last = observation_key(&tombstone.execution, tombstone.last_observation_sequence)?;
            if observations
                .get(last.as_slice())
                .map_err(error::redb)?
                .is_some()
                || observation_artifacts
                    .get(last.as_slice())
                    .map_err(error::redb)?
                    .is_some()
            {
                return Err(corruption(
                    "archived peer execution retained a compacted hot observation row",
                ));
            }
        }
    }

    if active_count != global.active
        || dispatch_count != global.dispatch_queued
        || hot_terminal_count != global.hot_terminal
        || tombstone_count != global.tombstones
        || available.len().map_err(error::redb)? != available_index_count
        || claims.len().map_err(error::redb)? != claim_index_count
        || terminal.len().map_err(error::redb)? != terminal_index_count
        || observations.len().map_err(error::redb)? != observation_count
        || observation_artifacts.len().map_err(error::redb)? != observation_artifact_count
        || hot.len().map_err(error::redb)?
            != u64::from(global.active).saturating_add(global.hot_terminal)
        || locations.len().map_err(error::redb)?
            != hot
                .len()
                .map_err(error::redb)?
                .saturating_add(tombstone_count)
        || requests.len().map_err(error::redb)? != locations.len().map_err(error::redb)?
    {
        return Err(corruption(
            "peer global counters or primary index cardinality drifted",
        ));
    }

    verify_peer_indexes(&read)?;
    verify_per_peer_accounting(&read)?;
    Ok(())
}

fn verify_claim_index(
    claims: &impl ReadableTable<&'static [u8], &'static str>,
    record: &PeerExecutionRecord,
    claim: &PeerDispatchClaim,
) -> Result<(), PersistenceError> {
    let key = claim_key(&record.execution, claim)?;
    if claims
        .get(key.as_slice())
        .map_err(error::redb)?
        .map(|value| value.value().to_owned())
        .as_deref()
        != Some(record.execution.as_str())
    {
        return Err(corruption("peer active-claim index is inconsistent"));
    }
    Ok(())
}

fn verify_terminal_index(
    terminal: &impl ReadableTable<&'static [u8], &'static str>,
    record: &PeerExecutionRecord,
    terminal_at: u64,
) -> Result<(), PersistenceError> {
    let key = ordered_key(terminal_at, record.execution.as_str())?;
    if terminal
        .get(key.as_slice())
        .map_err(error::redb)?
        .map(|value| value.value().to_owned())
        .as_deref()
        != Some(record.execution.as_str())
    {
        return Err(corruption("peer hot-terminal index is inconsistent"));
    }
    Ok(())
}

fn peer_accounting_read(
    read: &redb::ReadTransaction,
    peer: &PeerId,
) -> Result<PerPeerAccounting, PersistenceError> {
    let value = read
        .open_table(PEER_EXECUTION_ACCOUNTING)
        .map_err(error::redb)?
        .get(peer.as_str())
        .map_err(error::redb)?
        .map(|bytes| json::decode(bytes.value(), "peer relationship accounting"))
        .transpose()?
        .unwrap_or_else(|| PerPeerAccounting::empty(peer));
    if value.schema_version != PEER_ACCOUNTING_SCHEMA_VERSION || value.peer != *peer {
        return Err(corruption("per-peer accounting schema or key is invalid"));
    }
    Ok(value)
}

fn verify_per_peer_accounting(read: &redb::ReadTransaction) -> Result<(), PersistenceError> {
    let accounting = read
        .open_table(PEER_EXECUTION_ACCOUNTING)
        .map_err(error::redb)?;
    let hot = read.open_table(PEER_EXECUTIONS).map_err(error::redb)?;
    for row in accounting.iter().map_err(error::redb)? {
        let (key, bytes) = row.map_err(error::redb)?;
        if key.value() == PEER_EXECUTION_GLOBAL_ACCOUNTING_KEY {
            continue;
        }
        let value: PerPeerAccounting = json::decode(bytes.value(), "peer relationship accounting")?;
        if value.peer.as_str() != key.value()
            || value.schema_version != PEER_ACCOUNTING_SCHEMA_VERSION
        {
            return Err(corruption(
                "per-peer accounting row key/schema is inconsistent",
            ));
        }
        let mut actual = 0_u32;
        for record in hot.iter().map_err(error::redb)? {
            let (_key, bytes) = record.map_err(error::redb)?;
            let record = decode_record(bytes.value())?;
            if record.owner_peer == value.peer && record.phase.is_active() {
                actual = actual.saturating_add(1);
            }
        }
        if actual != value.active {
            return Err(corruption("per-peer active accounting drifted"));
        }
    }
    Ok(())
}

fn verify_peer_indexes(read: &redb::ReadTransaction) -> Result<(), PersistenceError> {
    let requests = read
        .open_table(PEER_EXECUTIONS_BY_REQUEST)
        .map_err(error::redb)?;
    for row in requests.iter().map_err(error::redb)? {
        let (stored_key, execution) = row.map_err(error::redb)?;
        let snapshot = snapshot_in_read_transaction_text(read, execution.value())?;
        if request_key(snapshot.owner_peer(), snapshot.request_id())? != stored_key.value() {
            return Err(corruption(
                "peer request index key disagrees with its authority",
            ));
        }
    }
    let locations = read
        .open_table(PEER_EXECUTION_LOCATIONS)
        .map_err(error::redb)?;
    for row in locations.iter().map_err(error::redb)? {
        let (execution, location) = row.map_err(error::redb)?;
        let snapshot = snapshot_in_read_transaction_text(read, execution.value())?;
        if !matches!(
            (location.value(), snapshot),
            (LOCATION_HOT, PeerExecutionSnapshot::Hot(_))
                | (LOCATION_ARCHIVED, PeerExecutionSnapshot::Archived(_))
        ) {
            return Err(corruption(
                "peer execution location points at the wrong authority",
            ));
        }
    }
    Ok(())
}

fn invalid(message: &str) -> PersistenceError {
    PersistenceError::InvalidDocument(message.to_owned())
}

fn missing(entity: &'static str, identity: &str) -> PersistenceError {
    PersistenceError::NotFound {
        entity,
        identity: identity.to_owned(),
    }
}

fn corruption(message: impl Into<String>) -> PersistenceError {
    PersistenceError::Storage {
        class: StorageFailureClass::Corruption,
        message: message.into(),
    }
}

fn valid_prefixed_blake3(value: &str) -> bool {
    value.strip_prefix("b3_").is_some_and(|digest| {
        digest.len() == 64
            && digest
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    })
}

fn valid_capability_digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}
