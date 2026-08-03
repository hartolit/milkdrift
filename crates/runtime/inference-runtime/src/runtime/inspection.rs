use domain_contracts::{
    LoadedModel, ModelHandle, ModelId, ModelLifecycleState, ModelLoader, RequestId,
};

use crate::{
    CleanupFailureReport, CleanupResource, CleanupRetryState, ModelSnapshot, RuntimeError,
    RuntimeSnapshot,
};

use super::{InferenceRuntime, PendingModel, PendingSequence, memory::saturating_u32};

impl<L> InferenceRuntime<L>
where
    L: ModelLoader,
{
    /// Returns immutable aggregate runtime state.
    #[must_use]
    pub fn snapshot(&self) -> RuntimeSnapshot {
        let maximum_attempts = self.maximum_cleanup_attempts();
        RuntimeSnapshot {
            loaded_models: saturating_u32(self.models.len()),
            active_requests: self.active_requests,
            reserved_footprint: self.reserved_footprint,
            generation_workspaces: self.generation_workspaces,
            reserved_generation_workspace: self.reserved_generation_workspace,
            pending_cleanup_models: saturating_u32(self.pending_models.len()),
            pending_cleanup_sequences: self.pending_cleanup_sequences,
            exhausted_cleanup_models: saturating_u32(
                self.pending_models
                    .values()
                    .filter(|pending| pending.attempts >= maximum_attempts)
                    .count(),
            ),
            exhausted_cleanup_sequences: saturating_u32(
                self.models
                    .values()
                    .flat_map(|slot| slot.pending_sequences.values())
                    .filter(|pending| pending.attempts >= maximum_attempts)
                    .count(),
            ),
            last_cleanup: self.last_cleanup,
            maintenance_error: self.maintenance_error,
            shutting_down: self.shutting_down,
        }
    }

    pub(crate) fn model_lifecycle_state(&self, model_id: ModelId) -> Option<ModelLifecycleState> {
        self.models
            .get(&model_id)
            .map(|slot| slot.lifecycle.state())
    }

    /// Collects per-model snapshots at a cold inspection boundary.
    #[must_use]
    pub fn model_snapshots(&self) -> Vec<ModelSnapshot> {
        self.models
            .values()
            .map(|slot| ModelSnapshot {
                handle: slot.handle,
                execution_device: slot.execution_device,
                lifecycle: slot.lifecycle.state(),
                descriptor: slot.descriptor,
                reserved_footprint: slot.reserved_footprint,
                active_requests: saturating_u32(slot.requests.len()),
                pending_cleanup_sequences: saturating_u32(slot.pending_sequences.len()),
                exhausted_cleanup_sequences: saturating_u32(
                    slot.pending_sequences
                        .values()
                        .filter(|pending| pending.attempts >= self.maximum_cleanup_attempts())
                        .count(),
                ),
                degraded: slot.poisoned,
            })
            .collect()
    }
    /// Returns whether a request still owns a normally active sequence.
    #[must_use]
    pub fn is_request_active(&self, request_id: RequestId) -> bool {
        self.request_index.contains_key(&request_id)
    }

    /// Returns whether a terminal request still owns quarantined cleanup state.
    #[must_use]
    pub fn is_request_cleanup_pending(&self, request_id: RequestId) -> bool {
        self.pending_request_index.contains_key(&request_id)
    }

    /// Returns the complete bounded retry state for one quarantined request.
    #[must_use]
    pub fn request_cleanup_state(&self, request_id: RequestId) -> Option<CleanupRetryState> {
        let model_id = *self.pending_request_index.get(&request_id)?;
        let pending = self
            .models
            .get(&model_id)?
            .pending_sequences
            .get(&request_id)?;
        Some(self.sequence_cleanup_state(model_id, pending))
    }

    /// Returns the retained two-failure report for one quarantined request.
    #[must_use]
    pub fn request_cleanup_failure(&self, request_id: RequestId) -> Option<CleanupFailureReport> {
        self.request_cleanup_state(request_id)
            .map(|state| state.failure)
    }

    /// Returns the complete bounded retry state for one quarantined model.
    #[must_use]
    pub fn model_cleanup_state(&self, model_id: ModelId) -> Option<CleanupRetryState> {
        self.pending_models
            .get(&model_id)
            .map(|pending| self.model_cleanup_retry_state(model_id, pending))
    }

    /// Returns whether a model is retained only for explicit unload cleanup.
    #[must_use]
    pub fn is_model_cleanup_pending(&self, model_id: ModelId) -> bool {
        self.pending_models.contains_key(&model_id)
    }

    /// Returns the cumulative unload-cancellation count retained by a resident
    /// or quarantined model.
    #[must_use]
    pub(crate) fn model_cancelled_requests_during_unload(&self, model_id: ModelId) -> Option<u32> {
        self.models
            .get(&model_id)
            .map(|slot| slot.cancelled_requests_during_unload)
            .or_else(|| {
                self.pending_models
                    .get(&model_id)
                    .map(|pending| pending.cancelled_requests)
            })
    }

    /// Returns whether any backend resource remains quarantined.
    #[must_use]
    pub fn has_pending_cleanup(&self) -> bool {
        self.pending_cleanup_sequences > 0 || !self.pending_models.is_empty()
    }

    /// Returns whether explicit native ownership remains inside the registry.
    #[must_use]
    pub(crate) fn owns_backend_resources(&self) -> bool {
        !self.models.is_empty() || !self.pending_models.is_empty()
    }

    /// Retains an unexpected maintenance failure for cold-path inspection.
    pub(crate) const fn record_maintenance_error(&mut self, error: RuntimeError) {
        self.maintenance_error = Some(error);
    }
    /// Returns one exact resident model snapshot for cold generation admission.
    #[must_use]
    pub fn model_snapshot(&self, handle: ModelHandle) -> Option<ModelSnapshot> {
        self.exact_model_snapshot(handle).ok()
    }

    /// Returns one resident snapshot or the exact missing/stale handle error.
    pub(crate) fn exact_model_snapshot(
        &self,
        handle: ModelHandle,
    ) -> Result<ModelSnapshot, RuntimeError> {
        let slot = self
            .models
            .get(&handle.id)
            .ok_or(RuntimeError::ModelNotLoaded(handle.id))?;
        if slot.handle != handle {
            return Err(RuntimeError::StaleModelHandle {
                provided: handle,
                current: slot.handle,
            });
        }
        Ok(ModelSnapshot {
            handle: slot.handle,
            execution_device: slot.execution_device,
            lifecycle: slot.lifecycle.state(),
            descriptor: slot.descriptor,
            reserved_footprint: slot.reserved_footprint,
            active_requests: saturating_u32(slot.requests.len()),
            pending_cleanup_sequences: saturating_u32(slot.pending_sequences.len()),
            exhausted_cleanup_sequences: saturating_u32(
                slot.pending_sequences
                    .values()
                    .filter(|pending| pending.attempts >= self.maximum_cleanup_attempts())
                    .count(),
            ),
            degraded: slot.poisoned,
        })
    }
    const fn sequence_cleanup_state(
        &self,
        model_id: ModelId,
        pending: &PendingSequence<<L::Model as LoadedModel>::Sequence>,
    ) -> CleanupRetryState {
        CleanupRetryState {
            resource: CleanupResource::Sequence {
                model_id,
                request_id: pending.request_id,
                sequence_id: pending.sequence_id,
            },
            failure: pending.failure,
            attempts: pending.attempts,
            maximum_attempts: self.maximum_cleanup_attempts(),
        }
    }

    const fn model_cleanup_retry_state(
        &self,
        model_id: ModelId,
        pending: &PendingModel<L::Model>,
    ) -> CleanupRetryState {
        CleanupRetryState {
            resource: CleanupResource::Model { model_id },
            failure: pending.failure,
            attempts: pending.attempts,
            maximum_attempts: self.maximum_cleanup_attempts(),
        }
    }

    pub(super) fn first_pending_cleanup_state(&self) -> Option<CleanupRetryState> {
        for (model_id, slot) in &self.models {
            if let Some((_, pending)) = slot.pending_sequences.first_key_value() {
                return Some(self.sequence_cleanup_state(*model_id, pending));
            }
        }
        self.pending_models
            .first_key_value()
            .map(|(model_id, pending)| self.model_cleanup_retry_state(*model_id, pending))
    }
}
