//! Checked private state machine for one retained model owner.

use domain_contracts::{MemoryFootprint, ModelHandle};
use inference_runtime::{CleanupResource, CleanupRetryState, CommandTicket};

use crate::{
    ApplicationFailure, ApplicationModelCleanupDisposition, ApplicationRetainedModel,
    ApplicationRetainedModelResource, ApplicationRetainedOwnership,
};

use super::evidence::{
    application_cleanup_disposition, application_cleanup_failure, application_cleanup_resource,
    application_primary_failure, application_retained_ownership,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::runtime) enum ModelCleanupOrigin {
    FailedMaterialization { handle: ModelHandle },
    IncompatibleCompletedModel { handle: ModelHandle },
    OrdinaryLoadedModelUnload { handle: ModelHandle },
    UnconfirmedLoadAfterDisconnection,
    UnconfirmedModelAfterDisconnection { handle: Option<ModelHandle> },
    TerminalShutdownRetention,
}

impl ModelCleanupOrigin {
    pub(super) const fn expected_handle(self) -> Option<ModelHandle> {
        match self {
            Self::FailedMaterialization { handle }
            | Self::IncompatibleCompletedModel { handle }
            | Self::OrdinaryLoadedModelUnload { handle } => Some(handle),
            Self::UnconfirmedModelAfterDisconnection { handle } => handle,
            Self::UnconfirmedLoadAfterDisconnection | Self::TerminalShutdownRetention => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::runtime) enum CleanupCommand {
    UnloadIncompatibleModel { handle: ModelHandle },
    InspectRetainedOwner { resource: CleanupResource },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::runtime) enum ModelCleanupAction {
    PendingCommandSubmission {
        command: CleanupCommand,
        attempts: u8,
    },
    SubmittedCommand {
        command: CleanupCommand,
        ticket: CommandTicket,
        attempts: u8,
    },
    WaitingForLowerRetry {
        resource: CleanupResource,
        attempts: u32,
        maximum_attempts: u32,
    },
    CoordinationRetryAvailable {
        command: CleanupCommand,
        attempts: u8,
        maximum_attempts: u8,
    },
    LowerExhausted {
        resource: CleanupResource,
        attempts: u32,
        maximum_attempts: u32,
    },
    Disconnected,
    RetainedUntilProcessExit,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::runtime) struct LowerCleanupAttempts {
    pub(in crate::runtime) attempts: u32,
    pub(in crate::runtime) maximum_attempts: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::runtime) struct ModelCleanupCoordinator {
    origin: ModelCleanupOrigin,
    lower_resource: Option<CleanupResource>,
    lower_attempts: Option<LowerCleanupAttempts>,
    action: ModelCleanupAction,
    retained: ApplicationRetainedModel,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::runtime) enum CleanupTransitionError {
    InvalidAction,
    WrongResource,
    WrongTicket,
}

impl ModelCleanupCoordinator {
    pub(super) fn incompatible_model(
        handle: ModelHandle,
        reserved_footprint: MemoryFootprint,
        failure: ApplicationFailure,
    ) -> Self {
        Self {
            origin: ModelCleanupOrigin::IncompatibleCompletedModel { handle },
            lower_resource: None,
            lower_attempts: None,
            action: ModelCleanupAction::PendingCommandSubmission {
                command: CleanupCommand::UnloadIncompatibleModel { handle },
                attempts: 0,
            },
            retained: ApplicationRetainedModel::new(
                ApplicationRetainedModelResource::LoadedModel { handle },
                ApplicationRetainedOwnership::Exact(reserved_footprint),
                ApplicationModelCleanupDisposition::Pending,
                failure,
                None,
            ),
        }
    }

    pub(super) fn unload_uncertainty(handle: ModelHandle, failure: ApplicationFailure) -> Self {
        Self {
            origin: ModelCleanupOrigin::OrdinaryLoadedModelUnload { handle },
            lower_resource: None,
            lower_attempts: None,
            action: ModelCleanupAction::PendingCommandSubmission {
                command: CleanupCommand::UnloadIncompatibleModel { handle },
                attempts: 0,
            },
            retained: ApplicationRetainedModel::new(
                ApplicationRetainedModelResource::LoadedModel { handle },
                ApplicationRetainedOwnership::Unknown,
                ApplicationModelCleanupDisposition::Pending,
                failure,
                None,
            ),
        }
    }

    pub(super) fn from_lower(
        cleanup: CleanupRetryState,
        primary_override: Option<ApplicationFailure>,
    ) -> Result<Self, CleanupTransitionError> {
        if cleanup.ownership().is_released() {
            return Err(CleanupTransitionError::InvalidAction);
        }
        let origin = origin_for_lower_resource(cleanup.resource());
        let primary_failure =
            primary_override.unwrap_or_else(|| application_primary_failure(cleanup.failure()));
        let action = lower_action(cleanup);
        Ok(Self {
            origin,
            lower_resource: Some(cleanup.resource()),
            lower_attempts: Some(lower_cleanup_attempts(cleanup)),
            action,
            retained: ApplicationRetainedModel::new(
                application_cleanup_resource(cleanup.resource()),
                application_retained_ownership(cleanup.ownership()),
                application_cleanup_disposition(cleanup),
                primary_failure,
                Some(application_cleanup_failure(cleanup.failure())),
            ),
        })
    }

    pub(super) fn contradictory_lower(
        cleanup: CleanupRetryState,
        primary_failure: ApplicationFailure,
        failure: ApplicationFailure,
    ) -> Self {
        let resource = cleanup.resource();
        Self {
            origin: origin_for_lower_resource(resource),
            lower_resource: Some(resource),
            lower_attempts: Some(lower_cleanup_attempts(cleanup)),
            action: ModelCleanupAction::PendingCommandSubmission {
                command: CleanupCommand::InspectRetainedOwner { resource },
                attempts: 0,
            },
            retained: ApplicationRetainedModel::new(
                application_cleanup_resource(resource),
                ApplicationRetainedOwnership::Unknown,
                ApplicationModelCleanupDisposition::Pending,
                primary_failure,
                Some(failure),
            ),
        }
    }

    pub(super) fn disconnected(
        origin: ModelCleanupOrigin,
        resource: ApplicationRetainedModelResource,
        ownership: ApplicationRetainedOwnership,
        failure: ApplicationFailure,
    ) -> Self {
        Self {
            origin,
            lower_resource: None,
            lower_attempts: None,
            action: ModelCleanupAction::Disconnected,
            retained: ApplicationRetainedModel::new(
                resource,
                ownership,
                ApplicationModelCleanupDisposition::WorkerDisconnected,
                failure,
                None,
            ),
        }
    }

    pub(super) fn terminal_unknown(failure: ApplicationFailure) -> Self {
        Self {
            origin: ModelCleanupOrigin::TerminalShutdownRetention,
            lower_resource: None,
            lower_attempts: None,
            action: ModelCleanupAction::RetainedUntilProcessExit,
            retained: ApplicationRetainedModel::new(
                ApplicationRetainedModelResource::UnconfirmedModel,
                ApplicationRetainedOwnership::Unknown,
                ApplicationModelCleanupDisposition::RetainedUntilProcessExit,
                failure,
                None,
            ),
        }
    }

    pub(super) const fn origin(&self) -> ModelCleanupOrigin {
        self.origin
    }

    pub(in crate::runtime) const fn action(&self) -> ModelCleanupAction {
        self.action
    }

    #[cfg(test)]
    pub(in crate::runtime) const fn lower_attempts(&self) -> Option<LowerCleanupAttempts> {
        self.lower_attempts
    }

    pub(super) const fn retained(&self) -> &ApplicationRetainedModel {
        &self.retained
    }

    pub(super) fn pending_command(&self) -> Option<(CleanupCommand, u8)> {
        match self.action {
            ModelCleanupAction::PendingCommandSubmission { command, attempts } => {
                Some((command, attempts))
            }
            _ => None,
        }
    }

    pub(super) fn begin_retained_owner_inspection(&mut self) -> Result<(), CleanupTransitionError> {
        let ModelCleanupAction::WaitingForLowerRetry { resource, .. } = self.action else {
            return Err(CleanupTransitionError::InvalidAction);
        };
        self.action = ModelCleanupAction::PendingCommandSubmission {
            command: CleanupCommand::InspectRetainedOwner { resource },
            attempts: 0,
        };
        Ok(())
    }

    pub(super) fn record_submitted(
        &mut self,
        command: CleanupCommand,
        ticket: CommandTicket,
        attempt: u8,
    ) -> Result<(), CleanupTransitionError> {
        match self.action {
            ModelCleanupAction::PendingCommandSubmission {
                command: expected, ..
            } if expected == command => {
                self.action = ModelCleanupAction::SubmittedCommand {
                    command,
                    ticket,
                    attempts: attempt,
                };
                self.retained
                    .set_cleanup(ApplicationModelCleanupDisposition::Pending, None);
                Ok(())
            }
            _ => Err(CleanupTransitionError::InvalidAction),
        }
    }

    pub(super) fn submitted_command(
        &self,
        ticket: CommandTicket,
    ) -> Result<(CleanupCommand, u8), CleanupTransitionError> {
        match self.action {
            ModelCleanupAction::SubmittedCommand {
                command,
                ticket: expected,
                attempts,
            } if expected == ticket => Ok((command, attempts)),
            ModelCleanupAction::SubmittedCommand { .. } => Err(CleanupTransitionError::WrongTicket),
            _ => Err(CleanupTransitionError::InvalidAction),
        }
    }

    pub(super) fn record_pending_submission_failure(
        &mut self,
        command: CleanupCommand,
        attempt: u8,
        maximum_attempts: u8,
        failure: ApplicationFailure,
    ) -> Result<(), CleanupTransitionError> {
        if !matches!(
            self.action,
            ModelCleanupAction::PendingCommandSubmission {
                command: expected,
                ..
            } | ModelCleanupAction::SubmittedCommand {
                command: expected,
                ..
            } if expected == command
        ) {
            return Err(CleanupTransitionError::InvalidAction);
        }
        if attempt >= maximum_attempts {
            self.action = ModelCleanupAction::CoordinationRetryAvailable {
                command,
                attempts: attempt,
                maximum_attempts,
            };
            self.retained.set_cleanup(
                ApplicationModelCleanupDisposition::CoordinationRetryAvailable {
                    attempts: attempt,
                    maximum_attempts,
                },
                Some(failure),
            );
        } else {
            self.action = ModelCleanupAction::PendingCommandSubmission {
                command,
                attempts: attempt,
            };
            self.retained
                .set_cleanup(ApplicationModelCleanupDisposition::Pending, Some(failure));
        }
        Ok(())
    }

    pub(super) fn retry_coordination(&mut self) -> Result<(), CleanupTransitionError> {
        let ModelCleanupAction::CoordinationRetryAvailable { command, .. } = self.action else {
            return Err(CleanupTransitionError::InvalidAction);
        };
        self.action = ModelCleanupAction::PendingCommandSubmission {
            command,
            attempts: 0,
        };
        self.retained
            .set_cleanup(ApplicationModelCleanupDisposition::Pending, None);
        Ok(())
    }

    pub(super) fn observe_lower(
        &mut self,
        cleanup: CleanupRetryState,
        primary_override: Option<ApplicationFailure>,
    ) -> Result<(), CleanupTransitionError> {
        if cleanup.ownership().is_released() || !self.accepts_resource(cleanup.resource()) {
            return Err(CleanupTransitionError::WrongResource);
        }
        self.lower_resource = Some(cleanup.resource());
        self.lower_attempts = Some(lower_cleanup_attempts(cleanup));
        self.action = lower_action(cleanup);
        let primary_failure = primary_override
            .or_else(|| Some(self.retained.primary_failure().clone()))
            .unwrap_or_else(|| application_primary_failure(cleanup.failure()));
        self.retained = ApplicationRetainedModel::new(
            application_cleanup_resource(cleanup.resource()),
            application_retained_ownership(cleanup.ownership()),
            application_cleanup_disposition(cleanup),
            primary_failure,
            Some(application_cleanup_failure(cleanup.failure())),
        );
        Ok(())
    }

    pub(super) fn record_unknown_evidence(
        &mut self,
        resource: ApplicationRetainedModelResource,
        failure: ApplicationFailure,
    ) {
        let primary = self.retained.primary_failure().clone();
        self.retained = ApplicationRetainedModel::new(
            resource,
            ApplicationRetainedOwnership::Unknown,
            ApplicationModelCleanupDisposition::Pending,
            primary,
            Some(failure),
        );
    }

    pub(super) fn replace_public_evidence(
        &mut self,
        resource: ApplicationRetainedModelResource,
        ownership: ApplicationRetainedOwnership,
    ) {
        self.retained = ApplicationRetainedModel::new(
            resource,
            ownership,
            self.retained.cleanup(),
            self.retained.primary_failure().clone(),
            self.retained.cleanup_failure().cloned(),
        );
    }

    pub(super) fn record_expected_model_uncertainty(
        &mut self,
        handle: ModelHandle,
        failure: ApplicationFailure,
    ) {
        let resource =
            match self.retained.resource() {
                resource @ (ApplicationRetainedModelResource::LoadedModel { handle: current }
                | ApplicationRetainedModelResource::IncompatibleModel {
                    handle: current,
                }) if current == handle => resource,
                _ => ApplicationRetainedModelResource::LoadedModel { handle },
            };
        let ownership = match self.retained.ownership() {
            ownership @ ApplicationRetainedOwnership::Unverified { .. } => ownership,
            ApplicationRetainedOwnership::Exact(_) | ApplicationRetainedOwnership::Unknown => {
                ApplicationRetainedOwnership::Unknown
            }
        };
        let primary = self.retained.primary_failure().clone();
        self.retained = ApplicationRetainedModel::new(
            resource,
            ownership,
            ApplicationModelCleanupDisposition::Pending,
            primary,
            Some(failure),
        );
    }

    pub(super) fn record_inspection_uncertainty(
        &mut self,
        attempts: u8,
        maximum_attempts: u8,
        failure: ApplicationFailure,
    ) -> Result<(), CleanupTransitionError> {
        let resource = self
            .lower_resource
            .ok_or(CleanupTransitionError::WrongResource)?;
        self.action = ModelCleanupAction::CoordinationRetryAvailable {
            command: CleanupCommand::InspectRetainedOwner { resource },
            attempts,
            maximum_attempts,
        };
        self.retained.set_cleanup(
            ApplicationModelCleanupDisposition::CoordinationRetryAvailable {
                attempts,
                maximum_attempts,
            },
            Some(failure),
        );
        Ok(())
    }

    pub(super) fn continue_correlated_inspection(
        &mut self,
        resource: CleanupResource,
    ) -> Result<(), CleanupTransitionError> {
        if !self.accepts_resource(resource) {
            return Err(CleanupTransitionError::WrongResource);
        }
        self.lower_resource = Some(resource);
        self.action = ModelCleanupAction::PendingCommandSubmission {
            command: CleanupCommand::InspectRetainedOwner { resource },
            attempts: 0,
        };
        self.retained
            .set_cleanup(ApplicationModelCleanupDisposition::Pending, None);
        Ok(())
    }

    pub(super) fn verify_release(
        &self,
        cleanup: CleanupRetryState,
    ) -> Result<CleanupResource, CleanupTransitionError> {
        if !cleanup.ownership().is_released() {
            return Err(CleanupTransitionError::InvalidAction);
        }
        if !self.accepts_resource(cleanup.resource()) {
            return Err(CleanupTransitionError::WrongResource);
        }
        Ok(cleanup.resource())
    }

    pub(super) fn mark_disconnected(&mut self, failure: ApplicationFailure) {
        self.action = ModelCleanupAction::Disconnected;
        self.retained.set_cleanup(
            ApplicationModelCleanupDisposition::WorkerDisconnected,
            Some(failure),
        );
    }

    pub(super) fn mark_retained_until_process_exit(&mut self, failure: Option<ApplicationFailure>) {
        self.action = ModelCleanupAction::RetainedUntilProcessExit;
        self.retained.set_cleanup(
            ApplicationModelCleanupDisposition::RetainedUntilProcessExit,
            failure,
        );
    }

    pub(super) fn accepts_resource(&self, resource: CleanupResource) -> bool {
        if self.lower_resource == Some(resource) {
            return true;
        }
        let Some(expected_handle) = self.origin.expected_handle() else {
            return self.lower_resource.is_none();
        };
        match self.origin {
            ModelCleanupOrigin::FailedMaterialization { .. } => matches!(
                resource,
                CleanupResource::FailedLoad { handle } if handle == expected_handle
            ),
            ModelCleanupOrigin::IncompatibleCompletedModel { .. }
            | ModelCleanupOrigin::OrdinaryLoadedModelUnload { .. } => matches!(
                resource,
                CleanupResource::Model { handle }
                    | CleanupResource::IncompatibleModel { handle }
                    | CleanupResource::Sequence { handle, .. }
                    if handle == expected_handle
            ),
            ModelCleanupOrigin::UnconfirmedLoadAfterDisconnection
            | ModelCleanupOrigin::UnconfirmedModelAfterDisconnection { .. }
            | ModelCleanupOrigin::TerminalShutdownRetention => self.lower_resource.is_none(),
        }
    }

    #[cfg(test)]
    pub(super) fn submitted_incompatible_for_test(
        handle: ModelHandle,
        ownership: ApplicationRetainedOwnership,
        primary_failure: ApplicationFailure,
        ticket: CommandTicket,
        attempts: u8,
    ) -> Self {
        Self {
            origin: ModelCleanupOrigin::IncompatibleCompletedModel { handle },
            lower_resource: None,
            lower_attempts: None,
            action: ModelCleanupAction::SubmittedCommand {
                command: CleanupCommand::UnloadIncompatibleModel { handle },
                ticket,
                attempts,
            },
            retained: ApplicationRetainedModel::new(
                ApplicationRetainedModelResource::LoadedModel { handle },
                ownership,
                ApplicationModelCleanupDisposition::Pending,
                primary_failure,
                None,
            ),
        }
    }
}

const fn origin_for_lower_resource(resource: CleanupResource) -> ModelCleanupOrigin {
    match resource {
        CleanupResource::FailedLoad { handle } => {
            ModelCleanupOrigin::FailedMaterialization { handle }
        }
        CleanupResource::IncompatibleModel { handle } => {
            ModelCleanupOrigin::IncompatibleCompletedModel { handle }
        }
        CleanupResource::Model { handle } | CleanupResource::Sequence { handle, .. } => {
            ModelCleanupOrigin::OrdinaryLoadedModelUnload { handle }
        }
    }
}

const fn lower_action(cleanup: CleanupRetryState) -> ModelCleanupAction {
    if cleanup.exhausted() {
        ModelCleanupAction::LowerExhausted {
            resource: cleanup.resource(),
            attempts: cleanup.attempts(),
            maximum_attempts: cleanup.maximum_attempts(),
        }
    } else {
        ModelCleanupAction::WaitingForLowerRetry {
            resource: cleanup.resource(),
            attempts: cleanup.attempts(),
            maximum_attempts: cleanup.maximum_attempts(),
        }
    }
}

const fn lower_cleanup_attempts(cleanup: CleanupRetryState) -> LowerCleanupAttempts {
    LowerCleanupAttempts {
        attempts: cleanup.attempts(),
        maximum_attempts: cleanup.maximum_attempts(),
    }
}
