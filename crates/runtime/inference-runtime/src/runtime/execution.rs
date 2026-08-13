use domain_contracts::{
    BackendSequence, CancellationReason, CancellationStatus, CapabilitySet, DecodeBuffers,
    DecodeInput, DecodeOutcome, FinishReason, ModelLoader, PrefillBuffers, PrefillInput,
    PrefillOutcome, RequestId, SequenceError, SequenceState, TokenId, decode_checked,
    prefill_checked,
};

use crate::{
    DecodeReceipt, FailureClass, FailureDetail, PrefillReceipt, RuntimeError, RuntimeOperation,
};

use super::{InferenceRuntime, memory::saturating_u64};

impl<L> InferenceRuntime<L>
where
    L: ModelLoader,
{
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
            let expected_logits = if emit_logits {
                usize::try_from(slot.descriptor.metadata.vocabulary_size).ok()
            } else {
                Some(0)
            };
            let request = slot
                .requests
                .get_mut(&request_id)
                .ok_or(RuntimeError::RequestNotActive(request_id))?;
            let previous_position = request.sequence.position();
            let outcome = if slot
                .descriptor
                .capabilities
                .operations
                .contains(CapabilitySet::PREFILL)
            {
                prefill_checked(
                    &mut slot.model,
                    &mut request.sequence,
                    PrefillInput::new(tokens, emit_logits),
                    PrefillBuffers::new(logits),
                    CancellationStatus::Running,
                )
            } else {
                Err(SequenceError::Unsupported)
            };
            let current_position = request.sequence.position();
            match outcome {
                Ok(PrefillOutcome::Ready {
                    consumed_tokens,
                    position,
                    logits_written,
                }) => {
                    let expected_position = previous_position.checked_add(tokens.len());
                    if consumed_tokens != tokens.len()
                        || expected_position != Some(current_position)
                        || position != current_position
                        || request.sequence.id() != request.sequence_id
                        || request.sequence.token_capacity() != request.token_capacity
                        || request.sequence.reported_plan() != request.accepted_plan
                        || request.sequence.state() != SequenceState::Ready
                        || expected_logits != Some(logits_written)
                        || logits_written > logits.len()
                    {
                        Err(RuntimeError::BackendContractViolation)
                    } else {
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
                }
                Ok(PrefillOutcome::Finished(reason)) => {
                    Ok((PrefillOutcome::Finished(reason), request.usage))
                }
                Err(error) => Err(RuntimeError::Sequence(error)),
            }
        };

        match operation {
            Ok((outcome, usage)) => {
                if let PrefillOutcome::Finished(reason) = outcome {
                    preserve_primary_cleanup(self.remove_request(
                        request_id,
                        finish_operation(reason),
                        finish_failure_detail(reason),
                    ))?;
                }
                Ok(PrefillReceipt { outcome, usage })
            }
            Err(primary) => {
                preserve_primary_cleanup(self.remove_request(
                    request_id,
                    RuntimeOperation::Prefill,
                    primary.failure_detail(),
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
            let expected_logits = usize::try_from(slot.descriptor.metadata.vocabulary_size).ok();
            let request = slot
                .requests
                .get_mut(&request_id)
                .ok_or(RuntimeError::RequestNotActive(request_id))?;
            let previous_position = request.sequence.position();
            let outcome = if slot
                .descriptor
                .capabilities
                .operations
                .contains(CapabilitySet::INCREMENTAL_DECODE)
            {
                decode_checked(
                    &mut slot.model,
                    &mut request.sequence,
                    DecodeInput::new(token),
                    DecodeBuffers::new(logits),
                    CancellationStatus::Running,
                )
            } else {
                Err(SequenceError::Unsupported)
            };
            let current_position = request.sequence.position();
            match outcome {
                Ok(DecodeOutcome::Ready {
                    position,
                    logits_written,
                }) => {
                    let expected_position = previous_position.checked_add(1);
                    if expected_position != Some(current_position)
                        || position != current_position
                        || request.sequence.id() != request.sequence_id
                        || request.sequence.token_capacity() != request.token_capacity
                        || request.sequence.reported_plan() != request.accepted_plan
                        || request.sequence.state() != SequenceState::Ready
                        || expected_logits != Some(logits_written)
                        || logits_written > logits.len()
                    {
                        Err(RuntimeError::BackendContractViolation)
                    } else {
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
                }
                Ok(DecodeOutcome::Finished(reason)) => {
                    Ok((DecodeOutcome::Finished(reason), request.usage))
                }
                Err(error) => Err(RuntimeError::Sequence(error)),
            }
        };

        match operation {
            Ok((outcome, usage)) => {
                if let DecodeOutcome::Finished(reason) = outcome {
                    preserve_primary_cleanup(self.remove_request(
                        request_id,
                        finish_operation(reason),
                        finish_failure_detail(reason),
                    ))?;
                }
                Ok(DecodeReceipt { outcome, usage })
            }
            Err(primary) => {
                preserve_primary_cleanup(self.remove_request(
                    request_id,
                    RuntimeOperation::Decode,
                    primary.failure_detail(),
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
            finish_failure_detail(reason),
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
            FailureDetail::Class(FailureClass::Cancellation),
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
        self.remove_request(request_id, operation, FailureDetail::Class(failure))
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
const fn finish_operation(reason: FinishReason) -> RuntimeOperation {
    if matches!(reason, FinishReason::Cancelled(_)) {
        RuntimeOperation::Cancellation
    } else {
        RuntimeOperation::Completion
    }
}

const fn finish_failure_detail(reason: FinishReason) -> FailureDetail {
    FailureDetail::Class(match reason {
        FinishReason::Cancelled(_) => FailureClass::Cancellation,
        FinishReason::BufferExhausted(_) => FailureClass::Capacity,
        FinishReason::EndOfSequence(_) | FinishReason::TokenLimit | FinishReason::StopCondition => {
            FailureClass::Completion
        }
        _ => FailureClass::Completion,
    })
}
