//! Application-owned model unload behavior translated into E0 lifecycle policy.

use crate::local::LocalCommand;
use crate::{ApplicationConfigurationField, ApplicationError, ApplicationRuntime};
use domain_contracts::{DrainTimeout, UnloadPolicy};

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
        let policy = self.unload_policy(behavior)?;
        let command = LocalCommand::UnloadModel {
            ticket: self.next_ticket()?,
            handle,
            policy,
        };
        self.submit_inference(command)?;
        self.state.begin_unloading();
        Ok(())
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
