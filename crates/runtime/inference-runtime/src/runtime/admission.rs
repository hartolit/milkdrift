use std::collections::BTreeMap;

use domain_contracts::{
    BackendSequence, CapabilitySet, CapacityExhausted, CapacityResource, ExecutionDevice,
    GenerationUsage, LoadConfiguration, LoadPlan, LoadedModel, MemoryFootprint, ModelDescriptor,
    ModelError, ModelHandle, ModelId, ModelLifecycle, ModelLifecycleState, ModelLoader,
    PreparedLoad, RequestId, SequenceConfiguration, SequenceId,
};

use crate::{
    CleanupFailureReport, CleanupResource, CleanupRetryState, FailureClass, LoadReceipt,
    RequestStartReceipt, RuntimeError, RuntimeOperation,
};

use super::{
    InferenceRuntime, ModelSlot, PendingModel, PendingModelOwner, PendingSequence, RequestSlot,
    cleanup::cleanup_retention_error,
    memory::{
        admit_footprint, checked_add_footprint, remaining_budget, saturating_u32, saturating_u64,
    },
};

impl<L> InferenceRuntime<L>
where
    L: ModelLoader,
{
    /// Inspects, admits, and loads one model synchronously.
    ///
    /// # Errors
    ///
    /// Returns an error if shutdown has started; the model identity is already loaded;
    /// a model, generation, or memory limit is exceeded; a lifecycle transition fails;
    /// or the backend cannot plan or load a model that satisfies its declared contract.
    #[expect(
        clippy::too_many_lines,
        reason = "model admission keeps preparation, peak reservation, materialization, verification, rollback, quarantine, and final commit in one auditable transaction"
    )]
    pub fn load_model(
        &mut self,
        model_id: ModelId,
        source: &L::Source,
        execution_device: ExecutionDevice,
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
        let previous_reserved = self.reserved_footprint;
        let remaining_budget = remaining_budget(self.limits.memory_budget, previous_reserved)?;
        let configuration = LoadConfiguration {
            handle,
            execution_device,
            memory_budget: remaining_budget,
        };
        let mut lifecycle = ModelLifecycle::new();
        lifecycle.begin_load()?;

        let prepared = self.loader.prepare_load(source, &configuration)?;
        let plan = *prepared.plan();
        validate_load_plan(&plan, configuration)?;
        let loading_reserved = admit_footprint(
            previous_reserved,
            plan.loading_peak_footprint,
            self.limits.memory_budget,
        )?;
        let final_reserved = admit_footprint(
            previous_reserved,
            plan.expected_footprint,
            self.limits.memory_budget,
        )?;
        self.reserved_footprint = loading_reserved;

        let mut model = match self.loader.load_prepared(prepared) {
            Ok(model) => model,
            Err(failed) => {
                let (primary, mut cleanup_owner) = failed.into_parts();
                if let Err(cleanup) = cleanup_owner.cleanup() {
                    let report = CleanupFailureReport::new(
                        RuntimeOperation::ModelLoad,
                        FailureClass::Load,
                        RuntimeOperation::FailedLoadCleanup,
                        RuntimeError::Synchronization(cleanup).failure_class(),
                    );
                    let pending = PendingModel {
                        owner: PendingModelOwner::FailedLoad(cleanup_owner),
                        footprint: plan.loading_peak_footprint,
                        failure: report,
                        attempts: 1,
                        cancelled_requests: 0,
                    };
                    let replaced = self.pending_models.insert(model_id, pending);
                    debug_assert!(replaced.is_none(), "model admission was preflighted");
                    self.generations.insert(model_id, handle.generation);
                    let state = CleanupRetryState {
                        resource: CleanupResource::FailedLoad { model_id },
                        failure: report,
                        attempts: 1,
                        maximum_attempts: self.maximum_cleanup_attempts(),
                    };
                    self.last_cleanup = Some(state);
                    return Err(cleanup_retention_error(state));
                }
                self.reserved_footprint = previous_reserved;
                return Err(RuntimeError::Load(primary));
            }
        };

        let actual_handle = model.handle();
        let actual_descriptor = *model.descriptor();
        let actual_device = model.execution_device();
        let actual_execution_scalar_type = model.execution_scalar_type();
        let actual_accounted_footprint = model.accounted_footprint();
        let validation = if actual_handle != handle
            || actual_descriptor != plan.descriptor
            || actual_device != execution_device
            || actual_execution_scalar_type != plan.execution_scalar_type
            || actual_accounted_footprint != plan.expected_footprint
        {
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
                    owner: PendingModelOwner::Complete(model),
                    footprint: plan.loading_peak_footprint,
                    failure: report,
                    attempts: 1,
                    cancelled_requests: 0,
                };
                let replaced = self.pending_models.insert(model_id, pending);
                debug_assert!(replaced.is_none(), "model admission was preflighted");
                self.generations.insert(model_id, handle.generation);
                let state = CleanupRetryState {
                    resource: CleanupResource::Model { model_id },
                    failure: report,
                    attempts: 1,
                    maximum_attempts: self.maximum_cleanup_attempts(),
                };
                self.last_cleanup = Some(state);
                return Err(cleanup_retention_error(state));
            }
            self.reserved_footprint = previous_reserved;
            return Err(primary);
        }

        let slot = ModelSlot {
            handle,
            execution_device: actual_device,
            execution_scalar_type: actual_execution_scalar_type,
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
        self.reserved_footprint = final_reserved;

        Ok(LoadReceipt {
            handle,
            execution_device: actual_device,
            execution_scalar_type: actual_execution_scalar_type,
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
        validate_requested_sequence_configuration(&slot.descriptor, configuration)?;
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
        if !sequence_configuration_is_supported(&slot.descriptor, plan.configuration)
            || plan.configuration != configuration
            || plan.logits_capacity != expected_logits_capacity
        {
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
        validate_requested_sequence_configuration(&slot.descriptor, configuration)?;
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
        if !sequence_configuration_is_supported(&slot.descriptor, plan.configuration)
            || plan.configuration != configuration
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
            sequence_id,
            token_capacity: expected_token_capacity,
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
}

fn validate_load_plan(
    plan: &LoadPlan,
    configuration: LoadConfiguration,
) -> Result<(), RuntimeError> {
    if plan.accepted_configuration != configuration {
        return Err(RuntimeError::BackendContractViolation);
    }
    validate_descriptor(&plan.descriptor)?;
    let final_host = plan
        .expected_footprint
        .checked_host_bytes()
        .ok_or(RuntimeError::BackendContractViolation)?;
    let final_device = plan
        .expected_footprint
        .checked_device_bytes()
        .ok_or(RuntimeError::BackendContractViolation)?;
    let loading_host = plan
        .loading_peak_footprint
        .checked_host_bytes()
        .ok_or(RuntimeError::BackendContractViolation)?;
    let loading_device = plan
        .loading_peak_footprint
        .checked_device_bytes()
        .ok_or(RuntimeError::BackendContractViolation)?;
    let final_footprint = plan.expected_footprint;
    let loading_footprint = plan.loading_peak_footprint;
    let loading_components_contain_final_ownership = loading_footprint.host_weight_bytes
        >= final_footprint.host_weight_bytes
        && loading_footprint.device_weight_bytes >= final_footprint.device_weight_bytes
        && loading_footprint.host_working_bytes >= final_footprint.host_working_bytes
        && loading_footprint.device_working_bytes >= final_footprint.device_working_bytes;
    if loading_host < final_host
        || loading_device < final_device
        || !loading_components_contain_final_ownership
        || loading_footprint.cache_bytes_per_token != final_footprint.cache_bytes_per_token
    {
        return Err(RuntimeError::BackendContractViolation);
    }
    Ok(())
}

const fn validate_descriptor(descriptor: &ModelDescriptor) -> Result<(), RuntimeError> {
    let metadata = descriptor.metadata;
    let capabilities = descriptor.capabilities;
    let numeric_limits_are_nonzero = !metadata.observed_tensor_scalar_types.is_empty()
        && metadata.vocabulary_size > 0
        && metadata.context_length > 0
        && capabilities.maximum_context_tokens > 0
        && capabilities.maximum_sequences > 0
        && capabilities.maximum_prefill_batch > 0;
    let context_limits_are_consistent = context_limits_are_ordered(
        metadata.context_length,
        capabilities.maximum_context_tokens,
        capabilities.maximum_prefill_batch,
    );
    let sequence_capability_is_consistent = capabilities.maximum_sequences <= 1
        || capabilities
            .operations
            .contains(CapabilitySet::MULTIPLE_SEQUENCES);
    if numeric_limits_are_nonzero
        && context_limits_are_consistent
        && sequence_capability_is_consistent
    {
        Ok(())
    } else {
        Err(RuntimeError::BackendContractViolation)
    }
}

const fn context_limits_are_ordered(
    native_context_limit: u32,
    sequence_context_limit: u32,
    prefill_limit: u32,
) -> bool {
    sequence_context_limit <= native_context_limit && prefill_limit <= sequence_context_limit
}

const fn sequence_configuration_is_supported(
    descriptor: &ModelDescriptor,
    configuration: SequenceConfiguration,
) -> bool {
    configuration.maximum_tokens.get() <= descriptor.capabilities.maximum_context_tokens
        && configuration.maximum_prefill_batch.get()
            <= descriptor.capabilities.maximum_prefill_batch
        && configuration.maximum_prefill_batch.get() <= configuration.maximum_tokens.get()
}

const fn validate_requested_sequence_configuration(
    descriptor: &ModelDescriptor,
    configuration: SequenceConfiguration,
) -> Result<(), RuntimeError> {
    if sequence_configuration_is_supported(descriptor, configuration) {
        Ok(())
    } else {
        Err(RuntimeError::Model(ModelError::Unsupported))
    }
}
