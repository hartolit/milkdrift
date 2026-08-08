use domain_contracts::{
    LifecycleAction, ModelId, ModelLifecycleState, ModelLoader, MonotonicMillis, UnloadPolicy,
};

use crate::{CleanupPoll, FailureClass, RuntimeError, RuntimeOperation, ShutdownReceipt};

use super::{InferenceRuntime, memory::saturating_u32};

impl<L> InferenceRuntime<L>
where
    L: ModelLoader,
{
    /// Cancels every request and unloads every resident model.
    ///
    /// Shutdown performs a finite best-effort pass over every independently owned
    /// model and request, then consumes every remaining automatic cleanup attempt.
    /// Exhausted resources remain quarantined and accounted instead of falling back
    /// to an unverified implicit drop.
    ///
    /// # Errors
    ///
    /// Returns an invariant or lifecycle error immediately. When explicit backend
    /// cleanup remains after the bounded retry policy is consumed, returns
    /// [`RuntimeError::CleanupRetryExhausted`] with the retained resource identity.
    pub fn shutdown(&mut self) -> Result<ShutdownReceipt, RuntimeError> {
        self.shutting_down = true;
        let pending_complete_models = self
            .pending_models
            .values()
            .filter(|pending| pending.owner.is_complete())
            .count();
        let initial_models =
            saturating_u32(self.models.len().saturating_add(pending_complete_models));
        let model_ids = self.models.keys().copied().collect::<Vec<_>>();
        let mut cancelled_requests = 0_u32;

        for model_id in model_ids {
            let Some(state) = self
                .models
                .get(&model_id)
                .map(|slot| slot.lifecycle.state())
            else {
                continue;
            };
            match state {
                ModelLifecycleState::Ready => {
                    let action = self
                        .models
                        .get_mut(&model_id)
                        .ok_or(RuntimeError::ModelNotLoaded(model_id))?
                        .lifecycle
                        .request_unload(UnloadPolicy::CancelActive, MonotonicMillis::new(0))?;
                    if action != LifecycleAction::ReleaseModel {
                        return Err(RuntimeError::BackendContractViolation);
                    }
                }
                ModelLifecycleState::Active { .. } => {
                    self.models
                        .get_mut(&model_id)
                        .ok_or(RuntimeError::ModelNotLoaded(model_id))?
                        .lifecycle
                        .request_unload(UnloadPolicy::CancelActive, MonotonicMillis::new(0))?;
                }
                ModelLifecycleState::Draining { .. }
                | ModelLifecycleState::Cancelling { .. }
                | ModelLifecycleState::Unloading => {}
                ModelLifecycleState::Absent
                | ModelLifecycleState::Loading
                | ModelLifecycleState::Failed { .. } => {
                    return Err(RuntimeError::Lifecycle(
                        domain_contracts::LifecycleError::InvalidTransition,
                    ));
                }
            }

            cancelled_requests =
                cancelled_requests.saturating_add(self.cancel_all_requests_for_shutdown(model_id)?);
            let ready_to_release = self.models.get(&model_id).is_some_and(|slot| {
                slot.requests.is_empty()
                    && slot.pending_sequences.is_empty()
                    && slot.lifecycle.state() == ModelLifecycleState::Unloading
            });
            if ready_to_release {
                match self.release_model_with_primary(
                    model_id,
                    RuntimeOperation::Shutdown,
                    FailureClass::Shutdown,
                ) {
                    Ok(())
                    | Err(
                        RuntimeError::CleanupFailed(_) | RuntimeError::CleanupRetryExhausted(_),
                    ) => {}
                    Err(error) => return Err(error),
                }
            }
        }

        loop {
            match self.poll_cleanup()? {
                CleanupPoll::Idle => break,
                CleanupPoll::Released(_)
                | CleanupPoll::RetryFailed(_)
                | CleanupPoll::Exhausted(_) => {}
            }
        }

        if let Some(state) = self.first_pending_cleanup_state() {
            return Err(RuntimeError::CleanupRetryExhausted(state));
        }
        if !self.models.is_empty() || self.active_requests != 0 {
            return Err(RuntimeError::BackendContractViolation);
        }

        Ok(ShutdownReceipt {
            unloaded_models: initial_models,
            cancelled_requests,
        })
    }
    fn cancel_all_requests_for_shutdown(&mut self, model_id: ModelId) -> Result<u32, RuntimeError> {
        let mut cancelled = 0_u32;
        loop {
            let request_id = self.models.get(&model_id).and_then(|slot| {
                slot.requests
                    .first_key_value()
                    .map(|(request_id, _)| *request_id)
            });
            let Some(request_id) = request_id else {
                break;
            };
            match self.remove_request(
                request_id,
                RuntimeOperation::Shutdown,
                FailureClass::Shutdown,
            ) {
                Ok(())
                | Err(RuntimeError::CleanupFailed(_) | RuntimeError::CleanupRetryExhausted(_)) => {
                    cancelled = cancelled.saturating_add(1);
                }
                Err(error) => return Err(error),
            }
        }
        Ok(cancelled)
    }
}
