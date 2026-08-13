use domain_contracts::{
    BackendSequence, LifecycleAction, LoadedModel, MemoryFootprint, ModelHandle, ModelId,
    ModelLifecycle, ModelLifecycleState, ModelLoader, RequestId, SequenceId, SequencePlan,
};

use crate::{
    CleanupFailureReport, CleanupPoll, CleanupResource, CleanupRetryState, FailureDetail,
    RetainedOwnership, RuntimeError, RuntimeOperation,
};

use super::{
    CleanupClass, InferenceRuntime, PendingSequence,
    memory::{checked_add_footprint, checked_sub_footprint},
    sequence_report_matches,
};

#[derive(Clone, Copy)]
enum CleanupSelection {
    Sequence {
        model_id: domain_contracts::ModelId,
        request_id: RequestId,
    },
    Model {
        model_id: domain_contracts::ModelId,
    },
}

#[derive(Clone, Copy)]
struct RequestRemovalTransition {
    model_id: ModelId,
    handle: ModelHandle,
    sequence_id: SequenceId,
    accepted_plan: SequencePlan,
    backend_footprint: MemoryFootprint,
    released_slot_footprint: MemoryFootprint,
    retained_slot_footprint: MemoryFootprint,
    released_runtime_footprint: MemoryFootprint,
    active_requests: u32,
    pending_cleanup_sequences: u32,
    lifecycle: ModelLifecycle,
    lifecycle_action: LifecycleAction,
}

enum RequestRemovalDisposition {
    Released { contract_violated: bool },
    Retained(CleanupFailureReport),
}

impl<L> InferenceRuntime<L>
where
    L: ModelLoader,
{
    /// Retries at most one non-exhausted quarantined cleanup operation.
    ///
    /// Selection rotates across sequence, failed-preparation, and complete-model
    /// classes and then within each class. Exhausted owners remain observable but
    /// are skipped. The initial cleanup failure counts as attempt one.
    ///
    /// # Errors
    ///
    /// Returns an invariant error only before invoking a backend cleanup when the
    /// published ownership indexes or exact accounting cannot support the release.
    pub fn poll_cleanup(&mut self) -> Result<CleanupPoll, RuntimeError> {
        let Some(selection) = self.next_cleanup_selection() else {
            return Ok(CleanupPoll::Idle);
        };
        match selection {
            CleanupSelection::Sequence {
                model_id,
                request_id,
            } => self.retry_sequence_cleanup(model_id, request_id),
            CleanupSelection::Model { model_id } => self.retry_model_cleanup(model_id),
        }
    }

    fn next_cleanup_selection(&mut self) -> Option<CleanupSelection> {
        let mut class = self.cleanup_scheduler.next_class;
        for _ in 0..3 {
            let selection = match class {
                CleanupClass::Sequence => {
                    self.next_sequence_cleanup().map(|(model_id, request_id)| {
                        CleanupSelection::Sequence {
                            model_id,
                            request_id,
                        }
                    })
                }
                CleanupClass::FailedPreparation | CleanupClass::CompleteModel => self
                    .next_model_cleanup(class)
                    .map(|model_id| CleanupSelection::Model { model_id }),
            };
            if let Some(selection) = selection {
                self.cleanup_scheduler.next_class = class.next();
                match selection {
                    CleanupSelection::Sequence {
                        model_id,
                        request_id,
                    } => self.cleanup_scheduler.sequence_cursor = Some((model_id, request_id)),
                    CleanupSelection::Model { model_id } => match class {
                        CleanupClass::FailedPreparation => {
                            self.cleanup_scheduler.failed_preparation_cursor = Some(model_id);
                        }
                        CleanupClass::CompleteModel => {
                            self.cleanup_scheduler.complete_model_cursor = Some(model_id);
                        }
                        CleanupClass::Sequence => {}
                    },
                }
                return Some(selection);
            }
            class = class.next();
        }
        None
    }

    fn next_sequence_cleanup(&self) -> Option<(domain_contracts::ModelId, RequestId)> {
        let maximum_attempts = self.maximum_cleanup_attempts();
        let cursor = self.cleanup_scheduler.sequence_cursor;
        let mut first = None;
        let mut after = None;
        for (model_id, slot) in &self.models {
            for (request_id, pending) in &slot.pending_sequences {
                if pending.attempts >= maximum_attempts {
                    continue;
                }
                let key = (*model_id, *request_id);
                if first.is_none() {
                    first = Some(key);
                }
                if cursor.is_some_and(|cursor| key > cursor) {
                    after = Some(key);
                    break;
                }
            }
            if after.is_some() {
                break;
            }
        }
        after.or(first)
    }

    fn next_model_cleanup(&self, class: CleanupClass) -> Option<domain_contracts::ModelId> {
        let maximum_attempts = self.maximum_cleanup_attempts();
        let cursor = match class {
            CleanupClass::FailedPreparation => self.cleanup_scheduler.failed_preparation_cursor,
            CleanupClass::CompleteModel => self.cleanup_scheduler.complete_model_cursor,
            CleanupClass::Sequence => return None,
        };
        let mut first = None;
        let mut after = None;
        for (model_id, pending) in &self.pending_models {
            if pending.attempts >= maximum_attempts || pending.owner.cleanup_class() != class {
                continue;
            }
            if first.is_none() {
                first = Some(*model_id);
            }
            if cursor.is_some_and(|cursor| *model_id > cursor) {
                after = Some(*model_id);
                break;
            }
        }
        after.or(first)
    }

    fn retry_sequence_cleanup(
        &mut self,
        model_id: domain_contracts::ModelId,
        request_id: RequestId,
    ) -> Result<CleanupPoll, RuntimeError> {
        let maximum_attempts = self.maximum_cleanup_attempts();
        let (
            handle,
            sequence_id,
            next_attempts,
            next_slot_reserved,
            next_reserved,
            next_pending_count,
        ) = {
            let slot = self
                .models
                .get(&model_id)
                .ok_or(RuntimeError::ModelNotLoaded(model_id))?;
            let pending = slot
                .pending_sequences
                .get(&request_id)
                .ok_or(RuntimeError::BackendContractViolation)?;
            if pending.attempts >= maximum_attempts
                || self.pending_request_index.get(&request_id) != Some(&model_id)
                || self.pending_sequence_index.get(&pending.sequence_id) != Some(&request_id)
            {
                return Err(RuntimeError::BackendContractViolation);
            }
            let exact_footprint = pending.ownership.exact_footprint();
            (
                slot.handle,
                pending.sequence_id,
                pending
                    .attempts
                    .checked_add(1)
                    .ok_or(RuntimeError::BackendContractViolation)?,
                match exact_footprint {
                    Some(footprint) => checked_sub_footprint(slot.reserved_footprint, footprint)?,
                    None => slot.reserved_footprint,
                },
                match exact_footprint {
                    Some(footprint) => checked_sub_footprint(self.reserved_footprint, footprint)?,
                    None => self.reserved_footprint,
                },
                self.pending_cleanup_sequences
                    .checked_sub(1)
                    .ok_or(RuntimeError::BackendContractViolation)?,
            )
        };

        let (mut pending, cleanup_result) = {
            let slot = self
                .models
                .get_mut(&model_id)
                .ok_or(RuntimeError::ModelNotLoaded(model_id))?;
            let mut pending = slot
                .pending_sequences
                .remove(&request_id)
                .ok_or(RuntimeError::BackendContractViolation)?;
            pending.attempts = next_attempts;
            pending.reconcile_contract();
            let result = slot.model.destroy_sequence(&mut pending.sequence);
            pending.reconcile_contract();
            (pending, result)
        };

        if let Err(error) = cleanup_result {
            pending.failure.cleanup_failure = FailureDetail::Sequence(error).class();
            pending.failure.cleanup_detail = FailureDetail::Sequence(error);
            let state = sequence_cleanup_state(handle, &pending, maximum_attempts);
            let slot = self
                .models
                .get_mut(&model_id)
                .ok_or(RuntimeError::ModelNotLoaded(model_id))?;
            if pending.ownership.blocks_admission() {
                slot.reserved_footprint = next_slot_reserved;
                self.reserved_footprint = next_reserved;
            }
            let replaced = slot.pending_sequences.insert(request_id, pending);
            debug_assert!(replaced.is_none(), "cleanup owner was removed for retry");
            self.last_cleanup = Some(state);
            return Ok(if state.exhausted() {
                CleanupPoll::Exhausted(state)
            } else {
                CleanupPoll::RetryFailed(state)
            });
        }

        let state = sequence_cleanup_state(handle, &pending, maximum_attempts).released();
        let slot = self
            .models
            .get_mut(&model_id)
            .ok_or(RuntimeError::ModelNotLoaded(model_id))?;
        slot.reserved_footprint = next_slot_reserved;
        slot.poisoned = !slot.pending_sequences.is_empty();
        self.pending_request_index.remove(&request_id);
        self.pending_sequence_index.remove(&sequence_id);
        self.pending_cleanup_sequences = next_pending_count;
        self.reserved_footprint = next_reserved;
        self.last_cleanup = Some(state);
        Ok(CleanupPoll::Released(state))
    }

    fn retry_model_cleanup(
        &mut self,
        model_id: domain_contracts::ModelId,
    ) -> Result<CleanupPoll, RuntimeError> {
        let maximum_attempts = self.maximum_cleanup_attempts();
        let (next_attempts, reserved_after_release) = {
            let pending = self
                .pending_models
                .get(&model_id)
                .ok_or(RuntimeError::ModelNotLoaded(model_id))?;
            if pending.attempts >= maximum_attempts {
                return Err(RuntimeError::BackendContractViolation);
            }
            let reserved_after_release = match pending.ownership.exact_footprint() {
                Some(footprint) => checked_sub_footprint(self.reserved_footprint, footprint)?,
                None => self.reserved_footprint,
            };
            (
                pending
                    .attempts
                    .checked_add(1)
                    .ok_or(RuntimeError::BackendContractViolation)?,
                reserved_after_release,
            )
        };

        let mut pending = self
            .pending_models
            .remove(&model_id)
            .ok_or(RuntimeError::ModelNotLoaded(model_id))?;
        pending.attempts = next_attempts;

        // A failed-preparation owner must continue to report the exact plan that
        // E0 admitted. Check both sides of the retry because a failing cleanup
        // attempt may itself mutate a dishonest backend report.
        pending.reconcile_failed_preparation_contract();
        let cleanup_result = pending.owner.cleanup();
        pending.reconcile_failed_preparation_contract();

        if let Err(error) = cleanup_result {
            pending.failure.cleanup_failure = FailureDetail::Synchronization(error).class();
            pending.failure.cleanup_detail = FailureDetail::Synchronization(error);
            if pending.ownership.blocks_admission() {
                // The formerly exact reservation is no longer represented as an
                // exact quantity. The conservative evidence remains observable,
                // and admission stays blocked until explicit cleanup succeeds.
                self.reserved_footprint = reserved_after_release;
            }
            let state = model_cleanup_state(&pending, maximum_attempts);
            let replaced = self.pending_models.insert(model_id, pending);
            debug_assert!(replaced.is_none(), "cleanup owner was removed for retry");
            self.last_cleanup = Some(state);
            return Ok(if state.exhausted() {
                CleanupPoll::Exhausted(state)
            } else {
                CleanupPoll::RetryFailed(state)
            });
        }

        let state = model_cleanup_state(&pending, maximum_attempts).released();
        self.reserved_footprint = reserved_after_release;
        self.last_cleanup = Some(state);
        Ok(CleanupPoll::Released(state))
    }

    pub(super) fn remove_request(
        &mut self,
        request_id: RequestId,
        primary_operation: RuntimeOperation,
        primary_detail: FailureDetail,
    ) -> Result<(), RuntimeError> {
        let transition = self.prepare_request_removal(request_id)?;
        let disposition =
            self.apply_request_removal(request_id, &transition, primary_operation, primary_detail)?;

        self.request_index.remove(&request_id);
        self.sequence_index.remove(&transition.sequence_id);
        self.active_requests = transition.active_requests;
        match disposition {
            RequestRemovalDisposition::Released { contract_violated } => {
                self.reserved_footprint = transition.released_runtime_footprint;
                if contract_violated {
                    Err(RuntimeError::BackendContractViolation)
                } else {
                    Ok(())
                }
            }
            RequestRemovalDisposition::Retained(report) => {
                self.pending_request_index
                    .insert(request_id, transition.model_id);
                self.pending_sequence_index
                    .insert(transition.sequence_id, request_id);
                self.pending_cleanup_sequences = transition.pending_cleanup_sequences;
                let ownership = self
                    .models
                    .get(&transition.model_id)
                    .and_then(|slot| slot.pending_sequences.get(&request_id))
                    .map(|pending| pending.ownership)
                    .ok_or(RuntimeError::BackendContractViolation)?;
                if ownership.blocks_admission() {
                    self.reserved_footprint = transition.released_runtime_footprint;
                }
                let state = CleanupRetryState {
                    resource: CleanupResource::Sequence {
                        handle: transition.handle,
                        request_id,
                        sequence_id: transition.sequence_id,
                    },
                    failure: report,
                    ownership,
                    attempts: 1,
                    maximum_attempts: self.maximum_cleanup_attempts(),
                };
                self.last_cleanup = Some(state);
                Err(cleanup_retention_error(state))
            }
        }
    }

    fn prepare_request_removal(
        &self,
        request_id: RequestId,
    ) -> Result<RequestRemovalTransition, RuntimeError> {
        let model_id = self.request_model_id(request_id)?;
        let slot = self
            .models
            .get(&model_id)
            .ok_or(RuntimeError::ModelNotLoaded(model_id))?;
        let request = slot
            .requests
            .get(&request_id)
            .ok_or(RuntimeError::RequestNotActive(request_id))?;
        if self.request_index.get(&request_id) != Some(&model_id)
            || self.sequence_index.get(&request.sequence_id) != Some(&request_id)
            || self.pending_request_index.contains_key(&request_id)
            || self
                .pending_sequence_index
                .contains_key(&request.sequence_id)
        {
            return Err(RuntimeError::BackendContractViolation);
        }
        let mut lifecycle = slot.lifecycle;
        let lifecycle_action = lifecycle.finish_request()?;
        let backend_footprint = request.accepted_plan.reservation.total_footprint;
        let total = checked_add_footprint(backend_footprint, request.workspace_footprint)?;
        Ok(RequestRemovalTransition {
            model_id,
            handle: slot.handle,
            sequence_id: request.sequence_id,
            accepted_plan: request.accepted_plan,
            backend_footprint,
            released_slot_footprint: checked_sub_footprint(slot.reserved_footprint, total)?,
            retained_slot_footprint: checked_sub_footprint(
                slot.reserved_footprint,
                request.workspace_footprint,
            )?,
            released_runtime_footprint: checked_sub_footprint(
                self.reserved_footprint,
                backend_footprint,
            )?,
            active_requests: self
                .active_requests
                .checked_sub(1)
                .ok_or(RuntimeError::BackendContractViolation)?,
            pending_cleanup_sequences: self
                .pending_cleanup_sequences
                .checked_add(1)
                .ok_or(RuntimeError::BackendContractViolation)?,
            lifecycle,
            lifecycle_action,
        })
    }

    fn apply_request_removal(
        &mut self,
        request_id: RequestId,
        transition: &RequestRemovalTransition,
        primary_operation: RuntimeOperation,
        primary_detail: FailureDetail,
    ) -> Result<RequestRemovalDisposition, RuntimeError> {
        let slot = self
            .models
            .get_mut(&transition.model_id)
            .ok_or(RuntimeError::ModelNotLoaded(transition.model_id))?;
        let mut request = slot
            .requests
            .remove(&request_id)
            .ok_or(RuntimeError::RequestNotActive(request_id))?;
        let report_before_cleanup = request.sequence.reported_plan();
        let contract_matched_before = sequence_report_matches(
            &request.sequence,
            transition.sequence_id,
            &transition.accepted_plan,
            report_before_cleanup,
        );
        let cleanup_result = slot.model.destroy_sequence(&mut request.sequence);
        let report_after_cleanup = request.sequence.reported_plan();
        let contract_matches = contract_matched_before
            && sequence_report_matches(
                &request.sequence,
                transition.sequence_id,
                &transition.accepted_plan,
                report_after_cleanup,
            );
        slot.lifecycle = transition.lifecycle;

        let Err(cleanup) = cleanup_result else {
            slot.reserved_footprint = transition.released_slot_footprint;
            if transition.lifecycle_action == LifecycleAction::ReleaseModel {
                debug_assert_eq!(slot.lifecycle.state(), ModelLifecycleState::Unloading);
            }
            return Ok(RequestRemovalDisposition::Released {
                contract_violated: !contract_matches,
            });
        };

        let report = CleanupFailureReport::with_details(
            primary_operation,
            primary_detail,
            RuntimeOperation::SequenceDestruction,
            FailureDetail::Sequence(cleanup),
        );
        let reported_footprint = report_after_cleanup.reservation.total_footprint;
        let ownership = if contract_matches {
            RetainedOwnership::Exact(transition.backend_footprint)
        } else {
            RetainedOwnership::Unverified {
                accepted_footprint: transition.backend_footprint,
                reported_footprint,
                conservative_footprint: super::memory::conservative_footprint(
                    transition.backend_footprint,
                    reported_footprint,
                ),
            }
        };
        let mut report = report;
        if !contract_matches {
            report.primary_failure = crate::FailureClass::BackendContract;
            report.primary_detail = FailureDetail::Class(crate::FailureClass::BackendContract);
        }
        let pending = PendingSequence {
            request_id,
            sequence_id: transition.sequence_id,
            sequence: request.sequence,
            accepted_plan: transition.accepted_plan,
            ownership,
            failure: report,
            attempts: 1,
        };
        slot.reserved_footprint = if ownership.exact_footprint().is_some() {
            transition.retained_slot_footprint
        } else {
            transition.released_slot_footprint
        };
        slot.poisoned = true;
        let replaced = slot.pending_sequences.insert(request_id, pending);
        debug_assert!(
            replaced.is_none(),
            "active request identity was prevalidated"
        );
        Ok(RequestRemovalDisposition::Retained(report))
    }
}

fn sequence_cleanup_state<S: domain_contracts::BackendSequence>(
    handle: domain_contracts::ModelHandle,
    pending: &PendingSequence<S>,
    maximum_attempts: u32,
) -> CleanupRetryState {
    CleanupRetryState {
        resource: CleanupResource::Sequence {
            handle,
            request_id: pending.request_id,
            sequence_id: pending.sequence_id,
        },
        failure: pending.failure,
        ownership: pending.ownership,
        attempts: pending.attempts,
        maximum_attempts,
    }
}

fn model_cleanup_state<M, P>(
    pending: &super::PendingModel<M, P>,
    maximum_attempts: u32,
) -> CleanupRetryState
where
    M: LoadedModel,
    P: domain_contracts::FailedLoadOwner,
{
    CleanupRetryState {
        resource: pending.owner.cleanup_resource(pending.handle),
        failure: pending.failure,
        ownership: pending.ownership,
        attempts: pending.attempts,
        maximum_attempts,
    }
}

pub(super) const fn cleanup_retention_error(state: CleanupRetryState) -> RuntimeError {
    if state.exhausted() {
        RuntimeError::CleanupRetryExhausted(state)
    } else {
        RuntimeError::CleanupFailed(state)
    }
}
