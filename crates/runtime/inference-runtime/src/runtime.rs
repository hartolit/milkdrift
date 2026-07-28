//! Synchronous single-owner registry used directly or through the hosted worker.

use std::collections::BTreeMap;

use domain_contracts::{
    BackendSequence, CancellationReason, CancellationStatus, CapacityExhausted, CapacityResource,
    DecodeBuffers, DecodeInput, DecodeOutcome, DeviceId, DeviceKind, FinishReason, GenerationUsage,
    LifecycleAction, LoadConfiguration, LoadedModel, MemoryBudget, MemoryFootprint,
    ModelGeneration, ModelHandle, ModelId, ModelLifecycle, ModelLifecycleState, ModelLoader,
    MonotonicMillis, PrefillBuffers, PrefillInput, PrefillOutcome, RequestId,
    SequenceConfiguration, SequenceId, TokenId, UnloadPolicy, decode_checked, prefill_checked,
};

use crate::{
    CleanupFailureReport, CleanupPoll, CleanupResource, CleanupRetryState, DecodeReceipt,
    FailureClass, LoadReceipt, MemoryKind, ModelSnapshot, PrefillReceipt, RequestStartReceipt,
    RuntimeError, RuntimeLimits, RuntimeOperation, RuntimeSnapshot, ShutdownReceipt, UnloadReceipt,
    UnloadStatus,
};

/// Synchronous inference registry with exclusive ownership of every loaded model.
pub struct InferenceRuntime<L>
where
    L: ModelLoader,
{
    loader: L,
    limits: RuntimeLimits,
    models: BTreeMap<ModelId, ModelSlot<L::Model>>,
    pending_models: BTreeMap<ModelId, PendingModel<L::Model>>,
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
    descriptor: domain_contracts::ModelDescriptor,
    lifecycle: ModelLifecycle,
    model: M,
    model_footprint: MemoryFootprint,
    reserved_footprint: MemoryFootprint,
    requests: BTreeMap<RequestId, RequestSlot<M::Sequence>>,
    pending_sequences: BTreeMap<RequestId, PendingSequence<M::Sequence>>,
    poisoned: bool,
    cancelled_requests_during_unload: u32,
}

struct PendingModel<M>
where
    M: LoadedModel,
{
    model: M,
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

    /// Inspects, admits, and loads one model synchronously.
    ///
    /// # Errors
    ///
    /// Returns an error if shutdown has started; the model identity is already loaded;
    /// a model, generation, or memory limit is exceeded; a lifecycle transition fails;
    /// or the backend cannot plan or load a model that satisfies its declared contract.
    pub fn load_model(
        &mut self,
        model_id: ModelId,
        source: &L::Source,
        device: DeviceId,
        device_kind: DeviceKind,
    ) -> Result<LoadReceipt, RuntimeError> {
        self.reject_if_shutting_down()?;
        if self.models.contains_key(&model_id) || self.pending_models.contains_key(&model_id) {
            return Err(RuntimeError::ModelAlreadyLoaded(model_id));
        }
        if self.models.len().saturating_add(self.pending_models.len())
            >= self.limits.maximum_loaded_models.get() as usize
        {
            return Err(RuntimeError::LoadedModelLimit {
                required: saturating_u32(
                    self.models.len().saturating_add(self.pending_models.len()),
                )
                .saturating_add(1),
                available: self.limits.maximum_loaded_models.get(),
            });
        }

        let handle = self.next_handle(model_id)?;
        let remaining_budget = remaining_budget(self.limits.memory_budget, self.reserved_footprint);
        let configuration = LoadConfiguration {
            handle,
            device,
            device_kind,
            memory_budget: remaining_budget,
        };
        let mut lifecycle = ModelLifecycle::new();
        lifecycle.begin_load()?;
        let plan = self.loader.plan_load(source, &configuration)?;
        let next_reserved = admit_footprint(
            self.reserved_footprint,
            plan.expected_footprint,
            self.limits.memory_budget,
        )?;
        let mut model = self.loader.load(source, &configuration)?;
        let validation =
            if model.handle() != handle || model.metadata() != &plan.descriptor.metadata {
                Err(RuntimeError::BackendContractViolation)
            } else {
                lifecycle.complete_load().map(|_| ()).map_err(Into::into)
            };
        if let Err(primary) = validation {
            if let Err(cleanup) = model.prepare_unload() {
                let report = CleanupFailureReport::new(
                    RuntimeOperation::ModelAdmission,
                    primary.failure_class(),
                    RuntimeOperation::ModelUnload,
                    RuntimeError::Synchronization(cleanup).failure_class(),
                );
                let pending = PendingModel {
                    model,
                    footprint: plan.expected_footprint,
                    failure: report,
                    attempts: 1,
                    cancelled_requests: 0,
                };
                self.pending_models.insert(model_id, pending);
                self.generations.insert(model_id, handle.generation);
                self.reserved_footprint = next_reserved;
                let state = CleanupRetryState {
                    resource: CleanupResource::Model { model_id },
                    failure: report,
                    attempts: 1,
                    maximum_attempts: self.maximum_cleanup_attempts(),
                };
                self.last_cleanup = Some(state);
                return Err(cleanup_retention_error(state));
            }
            return Err(primary);
        }

        let slot = ModelSlot {
            handle,
            descriptor: plan.descriptor,
            lifecycle,
            model,
            model_footprint: plan.expected_footprint,
            reserved_footprint: plan.expected_footprint,
            requests: BTreeMap::new(),
            pending_sequences: BTreeMap::new(),
            poisoned: false,
            cancelled_requests_during_unload: 0,
        };
        let replaced = self.models.insert(model_id, slot);
        debug_assert!(replaced.is_none(), "model admission was preflighted");
        self.generations.insert(model_id, handle.generation);
        self.reserved_footprint = next_reserved;

        Ok(LoadReceipt {
            handle,
            descriptor: plan.descriptor,
            reserved_footprint: plan.expected_footprint,
        })
    }

    /// Creates one independently owned backend sequence for a request.
    ///
    /// # Errors
    ///
    /// Returns an error if shutdown has started; a request or sequence identity is
    /// already active; a runtime, model, or memory capacity is exceeded; the model
    /// handle or lifecycle is invalid; or the backend cannot plan or create the sequence.
    pub fn start_request(
        &mut self,
        handle: ModelHandle,
        request_id: RequestId,
        sequence_id: SequenceId,
        configuration: SequenceConfiguration,
    ) -> Result<RequestStartReceipt, RuntimeError> {
        self.start_request_with_reservation(
            handle,
            request_id,
            sequence_id,
            configuration,
            MemoryFootprint::default(),
            None,
        )
    }

    /// Preflights backend and aggregate-memory requirements before host workspace allocation.
    ///
    /// The generation scheduler calls this on the same exclusively owned runtime immediately
    /// before allocating its fixed workspaces. Full identity and lifecycle validation is repeated
    /// during commit so this optimization cannot weaken admission invariants.
    pub(crate) fn preflight_generation_resources(
        &self,
        handle: ModelHandle,
        request_id: RequestId,
        sequence_id: SequenceId,
        configuration: SequenceConfiguration,
        workspace_footprint: MemoryFootprint,
        expected_logits_capacity: usize,
    ) -> Result<(), RuntimeError> {
        self.reject_if_shutting_down()?;
        if self.request_index.contains_key(&request_id)
            || self.pending_request_index.contains_key(&request_id)
        {
            return Err(RuntimeError::RequestAlreadyActive(request_id));
        }
        if self.sequence_index.contains_key(&sequence_id)
            || self.pending_sequence_index.contains_key(&sequence_id)
        {
            return Err(RuntimeError::SequenceAlreadyActive(sequence_id));
        }
        if self
            .active_requests
            .saturating_add(self.pending_cleanup_sequences)
            >= self.limits.maximum_active_requests.get()
        {
            return Err(RuntimeError::CapacityExhausted(CapacityExhausted::new(
                CapacityResource::ActiveRequests,
                u64::from(
                    self.active_requests
                        .saturating_add(self.pending_cleanup_sequences),
                )
                .saturating_add(1),
                u64::from(self.limits.maximum_active_requests.get()),
            )));
        }

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
        if slot.poisoned {
            return Err(RuntimeError::ModelDegraded(handle.id));
        }
        if !matches!(
            slot.lifecycle.state(),
            ModelLifecycleState::Ready | ModelLifecycleState::Active { .. }
        ) {
            return Err(RuntimeError::Lifecycle(
                domain_contracts::LifecycleError::InvalidTransition,
            ));
        }
        if slot
            .requests
            .len()
            .saturating_add(slot.pending_sequences.len())
            >= slot.descriptor.capabilities.maximum_sequences as usize
        {
            return Err(RuntimeError::CapacityExhausted(CapacityExhausted::new(
                CapacityResource::ActiveSequences,
                saturating_u64(
                    slot.requests
                        .len()
                        .saturating_add(slot.pending_sequences.len()),
                )
                .saturating_add(1),
                u64::from(slot.descriptor.capabilities.maximum_sequences),
            )));
        }

        let plan = slot.model.plan_sequence(&configuration)?;
        if plan.configuration != configuration || plan.logits_capacity != expected_logits_capacity {
            return Err(RuntimeError::BackendContractViolation);
        }
        let committed_footprint =
            checked_add_footprint(plan.expected_footprint, workspace_footprint)?;
        admit_footprint(
            self.reserved_footprint,
            committed_footprint,
            self.limits.memory_budget,
        )?;
        Ok(())
    }

    pub(crate) fn start_generation_request(
        &mut self,
        handle: ModelHandle,
        request_id: RequestId,
        sequence_id: SequenceId,
        configuration: SequenceConfiguration,
        workspace_footprint: MemoryFootprint,
        expected_logits_capacity: usize,
    ) -> Result<RequestStartReceipt, RuntimeError> {
        self.start_request_with_reservation(
            handle,
            request_id,
            sequence_id,
            configuration,
            workspace_footprint,
            Some(expected_logits_capacity),
        )
    }

    #[expect(
        clippy::too_many_lines,
        reason = "sequence admission keeps prepare, validate, quarantine, and commit in one \
                  auditable transaction"
    )]
    fn start_request_with_reservation(
        &mut self,
        handle: ModelHandle,
        request_id: RequestId,
        sequence_id: SequenceId,
        configuration: SequenceConfiguration,
        workspace_footprint: MemoryFootprint,
        expected_logits_capacity: Option<usize>,
    ) -> Result<RequestStartReceipt, RuntimeError> {
        self.reject_if_shutting_down()?;
        if self.request_index.contains_key(&request_id)
            || self.pending_request_index.contains_key(&request_id)
        {
            return Err(RuntimeError::RequestAlreadyActive(request_id));
        }
        if self.sequence_index.contains_key(&sequence_id)
            || self.pending_sequence_index.contains_key(&sequence_id)
        {
            return Err(RuntimeError::SequenceAlreadyActive(sequence_id));
        }
        if self
            .active_requests
            .saturating_add(self.pending_cleanup_sequences)
            >= self.limits.maximum_active_requests.get()
        {
            return Err(RuntimeError::CapacityExhausted(CapacityExhausted::new(
                CapacityResource::ActiveRequests,
                u64::from(
                    self.active_requests
                        .saturating_add(self.pending_cleanup_sequences),
                )
                .saturating_add(1),
                u64::from(self.limits.maximum_active_requests.get()),
            )));
        }

        let current_reserved = self.reserved_footprint;
        let current_generation_workspace = self.reserved_generation_workspace;
        let memory_budget = self.limits.memory_budget;
        let maximum_cleanup_attempts = self.maximum_cleanup_attempts();
        let is_generation_request = expected_logits_capacity.is_some();
        let next_active_requests = self
            .active_requests
            .checked_add(1)
            .ok_or(RuntimeError::BackendContractViolation)?;
        let next_generation_workspaces = if is_generation_request {
            self.generation_workspaces
                .checked_add(1)
                .ok_or(RuntimeError::BackendContractViolation)?
        } else {
            self.generation_workspaces
        };
        let next_reserved_generation_workspace = if is_generation_request {
            checked_add_footprint(current_generation_workspace, workspace_footprint)?
        } else {
            current_generation_workspace
        };
        let slot = self.exact_model_mut(handle)?;
        if slot.poisoned {
            return Err(RuntimeError::ModelDegraded(handle.id));
        }
        match slot.lifecycle.state() {
            ModelLifecycleState::Ready | ModelLifecycleState::Active { .. } => {}
            _ => {
                return Err(RuntimeError::Lifecycle(
                    domain_contracts::LifecycleError::InvalidTransition,
                ));
            }
        }
        if slot.requests.contains_key(&request_id) {
            return Err(RuntimeError::RequestAlreadyActive(request_id));
        }
        if slot
            .requests
            .len()
            .saturating_add(slot.pending_sequences.len())
            >= slot.descriptor.capabilities.maximum_sequences as usize
        {
            return Err(RuntimeError::CapacityExhausted(CapacityExhausted::new(
                CapacityResource::ActiveSequences,
                saturating_u64(
                    slot.requests
                        .len()
                        .saturating_add(slot.pending_sequences.len()),
                )
                .saturating_add(1),
                u64::from(slot.descriptor.capabilities.maximum_sequences),
            )));
        }

        let plan = slot.model.plan_sequence(&configuration)?;
        if plan.configuration != configuration
            || expected_logits_capacity.is_some_and(|expected| plan.logits_capacity != expected)
        {
            return Err(RuntimeError::BackendContractViolation);
        }
        let expected_token_capacity = usize::try_from(plan.configuration.maximum_tokens.get())
            .map_err(|_| RuntimeError::BackendContractViolation)?;
        let committed_footprint =
            checked_add_footprint(plan.expected_footprint, workspace_footprint)?;
        let backend_next_reserved =
            admit_footprint(current_reserved, plan.expected_footprint, memory_budget)?;
        let committed_next_reserved =
            admit_footprint(current_reserved, committed_footprint, memory_budget)?;
        let backend_next_slot_reserved =
            checked_add_footprint(slot.reserved_footprint, plan.expected_footprint)?;
        let committed_next_slot_reserved =
            checked_add_footprint(slot.reserved_footprint, committed_footprint)?;
        let mut sequence = slot.model.create_sequence(sequence_id, &configuration)?;
        if sequence.id() != sequence_id || sequence.token_capacity() != expected_token_capacity {
            let primary = RuntimeError::BackendContractViolation;
            if let Err(cleanup) = slot.model.destroy_sequence(&mut sequence) {
                let report = CleanupFailureReport::new(
                    RuntimeOperation::SequenceAdmission,
                    primary.failure_class(),
                    RuntimeOperation::SequenceDestruction,
                    RuntimeError::Sequence(cleanup).failure_class(),
                );
                slot.pending_sequences.insert(
                    request_id,
                    PendingSequence {
                        request_id,
                        sequence_id,
                        sequence,
                        footprint: plan.expected_footprint,
                        failure: report,
                        attempts: 1,
                    },
                );
                slot.reserved_footprint = backend_next_slot_reserved;
                slot.poisoned = true;
                self.pending_request_index.insert(request_id, handle.id);
                self.pending_sequence_index.insert(sequence_id, request_id);
                self.pending_cleanup_sequences = self
                    .pending_cleanup_sequences
                    .checked_add(1)
                    .ok_or(RuntimeError::BackendContractViolation)?;
                self.reserved_footprint = backend_next_reserved;
                let state = CleanupRetryState {
                    resource: CleanupResource::Sequence {
                        model_id: handle.id,
                        request_id,
                        sequence_id,
                    },
                    failure: report,
                    attempts: 1,
                    maximum_attempts: maximum_cleanup_attempts,
                };
                self.last_cleanup = Some(state);
                return Err(cleanup_retention_error(state));
            }
            return Err(primary);
        }
        if let Err(error) = slot.lifecycle.start_request() {
            let primary = RuntimeError::Lifecycle(error);
            if let Err(cleanup) = slot.model.destroy_sequence(&mut sequence) {
                let report = CleanupFailureReport::new(
                    RuntimeOperation::SequenceAdmission,
                    primary.failure_class(),
                    RuntimeOperation::SequenceDestruction,
                    RuntimeError::Sequence(cleanup).failure_class(),
                );
                slot.pending_sequences.insert(
                    request_id,
                    PendingSequence {
                        request_id,
                        sequence_id,
                        sequence,
                        footprint: plan.expected_footprint,
                        failure: report,
                        attempts: 1,
                    },
                );
                slot.reserved_footprint = backend_next_slot_reserved;
                slot.poisoned = true;
                self.pending_request_index.insert(request_id, handle.id);
                self.pending_sequence_index.insert(sequence_id, request_id);
                self.pending_cleanup_sequences = self
                    .pending_cleanup_sequences
                    .checked_add(1)
                    .ok_or(RuntimeError::BackendContractViolation)?;
                self.reserved_footprint = backend_next_reserved;
                let state = CleanupRetryState {
                    resource: CleanupResource::Sequence {
                        model_id: handle.id,
                        request_id,
                        sequence_id,
                    },
                    failure: report,
                    attempts: 1,
                    maximum_attempts: maximum_cleanup_attempts,
                };
                self.last_cleanup = Some(state);
                return Err(cleanup_retention_error(state));
            }
            return Err(primary);
        }

        let request = RequestSlot {
            sequence,
            backend_footprint: plan.expected_footprint,
            workspace_footprint,
            usage: GenerationUsage::default(),
        };
        let replaced = slot.requests.insert(request_id, request);
        debug_assert!(replaced.is_none(), "request admission was preflighted");
        slot.reserved_footprint = committed_next_slot_reserved;

        let previous_model = self.request_index.insert(request_id, handle.id);
        debug_assert!(previous_model.is_none(), "request index was preflighted");
        let previous_request = self.sequence_index.insert(sequence_id, request_id);
        debug_assert!(previous_request.is_none(), "sequence index was preflighted");
        self.active_requests = next_active_requests;
        self.reserved_footprint = committed_next_reserved;
        self.generation_workspaces = next_generation_workspaces;
        self.reserved_generation_workspace = next_reserved_generation_workspace;

        Ok(RequestStartReceipt {
            request_id,
            sequence_id,
            logits_capacity: plan.logits_capacity,
            reserved_footprint: committed_footprint,
        })
    }

    /// Executes one checked prompt-prefill operation.
    ///
    /// # Errors
    ///
    /// Returns an error if the request or its model is no longer active, the checked
    /// backend operation fails, or destroying a finished or failed sequence violates
    /// a backend, lifecycle, or memory-accounting invariant.
    pub fn prefill(
        &mut self,
        request_id: RequestId,
        tokens: &[TokenId],
        emit_logits: bool,
        logits: &mut [f32],
    ) -> Result<PrefillReceipt, RuntimeError> {
        let model_id = self.request_model_id(request_id)?;
        let operation = {
            let slot = self
                .models
                .get_mut(&model_id)
                .ok_or(RuntimeError::ModelNotLoaded(model_id))?;
            let request = slot
                .requests
                .get_mut(&request_id)
                .ok_or(RuntimeError::RequestNotActive(request_id))?;
            let outcome = prefill_checked(
                &mut slot.model,
                &mut request.sequence,
                PrefillInput::new(tokens, emit_logits),
                PrefillBuffers::new(logits),
                CancellationStatus::Running,
            );
            match outcome {
                Ok(PrefillOutcome::Ready {
                    consumed_tokens,
                    position,
                    logits_written,
                }) => {
                    request.usage.prompt_tokens = request
                        .usage
                        .prompt_tokens
                        .saturating_add(saturating_u64(consumed_tokens));
                    Ok((
                        PrefillOutcome::Ready {
                            consumed_tokens,
                            position,
                            logits_written,
                        },
                        request.usage,
                    ))
                }
                Ok(PrefillOutcome::Finished(reason)) => {
                    Ok((PrefillOutcome::Finished(reason), request.usage))
                }
                Err(error) => Err(error),
            }
        };

        match operation {
            Ok((outcome, usage)) => {
                if let PrefillOutcome::Finished(reason) = outcome {
                    preserve_primary_cleanup(self.remove_request(
                        request_id,
                        finish_operation(reason),
                        finish_failure_class(reason),
                    ))?;
                }
                Ok(PrefillReceipt { outcome, usage })
            }
            Err(error) => {
                let primary = RuntimeError::Sequence(error);
                preserve_primary_cleanup(self.remove_request(
                    request_id,
                    RuntimeOperation::Prefill,
                    primary.failure_class(),
                ))?;
                Err(primary)
            }
        }
    }

    /// Executes one checked incremental decode operation.
    ///
    /// # Errors
    ///
    /// Returns an error if the request or its model is no longer active, the checked
    /// backend operation fails, or destroying a finished or failed sequence violates
    /// a backend, lifecycle, or memory-accounting invariant.
    pub fn decode(
        &mut self,
        request_id: RequestId,
        token: TokenId,
        logits: &mut [f32],
    ) -> Result<DecodeReceipt, RuntimeError> {
        let model_id = self.request_model_id(request_id)?;
        let operation = {
            let slot = self
                .models
                .get_mut(&model_id)
                .ok_or(RuntimeError::ModelNotLoaded(model_id))?;
            let request = slot
                .requests
                .get_mut(&request_id)
                .ok_or(RuntimeError::RequestNotActive(request_id))?;
            let outcome = decode_checked(
                &mut slot.model,
                &mut request.sequence,
                DecodeInput::new(token),
                DecodeBuffers::new(logits),
                CancellationStatus::Running,
            );
            match outcome {
                Ok(DecodeOutcome::Ready {
                    position,
                    logits_written,
                }) => {
                    request.usage.generated_tokens =
                        request.usage.generated_tokens.saturating_add(1);
                    Ok((
                        DecodeOutcome::Ready {
                            position,
                            logits_written,
                        },
                        request.usage,
                    ))
                }
                Ok(DecodeOutcome::Finished(reason)) => {
                    Ok((DecodeOutcome::Finished(reason), request.usage))
                }
                Err(error) => Err(error),
            }
        };

        match operation {
            Ok((outcome, usage)) => {
                if let DecodeOutcome::Finished(reason) = outcome {
                    preserve_primary_cleanup(self.remove_request(
                        request_id,
                        finish_operation(reason),
                        finish_failure_class(reason),
                    ))?;
                }
                Ok(DecodeReceipt { outcome, usage })
            }
            Err(error) => {
                let primary = RuntimeError::Sequence(error);
                preserve_primary_cleanup(self.remove_request(
                    request_id,
                    RuntimeOperation::Decode,
                    primary.failure_class(),
                ))?;
                Err(primary)
            }
        }
    }

    /// Completes one request and drops its sequence at a safe boundary.
    ///
    /// # Errors
    ///
    /// Returns an error if the request or model is no longer active, sequence destruction
    /// fails, or removing the request violates a lifecycle or memory-accounting invariant.
    pub fn complete_request(
        &mut self,
        request_id: RequestId,
        reason: FinishReason,
    ) -> Result<FinishReason, RuntimeError> {
        self.remove_request(
            request_id,
            finish_operation(reason),
            finish_failure_class(reason),
        )?;
        Ok(reason)
    }

    /// Cancels one request and drops its sequence at a safe boundary.
    ///
    /// # Errors
    ///
    /// Returns an error if the request or model is no longer active, sequence destruction
    /// fails, or removing the request violates a lifecycle or memory-accounting invariant.
    pub fn cancel_request(
        &mut self,
        request_id: RequestId,
        reason: CancellationReason,
    ) -> Result<FinishReason, RuntimeError> {
        self.remove_request(
            request_id,
            RuntimeOperation::Cancellation,
            FailureClass::Cancellation,
        )?;
        Ok(FinishReason::Cancelled(reason))
    }

    /// Cleans a request after a generation-kernel failure while preserving its class.
    ///
    /// # Errors
    ///
    /// Returns the cleanup failure report when explicit sequence destruction fails.
    pub fn fail_request(
        &mut self,
        request_id: RequestId,
        operation: RuntimeOperation,
        failure: FailureClass,
    ) -> Result<(), RuntimeError> {
        self.remove_request(request_id, operation, failure)
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

    /// Releases one generation task's host workspace after terminal output publication.
    pub(crate) fn release_generation_workspace(
        &mut self,
        footprint: MemoryFootprint,
    ) -> Result<(), RuntimeError> {
        let next_generation_workspaces = self
            .generation_workspaces
            .checked_sub(1)
            .ok_or(RuntimeError::BackendContractViolation)?;
        let next_reserved_generation_workspace =
            checked_sub_footprint(self.reserved_generation_workspace, footprint)?;
        let next_reserved_footprint = checked_sub_footprint(self.reserved_footprint, footprint)?;
        self.generation_workspaces = next_generation_workspaces;
        self.reserved_generation_workspace = next_reserved_generation_workspace;
        self.reserved_footprint = next_reserved_footprint;
        Ok(())
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
                resource: CleanupResource::Model { model_id },
                failure: pending.failure,
                attempts: pending.attempts,
                maximum_attempts,
            };
            let released = pending.model.prepare_unload().is_ok();
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
        let pending = self
            .pending_models
            .remove(&model_id)
            .ok_or(RuntimeError::ModelNotLoaded(model_id))?;
        self.reserved_footprint =
            checked_sub_footprint(self.reserved_footprint, pending.footprint)?;
        Ok(CleanupPoll::Released(state))
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
                Some((*model_id, slot.handle))
            } else {
                None
            }
        });
        if let Some((model_id, handle)) = pending_unload {
            let result = self.release_model(model_id).map(|()| UnloadReceipt {
                handle,
                status: UnloadStatus::Unloaded,
                cancelled_requests: 0,
            });
            return Some((handle, result));
        }
        None
    }

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
        let initial_models =
            saturating_u32(self.models.len().saturating_add(self.pending_models.len()));
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

    fn first_pending_cleanup_state(&self) -> Option<CleanupRetryState> {
        for (model_id, slot) in &self.models {
            if let Some((_, pending)) = slot.pending_sequences.first_key_value() {
                return Some(self.sequence_cleanup_state(*model_id, pending));
            }
        }
        self.pending_models
            .first_key_value()
            .map(|(model_id, pending)| self.model_cleanup_retry_state(*model_id, pending))
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

    #[expect(
        clippy::too_many_lines,
        reason = "request removal keeps destruction, quarantine, lifecycle, indices, and accounting in one transaction"
    )]
    fn remove_request(
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
                let sequence_id = request.sequence.id();
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
            let sequence_id = request.sequence.id();
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

    fn cancel_all_requests(&mut self, model_id: ModelId) -> Result<u32, RuntimeError> {
        let mut cancelled = self
            .models
            .get(&model_id)
            .map(|slot| slot.cancelled_requests_during_unload)
            .ok_or(RuntimeError::ModelNotLoaded(model_id))?;
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
                RuntimeOperation::Cancellation,
                FailureClass::Cancellation,
            ) {
                Ok(()) => {
                    cancelled = cancelled.saturating_add(1);
                    if let Some(slot) = self.models.get_mut(&model_id) {
                        slot.cancelled_requests_during_unload = cancelled;
                    } else {
                        break;
                    }
                }
                Err(
                    error @ (RuntimeError::CleanupFailed(_)
                    | RuntimeError::CleanupRetryExhausted(_)),
                ) => {
                    cancelled = cancelled.saturating_add(1);
                    if let Some(slot) = self.models.get_mut(&model_id) {
                        slot.cancelled_requests_during_unload = cancelled;
                    }
                    return Err(error);
                }
                Err(error) => return Err(error),
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

    fn release_model_with_primary(
        &mut self,
        model_id: ModelId,
        primary_operation: RuntimeOperation,
        primary_failure: FailureClass,
    ) -> Result<(), RuntimeError> {
        let cleanup_failure = {
            let slot = self
                .models
                .get_mut(&model_id)
                .ok_or(RuntimeError::ModelNotLoaded(model_id))?;
            if !slot.requests.is_empty()
                || !slot.pending_sequences.is_empty()
                || slot.lifecycle.state() != ModelLifecycleState::Unloading
                || slot.reserved_footprint != slot.model_footprint
            {
                return Err(RuntimeError::Lifecycle(
                    domain_contracts::LifecycleError::InvalidTransition,
                ));
            }
            slot.model.prepare_unload().err()
        };

        if let Some(cleanup) = cleanup_failure {
            let report = CleanupFailureReport::new(
                primary_operation,
                primary_failure,
                RuntimeOperation::ModelUnload,
                RuntimeError::Synchronization(cleanup).failure_class(),
            );
            let slot = self
                .models
                .remove(&model_id)
                .ok_or(RuntimeError::ModelNotLoaded(model_id))?;
            self.pending_models.insert(
                model_id,
                PendingModel {
                    model: slot.model,
                    footprint: slot.model_footprint,
                    failure: report,
                    attempts: 1,
                    cancelled_requests: slot.cancelled_requests_during_unload,
                },
            );
            let state = CleanupRetryState {
                resource: CleanupResource::Model { model_id },
                failure: report,
                attempts: 1,
                maximum_attempts: self.maximum_cleanup_attempts(),
            };
            self.last_cleanup = Some(state);
            return Err(cleanup_retention_error(state));
        }

        self.models
            .get_mut(&model_id)
            .ok_or(RuntimeError::ModelNotLoaded(model_id))?
            .lifecycle
            .complete_unload()?;
        let slot = self
            .models
            .remove(&model_id)
            .ok_or(RuntimeError::ModelNotLoaded(model_id))?;
        self.reserved_footprint =
            checked_sub_footprint(self.reserved_footprint, slot.model_footprint)?;
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
                RuntimeError::CleanupFailed(state.failure)
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

const fn preserve_primary_cleanup(result: Result<(), RuntimeError>) -> Result<(), RuntimeError> {
    match result {
        Ok(()) | Err(RuntimeError::CleanupFailed(_) | RuntimeError::CleanupRetryExhausted(_)) => {
            Ok(())
        }
        Err(error) => Err(error),
    }
}

const fn cleanup_retention_error(state: CleanupRetryState) -> RuntimeError {
    if state.exhausted() {
        RuntimeError::CleanupRetryExhausted(state)
    } else {
        RuntimeError::CleanupFailed(state.failure)
    }
}

const fn finish_operation(reason: FinishReason) -> RuntimeOperation {
    if matches!(reason, FinishReason::Cancelled(_)) {
        RuntimeOperation::Cancellation
    } else {
        RuntimeOperation::Completion
    }
}

const fn finish_failure_class(reason: FinishReason) -> FailureClass {
    match reason {
        FinishReason::Cancelled(_) => FailureClass::Cancellation,
        FinishReason::BufferExhausted(_) => FailureClass::Capacity,
        FinishReason::EndOfSequence(_) | FinishReason::TokenLimit | FinishReason::StopCondition => {
            FailureClass::Completion
        }
        _ => FailureClass::Completion,
    }
}

const fn remaining_budget(limit: MemoryBudget, used: MemoryFootprint) -> MemoryBudget {
    MemoryBudget {
        host_bytes: limit.host_bytes.saturating_sub(used.host_bytes()),
        device_bytes: limit.device_bytes.saturating_sub(used.device_bytes()),
    }
}

fn admit_footprint(
    current: MemoryFootprint,
    additional: MemoryFootprint,
    budget: MemoryBudget,
) -> Result<MemoryFootprint, RuntimeError> {
    let next = checked_add_footprint(current, additional)?;
    let required_host = next.host_bytes();
    if required_host > budget.host_bytes {
        return Err(RuntimeError::InsufficientMemory {
            kind: MemoryKind::Host,
            required_bytes: required_host,
            available_bytes: budget.host_bytes,
        });
    }
    let required_device = next.device_bytes();
    if required_device > budget.device_bytes {
        return Err(RuntimeError::InsufficientMemory {
            kind: MemoryKind::Device,
            required_bytes: required_device,
            available_bytes: budget.device_bytes,
        });
    }
    Ok(next)
}

fn checked_add_footprint(
    left: MemoryFootprint,
    right: MemoryFootprint,
) -> Result<MemoryFootprint, RuntimeError> {
    Ok(MemoryFootprint {
        host_weight_bytes: left
            .host_weight_bytes
            .checked_add(right.host_weight_bytes)
            .ok_or(RuntimeError::MemoryArithmeticOverflow)?,
        device_weight_bytes: left
            .device_weight_bytes
            .checked_add(right.device_weight_bytes)
            .ok_or(RuntimeError::MemoryArithmeticOverflow)?,
        host_working_bytes: left
            .host_working_bytes
            .checked_add(right.host_working_bytes)
            .ok_or(RuntimeError::MemoryArithmeticOverflow)?,
        device_working_bytes: left
            .device_working_bytes
            .checked_add(right.device_working_bytes)
            .ok_or(RuntimeError::MemoryArithmeticOverflow)?,
        cache_bytes_per_token: left
            .cache_bytes_per_token
            .checked_add(right.cache_bytes_per_token)
            .ok_or(RuntimeError::MemoryArithmeticOverflow)?,
    })
}

fn checked_sub_footprint(
    left: MemoryFootprint,
    right: MemoryFootprint,
) -> Result<MemoryFootprint, RuntimeError> {
    Ok(MemoryFootprint {
        host_weight_bytes: left
            .host_weight_bytes
            .checked_sub(right.host_weight_bytes)
            .ok_or(RuntimeError::MemoryArithmeticUnderflow)?,
        device_weight_bytes: left
            .device_weight_bytes
            .checked_sub(right.device_weight_bytes)
            .ok_or(RuntimeError::MemoryArithmeticUnderflow)?,
        host_working_bytes: left
            .host_working_bytes
            .checked_sub(right.host_working_bytes)
            .ok_or(RuntimeError::MemoryArithmeticUnderflow)?,
        device_working_bytes: left
            .device_working_bytes
            .checked_sub(right.device_working_bytes)
            .ok_or(RuntimeError::MemoryArithmeticUnderflow)?,
        cache_bytes_per_token: left
            .cache_bytes_per_token
            .checked_sub(right.cache_bytes_per_token)
            .ok_or(RuntimeError::MemoryArithmeticUnderflow)?,
    })
}

fn saturating_u32(value: usize) -> u32 {
    u32::try_from(value).unwrap_or(u32::MAX)
}

fn saturating_u64(value: usize) -> u64 {
    u64::try_from(value).unwrap_or(u64::MAX)
}
