//! Complete claim, entry, release, uncertainty and restart-recovery transactions.
use super::{
    MAX_UNCERTAINTY_REASON_BYTES, accounting::global_accounting, accounting::put_global_accounting,
    accounting::release_active_accounting, available_key, bump_record, claim_key, corruption,
    exact_claim, exact_pre_entry_claim, execution_in_transaction_text, insert_terminal_index,
    invalid, owned_execution_in_transaction, put_execution, relationship_in_transaction,
    remove_claim_index, validation::validate_entry_authority, validation::validate_record,
};
use crate::{
    RedbStore, error, fault::FaultPoint, schema::PEER_ACTIVE_CLAIMS,
    schema::PEER_DISPATCH_AVAILABLE,
};
use milkdrift_capability::PeerId;
use milkdrift_peer_protocol::PeerExecutionId;
use milkdrift_persistence::{
    PeerClaimOutcome, PeerDispatchClaim, PeerDispatchClaimRequest, PeerEntryEvidence,
    PeerEntryOutcome, PeerEntryRequest, PeerExecutionPhase, PeerExecutionRecord,
    PeerRecoveryResult, PersistenceError, WorkerId,
};

impl RedbStore {
    pub(super) fn claim_dispatch(
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

    pub(super) fn mark_entered(
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
        self.faults.check(FaultPoint::BeforePeerEntryCommit)?;
        write.commit().map_err(error::redb)?;
        self.faults.check(FaultPoint::AfterPeerEntryCommit)?;
        Ok(PeerEntryOutcome::Entered(Box::new(record)))
    }

    pub(super) fn release_claim(
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

    pub(super) fn extend_claim(
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

    pub(super) fn mark_uncertain(
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
        self.faults.check(FaultPoint::BeforePeerUncertainCommit)?;
        write.commit().map_err(error::redb)?;
        self.faults.check(FaultPoint::AfterPeerUncertainCommit)?;
        Ok(record)
    }

    pub(super) fn recover_claims(
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
}
use redb::{ReadableTable, ReadableTableMetadata};
