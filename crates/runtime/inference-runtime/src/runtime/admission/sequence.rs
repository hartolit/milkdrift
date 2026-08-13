use domain_contracts::{
    BackendSequence, CapacityExhausted, CapacityResource, GenerationUsage, LoadedModel,
    MemoryFootprint, ModelDescriptor, ModelError, ModelHandle, ModelLifecycle, ModelLifecycleState,
    ModelLoader, RequestId, SequenceConfiguration, SequenceId, SequencePlan,
};

use crate::{
    CleanupFailureReport, CleanupResource, CleanupRetryState, FailureDetail, RequestStartReceipt,
    RetainedOwnership, RuntimeError, RuntimeOperation,
};

use super::super::{
    InferenceRuntime, ModelSlot, PendingSequence, RequestSlot,
    cleanup::cleanup_retention_error,
    memory::{admit_footprint, checked_add_footprint, conservative_footprint, saturating_u64},
};

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
    plan: SequencePlan,
    expected_token_capacity: usize,
    backend_footprint: MemoryFootprint,
    committed_footprint: MemoryFootprint,
    backend_next_reserved: MemoryFootprint,
    committed_next_reserved: MemoryFootprint,
    backend_next_slot_reserved: MemoryFootprint,
    committed_next_slot_reserved: MemoryFootprint,
}

pub(crate) struct SequenceAdmissionTransaction<'runtime, L>
where
    L: ModelLoader,
{
    runtime: &'runtime mut InferenceRuntime<L>,
    admission: SequenceAdmission,
    sequence: Option<<L::Model as LoadedModel>::Sequence>,
    lifecycle: ModelLifecycle,
}

enum SequencePreparation<S>
where
    S: BackendSequence,
{
    Ready {
        sequence: S,
        lifecycle: ModelLifecycle,
    },
    Retained {
        report: CleanupFailureReport,
        ownership: RetainedOwnership,
    },
    Rejected(RuntimeError),
}

impl<L> SequenceAdmissionTransaction<'_, L>
where
    L: ModelLoader,
{
    pub(crate) fn commit(mut self) -> RequestStartReceipt {
        let sequence = self
            .sequence
            .take()
            .unwrap_or_else(|| unreachable!("an uncommitted sequence has one owner"));
        self.runtime
            .commit_sequence_admission(self.admission, sequence, self.lifecycle)
    }

    pub(crate) fn rollback(mut self, primary: RuntimeError) -> RuntimeError {
        let sequence = self
            .sequence
            .take()
            .unwrap_or_else(|| unreachable!("an uncommitted sequence has one owner"));
        self.runtime
            .rollback_uncommitted_sequence(self.admission, sequence, primary)
    }
}

impl<L> Drop for SequenceAdmissionTransaction<'_, L>
where
    L: ModelLoader,
{
    fn drop(&mut self) {
        let Some(sequence) = self.sequence.take() else {
            return;
        };
        let _ = self.runtime.rollback_uncommitted_sequence(
            self.admission,
            sequence,
            RuntimeError::BackendContractViolation,
        );
    }
}

impl<L> InferenceRuntime<L>
where
    L: ModelLoader,
{
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
        if validate_sequence_plan(&slot.descriptor, &plan, configuration).is_err()
            || plan.logits_capacity != expected_logits_capacity
        {
            return Err(RuntimeError::BackendContractViolation);
        }
        let committed_footprint =
            checked_add_footprint(plan.reservation.total_footprint, workspace_footprint)?;
        admit_footprint(
            self.reserved_footprint,
            committed_footprint,
            self.limits.memory_budget,
        )?;
        Ok(())
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
        let transaction = self.prepare_sequence_transaction(request)?;
        Ok(transaction.commit())
    }

    pub(crate) fn prepare_generation_request(
        &mut self,
        handle: ModelHandle,
        request_id: RequestId,
        sequence_id: SequenceId,
        configuration: SequenceConfiguration,
        workspace_footprint: MemoryFootprint,
        expected_logits_capacity: usize,
    ) -> Result<SequenceAdmissionTransaction<'_, L>, RuntimeError> {
        self.prepare_sequence_transaction(SequenceAdmissionRequest {
            handle,
            request_id,
            sequence_id,
            configuration,
            workspace_footprint,
            expected_logits_capacity: Some(expected_logits_capacity),
        })
    }

    fn prepare_sequence_transaction(
        &mut self,
        request: SequenceAdmissionRequest,
    ) -> Result<SequenceAdmissionTransaction<'_, L>, RuntimeError> {
        let admission = self.prepare_sequence_admission(request)?;
        let preparation = {
            let slot = self.exact_model_mut(request.handle)?;
            prepare_sequence_in_slot(slot, admission)?
        };
        match preparation {
            SequencePreparation::Ready {
                sequence,
                lifecycle,
            } => Ok(SequenceAdmissionTransaction {
                runtime: self,
                admission,
                sequence: Some(sequence),
                lifecycle,
            }),
            SequencePreparation::Retained { report, ownership } => {
                Err(self.commit_retained_sequence(&admission, report, ownership))
            }
            SequencePreparation::Rejected(error) => Err(error),
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
        if validate_sequence_plan(&slot.descriptor, &plan, request.configuration).is_err()
            || request
                .expected_logits_capacity
                .is_some_and(|expected| plan.logits_capacity != expected)
        {
            return Err(RuntimeError::BackendContractViolation);
        }
        let expected_token_capacity = usize::try_from(plan.configuration.maximum_tokens.get())
            .map_err(|_| RuntimeError::BackendContractViolation)?;
        let backend_footprint = plan.reservation.total_footprint;
        let committed_footprint =
            checked_add_footprint(backend_footprint, request.workspace_footprint)?;
        Ok(SequenceAdmission {
            request,
            transition,
            plan,
            expected_token_capacity,
            backend_footprint,
            committed_footprint,
            backend_next_reserved: admit_footprint(
                current_reserved,
                backend_footprint,
                self.limits.memory_budget,
            )?,
            committed_next_reserved: admit_footprint(
                current_reserved,
                committed_footprint,
                self.limits.memory_budget,
            )?,
            backend_next_slot_reserved: checked_add_footprint(
                slot.reserved_footprint,
                backend_footprint,
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
        ownership: RetainedOwnership,
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
        if ownership.exact_footprint().is_some() {
            self.reserved_footprint = admission.backend_next_reserved;
        }
        let state = CleanupRetryState {
            resource: CleanupResource::Sequence {
                handle: request.handle,
                request_id: request.request_id,
                sequence_id: request.sequence_id,
            },
            failure: report,
            ownership,
            attempts: 1,
            maximum_attempts: self.maximum_cleanup_attempts(),
        };
        self.last_cleanup = Some(state);
        cleanup_retention_error(state)
    }

    fn commit_sequence_admission(
        &mut self,
        admission: SequenceAdmission,
        sequence: <L::Model as LoadedModel>::Sequence,
        lifecycle: ModelLifecycle,
    ) -> RequestStartReceipt {
        let request = admission.request;
        let slot = self.models.get_mut(&request.handle.id).unwrap_or_else(|| {
            unreachable!("an exclusively owned sequence transaction retains its model")
        });
        debug_assert_eq!(slot.handle, request.handle);
        debug_assert!(!slot.requests.contains_key(&request.request_id));
        let replaced = slot.requests.insert(
            request.request_id,
            RequestSlot {
                sequence_id: request.sequence_id,
                token_capacity: admission.expected_token_capacity,
                sequence,
                accepted_plan: admission.plan,
                workspace_footprint: request.workspace_footprint,
                usage: GenerationUsage::default(),
            },
        );
        debug_assert!(replaced.is_none(), "request admission was preflighted");
        slot.lifecycle = lifecycle;
        slot.reserved_footprint = admission.committed_next_slot_reserved;

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
            logits_capacity: admission.plan.logits_capacity,
            reserved_footprint: admission.committed_footprint,
        }
    }

    fn rollback_uncommitted_sequence(
        &mut self,
        admission: SequenceAdmission,
        sequence: <L::Model as LoadedModel>::Sequence,
        primary: RuntimeError,
    ) -> RuntimeError {
        let preparation = {
            let slot = self
                .models
                .get_mut(&admission.request.handle.id)
                .unwrap_or_else(|| {
                    unreachable!("an uncommitted sequence transaction retains its model")
                });
            rollback_sequence_in_slot(slot, admission, sequence, admission.plan, false, primary)
        };
        match preparation {
            SequencePreparation::Retained { report, ownership } => {
                self.commit_retained_sequence(&admission, report, ownership)
            }
            SequencePreparation::Rejected(error) => error,
            SequencePreparation::Ready { .. } => {
                unreachable!("rollback never produces a prepared transaction")
            }
        }
    }
}

fn prepare_sequence_in_slot<M>(
    slot: &mut ModelSlot<M>,
    admission: SequenceAdmission,
) -> Result<SequencePreparation<M::Sequence>, RuntimeError>
where
    M: LoadedModel,
{
    let request = admission.request;
    let sequence = slot
        .model
        .create_sequence(request.sequence_id, &request.configuration)?;
    let reported_plan = sequence.reported_plan();
    let backend_contradiction = sequence.id() != request.sequence_id
        || sequence.token_capacity() != admission.expected_token_capacity
        || reported_plan != admission.plan;
    let mut lifecycle = slot.lifecycle;
    let rejection = if backend_contradiction {
        Some(RuntimeError::BackendContractViolation)
    } else {
        lifecycle.start_request().err().map(RuntimeError::Lifecycle)
    };

    match rejection {
        Some(primary) => Ok(rollback_sequence_in_slot(
            slot,
            admission,
            sequence,
            reported_plan,
            backend_contradiction,
            primary,
        )),
        None => Ok(SequencePreparation::Ready {
            sequence,
            lifecycle,
        }),
    }
}

fn rollback_sequence_in_slot<M>(
    slot: &mut ModelSlot<M>,
    admission: SequenceAdmission,
    mut sequence: M::Sequence,
    reported_plan: SequencePlan,
    backend_contradiction: bool,
    primary: RuntimeError,
) -> SequencePreparation<M::Sequence>
where
    M: LoadedModel,
{
    let cleanup = match slot.model.destroy_sequence(&mut sequence) {
        Ok(()) => return SequencePreparation::Rejected(primary),
        Err(cleanup) => cleanup,
    };
    let report = CleanupFailureReport::with_details(
        RuntimeOperation::SequenceAdmission,
        primary.failure_detail(),
        RuntimeOperation::SequenceDestruction,
        FailureDetail::Sequence(cleanup),
    );
    // Identity, capacity, or plan contradiction makes physical extent
    // unverified. A runtime lifecycle rejection with a conforming report
    // still retains the already-admitted exact backend reservation.
    let ownership = if backend_contradiction {
        RetainedOwnership::Unverified {
            accepted_footprint: admission.backend_footprint,
            reported_footprint: reported_plan.reservation.total_footprint,
            conservative_footprint: conservative_footprint(
                admission.backend_footprint,
                reported_plan.reservation.total_footprint,
            ),
        }
    } else {
        RetainedOwnership::Exact(admission.backend_footprint)
    };
    let request = admission.request;
    let replaced = slot.pending_sequences.insert(
        request.request_id,
        PendingSequence {
            request_id: request.request_id,
            sequence_id: request.sequence_id,
            sequence,
            accepted_plan: admission.plan,
            ownership,
            failure: report,
            attempts: 1,
        },
    );
    debug_assert!(replaced.is_none(), "pending request index was preflighted");
    if ownership.exact_footprint().is_some() {
        slot.reserved_footprint = admission.backend_next_slot_reserved;
    }
    slot.poisoned = true;
    SequencePreparation::Retained { report, ownership }
}

fn validate_sequence_plan(
    descriptor: &ModelDescriptor,
    plan: &SequencePlan,
    requested: SequenceConfiguration,
) -> Result<(), RuntimeError> {
    if plan.configuration != requested
        || !sequence_configuration_is_supported(descriptor, plan.configuration)
        || !plan.reservation.is_consistent()
        || plan
            .reservation
            .persistent_footprint
            .checked_host_bytes()
            .is_none()
        || plan
            .reservation
            .persistent_footprint
            .checked_device_bytes()
            .is_none()
        || plan
            .reservation
            .transient_footprint
            .checked_host_bytes()
            .is_none()
        || plan
            .reservation
            .transient_footprint
            .checked_device_bytes()
            .is_none()
        || plan
            .reservation
            .total_footprint
            .checked_host_bytes()
            .is_none()
        || plan
            .reservation
            .total_footprint
            .checked_device_bytes()
            .is_none()
    {
        Err(RuntimeError::BackendContractViolation)
    } else {
        Ok(())
    }
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
