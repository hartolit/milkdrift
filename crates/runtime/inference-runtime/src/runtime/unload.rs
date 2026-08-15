use domain_contracts::{
    LifecycleAction, LoadedModel, ModelHandle, ModelId, ModelLifecycleState, ModelLoader,
    MonotonicMillis, UnloadPolicy,
};

use crate::{
    CleanupFailureReport, FailureClass, FailureDetail, RetainedOwnership, RuntimeError,
    RuntimeOperation, UnloadReceipt, UnloadStatus,
};

use super::{
    InferenceRuntime, PendingModel, PendingModelOwner,
    cleanup::{ModelRequestDrain, cleanup_retention_error},
    memory::checked_sub_footprint,
};

impl<L> InferenceRuntime<L>
where
    L: ModelLoader,
{
    /// Applies one explicit unload policy.
    ///
    /// # Errors
    ///
    /// Returns an error if the handle is unknown or stale, the unload lifecycle transition
    /// is invalid, an active sequence cannot be destroyed, model unload preparation fails,
    /// or releasing model resources violates runtime accounting invariants.
    pub fn unload_model(
        &mut self,
        handle: ModelHandle,
        policy: UnloadPolicy,
        now: MonotonicMillis,
    ) -> Result<UnloadReceipt, RuntimeError> {
        if !self.models.contains_key(&handle.id) {
            return self.absent_unload_receipt(handle);
        }
        let action = {
            let slot = self.exact_model_mut(handle)?;
            slot.lifecycle.request_unload(policy, now)?
        };

        match action {
            LifecycleAction::None => Ok(UnloadReceipt {
                handle,
                status: UnloadStatus::Draining,
                cancelled_requests: 0,
            }),
            LifecycleAction::CancelActive { .. } => {
                let cancelled_requests = self.cancel_all_requests(handle.id)?;
                if self.models.contains_key(&handle.id) {
                    self.release_model_with_primary(
                        handle.id,
                        RuntimeOperation::Cancellation,
                        FailureClass::Cancellation,
                    )?;
                }
                Ok(UnloadReceipt {
                    handle,
                    status: UnloadStatus::Unloaded,
                    cancelled_requests,
                })
            }
            LifecycleAction::ReleaseModel => {
                self.release_model(handle.id)?;
                Ok(UnloadReceipt {
                    handle,
                    status: UnloadStatus::Unloaded,
                    cancelled_requests: 0,
                })
            }
            LifecycleAction::UnloadComplete => Ok(UnloadReceipt {
                handle,
                status: UnloadStatus::AlreadyAbsent,
                cancelled_requests: 0,
            }),
        }
    }
    /// Enforces at most one timeout-driven or pending-unload transition.
    ///
    /// Calling this method at the configured host polling cadence guarantees that
    /// an expired drain window escalates without depending on event-consumer speed.
    ///
    /// # Errors
    ///
    /// Returns an error if a pending lifecycle transition fails, an active sequence
    /// cannot be destroyed, model unload preparation fails, or resource accounting
    /// invariants are violated while completing an unload.
    pub fn poll(&mut self, now: MonotonicMillis) -> Result<bool, RuntimeError> {
        match self.poll_unload_transition(now) {
            Some((_, Ok(_))) => Ok(true),
            Some((_, Err(error))) => Err(error),
            None => Ok(false),
        }
    }

    pub(crate) fn poll_unload_transition(
        &mut self,
        now: MonotonicMillis,
    ) -> Option<(ModelHandle, Result<UnloadReceipt, RuntimeError>)> {
        let mut expired = None;
        for (model_id, slot) in &mut self.models {
            if matches!(slot.lifecycle.state(), ModelLifecycleState::Draining { .. }) {
                match slot.lifecycle.poll(now) {
                    Ok(LifecycleAction::CancelActive { .. }) => {
                        expired = Some((*model_id, slot.handle));
                        break;
                    }
                    Ok(_) => {}
                    Err(error) => {
                        return Some((slot.handle, Err(error.into())));
                    }
                }
            }
        }
        if let Some((model_id, handle)) = expired {
            let result = self
                .cancel_all_requests(model_id)
                .and_then(|cancelled_requests| {
                    if self.models.contains_key(&model_id) {
                        self.release_model_with_primary(
                            model_id,
                            RuntimeOperation::Cancellation,
                            FailureClass::Cancellation,
                        )?;
                    }
                    Ok(UnloadReceipt {
                        handle,
                        status: UnloadStatus::Unloaded,
                        cancelled_requests,
                    })
                });
            return Some((handle, result));
        }

        let pending_cancellation = self.models.iter().find_map(|(model_id, slot)| {
            if matches!(
                slot.lifecycle.state(),
                ModelLifecycleState::Cancelling { .. }
            ) {
                Some((*model_id, slot.handle))
            } else {
                None
            }
        });
        if let Some((model_id, handle)) = pending_cancellation {
            let result = self
                .cancel_all_requests(model_id)
                .and_then(|cancelled_requests| {
                    if self.models.contains_key(&model_id) {
                        self.release_model_with_primary(
                            model_id,
                            RuntimeOperation::Cancellation,
                            FailureClass::Cancellation,
                        )?;
                    }
                    Ok(UnloadReceipt {
                        handle,
                        status: UnloadStatus::Unloaded,
                        cancelled_requests,
                    })
                });
            return Some((handle, result));
        }

        let pending_unload = self.models.iter().find_map(|(model_id, slot)| {
            if slot.lifecycle.state() == ModelLifecycleState::Unloading {
                Some((
                    *model_id,
                    slot.handle,
                    slot.cancelled_requests_during_unload,
                ))
            } else {
                None
            }
        });
        if let Some((model_id, handle, cancelled_requests)) = pending_unload {
            let result = self.release_model(model_id).map(|()| UnloadReceipt {
                handle,
                status: UnloadStatus::Unloaded,
                cancelled_requests,
            });
            return Some((handle, result));
        }
        None
    }
    fn cancel_all_requests(&mut self, model_id: ModelId) -> Result<u32, RuntimeError> {
        let mut cancelled = self
            .models
            .get(&model_id)
            .map(|slot| slot.cancelled_requests_during_unload)
            .ok_or(RuntimeError::ModelNotLoaded(model_id))?;
        loop {
            match self.drain_one_model_request(
                model_id,
                RuntimeOperation::Cancellation,
                FailureDetail::Class(FailureClass::Cancellation),
            )? {
                ModelRequestDrain::Empty => break,
                ModelRequestDrain::Released => {
                    cancelled = cancelled.saturating_add(1);
                    if let Some(slot) = self.models.get_mut(&model_id) {
                        slot.cancelled_requests_during_unload = cancelled;
                    } else {
                        break;
                    }
                }
                ModelRequestDrain::Retained => {
                    cancelled = cancelled.saturating_add(1);
                    if let Some(slot) = self.models.get_mut(&model_id) {
                        slot.cancelled_requests_during_unload = cancelled;
                    }
                    let state = self
                        .last_cleanup
                        .ok_or(RuntimeError::BackendContractViolation)?;
                    return Err(cleanup_retention_error(state));
                }
            }
        }
        Ok(cancelled)
    }

    fn release_model(&mut self, model_id: ModelId) -> Result<(), RuntimeError> {
        self.release_model_with_primary(
            model_id,
            RuntimeOperation::ModelUnload,
            FailureClass::Completion,
        )
    }

    pub(super) fn release_model_with_primary(
        &mut self,
        model_id: ModelId,
        primary_operation: RuntimeOperation,
        primary_failure: FailureClass,
    ) -> Result<(), RuntimeError> {
        let next_reserved = {
            let slot = self
                .models
                .get(&model_id)
                .ok_or(RuntimeError::ModelNotLoaded(model_id))?;
            if self.pending_models.contains_key(&model_id)
                || !slot.requests.is_empty()
                || !slot.pending_sequences.is_empty()
                || slot.lifecycle.state() != ModelLifecycleState::Unloading
                || slot.reserved_footprint != slot.model_footprint
            {
                return Err(RuntimeError::Lifecycle(
                    domain_contracts::LifecycleError::InvalidTransition,
                ));
            }
            let mut released_lifecycle = slot.lifecycle;
            released_lifecycle.complete_unload()?;
            checked_sub_footprint(self.reserved_footprint, slot.model_footprint)?
        };

        let mut slot = self
            .models
            .remove(&model_id)
            .ok_or(RuntimeError::ModelNotLoaded(model_id))?;
        if let Err(cleanup) = slot.model.prepare_unload() {
            let report = CleanupFailureReport::with_details(
                primary_operation,
                FailureDetail::Class(primary_failure),
                RuntimeOperation::ModelUnload,
                FailureDetail::Synchronization(cleanup),
            );
            let handle = slot.handle;
            let ownership = RetainedOwnership::Exact(slot.model_footprint);
            let pending = PendingModel {
                handle,
                owner: PendingModelOwner::VerifiedModel(slot.model),
                ownership,
                failure: report,
                attempts: 1,
                cancelled_requests: slot.cancelled_requests_during_unload,
            };
            let state = pending.cleanup_state(self.maximum_cleanup_attempts());
            let replaced = self.pending_models.insert(model_id, pending);
            debug_assert!(replaced.is_none(), "model release was prevalidated");
            self.last_cleanup = Some(state);
            return Err(cleanup_retention_error(state));
        }

        self.reserved_footprint = next_reserved;
        Ok(())
    }

    fn absent_unload_receipt(&self, handle: ModelHandle) -> Result<UnloadReceipt, RuntimeError> {
        if let Some(state) = self.model_cleanup_state(handle.id) {
            let generation = self
                .generations
                .get(&handle.id)
                .copied()
                .ok_or(RuntimeError::ModelNotLoaded(handle.id))?;
            let current = ModelHandle::new(handle.id, generation);
            if current != handle {
                return Err(RuntimeError::StaleModelHandle {
                    provided: handle,
                    current,
                });
            }
            return Err(if state.exhausted() {
                RuntimeError::CleanupRetryExhausted(state)
            } else {
                RuntimeError::CleanupFailed(state)
            });
        }
        let Some(generation) = self.generations.get(&handle.id).copied() else {
            return Err(RuntimeError::ModelNotLoaded(handle.id));
        };
        let current = ModelHandle::new(handle.id, generation);
        if current != handle {
            return Err(RuntimeError::StaleModelHandle {
                provided: handle,
                current,
            });
        }
        Ok(UnloadReceipt {
            handle,
            status: UnloadStatus::AlreadyAbsent,
            cancelled_requests: 0,
        })
    }
}
