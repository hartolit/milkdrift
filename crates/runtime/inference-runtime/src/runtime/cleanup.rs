use domain_contracts::{LifecycleAction, LoadedModel, ModelLifecycleState, ModelLoader, RequestId};

use crate::{
    CleanupFailureReport, CleanupPoll, CleanupResource, CleanupRetryState, FailureClass,
    RuntimeError, RuntimeOperation,
};

use super::{
    InferenceRuntime, PendingSequence,
    memory::{checked_add_footprint, checked_sub_footprint},
};

impl<L> InferenceRuntime<L>
where
    L: ModelLoader,
{
    /// Retries at most one non-exhausted quarantined cleanup operation.
    ///
    /// The initial cleanup failure counts as attempt one. Each call performs at
    /// most one additional backend cleanup attempt and never revisits a resource
    /// whose configured total-attempt budget is exhausted.
    ///
    /// # Errors
    ///
    /// Returns an invariant error only when ownership indices or memory accounting
    /// cannot be updated after a successful backend cleanup. Expected backend retry
    /// failures are represented by [`CleanupPoll`].
    #[expect(
        clippy::too_many_lines,
        reason = "the bounded cleanup transaction keeps retry, ownership transfer, and accounting contiguous"
    )]
    pub fn poll_cleanup(&mut self) -> Result<CleanupPoll, RuntimeError> {
        let maximum_attempts = self.maximum_cleanup_attempts();
        let pending_sequence = self.models.iter().find_map(|(model_id, slot)| {
            slot.pending_sequences
                .iter()
                .find(|(_, pending)| pending.attempts < maximum_attempts)
                .map(|(request_id, _)| (*model_id, *request_id))
        });
        if let Some((model_id, request_id)) = pending_sequence {
            let (state, released) = {
                let slot = self
                    .models
                    .get_mut(&model_id)
                    .ok_or(RuntimeError::ModelNotLoaded(model_id))?;
                let pending = slot
                    .pending_sequences
                    .get_mut(&request_id)
                    .ok_or(RuntimeError::BackendContractViolation)?;
                pending.attempts = pending
                    .attempts
                    .checked_add(1)
                    .ok_or(RuntimeError::BackendContractViolation)?;
                let state = CleanupRetryState {
                    resource: CleanupResource::Sequence {
                        model_id,
                        request_id,
                        sequence_id: pending.sequence_id,
                    },
                    failure: pending.failure,
                    attempts: pending.attempts,
                    maximum_attempts,
                };
                let released = slot.model.destroy_sequence(&mut pending.sequence).is_ok();
                (state, released)
            };
            self.last_cleanup = Some(state);
            if !released {
                return Ok(if state.exhausted() {
                    CleanupPoll::Exhausted(state)
                } else {
                    CleanupPoll::RetryFailed(state)
                });
            }

            let (sequence_id, footprint, lifecycle, release_primary) = {
                let slot = self
                    .models
                    .get_mut(&model_id)
                    .ok_or(RuntimeError::ModelNotLoaded(model_id))?;
                let removed = slot
                    .pending_sequences
                    .remove(&request_id)
                    .ok_or(RuntimeError::BackendContractViolation)?;
                debug_assert_eq!(removed.request_id, request_id);
                slot.reserved_footprint =
                    checked_sub_footprint(slot.reserved_footprint, removed.footprint)?;
                slot.poisoned = !slot.pending_sequences.is_empty();
                (
                    removed.sequence_id,
                    removed.footprint,
                    slot.lifecycle.state(),
                    removed.failure,
                )
            };
            self.pending_request_index.remove(&request_id);
            self.pending_sequence_index.remove(&sequence_id);
            self.pending_cleanup_sequences = self
                .pending_cleanup_sequences
                .checked_sub(1)
                .ok_or(RuntimeError::BackendContractViolation)?;
            self.reserved_footprint = checked_sub_footprint(self.reserved_footprint, footprint)?;
            if lifecycle == ModelLifecycleState::Unloading {
                match self.release_model_with_primary(
                    model_id,
                    release_primary.primary_operation,
                    release_primary.primary_failure,
                ) {
                    Ok(()) => {}
                    Err(
                        RuntimeError::CleanupFailed(_) | RuntimeError::CleanupRetryExhausted(_),
                    ) => {
                        return Ok(CleanupPoll::Released(state));
                    }
                    Err(error) => return Err(error),
                }
            }
            return Ok(CleanupPoll::Released(state));
        }

        let pending_model_id = self.pending_models.iter().find_map(|(model_id, pending)| {
            (pending.attempts < maximum_attempts).then_some(*model_id)
        });
        let Some(model_id) = pending_model_id else {
            return Ok(CleanupPoll::Idle);
        };
        let pending_footprint = self
            .pending_models
            .get(&model_id)
            .ok_or(RuntimeError::ModelNotLoaded(model_id))?
            .footprint;
        let next_reserved = checked_sub_footprint(self.reserved_footprint, pending_footprint)?;
        let (state, released) = {
            let pending = self
                .pending_models
                .get_mut(&model_id)
                .ok_or(RuntimeError::ModelNotLoaded(model_id))?;
            pending.attempts = pending
                .attempts
                .checked_add(1)
                .ok_or(RuntimeError::BackendContractViolation)?;
            let state = CleanupRetryState {
                resource: pending.owner.cleanup_resource(model_id),
                failure: pending.failure,
                attempts: pending.attempts,
                maximum_attempts,
            };
            let released = pending.owner.cleanup().is_ok();
            (state, released)
        };
        self.last_cleanup = Some(state);
        if !released {
            return Ok(if state.exhausted() {
                CleanupPoll::Exhausted(state)
            } else {
                CleanupPoll::RetryFailed(state)
            });
        }
        let removed = self
            .pending_models
            .remove(&model_id)
            .ok_or(RuntimeError::ModelNotLoaded(model_id))?;
        debug_assert_eq!(removed.footprint, pending_footprint);
        self.reserved_footprint = next_reserved;
        Ok(CleanupPoll::Released(state))
    }
    #[expect(
        clippy::too_many_lines,
        reason = "request removal keeps destruction, quarantine, lifecycle, indices, and accounting in one transaction"
    )]
    pub(super) fn remove_request(
        &mut self,
        request_id: RequestId,
        primary_operation: RuntimeOperation,
        primary_failure: FailureClass,
    ) -> Result<(), RuntimeError> {
        let model_id = self.request_model_id(request_id)?;
        let cleanup_failure = {
            let slot = self
                .models
                .get_mut(&model_id)
                .ok_or(RuntimeError::ModelNotLoaded(model_id))?;
            let request = slot
                .requests
                .get_mut(&request_id)
                .ok_or(RuntimeError::RequestNotActive(request_id))?;
            slot.model.destroy_sequence(&mut request.sequence).err()
        };

        if let Some(cleanup) = cleanup_failure {
            let report = CleanupFailureReport::new(
                primary_operation,
                primary_failure,
                RuntimeOperation::SequenceDestruction,
                RuntimeError::Sequence(cleanup).failure_class(),
            );
            let (sequence_id, action) = {
                let slot = self
                    .models
                    .get_mut(&model_id)
                    .ok_or(RuntimeError::ModelNotLoaded(model_id))?;
                let request = slot
                    .requests
                    .remove(&request_id)
                    .ok_or(RuntimeError::BackendContractViolation)?;
                let sequence_id = request.sequence_id;
                let pending = PendingSequence {
                    request_id,
                    sequence_id,
                    sequence: request.sequence,
                    footprint: request.backend_footprint,
                    failure: report,
                    attempts: 1,
                };
                slot.reserved_footprint =
                    checked_sub_footprint(slot.reserved_footprint, request.workspace_footprint)?;
                slot.pending_sequences.insert(request_id, pending);
                slot.poisoned = true;
                let action = slot.lifecycle.finish_request()?;
                (sequence_id, action)
            };
            self.request_index.remove(&request_id);
            self.sequence_index.remove(&sequence_id);
            self.pending_request_index.insert(request_id, model_id);
            self.pending_sequence_index.insert(sequence_id, request_id);
            self.active_requests = self
                .active_requests
                .checked_sub(1)
                .ok_or(RuntimeError::BackendContractViolation)?;
            self.pending_cleanup_sequences = self
                .pending_cleanup_sequences
                .checked_add(1)
                .ok_or(RuntimeError::BackendContractViolation)?;
            let state = CleanupRetryState {
                resource: CleanupResource::Sequence {
                    model_id,
                    request_id,
                    sequence_id,
                },
                failure: report,
                attempts: 1,
                maximum_attempts: self.maximum_cleanup_attempts(),
            };
            self.last_cleanup = Some(state);
            if action == LifecycleAction::ReleaseModel {
                debug_assert_eq!(
                    self.models
                        .get(&model_id)
                        .map(|slot| slot.lifecycle.state()),
                    Some(ModelLifecycleState::Unloading)
                );
            }
            return Err(cleanup_retention_error(state));
        }

        let (sequence_id, backend_footprint, action) = {
            let slot = self
                .models
                .get_mut(&model_id)
                .ok_or(RuntimeError::ModelNotLoaded(model_id))?;
            let request = slot
                .requests
                .remove(&request_id)
                .ok_or(RuntimeError::BackendContractViolation)?;
            let sequence_id = request.sequence_id;
            let total_footprint =
                checked_add_footprint(request.backend_footprint, request.workspace_footprint)?;
            slot.reserved_footprint =
                checked_sub_footprint(slot.reserved_footprint, total_footprint)?;
            let action = slot.lifecycle.finish_request()?;
            (sequence_id, request.backend_footprint, action)
        };
        self.request_index.remove(&request_id);
        self.sequence_index.remove(&sequence_id);
        self.active_requests = self
            .active_requests
            .checked_sub(1)
            .ok_or(RuntimeError::BackendContractViolation)?;
        self.reserved_footprint =
            checked_sub_footprint(self.reserved_footprint, backend_footprint)?;

        if action == LifecycleAction::ReleaseModel {
            self.release_model_with_primary(model_id, primary_operation, primary_failure)?;
        }
        Ok(())
    }
}

pub(super) const fn cleanup_retention_error(state: CleanupRetryState) -> RuntimeError {
    if state.exhausted() {
        RuntimeError::CleanupRetryExhausted(state)
    } else {
        RuntimeError::CleanupFailed(state.failure)
    }
}
