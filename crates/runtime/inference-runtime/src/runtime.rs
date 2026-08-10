//! Synchronous single-owner registry used directly or through the hosted worker.

use std::collections::BTreeMap;

use domain_contracts::{
    BackendSequence, ExecutionDevice, GenerationUsage, LoadedModel, MemoryFootprint,
    ModelDescriptor, ModelGeneration, ModelHandle, ModelId, ModelLifecycle, ModelLoader,
    PreparedLoad, RequestId, ScalarType, SequenceId, SynchronizationError,
};

use crate::{
    CleanupFailureReport, CleanupResource, CleanupRetryState, RetainedOwnership, RuntimeError,
    RuntimeLimits,
};

mod admission;
mod cleanup;
mod execution;
mod inspection;
mod memory;
mod shutdown;
mod unload;

/// Synchronous inference registry with exclusive ownership of every loaded model.
///
/// Explicit [`Self::shutdown`] is the ordinary release protocol. Dropping a registry
/// that still owns backend resources retains those owners until process exit rather
/// than treating implicit backend `Drop` as successful cleanup.
pub struct InferenceRuntime<L>
where
    L: ModelLoader,
{
    loader: L,
    limits: RuntimeLimits,
    models: BTreeMap<ModelId, ModelSlot<L::Model>>,
    pending_models: BTreeMap<ModelId, PendingModel<L::Model, L::Prepared>>,
    request_index: BTreeMap<RequestId, ModelId>,
    sequence_index: BTreeMap<SequenceId, RequestId>,
    pending_request_index: BTreeMap<RequestId, ModelId>,
    pending_sequence_index: BTreeMap<SequenceId, RequestId>,
    generations: BTreeMap<ModelId, ModelGeneration>,
    reserved_footprint: MemoryFootprint,
    reserved_generation_workspace: MemoryFootprint,
    active_requests: u32,
    generation_workspaces: u32,
    pending_cleanup_sequences: u32,
    last_cleanup: Option<CleanupRetryState>,
    cleanup_scheduler: CleanupScheduler,
    maintenance_error: Option<RuntimeError>,
    shutting_down: bool,
}

struct ModelSlot<M>
where
    M: LoadedModel,
{
    handle: ModelHandle,
    execution_device: ExecutionDevice,
    execution_scalar_type: ScalarType,
    descriptor: ModelDescriptor,
    lifecycle: ModelLifecycle,
    model: M,
    model_footprint: MemoryFootprint,
    reserved_footprint: MemoryFootprint,
    requests: BTreeMap<RequestId, RequestSlot<M::Sequence>>,
    pending_sequences: BTreeMap<RequestId, PendingSequence<M::Sequence>>,
    poisoned: bool,
    cancelled_requests_during_unload: u32,
}

enum PendingModelOwner<M, P>
where
    M: LoadedModel,
    P: PreparedLoad,
{
    VerifiedModel(M),
    IncompatibleModel(M),
    FailedPreparation(P),
}

impl<M, P> PendingModelOwner<M, P>
where
    M: LoadedModel,
    P: PreparedLoad,
{
    fn cleanup(&mut self) -> Result<(), SynchronizationError> {
        match self {
            Self::VerifiedModel(model) | Self::IncompatibleModel(model) => model.prepare_unload(),
            Self::FailedPreparation(prepared) => prepared.cleanup(),
        }
    }

    const fn cleanup_resource(&self, handle: ModelHandle) -> CleanupResource {
        match self {
            Self::VerifiedModel(_) => CleanupResource::Model { handle },
            Self::IncompatibleModel(_) => CleanupResource::IncompatibleModel { handle },
            Self::FailedPreparation(_) => CleanupResource::FailedLoad { handle },
        }
    }

    const fn cleanup_class(&self) -> CleanupClass {
        match self {
            Self::FailedPreparation(_) => CleanupClass::FailedPreparation,
            Self::VerifiedModel(_) | Self::IncompatibleModel(_) => CleanupClass::CompleteModel,
        }
    }

    const fn is_complete(&self) -> bool {
        matches!(self, Self::VerifiedModel(_) | Self::IncompatibleModel(_))
    }
}

struct PendingModel<M, P>
where
    M: LoadedModel,
    P: PreparedLoad,
{
    handle: ModelHandle,
    owner: PendingModelOwner<M, P>,
    ownership: RetainedOwnership,
    failure: CleanupFailureReport,
    attempts: u32,
    cancelled_requests: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CleanupClass {
    Sequence,
    FailedPreparation,
    CompleteModel,
}

impl CleanupClass {
    const fn next(self) -> Self {
        match self {
            Self::Sequence => Self::FailedPreparation,
            Self::FailedPreparation => Self::CompleteModel,
            Self::CompleteModel => Self::Sequence,
        }
    }
}

struct CleanupScheduler {
    next_class: CleanupClass,
    sequence_cursor: Option<(ModelId, RequestId)>,
    failed_preparation_cursor: Option<ModelId>,
    complete_model_cursor: Option<ModelId>,
}

impl CleanupScheduler {
    const fn new() -> Self {
        Self {
            next_class: CleanupClass::Sequence,
            sequence_cursor: None,
            failed_preparation_cursor: None,
            complete_model_cursor: None,
        }
    }
}

struct PendingSequence<S>
where
    S: BackendSequence,
{
    request_id: RequestId,
    sequence_id: SequenceId,
    sequence: S,
    footprint: MemoryFootprint,
    failure: CleanupFailureReport,
    attempts: u32,
}

struct RequestSlot<S>
where
    S: BackendSequence,
{
    sequence_id: SequenceId,
    token_capacity: usize,
    sequence: S,
    backend_footprint: MemoryFootprint,
    workspace_footprint: MemoryFootprint,
    usage: GenerationUsage,
}

impl<L> Drop for InferenceRuntime<L>
where
    L: ModelLoader,
{
    fn drop(&mut self) {
        if self.models.is_empty() && self.pending_models.is_empty() {
            return;
        }
        let models = std::mem::take(&mut self.models);
        let pending_models = std::mem::take(&mut self.pending_models);
        std::mem::forget(models);
        std::mem::forget(pending_models);
    }
}

impl<L> InferenceRuntime<L>
where
    L: ModelLoader,
{
    /// Creates an empty registry around one concrete backend loader.
    #[must_use]
    pub fn new(loader: L, limits: RuntimeLimits) -> Self {
        Self {
            loader,
            limits,
            models: BTreeMap::new(),
            pending_models: BTreeMap::new(),
            request_index: BTreeMap::new(),
            sequence_index: BTreeMap::new(),
            pending_request_index: BTreeMap::new(),
            pending_sequence_index: BTreeMap::new(),
            generations: BTreeMap::new(),
            reserved_footprint: MemoryFootprint::default(),
            reserved_generation_workspace: MemoryFootprint::default(),
            active_requests: 0,
            generation_workspaces: 0,
            pending_cleanup_sequences: 0,
            last_cleanup: None,
            cleanup_scheduler: CleanupScheduler::new(),
            maintenance_error: None,
            shutting_down: false,
        }
    }

    const fn reject_if_shutting_down(&self) -> Result<(), RuntimeError> {
        if self.shutting_down {
            Err(RuntimeError::ShuttingDown)
        } else {
            Ok(())
        }
    }

    const fn maximum_cleanup_attempts(&self) -> u32 {
        self.limits.cleanup_retry.maximum_attempts.get()
    }

    fn unverified_owner_count(&self) -> u32 {
        u32::try_from(
            self.pending_models
                .values()
                .filter(|pending| pending.ownership.blocks_admission())
                .count(),
        )
        .unwrap_or(u32::MAX)
    }

    fn reject_if_admission_blocked(&self) -> Result<(), RuntimeError> {
        let owners = self.unverified_owner_count();
        if owners == 0 {
            Ok(())
        } else {
            Err(RuntimeError::AdmissionBlockedByUnverifiedOwnership { owners })
        }
    }

    fn next_handle(&self, model_id: ModelId) -> Result<ModelHandle, RuntimeError> {
        let current = self
            .generations
            .get(&model_id)
            .map_or(0, |value| value.get());
        let next = current
            .checked_add(1)
            .ok_or(RuntimeError::ModelGenerationExhausted(model_id))?;
        Ok(ModelHandle::new(model_id, ModelGeneration::new(next)))
    }

    fn exact_model(&self, handle: ModelHandle) -> Result<&ModelSlot<L::Model>, RuntimeError> {
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
        Ok(slot)
    }

    fn exact_model_mut(
        &mut self,
        handle: ModelHandle,
    ) -> Result<&mut ModelSlot<L::Model>, RuntimeError> {
        let slot = self
            .models
            .get_mut(&handle.id)
            .ok_or(RuntimeError::ModelNotLoaded(handle.id))?;
        if slot.handle != handle {
            return Err(RuntimeError::StaleModelHandle {
                provided: handle,
                current: slot.handle,
            });
        }
        Ok(slot)
    }

    fn request_model_id(&self, request_id: RequestId) -> Result<ModelId, RuntimeError> {
        self.request_index
            .get(&request_id)
            .copied()
            .ok_or(RuntimeError::RequestNotActive(request_id))
    }
}
