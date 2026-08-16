use super::*;

pub(crate) struct FaultLoader {
    pub(crate) faults: Faults,
    pub(crate) counts: Rc<CleanupCounts>,
}

pub(crate) struct FaultPrepared {
    pub(crate) plan: LoadPlan,
    pub(crate) alternate_plan: LoadPlan,
    pub(crate) source: FaultSource,
    pub(crate) faults: Faults,
    pub(crate) counts: Rc<CleanupCounts>,
    pub(crate) plan_reads: Cell<u32>,
    pub(crate) remaining_cleanup_failures: u32,
    pub(crate) partial_resources_retained: bool,
}

fn reported_fault_plan(prepared: &FaultPrepared) -> &LoadPlan {
    let reads = prepared.plan_reads.get();
    prepared.plan_reads.set(reads.saturating_add(1));
    prepared
        .counts
        .plan_reads
        .set(prepared.counts.plan_reads.get().saturating_add(1));
    if reads % 2 == 1 && prepared.faults.contains(Faults::ALTERNATING_PLAN_REPORT) {
        &prepared.alternate_plan
    } else {
        &prepared.plan
    }
}

impl PreparedLoad for FaultPrepared {
    type Failed = FaultPrepared;

    fn plan(&self) -> &LoadPlan {
        reported_fault_plan(self)
    }
}

impl FailedLoadOwner for FaultPrepared {
    fn plan(&self) -> &LoadPlan {
        reported_fault_plan(self)
    }

    fn cleanup(&mut self) -> Result<(), SynchronizationError> {
        self.counts
            .failed_load_cleanups
            .set(self.counts.failed_load_cleanups.get().saturating_add(1));
        if self.faults.contains(Faults::FAIL_FAILED_LOAD_CLEANUP)
            || self.remaining_cleanup_failures > 0
        {
            self.remaining_cleanup_failures = self.remaining_cleanup_failures.saturating_sub(1);
            if self
                .faults
                .contains(Faults::MUTATE_FAILED_PLAN_ON_CLEANUP_FAILURE)
            {
                let bytes = self
                    .plan
                    .loading_peak_footprint
                    .host_working_bytes()
                    .as_u64()
                    .saturating_add(7);
                self.plan.loading_peak_footprint = self
                    .plan
                    .loading_peak_footprint
                    .with_host_working_bytes(ByteCount::from_u64(bytes));
            }
            return Err(SynchronizationError::Backend(backend_failure(4)));
        }
        if !self.partial_resources_retained {
            return Err(SynchronizationError::InvalidState);
        }
        self.partial_resources_retained = false;
        self.counts.successful_failed_load_cleanups.set(
            self.counts
                .successful_failed_load_cleanups
                .get()
                .saturating_add(1),
        );
        self.counts.retained_partial_load_bytes.set(
            self.counts
                .retained_partial_load_bytes
                .get()
                .saturating_sub(loading_peak_host_bytes()),
        );
        Ok(())
    }
}

impl Drop for FaultPrepared {
    fn drop(&mut self) {
        self.counts
            .prepared_drops
            .set(self.counts.prepared_drops.get().saturating_add(1));
        if self.partial_resources_retained {
            self.counts
                .retained_prepared_drops
                .set(self.counts.retained_prepared_drops.get().saturating_add(1));
        }
    }
}

impl ModelLoader for FaultLoader {
    type Source = FaultSource;
    type Prepared = FaultPrepared;
    type FailedPreparation = FaultPrepared;
    type Model = FaultModel;

    fn inspect(&self, source: &Self::Source) -> Result<ModelDescriptor, LoadError> {
        let faults = self.faults.union(source.faults);
        let mut descriptor = descriptor(source.source_scalar_type);
        if faults.contains(Faults::MISSING_MULTIPLE_SEQUENCES) {
            descriptor.capabilities.operations = CapabilitySet::PREFILL
                .union(CapabilitySet::INCREMENTAL_DECODE)
                .union(CapabilitySet::EXPLICIT_SYNCHRONIZATION);
        }
        if faults.contains(Faults::ZERO_VOCABULARY) {
            descriptor.metadata.vocabulary_size = 0;
        }
        if faults.contains(Faults::ZERO_CONTEXT_LENGTH) {
            descriptor.metadata.context_length = 0;
        }
        if faults.contains(Faults::ZERO_MAXIMUM_CONTEXT) {
            descriptor.capabilities.maximum_context_tokens = 0;
        }
        if faults.contains(Faults::ZERO_MAXIMUM_SEQUENCES) {
            descriptor.capabilities.maximum_sequences = 0;
        }
        if faults.contains(Faults::ZERO_MAXIMUM_PREFILL) {
            descriptor.capabilities.maximum_prefill_batch = 0;
        }
        if faults.contains(Faults::CONTEXT_EXCEEDS_METADATA) {
            descriptor.capabilities.maximum_context_tokens =
                descriptor.metadata.context_length.saturating_add(1);
        }
        if faults.contains(Faults::PREFILL_EXCEEDS_CONTEXT) {
            descriptor.capabilities.maximum_prefill_batch = descriptor
                .capabilities
                .maximum_context_tokens
                .saturating_add(1);
        }
        if faults.contains(Faults::EMPTY_OBSERVED_TENSOR_SET) {
            descriptor.metadata.observed_tensor_scalar_types = ScalarTypeSet::EMPTY;
        }
        Ok(descriptor)
    }

    fn prepare_load(
        &mut self,
        source: &Self::Source,
        configuration: &LoadConfiguration,
    ) -> Result<Self::Prepared, LoadError> {
        self.counts
            .preparations
            .set(self.counts.preparations.get().saturating_add(1));
        let faults = self.faults.union(source.faults);
        let descriptor = self.inspect(source)?;
        let mut accepted_configuration = *configuration;
        if faults.contains(Faults::WRONG_ACCEPTED_CONFIGURATION) {
            accepted_configuration.execution_device.id = DeviceId::new(
                accepted_configuration
                    .execution_device
                    .id
                    .get()
                    .saturating_add(1),
            );
        }
        let mut expected_footprint = descriptor.estimated_footprint;
        if faults.contains(Faults::OVERFLOWING_FINAL_FOOTPRINT) {
            expected_footprint = expected_footprint
                .with_host_weight_bytes(ByteCount::MAX)
                .with_host_working_bytes(ByteCount::from_u64(1));
        }
        let mut loading_peak_footprint = loading_peak_footprint();
        if faults.contains(Faults::OVERFLOWING_LOADING_PEAK) {
            loading_peak_footprint = loading_peak_footprint
                .with_host_weight_bytes(ByteCount::MAX)
                .with_host_working_bytes(ByteCount::from_u64(1));
        }
        if faults.contains(Faults::LOADING_PEAK_BELOW_FINAL) {
            loading_peak_footprint =
                loading_peak_footprint.with_host_working_bytes(ByteCount::ZERO);
        }

        if faults.contains(Faults::RECLASSIFIED_LOADING_PEAK) {
            let reclassified = loading_peak_footprint
                .host_working_bytes()
                .as_u64()
                .saturating_add(loading_peak_footprint.host_weight_bytes().as_u64());
            loading_peak_footprint = loading_peak_footprint
                .with_host_working_bytes(ByteCount::from_u64(reclassified))
                .with_host_weight_bytes(ByteCount::ZERO);
        }
        let plan = LoadPlan {
            accepted_configuration,
            descriptor,
            execution_scalar_type: source.planned_execution_scalar_type,
            final_footprint: expected_footprint,
            loading_peak_footprint,
        };
        let mut alternate_plan = plan;
        let alternate_bytes = alternate_plan
            .loading_peak_footprint
            .host_working_bytes()
            .as_u64()
            .saturating_add(1);
        alternate_plan.loading_peak_footprint = alternate_plan
            .loading_peak_footprint
            .with_host_working_bytes(ByteCount::from_u64(alternate_bytes));
        Ok(FaultPrepared {
            plan,
            alternate_plan,
            source: *source,
            faults,
            counts: Rc::clone(&self.counts),
            plan_reads: Cell::new(0),
            remaining_cleanup_failures: u32::from(
                faults.contains(Faults::FAIL_FAILED_LOAD_CLEANUP_ONCE),
            ),
            partial_resources_retained: false,
        })
    }

    fn load_prepared(
        &mut self,
        mut prepared: Self::Prepared,
    ) -> Result<Self::Model, FailedLoad<Self::FailedPreparation>> {
        self.counts
            .model_loads
            .set(self.counts.model_loads.get().saturating_add(1));
        let faults = prepared.faults;
        if faults.contains(Faults::FAIL_LOAD) {
            prepared.partial_resources_retained = true;
            self.counts.retained_partial_load_bytes.set(
                self.counts
                    .retained_partial_load_bytes
                    .get()
                    .saturating_add(loading_peak_host_bytes()),
            );
            return Err(FailedLoad::new(failed_load_error(), prepared));
        }

        let source = prepared.source;
        let configuration = prepared.plan.accepted_configuration;
        let mut descriptor = prepared.plan.descriptor;
        if faults.contains(Faults::MISMATCHED_METADATA) {
            descriptor.metadata.vocabulary_size =
                descriptor.metadata.vocabulary_size.saturating_add(1);
        }
        if faults.contains(Faults::MISMATCHED_DESCRIPTOR) {
            descriptor.capabilities.maximum_prefill_batch = descriptor
                .capabilities
                .maximum_prefill_batch
                .saturating_add(1);
        }
        let handle = if faults.contains(Faults::WRONG_MODEL_HANDLE) {
            ModelHandle::new(ModelId::new(999), configuration.handle.generation)
        } else {
            configuration.handle
        };
        let mut execution_device = configuration.execution_device;
        if faults.contains(Faults::WRONG_DEVICE_ID) {
            execution_device.id = DeviceId::new(execution_device.id.get().saturating_add(1));
        }
        if faults.contains(Faults::WRONG_DEVICE_KIND) {
            execution_device.kind = DeviceKind::Cuda;
        }
        let execution_scalar_type = if faults.contains(Faults::SOURCE_SCALAR_AS_EXECUTION_SCALAR) {
            descriptor
                .metadata
                .configuration_declared_scalar_type
                .unwrap_or(source.source_scalar_type)
        } else if faults.contains(Faults::UNSUPPORTED_ACTUAL_EXECUTION_SCALAR) {
            ScalarType::Other(u16::MAX)
        } else if faults.contains(Faults::WRONG_EXECUTION_SCALAR) {
            ScalarType::F16
        } else {
            source.planned_execution_scalar_type
        };
        let mut reported_footprint = prepared.plan.final_footprint;
        if faults.contains(Faults::WRONG_MODEL_FOOTPRINT) {
            let bytes = reported_footprint
                .host_working_bytes()
                .as_u64()
                .saturating_add(1);
            reported_footprint =
                reported_footprint.with_host_working_bytes(ByteCount::from_u64(bytes));
        }
        if faults.contains(Faults::REPORTED_LARGER_THAN_PEAK) {
            reported_footprint = reported_footprint
                .with_host_weight_bytes(ByteCount::from_u64(200))
                .with_host_working_bytes(ByteCount::from_u64(100));
        }
        if faults.contains(Faults::REPORTED_RECLASSIFIED_TO_DEVICE) {
            reported_footprint = footprint(0, 100, 0, 10);
        }
        if faults.contains(Faults::REPORTED_OVERFLOWING_HOST) {
            reported_footprint = reported_footprint
                .with_host_weight_bytes(ByteCount::MAX)
                .with_host_working_bytes(ByteCount::from_u64(1));
        }
        if faults.contains(Faults::REPORTED_OVERFLOWING_DEVICE) {
            reported_footprint = reported_footprint
                .with_device_weight_bytes(ByteCount::MAX)
                .with_device_working_bytes(ByteCount::from_u64(1));
        }
        if faults.contains(Faults::REPORTED_SMALLER_THAN_FINAL) {
            reported_footprint = footprint(50, 0, 0, 0);
        }
        Ok(FaultModel {
            handle,
            execution_device,
            execution_scalar_type,
            descriptor,
            reported_footprint,
            remaining_model_cleanup_failures: if faults.contains(Faults::FAIL_MODEL_CLEANUP_TWICE) {
                2
            } else {
                u32::from(faults.contains(Faults::FAIL_MODEL_CLEANUP_ONCE))
            },
            faults,
            counts: Rc::clone(&self.counts),
            released: false,
        })
    }
}
