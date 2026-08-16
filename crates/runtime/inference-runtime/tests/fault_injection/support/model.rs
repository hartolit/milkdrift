use super::*;

pub(crate) struct FaultModel {
    pub(crate) handle: ModelHandle,
    pub(crate) execution_device: ExecutionDevice,
    pub(crate) execution_scalar_type: ScalarType,
    pub(crate) descriptor: ModelDescriptor,
    pub(crate) reported_footprint: MemoryFootprint,
    pub(crate) remaining_model_cleanup_failures: u32,
    pub(crate) faults: Faults,
    pub(crate) counts: Rc<CleanupCounts>,
    pub(crate) released: bool,
}

impl LoadedModel for FaultModel {
    type Sequence = FaultSequence;

    fn handle(&self) -> ModelHandle {
        self.handle
    }

    fn descriptor(&self) -> &ModelDescriptor {
        &self.descriptor
    }

    fn execution_device(&self) -> ExecutionDevice {
        self.execution_device
    }

    fn execution_scalar_type(&self) -> ScalarType {
        self.execution_scalar_type
    }

    fn reported_footprint(&self) -> MemoryFootprint {
        self.reported_footprint
    }

    fn plan_sequence(
        &self,
        configuration: &SequenceConfiguration,
    ) -> Result<SequencePlan, ModelError> {
        let accepted = if self.faults.contains(Faults::CONTRADICTORY_SEQUENCE_PLAN) {
            SequenceConfiguration::new(
                NonZeroU32::new(17).unwrap_or(NonZeroU32::MIN),
                configuration.maximum_prefill_batch,
            )
        } else {
            *configuration
        };
        let reservation = sequence_reservation();
        Ok(SequencePlan {
            configuration: accepted,
            reservation,
            logits_capacity: self.descriptor.metadata.vocabulary_size as usize,
        })
    }

    fn create_sequence(
        &mut self,
        sequence_id: SequenceId,
        configuration: &SequenceConfiguration,
    ) -> Result<Self::Sequence, ModelError> {
        let mut plan = self.plan_sequence(configuration)?;
        if self.faults.contains(Faults::UNDERREPORTED_SEQUENCE_REPORT) {
            plan.reservation = sequence_report_reservation(4, 0);
        }
        if self.faults.contains(Faults::OVERREPORTED_SEQUENCE_REPORT) {
            plan.reservation = sequence_report_reservation(16, 0);
        }
        if self.faults.contains(Faults::RECLASSIFIED_SEQUENCE_REPORT) {
            plan.reservation = sequence_report_reservation(0, 8);
        }
        self.counts
            .sequence_creations
            .set(self.counts.sequence_creations.get().saturating_add(1));
        let id = if self.faults.contains(Faults::WRONG_SEQUENCE_ID) {
            SequenceId::new(999)
        } else {
            sequence_id
        };
        let token_capacity = if self.faults.contains(Faults::WRONG_SEQUENCE_CAPACITY) {
            1
        } else {
            usize::try_from(configuration.maximum_tokens.get())
                .map_err(|_| ModelError::Backend(backend_failure(1)))?
        };
        Ok(FaultSequence {
            id,
            state: if self.faults.contains(Faults::WRONG_INITIAL_SEQUENCE_STATE) {
                SequenceState::Ready
            } else {
                SequenceState::Empty
            },
            position: usize::from(
                self.faults
                    .contains(Faults::WRONG_INITIAL_SEQUENCE_POSITION),
            ),
            token_capacity,
            plan,
            faults: self.faults,
        })
    }

    fn prefill_buffer_requirements(
        &self,
        _sequence: &Self::Sequence,
        _input: &PrefillInput<'_>,
    ) -> PrefillBufferRequirements {
        PrefillBufferRequirements { logits: 0 }
    }

    fn decode_buffer_requirements(
        &self,
        _sequence: &Self::Sequence,
        _input: DecodeInput,
    ) -> DecodeBufferRequirements {
        DecodeBufferRequirements { logits: 0 }
    }

    fn prefill_prepared(
        &mut self,
        sequence: &mut Self::Sequence,
        input: PrefillInput<'_>,
        _buffers: PreparedPrefillBuffers<'_>,
    ) -> Result<PrefillOutcome, SequenceError> {
        sequence.position = sequence
            .position
            .checked_add(input.tokens.len())
            .ok_or(SequenceError::InvalidState)?;
        sequence.state = SequenceState::Ready;
        if sequence
            .faults
            .contains(Faults::MUTATE_SEQUENCE_REPORT_AFTER_PREFILL)
        {
            sequence.plan.reservation = sequence_report_reservation(16, 0);
        }
        Ok(PrefillOutcome::Ready {
            consumed_tokens: input.tokens.len(),
            position: sequence.position,
            logits_written: 0,
        })
    }

    fn decode_prepared(
        &mut self,
        sequence: &mut Self::Sequence,
        _input: DecodeInput,
        _buffers: PreparedDecodeBuffers<'_>,
    ) -> Result<DecodeOutcome, SequenceError> {
        sequence.position = sequence
            .position
            .checked_add(1)
            .ok_or(SequenceError::InvalidState)?;
        sequence.state = SequenceState::Ready;
        Ok(DecodeOutcome::Ready {
            position: sequence.position,
            logits_written: 0,
        })
    }

    fn destroy_sequence(&mut self, sequence: &mut Self::Sequence) -> Result<(), SequenceError> {
        self.counts
            .sequence_destructions
            .set(self.counts.sequence_destructions.get().saturating_add(1));
        if self
            .faults
            .contains(Faults::MUTATE_SEQUENCE_REPORT_ON_CLEANUP_FAILURE)
        {
            sequence.plan.reservation = sequence_report_reservation(16, 0);
        }
        if self
            .faults
            .contains(Faults::MUTATE_SEQUENCE_ID_ON_CLEANUP_FAILURE)
        {
            sequence.id = SequenceId::new(999);
        }
        if self
            .faults
            .contains(Faults::MUTATE_SEQUENCE_CAPACITY_ON_CLEANUP_FAILURE)
        {
            sequence.token_capacity = 1;
        }
        if self.faults.contains(Faults::FAIL_SEQUENCE_DESTRUCTION) {
            return Err(SequenceError::Backend(backend_failure(2)));
        }
        sequence.state = SequenceState::Finished;
        Ok(())
    }

    fn reset_sequence(&mut self, sequence: &mut Self::Sequence) -> Result<(), SequenceError> {
        sequence.state = SequenceState::Empty;
        Ok(())
    }

    fn synchronize(&mut self) -> Result<(), SynchronizationError> {
        Ok(())
    }

    fn prepare_unload(&mut self) -> Result<(), SynchronizationError> {
        self.counts
            .model_cleanups
            .set(self.counts.model_cleanups.get().saturating_add(1));
        if self.faults.contains(Faults::FAIL_MODEL_CLEANUP)
            || self.remaining_model_cleanup_failures > 0
        {
            self.remaining_model_cleanup_failures =
                self.remaining_model_cleanup_failures.saturating_sub(1);
            if self
                .faults
                .contains(Faults::MUTATE_MODEL_REPORT_ON_CLEANUP_FAILURE)
            {
                let bytes = self
                    .reported_footprint
                    .device_working_bytes()
                    .as_u64()
                    .saturating_add(11);
                self.reported_footprint = self
                    .reported_footprint
                    .with_device_working_bytes(ByteCount::from_u64(bytes));
            }
            Err(SynchronizationError::Backend(backend_failure(3)))
        } else if self.released {
            Err(SynchronizationError::InvalidState)
        } else {
            self.released = true;
            self.counts.successful_model_cleanups.set(
                self.counts
                    .successful_model_cleanups
                    .get()
                    .saturating_add(1),
            );
            Ok(())
        }
    }
}

impl Drop for FaultModel {
    fn drop(&mut self) {
        if !self.released {
            self.counts
                .model_drops_while_owned
                .set(self.counts.model_drops_while_owned.get().saturating_add(1));
        }
    }
}
