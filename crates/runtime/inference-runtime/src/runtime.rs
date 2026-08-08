//! Synchronous single-owner registry used directly or through the hosted worker.

use std::collections::BTreeMap;

use domain_contracts::{
    BackendSequence, ExecutionDevice, GenerationUsage, LoadedModel, MemoryFootprint,
    ModelDescriptor, ModelGeneration, ModelHandle, ModelId, ModelLifecycle, ModelLoader,
    PreparedLoad, RequestId, ScalarType, SequenceId, SynchronizationError,
};

use crate::{
    CleanupFailureReport, CleanupResource, CleanupRetryState, RuntimeError, RuntimeLimits,
};

mod admission;
mod cleanup;
mod execution;
mod inspection;
mod memory;
mod shutdown;
mod unload;

/// Synchronous inference registry with exclusive ownership of every loaded model.
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
    Complete(M),
    FailedLoad(P),
}

impl<M, P> PendingModelOwner<M, P>
where
    M: LoadedModel,
    P: PreparedLoad,
{
    fn cleanup(&mut self) -> Result<(), SynchronizationError> {
        match self {
            Self::Complete(model) => model.prepare_unload(),
            Self::FailedLoad(prepared) => prepared.cleanup(),
        }
    }

    const fn cleanup_resource(&self, model_id: ModelId) -> CleanupResource {
        match self {
            Self::Complete(_) => CleanupResource::Model { model_id },
            Self::FailedLoad(_) => CleanupResource::FailedLoad { model_id },
        }
    }

    const fn is_complete(&self) -> bool {
        matches!(self, Self::Complete(_))
    }
}

struct PendingModel<M, P>
where
    M: LoadedModel,
    P: PreparedLoad,
{
    owner: PendingModelOwner<M, P>,
    footprint: MemoryFootprint,
    failure: CleanupFailureReport,
    attempts: u32,
    cancelled_requests: u32,
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
