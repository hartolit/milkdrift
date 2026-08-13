//! Retained model cleanup command coordination and public-state publication.

mod coordinator;
mod evidence;

use domain_contracts::{MemoryFootprint, ModelHandle};
use inference_runtime::{
    CleanupResource, CleanupRetryState, CommandTicket, RetainedModelSnapshot, RuntimeCommand,
    RuntimeError, RuntimeSnapshot, UnloadStatus,
};

pub(super) use coordinator::{
    CleanupCommand, ModelCleanupAction, ModelCleanupCoordinator, ModelCleanupOrigin,
};
use evidence::{application_cleanup_resource, application_primary_failure};

use crate::{
    ApplicationActivity, ApplicationError, ApplicationEvent, ApplicationFailure,
    ApplicationFailureKind, ApplicationModelCleanupDisposition, ApplicationRetainedModelResource,
    ApplicationRetainedOwnership, ApplicationRuntime,
};

pub(super) const MAXIMUM_CLEANUP_COORDINATION_SUBMISSION_ATTEMPTS: u8 = 3;

impl ApplicationRuntime {
    /// Re-enables a fresh bounded E1 coordination round.
    ///
    /// This transition is accepted only from `CoordinationRetryAvailable`. It can
    /// never reopen lower exhaustion, disconnection, or process-lifetime retention.
    ///
    /// # Errors
    ///
    /// Returns an error when no retained owner exists or its current action is not retryable.
    pub fn retry_model_cleanup(&mut self) -> Result<(), ApplicationError> {
        let coordinator = self
            .model_cleanup
            .as_mut()
            .ok_or(ApplicationError::NoRetainedModelCleanup)?;
        coordinator
            .retry_coordination()
            .map_err(|_| ApplicationError::ModelCleanupNotRetryable)?;
        self.publish_model_cleanup();
        Ok(())
    }

    /// Advances at most one pending E1 cleanup-coordination action.
    pub(super) fn progress_model_cleanup_coordination(&mut self) -> Option<ApplicationEvent> {
        if matches!(
            self.model_cleanup
                .as_ref()
                .map(ModelCleanupCoordinator::action),
            Some(ModelCleanupAction::WaitingForLowerRetry { .. })
        ) && self
            .model_cleanup
            .as_mut()
            .and_then(|coordinator| coordinator.begin_retained_owner_inspection().err())
            .is_some()
        {
            return Some(self.record_internal_transition_failure(
                "retained cleanup could not enter owner inspection",
            ));
        }

        let (command, attempts) = self
            .model_cleanup
            .as_ref()
            .and_then(ModelCleanupCoordinator::pending_command)?;
        let attempt = attempts.saturating_add(1);
        let submission = match command {
            CleanupCommand::UnloadIncompatibleModel { handle } => {
                self.submit_model_unload(handle, crate::ModelUnloadBehavior::Drain)
            }
            CleanupCommand::InspectRetainedOwner { .. } => {
                let ticket = match self.next_ticket() {
                    Ok(ticket) => ticket,
                    Err(error) => {
                        return Some(self.record_coordination_failure(command, attempt, &error));
                    }
                };
                self.submit_inference(RuntimeCommand::Snapshot { ticket })
                    .map(|()| ticket)
            }
        };
        match submission {
            Ok(submitted_ticket) => {
                let transition = self.model_cleanup.as_mut().map(|coordinator| {
                    coordinator.record_submitted(command, submitted_ticket, attempt)
                });
                if !matches!(transition, Some(Ok(()))) {
                    return Some(self.record_internal_transition_failure(
                        "cleanup command submission did not match the active coordinator action",
                    ));
                }
                self.publish_model_cleanup();
                None
            }
            Err(ApplicationError::RuntimeBusy) => {
                if attempt >= MAXIMUM_CLEANUP_COORDINATION_SUBMISSION_ATTEMPTS {
                    Some(self.record_coordination_failure(
                        command,
                        attempt,
                        &ApplicationError::RuntimeBusy,
                    ))
                } else {
                    let failure = coordination_failure(command, &ApplicationError::RuntimeBusy);
                    let transition = self.model_cleanup.as_mut().map(|coordinator| {
                        coordinator.record_pending_submission_failure(
                            command,
                            attempt,
                            MAXIMUM_CLEANUP_COORDINATION_SUBMISSION_ATTEMPTS,
                            failure,
                        )
                    });
                    if !matches!(transition, Some(Ok(()))) {
                        return Some(self.record_internal_transition_failure(
                            "busy cleanup submission did not match the active action",
                        ));
                    }
                    self.publish_model_cleanup();
                    None
                }
            }
            Err(ApplicationError::RuntimeDisconnected) => Some(self.current_cleanup_event()),
            Err(error) => Some(self.record_coordination_failure(command, attempt, &error)),
        }
    }

    fn record_coordination_failure(
        &mut self,
        command: CleanupCommand,
        attempt: u8,
        error: &ApplicationError,
    ) -> ApplicationEvent {
        let failure = coordination_failure(command, error);
        let transition = self.model_cleanup.as_mut().map(|coordinator| {
            coordinator.record_pending_submission_failure(
                command,
                attempt,
                MAXIMUM_CLEANUP_COORDINATION_SUBMISSION_ATTEMPTS,
                failure,
            )
        });
        if !matches!(transition, Some(Ok(()))) {
            return self.record_internal_transition_failure(
                "cleanup coordination failure did not match the active action",
            );
        }
        self.publish_model_cleanup();
        self.current_cleanup_event()
    }

    fn record_internal_transition_failure(&mut self, message: &'static str) -> ApplicationEvent {
        let failure = ApplicationFailure::new(ApplicationFailureKind::RetainedCleanup, message);
        if let Some(coordinator) = self.model_cleanup.as_mut() {
            let resource = coordinator.retained().resource();
            coordinator.record_unknown_evidence(resource, failure);
        }
        self.publish_model_cleanup();
        self.current_cleanup_event()
    }

    pub(super) fn process_retained_model_cleanup_snapshot(
        &mut self,
        ticket: CommandTicket,
        snapshot: &RuntimeSnapshot,
        retained_models: &[RetainedModelSnapshot],
    ) -> Option<ApplicationEvent> {
        let (command, _) = self
            .model_cleanup
            .as_ref()?
            .submitted_command(ticket)
            .ok()?;
        let CleanupCommand::InspectRetainedOwner { resource } = command else {
            return None;
        };

        let matching_live = retained_models
            .iter()
            .map(|retained| retained.cleanup)
            .filter(|cleanup| {
                self.model_cleanup
                    .as_ref()
                    .is_some_and(|coordinator| coordinator.accepts_resource(cleanup.resource))
            })
            .collect::<Vec<_>>();
        if matching_live.len() > 1 {
            return Some(self.record_multiple_live_owners());
        }

        let matching_last = snapshot.last_cleanup.filter(|cleanup| {
            self.model_cleanup
                .as_ref()
                .is_some_and(|coordinator| coordinator.accepts_resource(cleanup.resource))
        });
        if let Some(live) = matching_live.first().copied() {
            if matching_last
                .is_some_and(|last| last.resource == live.resource && last.ownership.is_released())
            {
                let failure = ApplicationFailure::new(
                    ApplicationFailureKind::RetainedCleanup,
                    "runtime snapshot simultaneously reported explicit release and a live retained owner for the same cleanup resource",
                );
                if let Some(coordinator) = self.model_cleanup.as_mut() {
                    coordinator.record_unknown_evidence(
                        application_cleanup_resource(live.resource),
                        failure,
                    );
                    let _ = coordinator.continue_correlated_inspection(live.resource);
                }
                self.publish_model_cleanup();
                return Some(self.current_cleanup_event());
            }
            let was_exhausted = self.cleanup_is_lower_exhausted();
            self.observe_lower_cleanup(live, None);
            return (live.exhausted() && !was_exhausted).then(|| self.current_cleanup_event());
        }

        if let Some(cleanup) = matching_last {
            if cleanup.ownership.is_released() {
                if self
                    .model_cleanup
                    .as_ref()
                    .and_then(|coordinator| coordinator.verify_release(cleanup).ok())
                    .is_none()
                {
                    return Some(self.record_internal_transition_failure(
                        "explicit cleanup release did not match the active coordinator resource",
                    ));
                }
                if matches!(cleanup.resource, CleanupResource::Sequence { .. })
                    && self.correlated_unload_exists()
                {
                    if let Some(coordinator) = self.model_cleanup.as_mut() {
                        let _ = coordinator.continue_correlated_inspection(cleanup.resource);
                    }
                    self.publish_model_cleanup();
                    return Some(self.current_cleanup_event());
                }
                return Some(self.release_retained_model(resource));
            }
            let was_exhausted = self.cleanup_is_lower_exhausted();
            self.observe_lower_cleanup(cleanup, None);
            return (cleanup.exhausted() && !was_exhausted).then(|| self.current_cleanup_event());
        }

        if self.correlated_unload_exists() {
            if let Some(coordinator) = self.model_cleanup.as_mut() {
                let _ = coordinator.continue_correlated_inspection(resource);
            }
            self.publish_model_cleanup();
            return None;
        }

        let failure = ApplicationFailure::new(
            ApplicationFailureKind::RetainedCleanup,
            "retained cleanup inspection did not contain the expected owner or an explicit release record",
        );
        if let Some(coordinator) = self.model_cleanup.as_mut() {
            let _ = coordinator.record_inspection_uncertainty(
                MAXIMUM_CLEANUP_COORDINATION_SUBMISSION_ATTEMPTS,
                MAXIMUM_CLEANUP_COORDINATION_SUBMISSION_ATTEMPTS,
                failure,
            );
        }
        self.publish_model_cleanup();
        Some(self.current_cleanup_event())
    }

    fn record_multiple_live_owners(&mut self) -> ApplicationEvent {
        let failure = ApplicationFailure::new(
            ApplicationFailureKind::RetainedCleanup,
            "runtime snapshot reported more than one cleanup resource for the active E1 owner",
        );
        if let Some(coordinator) = self.model_cleanup.as_mut() {
            coordinator.record_unknown_evidence(coordinator.retained().resource(), failure.clone());
            let _ = coordinator.record_inspection_uncertainty(
                MAXIMUM_CLEANUP_COORDINATION_SUBMISSION_ATTEMPTS,
                MAXIMUM_CLEANUP_COORDINATION_SUBMISSION_ATTEMPTS,
                failure,
            );
        }
        self.publish_model_cleanup();
        self.current_cleanup_event()
    }

    fn cleanup_is_lower_exhausted(&self) -> bool {
        matches!(
            self.model_cleanup
                .as_ref()
                .map(ModelCleanupCoordinator::action),
            Some(ModelCleanupAction::LowerExhausted { .. })
        )
    }

    fn correlated_unload_exists(&self) -> bool {
        let Some(coordinator) = self.model_cleanup.as_ref() else {
            return false;
        };
        let expected = coordinator.origin().expected_handle();
        self.pending_unload
            .is_some_and(|transaction| Some(transaction.handle) == expected)
            || matches!(
                coordinator.origin(),
                ModelCleanupOrigin::IncompatibleCompletedModel { .. }
                    | ModelCleanupOrigin::OrdinaryLoadedModelUnload { .. }
            )
    }

    pub(super) fn begin_runtime_retention(
        &mut self,
        cleanup: CleanupRetryState,
        primary_override: Option<ApplicationFailure>,
    ) {
        let sequence_evidence = self.sequence_model_evidence(cleanup.resource);
        if cleanup.ownership.is_released() {
            let primary = primary_override
                .or_else(|| {
                    self.model_cleanup
                        .as_ref()
                        .map(|coordinator| coordinator.retained().primary_failure().clone())
                })
                .unwrap_or_else(|| application_primary_failure(cleanup.failure));
            let failure = ApplicationFailure::from_debug(
                ApplicationFailureKind::RetainedCleanup,
                "lower cleanup failure contradicted its released-ownership claim",
                cleanup.failure,
            );
            self.model_cleanup = Some(ModelCleanupCoordinator::contradictory_lower(
                cleanup, primary, failure,
            ));
            self.publish_model_cleanup();
            return;
        }

        let transition = if let Some(coordinator) = self.model_cleanup.as_mut() {
            coordinator.observe_lower(cleanup, primary_override)
        } else {
            match ModelCleanupCoordinator::from_lower(cleanup, primary_override) {
                Ok(coordinator) => {
                    self.model_cleanup = Some(coordinator);
                    Ok(())
                }
                Err(error) => Err(error),
            }
        };
        if transition.is_err() {
            let _event = self.record_internal_transition_failure(
                "lower cleanup resource did not match the active E1 cleanup coordinator",
            );
        } else {
            if let Some((resource, ownership)) = sequence_evidence
                && let Some(coordinator) = self.model_cleanup.as_mut()
            {
                coordinator.replace_public_evidence(resource, ownership);
            }
            self.publish_model_cleanup();
        }
    }

    fn sequence_model_evidence(
        &self,
        resource: CleanupResource,
    ) -> Option<(
        ApplicationRetainedModelResource,
        ApplicationRetainedOwnership,
    )> {
        let CleanupResource::Sequence { handle, .. } = resource else {
            return None;
        };
        if let Some(loaded) = self
            .state
            .loaded()
            .filter(|loaded| loaded.handle() == handle)
        {
            return Some((
                ApplicationRetainedModelResource::LoadedModel { handle },
                ApplicationRetainedOwnership::Exact(loaded.reserved_footprint()),
            ));
        }
        self.model_cleanup.as_ref().and_then(|coordinator| {
            (coordinator.origin().expected_handle() == Some(handle)).then(|| {
                (
                    coordinator.retained().resource(),
                    coordinator.retained().ownership(),
                )
            })
        })
    }

    fn observe_lower_cleanup(
        &mut self,
        cleanup: CleanupRetryState,
        primary_override: Option<ApplicationFailure>,
    ) {
        self.begin_runtime_retention(cleanup, primary_override);
    }

    pub(super) fn reject_incompatible_model(
        &mut self,
        handle: ModelHandle,
        reserved_footprint: MemoryFootprint,
        failure: ApplicationFailure,
    ) -> ApplicationEvent {
        self.model_cleanup = Some(ModelCleanupCoordinator::incompatible_model(
            handle,
            reserved_footprint,
            failure.clone(),
        ));
        self.publish_model_cleanup();
        match self.progress_model_cleanup_coordination() {
            None => ApplicationEvent::ModelCompatibilityFailed { failure },
            Some(event) => event,
        }
    }

    pub(super) fn process_model_unload(
        &mut self,
        ticket: CommandTicket,
        result: &Result<inference_runtime::UnloadReceipt, RuntimeError>,
    ) -> Option<ApplicationEvent> {
        let cleanup_command = self
            .model_cleanup
            .as_ref()
            .and_then(|coordinator| coordinator.submitted_command(ticket).ok());
        if matches!(
            cleanup_command,
            Some((CleanupCommand::UnloadIncompatibleModel { .. }, _))
        ) {
            return Some(self.process_cleanup_model_unload(ticket, result));
        }

        let transaction = self.pending_unload?;
        if transaction.ticket != ticket {
            return None;
        }
        match result {
            Ok(receipt) if receipt.handle != transaction.handle => {
                Some(self.begin_unload_contract_uncertainty(
                    transaction.handle,
                    "model unload receipt returned a different model identity",
                ))
            }
            Ok(receipt) => match receipt.status {
                UnloadStatus::Draining => Some(ApplicationEvent::ModelDraining {
                    handle: receipt.handle,
                }),
                UnloadStatus::AlreadyAbsent | UnloadStatus::Unloaded => {
                    self.pending_unload = None;
                    self.model_cleanup = None;
                    self.state.clear_retained_model();
                    self.state.clear_loaded();
                    Some(ApplicationEvent::ModelUnloaded {
                        handle: receipt.handle,
                        cancelled_requests: receipt.cancelled_requests,
                    })
                }
            },
            Err(
                RuntimeError::CleanupFailed(cleanup) | RuntimeError::CleanupRetryExhausted(cleanup),
            ) if cleanup_resource_belongs_to_model(cleanup.resource, transaction.handle) => {
                self.begin_runtime_retention(*cleanup, None);
                Some(self.current_cleanup_event())
            }
            Err(RuntimeError::CleanupFailed(_) | RuntimeError::CleanupRetryExhausted(_)) => {
                Some(self.begin_unload_contract_uncertainty(
                    transaction.handle,
                    "model unload cleanup state returned a different resource identity",
                ))
            }
            Err(error) => {
                self.pending_unload = None;
                self.state.set_idle();
                Some(ApplicationEvent::ModelUnloadFailed {
                    failure: ApplicationFailure::from_debug(
                        ApplicationFailureKind::Inference,
                        "model unload failed",
                        error,
                    ),
                })
            }
        }
    }

    fn begin_unload_contract_uncertainty(
        &mut self,
        handle: ModelHandle,
        message: &'static str,
    ) -> ApplicationEvent {
        self.pending_unload = None;
        let failure = ApplicationFailure::new(ApplicationFailureKind::IncompatibleReceipt, message);
        self.model_cleanup = Some(ModelCleanupCoordinator::unload_uncertainty(handle, failure));
        self.publish_model_cleanup();
        self.progress_model_cleanup_coordination()
            .unwrap_or_else(|| self.current_cleanup_event())
    }

    fn process_cleanup_model_unload(
        &mut self,
        ticket: CommandTicket,
        result: &Result<inference_runtime::UnloadReceipt, RuntimeError>,
    ) -> ApplicationEvent {
        let Some((CleanupCommand::UnloadIncompatibleModel { handle }, attempts)) = self
            .model_cleanup
            .as_ref()
            .and_then(|coordinator| coordinator.submitted_command(ticket).ok())
        else {
            return self.current_cleanup_event();
        };
        match result {
            Ok(receipt) if receipt.handle != handle => self.record_cleanup_command_failure(
                CleanupCommand::UnloadIncompatibleModel { handle },
                attempts,
                ApplicationFailure::new(
                    ApplicationFailureKind::RetainedCleanup,
                    "automatic retained-model unload returned a different model identity",
                ),
            ),
            Ok(receipt) => match receipt.status {
                UnloadStatus::Draining => ApplicationEvent::ModelDraining {
                    handle: receipt.handle,
                },
                UnloadStatus::AlreadyAbsent | UnloadStatus::Unloaded => {
                    self.model_cleanup = None;
                    self.state.clear_retained_model();
                    self.state.clear_loaded();
                    ApplicationEvent::ModelUnloaded {
                        handle: receipt.handle,
                        cancelled_requests: receipt.cancelled_requests,
                    }
                }
            },
            Err(
                RuntimeError::CleanupFailed(cleanup) | RuntimeError::CleanupRetryExhausted(cleanup),
            ) if self
                .model_cleanup
                .as_ref()
                .is_some_and(|coordinator| coordinator.accepts_resource(cleanup.resource)) =>
            {
                self.begin_runtime_retention(*cleanup, None);
                self.current_cleanup_event()
            }
            Err(RuntimeError::CleanupFailed(_) | RuntimeError::CleanupRetryExhausted(_)) => {
                let mismatch = ApplicationFailure::new(
                    ApplicationFailureKind::RetainedCleanup,
                    "automatic retained-model unload cleanup state returned a different resource identity",
                );
                if let Some(coordinator) = self.model_cleanup.as_mut() {
                    coordinator.record_expected_model_uncertainty(handle, mismatch.clone());
                }
                self.record_cleanup_command_failure(
                    CleanupCommand::UnloadIncompatibleModel { handle },
                    attempts,
                    mismatch,
                )
            }
            Err(error) => self.record_cleanup_command_failure(
                CleanupCommand::UnloadIncompatibleModel { handle },
                attempts,
                ApplicationFailure::from_debug(
                    ApplicationFailureKind::RetainedCleanup,
                    "automatic retained-model unload failed",
                    error,
                ),
            ),
        }
    }

    fn record_cleanup_command_failure(
        &mut self,
        command: CleanupCommand,
        attempts: u8,
        failure: ApplicationFailure,
    ) -> ApplicationEvent {
        let transition = self.model_cleanup.as_mut().map(|coordinator| {
            coordinator.record_pending_submission_failure(
                command,
                attempts,
                MAXIMUM_CLEANUP_COORDINATION_SUBMISSION_ATTEMPTS,
                failure,
            )
        });
        if !matches!(transition, Some(Ok(()))) {
            return self.record_internal_transition_failure(
                "cleanup command result did not match its submitted operation",
            );
        }
        self.publish_model_cleanup();
        self.current_cleanup_event()
    }

    pub(crate) fn mark_model_worker_disconnected(&mut self) {
        let failure = ApplicationFailure::new(
            ApplicationFailureKind::Worker,
            "inference worker disconnected without proving model ownership release",
        );
        if let Some(coordinator) = self.model_cleanup.as_mut() {
            coordinator.mark_disconnected(failure);
        } else if let Some(loaded) = self.state.loaded().cloned() {
            self.model_cleanup = Some(ModelCleanupCoordinator::disconnected(
                ModelCleanupOrigin::UnconfirmedModelAfterDisconnection {
                    handle: Some(loaded.handle()),
                },
                ApplicationRetainedModelResource::LoadedModel {
                    handle: loaded.handle(),
                },
                ApplicationRetainedOwnership::Exact(loaded.reserved_footprint()),
                failure,
            ));
        } else if self.pending_load.is_some()
            || matches!(
                self.state.activity(),
                ApplicationActivity::Loading | ApplicationActivity::Unloading
            )
        {
            self.model_cleanup = Some(ModelCleanupCoordinator::disconnected(
                ModelCleanupOrigin::UnconfirmedLoadAfterDisconnection,
                ApplicationRetainedModelResource::UnconfirmedLoad,
                ApplicationRetainedOwnership::Unknown,
                failure,
            ));
        }
        self.pending_load = None;
        self.pending_unload = None;
        self.publish_model_cleanup();
    }

    pub(crate) fn mark_terminal_worker_failure(&mut self, error: &RuntimeError) {
        let had_model_evidence = self.state.loaded().is_some()
            || self.state.retained_model().is_some()
            || self.pending_load.is_some()
            || self.pending_unload.is_some();
        if let RuntimeError::CleanupFailed(cleanup) | RuntimeError::CleanupRetryExhausted(cleanup) =
            error
        {
            self.begin_runtime_retention(*cleanup, None);
        }
        self.pending_load = None;
        self.pending_unload = None;
        self.generation.confirm_runtime_shutdown();
        self.state.clear_normal_runtime_ownership_for_shutdown();
        let failure = ApplicationFailure::from_debug(
            ApplicationFailureKind::Inference,
            "terminal inference shutdown did not prove model release",
            error,
        );
        if let Some(coordinator) = self.model_cleanup.as_mut() {
            coordinator.mark_retained_until_process_exit(Some(failure));
        } else if had_model_evidence {
            self.model_cleanup = Some(ModelCleanupCoordinator::terminal_unknown(failure));
        }
        self.publish_model_cleanup();
    }

    pub(crate) fn mark_terminal_process_retention(
        &mut self,
        first: CleanupRetryState,
        summary: inference_runtime::TerminalRetentionSummary,
    ) {
        self.pending_load = None;
        self.pending_unload = None;
        self.generation.confirm_runtime_shutdown();
        self.state.clear_normal_runtime_ownership_for_shutdown();
        let model_owners = summary
            .failed_preparations
            .saturating_add(summary.verified_models)
            .saturating_add(summary.incompatible_models);
        if model_owners == 0 {
            return;
        }
        if let Some(coordinator) = self.model_cleanup.as_mut() {
            coordinator.mark_retained_until_process_exit(None);
        } else if is_model_resource(first.resource) {
            self.model_cleanup = ModelCleanupCoordinator::from_lower(first, None).ok();
            if let Some(coordinator) = self.model_cleanup.as_mut() {
                coordinator.mark_retained_until_process_exit(None);
            }
        } else {
            self.model_cleanup = Some(ModelCleanupCoordinator::terminal_unknown(
                ApplicationFailure::new(
                    ApplicationFailureKind::RetainedCleanup,
                    "terminal shutdown retained model ownership until process exit",
                ),
            ));
        }
        self.publish_model_cleanup();
    }

    fn release_retained_model(&mut self, resource: CleanupResource) -> ApplicationEvent {
        let public_resource = self.model_cleanup.as_ref().map_or_else(
            || application_cleanup_resource(resource),
            |coordinator| coordinator.retained().resource(),
        );
        self.model_cleanup = None;
        self.state.clear_retained_model();
        ApplicationEvent::ModelCleanupReleased {
            resource: public_resource,
        }
    }

    fn publish_model_cleanup(&mut self) {
        if let Some(coordinator) = self.model_cleanup.as_ref() {
            self.state
                .set_retained_model(coordinator.retained().clone());
        }
    }

    pub(super) fn current_cleanup_event(&self) -> ApplicationEvent {
        let (resource, disposition) = self.model_cleanup.as_ref().map_or(
            (
                ApplicationRetainedModelResource::UnconfirmedModel,
                ApplicationModelCleanupDisposition::WorkerDisconnected,
            ),
            |coordinator| {
                (
                    coordinator.retained().resource(),
                    coordinator.retained().cleanup(),
                )
            },
        );
        ApplicationEvent::ModelCleanupPending {
            resource,
            disposition,
        }
    }

    pub(crate) fn confirm_runtime_shutdown_released(&mut self) {
        self.pending_load = None;
        self.pending_unload = None;
        self.model_cleanup = None;
        self.generation.confirm_runtime_shutdown();
        self.state.confirm_runtime_shutdown_released();
    }

    #[cfg(test)]
    pub(super) fn install_submitted_cleanup_inspection(
        &mut self,
        resource: CleanupResource,
        ticket: CommandTicket,
    ) {
        if let Some(coordinator) = self.model_cleanup.as_mut() {
            if matches!(
                coordinator.action(),
                ModelCleanupAction::WaitingForLowerRetry { .. }
            ) {
                let _ = coordinator.begin_retained_owner_inspection();
            }
            let attempts = coordinator
                .pending_command()
                .map_or(1, |(_, attempts)| attempts.saturating_add(1));
            let _ = coordinator.record_submitted(
                CleanupCommand::InspectRetainedOwner { resource },
                ticket,
                attempts,
            );
        }
        self.publish_model_cleanup();
    }

    #[cfg(test)]
    pub(super) fn install_submitted_incompatible_cleanup(
        &mut self,
        handle: ModelHandle,
        ownership: ApplicationRetainedOwnership,
        primary_failure: ApplicationFailure,
        ticket: CommandTicket,
        attempts: u8,
    ) {
        self.model_cleanup = Some(ModelCleanupCoordinator::submitted_incompatible_for_test(
            handle,
            ownership,
            primary_failure,
            ticket,
            attempts,
        ));
        self.publish_model_cleanup();
    }
}

fn coordination_failure(command: CleanupCommand, error: &ApplicationError) -> ApplicationFailure {
    let context = match command {
        CleanupCommand::UnloadIncompatibleModel { .. } => {
            "automatic retained-model unload could not be submitted"
        }
        CleanupCommand::InspectRetainedOwner { .. } => {
            "retained cleanup inspection could not be submitted"
        }
    };
    ApplicationFailure::new(
        ApplicationFailureKind::RetainedCleanup,
        format!("{context}: {error}"),
    )
}

fn cleanup_resource_belongs_to_model(
    resource: CleanupResource,
    expected_handle: ModelHandle,
) -> bool {
    matches!(
        resource,
        CleanupResource::Model { handle } | CleanupResource::Sequence { handle, .. }
            if handle == expected_handle
    )
}

const fn is_model_resource(resource: CleanupResource) -> bool {
    matches!(
        resource,
        CleanupResource::Model { .. }
            | CleanupResource::IncompatibleModel { .. }
            | CleanupResource::FailedLoad { .. }
    )
}
