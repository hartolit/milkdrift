//! Application-owned model unload behavior translated into E0 lifecycle policy.

use crate::{ApplicationConfigurationField, ApplicationError, ApplicationRuntime};
use domain_contracts::{DrainTimeout, ModelHandle, UnloadPolicy};
use inference_runtime::RuntimeCommand;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ModelUnloadTransaction {
    pub(crate) ticket: inference_runtime::CommandTicket,
    pub(crate) handle: ModelHandle,
}

/// Frontend-neutral behavior used when releasing the resident model.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ModelUnloadBehavior {
    /// Reject the unload request while generation work remains active.
    RejectIfBusy,
    /// Cancel active generation at its next safe E0 boundary, then unload.
    CancelActive,
    /// Allow active generation to finish until the configured drain deadline.
    #[default]
    Drain,
}

impl ApplicationRuntime {
    /// Requests deterministic release of the resident model using explicit application policy.
    ///
    /// `unload_model()` remains the convenience path for [`ModelUnloadBehavior::Drain`].
    /// This method exists so frontends and tests can request reject, safe-boundary cancel, or
    /// bounded-drain behavior without exposing E0's [`UnloadPolicy`] type.
    ///
    /// # Errors
    ///
    /// Returns an error when another model-lifecycle operation is active, no model is loaded,
    /// the configured drain timeout is invalid, or the inference worker cannot accept the
    /// command.
    pub fn unload_model_with_behavior(
        &mut self,
        behavior: ModelUnloadBehavior,
    ) -> Result<(), ApplicationError> {
        self.require_idle()?;
        let handle = self
            .state
            .loaded()
            .ok_or(ApplicationError::NoLoadedModel)?
            .handle();
        self.request_model_unload(handle, behavior)
    }

    pub(crate) fn request_model_unload(
        &mut self,
        handle: ModelHandle,
        behavior: ModelUnloadBehavior,
    ) -> Result<(), ApplicationError> {
        let ticket = self.submit_model_unload(handle, behavior)?;
        self.pending_unload = Some(ModelUnloadTransaction { ticket, handle });
        Ok(())
    }

    pub(crate) fn submit_model_unload(
        &mut self,
        handle: ModelHandle,
        behavior: ModelUnloadBehavior,
    ) -> Result<inference_runtime::CommandTicket, ApplicationError> {
        let policy = self.unload_policy(behavior)?;
        let ticket = self.next_ticket()?;
        let command = RuntimeCommand::UnloadModel {
            ticket,
            handle,
            policy,
        };
        self.submit_inference(command)?;
        self.state.begin_unloading();
        Ok(ticket)
    }

    fn unload_policy(
        &self,
        behavior: ModelUnloadBehavior,
    ) -> Result<UnloadPolicy, ApplicationError> {
        match behavior {
            ModelUnloadBehavior::RejectIfBusy => Ok(UnloadPolicy::RejectIfBusy),
            ModelUnloadBehavior::CancelActive => Ok(UnloadPolicy::CancelActive),
            ModelUnloadBehavior::Drain => {
                let timeout =
                    DrainTimeout::from_millis(self.preferences().drain_timeout_milliseconds)
                        .map_err(|_| {
                            ApplicationError::InvalidConfiguration(
                                ApplicationConfigurationField::DrainTimeout,
                            )
                        })?;
                Ok(UnloadPolicy::Drain { timeout })
            }
        }
    }
}
