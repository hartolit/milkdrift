use domain_contracts::{
    LifecycleAction, ModelId, ModelLifecycleState, ModelLoader, MonotonicMillis, UnloadPolicy,
};

use crate::{
    CleanupPoll, FailureClass, FailureDetail, RetainedOwnership, RuntimeError, RuntimeOperation,
    ShutdownReceipt, TerminalRetentionSummary,
};

use super::{
    InferenceRuntime, PendingModelOwner,
    cleanup::ModelRequestDrain,
    memory::{add_conservative_footprint, saturating_u32},
};

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
    /// [`RuntimeError::TerminalCleanupRetention`] with retained ownership identity and summary.
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
            cancelled_requests =
                cancelled_requests.saturating_add(self.prepare_model_for_shutdown(model_id)?);
        }
        self.consume_shutdown_cleanup_budget()?;

        if let Some(first) = self.first_pending_cleanup_state() {
            return Err(RuntimeError::TerminalCleanupRetention {
                first,
                summary: self.terminal_retention_summary(),
            });
        }
        if !self.models.is_empty() || self.active_requests != 0 {
            return Err(RuntimeError::BackendContractViolation);
        }
        Ok(ShutdownReceipt {
            unloaded_models: initial_models,
            cancelled_requests,
        })
    }

    fn prepare_model_for_shutdown(&mut self, model_id: ModelId) -> Result<u32, RuntimeError> {
        let Some(state) = self
            .models
            .get(&model_id)
            .map(|slot| slot.lifecycle.state())
        else {
            return Ok(0);
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

        let cancelled = self.cancel_all_requests_for_shutdown(model_id)?;
        self.release_shutdown_model_if_ready(model_id)?;
        Ok(cancelled)
    }

    fn release_shutdown_model_if_ready(&mut self, model_id: ModelId) -> Result<bool, RuntimeError> {
        let ready = self.models.get(&model_id).is_some_and(|slot| {
            slot.requests.is_empty()
                && slot.pending_sequences.is_empty()
                && slot.lifecycle.state() == ModelLifecycleState::Unloading
        });
        if !ready {
            return Ok(false);
        }
        match self.release_model_with_primary(
            model_id,
            RuntimeOperation::Shutdown,
            FailureClass::Shutdown,
        ) {
            Ok(())
            | Err(RuntimeError::CleanupFailed(_) | RuntimeError::CleanupRetryExhausted(_)) => {
                Ok(true)
            }
            Err(error) => Err(error),
        }
    }

    fn consume_shutdown_cleanup_budget(&mut self) -> Result<(), RuntimeError> {
        loop {
            let ready_model = self.models.iter().find_map(|(model_id, slot)| {
                (slot.requests.is_empty()
                    && slot.pending_sequences.is_empty()
                    && slot.lifecycle.state() == ModelLifecycleState::Unloading)
                    .then_some(*model_id)
            });
            let progressed = if let Some(model_id) = ready_model {
                self.release_shutdown_model_if_ready(model_id)?
            } else {
                false
            };
            match self.poll_cleanup()? {
                CleanupPoll::Idle if !progressed => return Ok(()),
                CleanupPoll::Idle
                | CleanupPoll::Released(_)
                | CleanupPoll::RetryFailed(_)
                | CleanupPoll::Exhausted(_) => {}
            }
        }
    }
    fn terminal_retention_summary(&self) -> TerminalRetentionSummary {
        let mut summary = TerminalRetentionSummary {
            verified_models: saturating_u32(self.models.len()),
            ..TerminalRetentionSummary::default()
        };
        for pending in self.pending_models.values() {
            match &pending.owner {
                PendingModelOwner::FailedPreparation { .. } => {
                    summary.failed_preparations = summary.failed_preparations.saturating_add(1);
                }
                PendingModelOwner::VerifiedModel(_) => {
                    summary.verified_models = summary.verified_models.saturating_add(1);
                }
                PendingModelOwner::IncompatibleModel(_) => {
                    summary.incompatible_models = summary.incompatible_models.saturating_add(1);
                }
            }
            if let RetainedOwnership::Unverified {
                conservative_footprint,
                ..
            } = pending.ownership
            {
                summary.unverified_conservative_footprint = add_conservative_footprint(
                    summary.unverified_conservative_footprint,
                    conservative_footprint,
                );
            }
        }
        summary.sequences = saturating_u32(
            self.models
                .values()
                .map(|slot| slot.pending_sequences.len())
                .sum(),
        );
        for pending in self
            .models
            .values()
            .flat_map(|slot| slot.pending_sequences.values())
        {
            if let RetainedOwnership::Unverified {
                conservative_footprint,
                ..
            } = pending.ownership
            {
                summary.unverified_conservative_footprint = add_conservative_footprint(
                    summary.unverified_conservative_footprint,
                    conservative_footprint,
                );
            }
        }
        summary
    }

    fn cancel_all_requests_for_shutdown(&mut self, model_id: ModelId) -> Result<u32, RuntimeError> {
        let mut cancelled = 0_u32;
        loop {
            match self.drain_one_model_request(
                model_id,
                RuntimeOperation::Shutdown,
                FailureDetail::Class(FailureClass::Shutdown),
            )? {
                ModelRequestDrain::Empty => break,
                ModelRequestDrain::Released | ModelRequestDrain::Retained => {
                    cancelled = cancelled.saturating_add(1);
                }
            }
        }
        Ok(cancelled)
    }
}
