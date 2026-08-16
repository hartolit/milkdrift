//! Runtime accessors, event routing, tickets, submission, and shutdown forwarding.

use candle_backend::CandleLlamaSource;
use host_runtime::TryReceiveError;
use inference_runtime::{CommandTicket, RuntimeCommand, RuntimeEvent};

use crate::local::LocalSubmitError;
use crate::{
    ApplicationActivity, ApplicationError, ApplicationEvent, ApplicationPreferences,
    ApplicationRuntime, ApplicationState,
};

impl ApplicationRuntime {
    /// Returns persisted settings or the configured defaults used for this process.
    #[must_use]
    pub const fn preferences(&self) -> &ApplicationPreferences {
        &self.preferences
    }

    /// Returns the current frontend-neutral state.
    #[must_use]
    pub const fn state(&self) -> &ApplicationState {
        &self.state
    }

    /// Processes at most one pending Hub or inference event without blocking.
    #[must_use]
    pub fn poll_event(&mut self) -> Option<ApplicationEvent> {
        if let Some(event) = self.take_generation_event() {
            return Some(event);
        }

        if self.state.hub_available() {
            match self.hub_results.try_receive() {
                Ok(event) => return Some(self.process_hub_event(event)),
                Err(TryReceiveError::Empty) => {}
                Err(TryReceiveError::Disconnected) => {
                    self.state.disconnect_hub();
                    if self.state.activity() == ApplicationActivity::Resolving
                        && let Err(error) = self.state.fail_resolution()
                    {
                        return Some(ApplicationEvent::ModelResolutionFailed {
                            failure: crate::ApplicationFailure::from_debug(
                                crate::ApplicationFailureKind::ArtifactResolution,
                                "Hub disconnect resolution transition was rejected",
                                error,
                            ),
                        });
                    }
                    return Some(ApplicationEvent::HubDisconnected);
                }
            }
        }

        if self.state.inference_available() {
            match self.local.try_receive() {
                Ok(event) => return self.process_runtime_event(&event),
                Err(inference_runtime::RuntimeReceiveError::Timeout) => {}
                Err(inference_runtime::RuntimeReceiveError::Disconnected) => {
                    self.shutdown_control.record_inference_disconnect();
                    self.state.disconnect_inference();
                    self.mark_model_worker_disconnected();
                    self.handle_generation_runtime_disconnected();
                    return Some(ApplicationEvent::RuntimeDisconnected);
                }
            }
        }
        if let Some(event) = self.progress_model_cleanup_coordination() {
            return Some(event);
        }
        self.pump_generation_event()
    }

    /// Cooperatively shuts down all workers and waits only to configured hard deadlines.
    ///
    /// # Errors
    ///
    /// Returns a retained terminal inference shutdown failure when present; otherwise returns the
    /// first worker command, timeout, or join failure encountered.
    pub fn shutdown(&mut self) -> Result<(), ApplicationError> {
        crate::shutdown::shutdown(self)
    }

    pub(crate) fn next_ticket(&mut self) -> Result<CommandTicket, ApplicationError> {
        let ticket = CommandTicket::new(self.next_ticket);
        self.next_ticket = self
            .next_ticket
            .checked_add(1)
            .ok_or(ApplicationError::TicketExhausted)?;
        Ok(ticket)
    }

    pub(crate) fn require_idle(&self) -> Result<(), ApplicationError> {
        let activity = self.state.activity();
        if activity == ApplicationActivity::Idle {
            Ok(())
        } else {
            Err(ApplicationError::Busy(activity))
        }
    }

    pub(crate) fn submit_inference(
        &mut self,
        command: RuntimeCommand<CandleLlamaSource>,
    ) -> Result<(), ApplicationError> {
        #[cfg(test)]
        let submitted_load_device = match &command {
            RuntimeCommand::LoadModel {
                execution_device, ..
            } => Some(*execution_device),
            _ => None,
        };
        #[cfg(test)]
        if self.forced_inference_busy_submissions > 0 {
            self.forced_inference_busy_submissions -= 1;
            return Err(ApplicationError::RuntimeBusy);
        }
        #[cfg(test)]
        if self.forced_unsent_command_disconnects > 0 {
            self.forced_unsent_command_disconnects -= 1;
            return Err(ApplicationError::RuntimeDisconnected);
        }

        match self.local.submit(command) {
            Ok(()) => {
                #[cfg(test)]
                if let Some(device) = submitted_load_device {
                    self.last_submitted_load_device = Some(device);
                }
                Ok(())
            }
            Err(LocalSubmitError::Full) => Err(ApplicationError::RuntimeBusy),
            Err(LocalSubmitError::Disconnected) => {
                self.shutdown_control.record_inference_disconnect();
                self.state.disconnect_inference();
                self.mark_model_worker_disconnected();
                Err(ApplicationError::RuntimeDisconnected)
            }
        }
    }

    fn process_runtime_event(&mut self, event: &RuntimeEvent) -> Option<ApplicationEvent> {
        match event {
            RuntimeEvent::ModelLoaded { ticket, result } => {
                self.process_model_loaded(*ticket, result)
            }
            RuntimeEvent::ModelUnload { ticket, result } => {
                self.process_model_unload(*ticket, result)
            }
            RuntimeEvent::GenerationAdmitted { .. }
            | RuntimeEvent::GenerationCancellationRequested { .. } => {
                self.process_generation_runtime_event(event)
            }
            RuntimeEvent::Snapshot {
                ticket,
                runtime,
                retained_models,
                ..
            } => self.process_retained_model_cleanup_snapshot(
                *ticket,
                runtime,
                retained_models.as_slice(),
            ),
            RuntimeEvent::Shutdown { .. }
            | RuntimeEvent::RequestStarted { .. }
            | RuntimeEvent::PrefillCompleted { .. }
            | RuntimeEvent::DecodeCompleted { .. }
            | RuntimeEvent::RequestFinished { .. } => None,
        }
    }
}
