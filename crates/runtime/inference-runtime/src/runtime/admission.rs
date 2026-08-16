use std::collections::BTreeMap;

use domain_contracts::{
    CapabilitySet, ExecutionDevice, FailedLoad, LoadConfiguration, LoadPlan, LoadedModel,
    MemoryFootprint, ModelDescriptor, ModelHandle, ModelId, ModelLifecycle, ModelLoader,
    PreparedLoad, ScalarType,
};

use crate::error::{CleanupRetryProgress, RetainedOwner};
use crate::{
    CleanupFailureReport, CleanupRetryState, FailureDetail, LoadReceipt, RuntimeError,
    RuntimeOperation,
};

use super::{
    InferenceRuntime, ModelSlot, PendingModel, PendingModelOwner,
    cleanup::cleanup_retention_error,
    memory::{admit_footprint, conservative_footprint, remaining_budget, saturating_u32},
};

mod sequence;
pub(crate) use sequence::SequenceAdmissionTransaction;

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
                    ownership: RetainedOwner::Exact(admission.plan.final_footprint),
                    failure: report,
                    retry: CleanupRetryProgress::initial(self.maximum_cleanup_attempts()),
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
                ownership: RetainedOwner::Exact(plan.loading_peak_footprint),
                failure: report,
                retry: CleanupRetryProgress::initial(self.maximum_cleanup_attempts()),
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
            cleanup_retention_error(self.commit_pending_model(model_id, pending, exact_reserved))
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
            let ownership = RetainedOwner::Unverified {
                accepted_footprint: plan.loading_peak_footprint,
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
                retry: CleanupRetryProgress::initial(self.maximum_cleanup_attempts()),
                cancelled_requests: 0,
            };
            cleanup_retention_error(self.commit_pending_model(
                model_id,
                pending,
                preflight.previous_reserved,
            ))
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
        let state = pending.cleanup_state();
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
