use std::collections::BTreeMap;

use domain_contracts::{
    BackendSequence, CapabilitySet, CapacityExhausted, CapacityResource, ExecutionDevice,
    FailedLoad, GenerationUsage, LoadConfiguration, LoadPlan, LoadedModel, MemoryFootprint,
    ModelDescriptor, ModelError, ModelHandle, ModelId, ModelLifecycle, ModelLifecycleState,
    ModelLoader, PreparedLoad, RequestId, ScalarType, SequenceConfiguration, SequenceId,
};

use crate::{
    CleanupFailureReport, CleanupResource, CleanupRetryState, FailureDetail, LoadReceipt,
    RequestStartReceipt, RetainedOwnership, RuntimeError, RuntimeOperation,
};

use super::{
    InferenceRuntime, ModelSlot, PendingModel, PendingModelOwner, PendingSequence, RequestSlot,
    cleanup::cleanup_retention_error,
    memory::{
        admit_footprint, checked_add_footprint, conservative_footprint, remaining_budget,
        saturating_u32, saturating_u64,
    },
};

#[derive(Clone, Copy)]
struct LoadPreflight {
    handle: ModelHandle,
    configuration: LoadConfiguration,
    previous_reserved: MemoryFootprint,
    lifecycle: ModelLifecycle,
}

#[derive(Clone, Copy)]
struct PreparedAdmission {
    plan: LoadPlan,
    loading_reserved: MemoryFootprint,
    final_reserved: MemoryFootprint,
}

#[derive(Clone, Copy)]
struct CompleteModelReport {
    handle: ModelHandle,
    descriptor: ModelDescriptor,
    execution_device: ExecutionDevice,
    execution_scalar_type: ScalarType,
    footprint: MemoryFootprint,
}

impl CompleteModelReport {
    fn read<M: LoadedModel>(model: &M) -> Self {
        Self {
            handle: model.handle(),
            descriptor: *model.descriptor(),
            execution_device: model.execution_device(),
            execution_scalar_type: model.execution_scalar_type(),
            footprint: model.reported_footprint(),
        }
    }

    fn matches(self, preflight: LoadPreflight, plan: LoadPlan) -> bool {
        self.handle == preflight.handle
            && self.descriptor == plan.descriptor
            && self.execution_device == preflight.configuration.execution_device
            && self.execution_scalar_type == plan.execution_scalar_type
            && self.footprint == plan.final_footprint
    }
}

#[derive(Clone, Copy)]
struct SequenceAdmissionRequest {
    handle: ModelHandle,
    request_id: RequestId,
    sequence_id: SequenceId,
    configuration: SequenceConfiguration,
    workspace_footprint: MemoryFootprint,
    expected_logits_capacity: Option<usize>,
}

#[derive(Clone, Copy)]
struct SequenceRuntimeTransition {
    active_requests: u32,
    generation_workspaces: u32,
    reserved_generation_workspace: MemoryFootprint,
    pending_cleanup_sequences: u32,
}

#[derive(Clone, Copy)]
struct SequenceAdmission {
    request: SequenceAdmissionRequest,
    transition: SequenceRuntimeTransition,
    expected_token_capacity: usize,
    logits_capacity: usize,
    backend_footprint: MemoryFootprint,
    committed_footprint: MemoryFootprint,
    backend_next_reserved: MemoryFootprint,
    committed_next_reserved: MemoryFootprint,
    backend_next_slot_reserved: MemoryFootprint,
    committed_next_slot_reserved: MemoryFootprint,
}

enum SequenceSlotDisposition {
    Retained(CleanupFailureReport),
    Committed,
}

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
    pub fn load_model(
        &mut self,
        model_id: ModelId,
        source: &L::Source,
        execution_device: ExecutionDevice,
    ) -> Result<LoadReceipt, RuntimeError> {
        let preflight = self.load_preflight(model_id, execution_device)?;
        let prepared = self.loader.prepare_load(source, &preflight.configuration)?;
        let admission = admit_preparation(*prepared.plan(), preflight, self.limits.memory_budget)?;
        self.reserved_footprint = admission.loading_reserved;

        let mut model = match self.loader.load_prepared(prepared) {
            Ok(model) => model,
            Err(failed) => {
                return Err(self.finish_materialization_failure(
                    model_id,
                    preflight,
                    admission.plan,
                    failed,
                ));
            }
        };

        let report = CompleteModelReport::read(&model);
        if !report.matches(preflight, admission.plan) {
            return Err(self.reject_incompatible_complete_model(
                model_id,
                preflight,
                admission.plan,
                report.footprint,
                model,
            ));
        }

        let mut lifecycle = preflight.lifecycle;
        if let Err(error) = lifecycle.complete_load() {
            let primary = RuntimeError::Lifecycle(error);
            if let Err(cleanup) = model.prepare_unload() {
                let report = CleanupFailureReport::with_details(
                    RuntimeOperation::ModelAdmission,
                    primary.failure_detail(),
                    RuntimeOperation::ModelUnload,
                    FailureDetail::Synchronization(cleanup),
                );
                let pending = PendingModel {
                    handle: preflight.handle,
                    owner: PendingModelOwner::VerifiedModel(model),
                    ownership: RetainedOwnership::Exact(admission.plan.final_footprint),
                    failure: report,
                    attempts: 1,
                    cancelled_requests: 0,
                };
                let state = self.commit_pending_model(model_id, pending, admission.final_reserved);
                return Err(cleanup_retention_error(state));
            }
            self.reserved_footprint = preflight.previous_reserved;
            return Err(primary);
        }

        Ok(self.commit_loaded_model(model_id, preflight, &admission, report, lifecycle, model))
    }

    fn load_preflight(
        &self,
        model_id: ModelId,
        execution_device: ExecutionDevice,
    ) -> Result<LoadPreflight, RuntimeError> {
        self.reject_if_shutting_down()?;
        self.reject_if_admission_blocked()?;
        if self.models.contains_key(&model_id) || self.pending_models.contains_key(&model_id) {
            return Err(RuntimeError::ModelAlreadyLoaded(model_id));
        }
        let owned_models = self.models.len().saturating_add(self.pending_models.len());
        if owned_models >= self.limits.maximum_loaded_models.get() as usize {
            return Err(RuntimeError::LoadedModelLimit {
                required: saturating_u32(owned_models).saturating_add(1),
                available: self.limits.maximum_loaded_models.get(),
            });
        }

        let handle = self.next_handle(model_id)?;
        let previous_reserved = self.reserved_footprint;
        let configuration = LoadConfiguration {
            handle,
            execution_device,
            memory_budget: remaining_budget(self.limits.memory_budget, previous_reserved)?,
        };
        let mut lifecycle = ModelLifecycle::new();
        lifecycle.begin_load()?;
        Ok(LoadPreflight {
            handle,
            configuration,
            previous_reserved,
            lifecycle,
        })
    }

    fn finish_materialization_failure(
        &mut self,
        model_id: ModelId,
        preflight: LoadPreflight,
        plan: LoadPlan,
        mut failed: FailedLoad<L::FailedPreparation>,
    ) -> RuntimeError {
        let load_failure = failed.primary();
        let failed_plan_before_cleanup = *failed.plan();
        let cleanup_result = failed.cleanup();
        let failed_plan_after_cleanup = *failed.plan();
        let plan_matches = failed_plan_before_cleanup == plan && failed_plan_after_cleanup == plan;
        let primary = if plan_matches {
            RuntimeError::Load(load_failure)
        } else {
            RuntimeError::BackendContractViolation
        };

        if let Err(cleanup) = cleanup_result {
            let report = CleanupFailureReport::with_details(
                if plan_matches {
                    RuntimeOperation::ModelLoad
                } else {
                    RuntimeOperation::ModelAdmission
                },
                primary.failure_detail(),
                RuntimeOperation::FailedLoadCleanup,
                FailureDetail::Synchronization(cleanup),
            );
            let mut pending = PendingModel {
                handle: preflight.handle,
                owner: PendingModelOwner::FailedPreparation {
                    owner: failed,
                    accepted_plan: plan,
                },
                ownership: RetainedOwnership::Exact(plan.loading_peak_footprint),
                failure: report,
                attempts: 1,
                cancelled_requests: 0,
            };

            // Preserve every contradictory report observed around the cleanup
            // attempt. A backend cannot shrink previously published conservative
            // evidence by changing its report again on a later retry.
            pending.reconcile_failed_preparation_report(plan, failed_plan_before_cleanup);
            pending.reconcile_failed_preparation_report(plan, failed_plan_after_cleanup);
            let exact_reserved = if pending.ownership.exact_footprint().is_some() {
                self.reserved_footprint
            } else {
                preflight.previous_reserved
            };
            let state = self.commit_pending_model(model_id, pending, exact_reserved);
            cleanup_retention_error(state)
        } else {
            self.reserved_footprint = preflight.previous_reserved;
            primary
        }
    }

    fn reject_incompatible_complete_model(
        &mut self,
        model_id: ModelId,
        preflight: LoadPreflight,
        plan: LoadPlan,
        reported_footprint: MemoryFootprint,
        mut model: L::Model,
    ) -> RuntimeError {
        let primary = RuntimeError::BackendContractViolation;
        if let Err(cleanup) = model.prepare_unload() {
            let report = CleanupFailureReport::with_details(
                RuntimeOperation::ModelAdmission,
                primary.failure_detail(),
                RuntimeOperation::ModelUnload,
                FailureDetail::Synchronization(cleanup),
            );
            let ownership = RetainedOwnership::Unverified {
                accepted_loading_peak: plan.loading_peak_footprint,
                reported_footprint,
                conservative_footprint: conservative_footprint(
                    plan.loading_peak_footprint,
                    reported_footprint,
                ),
            };
            let pending = PendingModel {
                handle: preflight.handle,
                owner: PendingModelOwner::IncompatibleModel(model),
                ownership,
                failure: report,
                attempts: 1,
                cancelled_requests: 0,
            };
            let state = self.commit_pending_model(model_id, pending, preflight.previous_reserved);
            cleanup_retention_error(state)
        } else {
            self.reserved_footprint = preflight.previous_reserved;
            primary
        }
    }

    fn commit_pending_model(
        &mut self,
        model_id: ModelId,
        pending: PendingModel<L::Model, FailedLoad<L::FailedPreparation>>,
        exact_reserved: MemoryFootprint,
    ) -> CleanupRetryState {
        let state = CleanupRetryState {
            resource: pending.owner.cleanup_resource(pending.handle),
            failure: pending.failure,
            ownership: pending.ownership,
            attempts: pending.attempts,
            maximum_attempts: self.maximum_cleanup_attempts(),
        };
        self.reserved_footprint = exact_reserved;
        self.generations.insert(model_id, pending.handle.generation);
        let replaced = self.pending_models.insert(model_id, pending);
        debug_assert!(replaced.is_none(), "model admission was preflighted");
        self.last_cleanup = Some(state);
        state
    }

    fn commit_loaded_model(
        &mut self,
        model_id: ModelId,
        preflight: LoadPreflight,
        admission: &PreparedAdmission,
        report: CompleteModelReport,
        lifecycle: ModelLifecycle,
        model: L::Model,
    ) -> LoadReceipt {
        let slot = ModelSlot {
            handle: preflight.handle,
            execution_device: report.execution_device,
            execution_scalar_type: report.execution_scalar_type,
            descriptor: admission.plan.descriptor,
            lifecycle,
            model,
            model_footprint: admission.plan.final_footprint,
            reserved_footprint: admission.plan.final_footprint,
            requests: BTreeMap::new(),
            pending_sequences: BTreeMap::new(),
            poisoned: false,
            cancelled_requests_during_unload: 0,
        };
        let replaced = self.models.insert(model_id, slot);
        debug_assert!(replaced.is_none(), "model admission was preflighted");
        self.generations
            .insert(model_id, preflight.handle.generation);
        self.reserved_footprint = admission.final_reserved;

        LoadReceipt {
            handle: preflight.handle,
            execution_device: report.execution_device,
            execution_scalar_type: report.execution_scalar_type,
            descriptor: admission.plan.descriptor,
            reserved_footprint: admission.plan.final_footprint,
        }
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
        self.reject_if_admission_blocked()?;
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

    fn start_request_with_reservation(
        &mut self,
        handle: ModelHandle,
        request_id: RequestId,
        sequence_id: SequenceId,
        configuration: SequenceConfiguration,
        workspace_footprint: MemoryFootprint,
        expected_logits_capacity: Option<usize>,
    ) -> Result<RequestStartReceipt, RuntimeError> {
        let request = SequenceAdmissionRequest {
            handle,
            request_id,
            sequence_id,
            configuration,
            workspace_footprint,
            expected_logits_capacity,
        };
        let admission = self.prepare_sequence_admission(request)?;
        let disposition = {
            let slot = self.exact_model_mut(handle)?;
            apply_sequence_to_slot(slot, &admission)?
        };
        match disposition {
            SequenceSlotDisposition::Retained(report) => {
                Err(self.commit_retained_sequence(&admission, report))
            }
            SequenceSlotDisposition::Committed => Ok(self.commit_sequence_admission(&admission)),
        }
    }

    fn prepare_sequence_admission(
        &self,
        request: SequenceAdmissionRequest,
    ) -> Result<SequenceAdmission, RuntimeError> {
        let transition = self.prepare_sequence_runtime_transition(request)?;
        let current_reserved = self.reserved_footprint;
        let slot = self.exact_model(request.handle)?;
        if slot.poisoned {
            return Err(RuntimeError::ModelDegraded(request.handle.id));
        }
        if !matches!(
            slot.lifecycle.state(),
            ModelLifecycleState::Ready | ModelLifecycleState::Active { .. }
        ) {
            return Err(RuntimeError::Lifecycle(
                domain_contracts::LifecycleError::InvalidTransition,
            ));
        }
        validate_requested_sequence_configuration(&slot.descriptor, request.configuration)?;
        if slot.requests.contains_key(&request.request_id) {
            return Err(RuntimeError::RequestAlreadyActive(request.request_id));
        }
        let owned_sequences = slot
            .requests
            .len()
            .saturating_add(slot.pending_sequences.len());
        if owned_sequences >= slot.descriptor.capabilities.maximum_sequences as usize {
            return Err(RuntimeError::CapacityExhausted(CapacityExhausted::new(
                CapacityResource::ActiveSequences,
                saturating_u64(owned_sequences).saturating_add(1),
                u64::from(slot.descriptor.capabilities.maximum_sequences),
            )));
        }

        let plan = slot.model.plan_sequence(&request.configuration)?;
        if !sequence_configuration_is_supported(&slot.descriptor, plan.configuration)
            || plan.configuration != request.configuration
            || request
                .expected_logits_capacity
                .is_some_and(|expected| plan.logits_capacity != expected)
        {
            return Err(RuntimeError::BackendContractViolation);
        }
        let expected_token_capacity = usize::try_from(plan.configuration.maximum_tokens.get())
            .map_err(|_| RuntimeError::BackendContractViolation)?;
        let committed_footprint =
            checked_add_footprint(plan.expected_footprint, request.workspace_footprint)?;
        Ok(SequenceAdmission {
            request,
            transition,
            expected_token_capacity,
            logits_capacity: plan.logits_capacity,
            backend_footprint: plan.expected_footprint,
            committed_footprint,
            backend_next_reserved: admit_footprint(
                current_reserved,
                plan.expected_footprint,
                self.limits.memory_budget,
            )?,
            committed_next_reserved: admit_footprint(
                current_reserved,
                committed_footprint,
                self.limits.memory_budget,
            )?,
            backend_next_slot_reserved: checked_add_footprint(
                slot.reserved_footprint,
                plan.expected_footprint,
            )?,
            committed_next_slot_reserved: checked_add_footprint(
                slot.reserved_footprint,
                committed_footprint,
            )?,
        })
    }

    fn prepare_sequence_runtime_transition(
        &self,
        request: SequenceAdmissionRequest,
    ) -> Result<SequenceRuntimeTransition, RuntimeError> {
        self.reject_if_shutting_down()?;
        self.reject_if_admission_blocked()?;
        if self.request_index.contains_key(&request.request_id)
            || self.pending_request_index.contains_key(&request.request_id)
        {
            return Err(RuntimeError::RequestAlreadyActive(request.request_id));
        }
        if self.sequence_index.contains_key(&request.sequence_id)
            || self
                .pending_sequence_index
                .contains_key(&request.sequence_id)
        {
            return Err(RuntimeError::SequenceAlreadyActive(request.sequence_id));
        }
        let owned_requests = self
            .active_requests
            .saturating_add(self.pending_cleanup_sequences);
        if owned_requests >= self.limits.maximum_active_requests.get() {
            return Err(RuntimeError::CapacityExhausted(CapacityExhausted::new(
                CapacityResource::ActiveRequests,
                u64::from(owned_requests).saturating_add(1),
                u64::from(self.limits.maximum_active_requests.get()),
            )));
        }

        let is_generation_request = request.expected_logits_capacity.is_some();
        Ok(SequenceRuntimeTransition {
            active_requests: self
                .active_requests
                .checked_add(1)
                .ok_or(RuntimeError::BackendContractViolation)?,
            generation_workspaces: if is_generation_request {
                self.generation_workspaces
                    .checked_add(1)
                    .ok_or(RuntimeError::BackendContractViolation)?
            } else {
                self.generation_workspaces
            },
            reserved_generation_workspace: if is_generation_request {
                checked_add_footprint(
                    self.reserved_generation_workspace,
                    request.workspace_footprint,
                )?
            } else {
                self.reserved_generation_workspace
            },
            pending_cleanup_sequences: self
                .pending_cleanup_sequences
                .checked_add(1)
                .ok_or(RuntimeError::BackendContractViolation)?,
        })
    }

    fn commit_retained_sequence(
        &mut self,
        admission: &SequenceAdmission,
        report: CleanupFailureReport,
    ) -> RuntimeError {
        let request = admission.request;
        let previous_model = self
            .pending_request_index
            .insert(request.request_id, request.handle.id);
        debug_assert!(
            previous_model.is_none(),
            "pending request index was preflighted"
        );
        let previous_request = self
            .pending_sequence_index
            .insert(request.sequence_id, request.request_id);
        debug_assert!(
            previous_request.is_none(),
            "pending sequence index was preflighted"
        );
        self.pending_cleanup_sequences = admission.transition.pending_cleanup_sequences;
        self.reserved_footprint = admission.backend_next_reserved;
        let state = CleanupRetryState {
            resource: CleanupResource::Sequence {
                handle: request.handle,
                request_id: request.request_id,
                sequence_id: request.sequence_id,
            },
            failure: report,
            ownership: RetainedOwnership::Exact(admission.backend_footprint),
            attempts: 1,
            maximum_attempts: self.maximum_cleanup_attempts(),
        };
        self.last_cleanup = Some(state);
        cleanup_retention_error(state)
    }

    fn commit_sequence_admission(&mut self, admission: &SequenceAdmission) -> RequestStartReceipt {
        let request = admission.request;
        let previous_model = self
            .request_index
            .insert(request.request_id, request.handle.id);
        debug_assert!(previous_model.is_none(), "request index was preflighted");
        let previous_request = self
            .sequence_index
            .insert(request.sequence_id, request.request_id);
        debug_assert!(previous_request.is_none(), "sequence index was preflighted");
        self.active_requests = admission.transition.active_requests;
        self.reserved_footprint = admission.committed_next_reserved;
        self.generation_workspaces = admission.transition.generation_workspaces;
        self.reserved_generation_workspace = admission.transition.reserved_generation_workspace;

        RequestStartReceipt {
            request_id: request.request_id,
            sequence_id: request.sequence_id,
            logits_capacity: admission.logits_capacity,
            reserved_footprint: admission.committed_footprint,
        }
    }
}

fn apply_sequence_to_slot<M>(
    slot: &mut ModelSlot<M>,
    admission: &SequenceAdmission,
) -> Result<SequenceSlotDisposition, RuntimeError>
where
    M: LoadedModel,
{
    let request = admission.request;
    let mut sequence = slot
        .model
        .create_sequence(request.sequence_id, &request.configuration)?;
    let rejection = if sequence.id() != request.sequence_id
        || sequence.token_capacity() != admission.expected_token_capacity
    {
        Some(RuntimeError::BackendContractViolation)
    } else {
        slot.lifecycle
            .start_request()
            .err()
            .map(RuntimeError::Lifecycle)
    };

    if let Some(primary) = rejection {
        let Err(cleanup) = slot.model.destroy_sequence(&mut sequence) else {
            return Err(primary);
        };
        let report = CleanupFailureReport::with_details(
            RuntimeOperation::SequenceAdmission,
            primary.failure_detail(),
            RuntimeOperation::SequenceDestruction,
            FailureDetail::Sequence(cleanup),
        );
        let replaced = slot.pending_sequences.insert(
            request.request_id,
            PendingSequence {
                request_id: request.request_id,
                sequence_id: request.sequence_id,
                sequence,
                footprint: admission.backend_footprint,
                failure: report,
                attempts: 1,
            },
        );
        debug_assert!(replaced.is_none(), "pending request index was preflighted");
        slot.reserved_footprint = admission.backend_next_slot_reserved;
        slot.poisoned = true;
        return Ok(SequenceSlotDisposition::Retained(report));
    }

    let replaced = slot.requests.insert(
        request.request_id,
        RequestSlot {
            sequence_id: request.sequence_id,
            token_capacity: admission.expected_token_capacity,
            sequence,
            backend_footprint: admission.backend_footprint,
            workspace_footprint: request.workspace_footprint,
            usage: GenerationUsage::default(),
        },
    );
    debug_assert!(replaced.is_none(), "request admission was preflighted");
    slot.reserved_footprint = admission.committed_next_slot_reserved;
    Ok(SequenceSlotDisposition::Committed)
}

fn admit_preparation(
    plan: LoadPlan,
    preflight: LoadPreflight,
    memory_budget: domain_contracts::MemoryBudget,
) -> Result<PreparedAdmission, RuntimeError> {
    validate_load_plan(&plan, preflight.configuration)?;
    let loading_reserved = admit_footprint(
        preflight.previous_reserved,
        plan.loading_peak_footprint,
        memory_budget,
    )?;
    let final_reserved = admit_footprint(
        preflight.previous_reserved,
        plan.final_footprint,
        memory_budget,
    )?;
    Ok(PreparedAdmission {
        plan,
        loading_reserved,
        final_reserved,
    })
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
        .final_footprint
        .checked_host_bytes()
        .ok_or(RuntimeError::BackendContractViolation)?;
    let final_device = plan
        .final_footprint
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
    let final_footprint = plan.final_footprint;
    let loading_footprint = plan.loading_peak_footprint;
    if loading_host < final_host
        || loading_device < final_device
        || !loading_footprint.contains_components(final_footprint)
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
