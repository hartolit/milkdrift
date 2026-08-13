//! Hosted-worker command application at the runtime ownership boundary.

use domain_contracts::{
    CancellationReason, ExecutionDevice, FinishReason, ModelHandle, ModelId, ModelLoader,
    RequestId, SequenceConfiguration, SequenceId, TokenId, UnloadPolicy,
};

use crate::{CommandTicket, GenerationRequest, RuntimeCommand, RuntimeError, RuntimeEvent};

use super::{CommandOutcome, WorkerState, WorkerStop, shutdown_runtime};

impl CommandOutcome {
    fn continuing(event: RuntimeEvent) -> Self {
        Self { event, stop: None }
    }

    fn stopping(event: RuntimeEvent, stop: WorkerStop) -> Self {
        Self {
            event,
            stop: Some(stop),
        }
    }
}

impl<L> WorkerState<'_, L>
where
    L: ModelLoader,
{
    pub(super) fn dispatch(&mut self, command: RuntimeCommand<L::Source>) -> CommandOutcome {
        match command {
            RuntimeCommand::LoadModel {
                ticket,
                model_id,
                source,
                execution_device,
            } => self.load_model(ticket, model_id, &source, execution_device),
            RuntimeCommand::StartRequest {
                ticket,
                handle,
                request_id,
                sequence_id,
                configuration,
            } => self.start_request(ticket, handle, request_id, sequence_id, configuration),
            RuntimeCommand::Generate {
                ticket,
                handle,
                request,
            } => self.admit_generation(ticket, handle, request),
            RuntimeCommand::Prefill {
                ticket,
                request_id,
                tokens,
                emit_logits,
                logits,
            } => self.prefill(ticket, request_id, &tokens, emit_logits, logits),
            RuntimeCommand::Decode {
                ticket,
                request_id,
                token,
                logits,
            } => self.decode(ticket, request_id, token, logits),
            RuntimeCommand::CompleteRequest {
                ticket,
                request_id,
                reason,
            } => self.complete_request(ticket, request_id, reason),
            RuntimeCommand::CancelRequest {
                ticket,
                request_id,
                reason,
            } => self.cancel_request(ticket, request_id, reason),
            RuntimeCommand::UnloadModel {
                ticket,
                handle,
                policy,
            } => self.unload_model(ticket, handle, policy),
            RuntimeCommand::Snapshot { ticket } => self.snapshot(ticket),
            RuntimeCommand::Shutdown { ticket } => self.shutdown(ticket),
        }
    }

    fn load_model(
        &mut self,
        ticket: CommandTicket,
        model_id: ModelId,
        source: &L::Source,
        execution_device: ExecutionDevice,
    ) -> CommandOutcome {
        CommandOutcome::continuing(RuntimeEvent::ModelLoaded {
            ticket,
            result: self.runtime.load_model(model_id, source, execution_device),
        })
    }

    fn start_request(
        &mut self,
        ticket: CommandTicket,
        handle: ModelHandle,
        request_id: RequestId,
        sequence_id: SequenceId,
        configuration: SequenceConfiguration,
    ) -> CommandOutcome {
        let result = if self.scheduler.contains(request_id) {
            Err(RuntimeError::RequestAlreadyActive(request_id))
        } else {
            self.runtime
                .start_request(handle, request_id, sequence_id, configuration)
        };
        CommandOutcome::continuing(RuntimeEvent::RequestStarted { ticket, result })
    }

    fn admit_generation(
        &mut self,
        ticket: CommandTicket,
        handle: ModelHandle,
        request: GenerationRequest,
    ) -> CommandOutcome {
        CommandOutcome::continuing(RuntimeEvent::GenerationAdmitted {
            ticket,
            result: self
                .scheduler
                .admit(&mut self.runtime, self.token_output, handle, request),
        })
    }

    fn prefill(
        &mut self,
        ticket: CommandTicket,
        request_id: RequestId,
        tokens: &[TokenId],
        emit_logits: bool,
        mut logits: Vec<f32>,
    ) -> CommandOutcome {
        let result = self
            .runtime
            .prefill(request_id, tokens, emit_logits, logits.as_mut_slice());
        CommandOutcome::continuing(RuntimeEvent::PrefillCompleted {
            ticket,
            request_id,
            result,
            logits,
        })
    }

    fn decode(
        &mut self,
        ticket: CommandTicket,
        request_id: RequestId,
        token: TokenId,
        mut logits: Vec<f32>,
    ) -> CommandOutcome {
        let result = self
            .runtime
            .decode(request_id, token, logits.as_mut_slice());
        CommandOutcome::continuing(RuntimeEvent::DecodeCompleted {
            ticket,
            request_id,
            result,
            logits,
        })
    }

    fn complete_request(
        &mut self,
        ticket: CommandTicket,
        request_id: RequestId,
        reason: FinishReason,
    ) -> CommandOutcome {
        CommandOutcome::continuing(RuntimeEvent::RequestFinished {
            ticket,
            request_id,
            result: self.runtime.complete_request(request_id, reason),
        })
    }

    fn cancel_request(
        &mut self,
        ticket: CommandTicket,
        request_id: RequestId,
        reason: CancellationReason,
    ) -> CommandOutcome {
        if self.scheduler.contains(request_id) {
            CommandOutcome::continuing(RuntimeEvent::GenerationCancellationRequested {
                ticket,
                request_id,
                result: self.scheduler.request_cancellation(request_id, reason),
            })
        } else {
            CommandOutcome::continuing(RuntimeEvent::RequestFinished {
                ticket,
                request_id,
                result: self.runtime.cancel_request(request_id, reason),
            })
        }
    }

    fn unload_model(
        &mut self,
        ticket: CommandTicket,
        handle: ModelHandle,
        policy: UnloadPolicy,
    ) -> CommandOutcome {
        if matches!(policy, UnloadPolicy::CancelActive) {
            self.scheduler
                .request_model_cancellation(handle.id, CancellationReason::ModelUnload);
        }
        CommandOutcome::continuing(RuntimeEvent::ModelUnload {
            ticket,
            result: self.runtime.unload_model(handle, policy, self.clock.now()),
        })
    }

    fn snapshot(&self, ticket: CommandTicket) -> CommandOutcome {
        CommandOutcome::continuing(RuntimeEvent::Snapshot {
            ticket,
            runtime: self.runtime.snapshot(),
            models: self.runtime.model_snapshots(),
            retained_models: self.runtime.retained_model_snapshots(),
        })
    }

    fn shutdown(&mut self, ticket: CommandTicket) -> CommandOutcome {
        let result = shutdown_runtime(&mut self.runtime, &mut self.scheduler);
        let stop = if result.is_err() && self.runtime.owns_backend_resources() {
            WorkerStop::RetainUntilProcessExit
        } else {
            WorkerStop::DropRuntime
        };
        CommandOutcome::stopping(RuntimeEvent::Shutdown { ticket, result }, stop)
    }
}
