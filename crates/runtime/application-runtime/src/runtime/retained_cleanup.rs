//! Retained model ownership, cleanup coordination, and disconnect truth.

use domain_contracts::{MemoryFootprint, ModelHandle};
use inference_runtime::{
    CleanupFailureReport, CleanupResource, CleanupRetryState, CommandTicket, ConservativeFootprint,
    RetainedModelSnapshot, RetainedOwnership, RuntimeCommand, RuntimeError, RuntimeSnapshot,
    UnloadStatus,
};

use crate::{
    ApplicationActivity, ApplicationConservativeFootprint, ApplicationError, ApplicationEvent,
    ApplicationFailure, ApplicationFailureKind, ApplicationModelCleanupDisposition,
    ApplicationRetainedModel, ApplicationRetainedModelResource, ApplicationRetainedOwnership,
    ApplicationRuntime,
};

pub(super) const MAXIMUM_INCOMPATIBLE_UNLOAD_SUBMISSION_ATTEMPTS: u8 = 3;
const MAXIMUM_LOAD_CLEANUP_INSPECTION_SUBMISSION_ATTEMPTS: u8 = 3;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct IncompatibleModelCleanup {
    pub(super) handle: ModelHandle,
    pub(super) compatibility_failure: ApplicationFailure,
    pub(super) unload: IncompatibleModelUnload,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum IncompatibleModelUnload {
    PendingSubmission {
        attempts: u8,
        last_failure: Option<ApplicationFailure>,
    },
    Submitted {
        ticket: CommandTicket,
        attempts: u8,
        last_failure: Option<ApplicationFailure>,
    },
    RetryExhausted {
        attempts: u8,
        last_failure: ApplicationFailure,
    },
    WorkerDisconnected,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct RetainedModelCleanup {
    pub(super) resource: CleanupResource,
    pub(super) inspection: RetainedModelInspection,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum RetainedModelInspection {
    PendingSubmission { attempts: u8 },
    Submitted { ticket: CommandTicket },
    CoordinationRetryAvailable { attempts: u8 },
    LowerExhausted,
    WorkerDisconnected,
}

impl ApplicationRuntime {
    /// Re-enables bounded E1 coordination after submission or inspection exhaustion.
    ///
    /// This never resets E0's cleanup-attempt policy. Lower exhaustion, worker
    /// disconnection, and process-lifetime retention remain non-retryable.
    ///
    /// # Errors
    ///
    /// Returns an error when no retained model exists or its current disposition
    /// does not permit another E1 coordination round.
    pub fn retry_model_cleanup(&mut self) -> Result<(), ApplicationError> {
        if self.state.retained_model().is_none() {
            return Err(ApplicationError::NoRetainedModelCleanup);
        }
        if !self.state.can_retry_model_cleanup() {
            return Err(ApplicationError::ModelCleanupNotRetryable);
        }

        let mut retry_enabled = false;
        if let Some(cleanup) = self.incompatible_model_cleanup.as_mut()
            && matches!(
                cleanup.unload,
                IncompatibleModelUnload::RetryExhausted { .. }
            )
        {
            cleanup.unload = IncompatibleModelUnload::PendingSubmission {
                attempts: 0,
                last_failure: None,
            };
            retry_enabled = true;
        }
        if let Some(cleanup) = self.retained_model_cleanup.as_mut()
            && matches!(
                cleanup.inspection,
                RetainedModelInspection::CoordinationRetryAvailable { .. }
            )
        {
            cleanup.inspection = RetainedModelInspection::PendingSubmission { attempts: 0 };
            retry_enabled = true;
        }
        if !retry_enabled {
            return Err(ApplicationError::ModelCleanupNotRetryable);
        }

        if let Some(retained) = self.state.retained_model_mut() {
            retained.set_cleanup(ApplicationModelCleanupDisposition::Pending, None);
        }
        self.state.begin_retained_cleanup();
        Ok(())
    }

    pub(super) fn retry_retained_model_cleanup_inspection(&mut self) -> Option<ApplicationEvent> {
        let (resource, attempts) = self
            .retained_model_cleanup
            .and_then(|cleanup| match cleanup.inspection {
                RetainedModelInspection::PendingSubmission { attempts } => {
                    Some((cleanup.resource, attempts))
                }
                RetainedModelInspection::Submitted { .. }
                | RetainedModelInspection::CoordinationRetryAvailable { .. }
                | RetainedModelInspection::LowerExhausted
                | RetainedModelInspection::WorkerDisconnected => None,
            })?;
        let attempt = attempts.saturating_add(1);
        let ticket = match self.next_ticket() {
            Ok(ticket) => ticket,
            Err(error) => {
                return Some(self.record_cleanup_coordination_failure(resource, attempt, &error));
            }
        };
        match self.submit_inference(RuntimeCommand::Snapshot { ticket }) {
            Ok(()) => {
                self.retained_model_cleanup = Some(RetainedModelCleanup {
                    resource,
                    inspection: RetainedModelInspection::Submitted { ticket },
                });
                None
            }
            Err(ApplicationError::RuntimeBusy)
                if attempt < MAXIMUM_LOAD_CLEANUP_INSPECTION_SUBMISSION_ATTEMPTS =>
            {
                self.retained_model_cleanup = Some(RetainedModelCleanup {
                    resource,
                    inspection: RetainedModelInspection::PendingSubmission { attempts: attempt },
                });
                None
            }
            Err(ApplicationError::RuntimeDisconnected) => self
                .state
                .retained_model()
                .cloned()
                .map(|cleanup| ApplicationEvent::ModelCleanupPending { cleanup })
                .or(Some(ApplicationEvent::RuntimeDisconnected)),
            Err(error) => Some(self.record_cleanup_coordination_failure(resource, attempt, &error)),
        }
    }

    fn record_cleanup_coordination_failure(
        &mut self,
        resource: CleanupResource,
        attempts: u8,
        error: &ApplicationError,
    ) -> ApplicationEvent {
        self.retained_model_cleanup = Some(RetainedModelCleanup {
            resource,
            inspection: RetainedModelInspection::CoordinationRetryAvailable { attempts },
        });
        let failure = ApplicationFailure::new(
            ApplicationFailureKind::RetainedCleanup,
            format!("retained cleanup inspection could not be submitted: {error}"),
        );
        if let Some(retained) = self.state.retained_model_mut() {
            retained.set_cleanup(
                ApplicationModelCleanupDisposition::CoordinationRetryAvailable {
                    attempts,
                    maximum_attempts: MAXIMUM_LOAD_CLEANUP_INSPECTION_SUBMISSION_ATTEMPTS,
                },
                Some(failure),
            );
        }
        self.state.begin_retained_cleanup();
        self.current_cleanup_event()
    }

    pub(super) fn process_retained_model_cleanup_snapshot(
        &mut self,
        ticket: CommandTicket,
        snapshot: &RuntimeSnapshot,
        retained_models: &[RetainedModelSnapshot],
    ) -> Option<ApplicationEvent> {
        let resource = match self.retained_model_cleanup {
            Some(RetainedModelCleanup {
                resource,
                inspection: RetainedModelInspection::Submitted { ticket: expected },
            }) if expected == ticket => resource,
            _ => return None,
        };

        let resource_handle = cleanup_resource_handle(resource);
        let correlated_handle = self
            .model_unload_correlation_exists(resource_handle)
            .then_some(resource_handle);
        let belongs_to_correlated_unload = |candidate| {
            correlated_handle
                .is_some_and(|handle| cleanup_resource_belongs_to_model(candidate, handle))
        };
        let matching_last_cleanup = snapshot.last_cleanup.filter(|cleanup| {
            cleanup.resource == resource || belongs_to_correlated_unload(cleanup.resource)
        });
        if let Some(cleanup) = matching_last_cleanup {
            if cleanup.ownership.is_released() {
                if correlated_handle.is_some() {
                    self.retained_model_cleanup = Some(RetainedModelCleanup {
                        resource: cleanup.resource,
                        inspection: RetainedModelInspection::PendingSubmission { attempts: 0 },
                    });
                    if let Some(retained) = self.state.retained_model_mut() {
                        retained.set_cleanup(ApplicationModelCleanupDisposition::Pending, None);
                    }
                    self.state.begin_retained_cleanup();
                    return Some(self.current_cleanup_event());
                }
                return Some(self.release_retained_model(resource));
            }
            let was_exhausted = self.state.retained_model().is_some_and(|retained| {
                matches!(
                    retained.cleanup(),
                    ApplicationModelCleanupDisposition::LowerExhausted { .. }
                )
            });
            self.begin_runtime_retention(cleanup, None);
            if cleanup.exhausted() && !was_exhausted {
                return Some(self.current_cleanup_event());
            }
            return None;
        }

        if let Some(state) = retained_models
            .iter()
            .map(|retained| retained.cleanup)
            .find(|cleanup| {
                cleanup.resource == resource || belongs_to_correlated_unload(cleanup.resource)
            })
        {
            let was_exhausted = self.state.retained_model().is_some_and(|retained| {
                matches!(
                    retained.cleanup(),
                    ApplicationModelCleanupDisposition::LowerExhausted { .. }
                )
            });
            self.begin_runtime_retention(state, None);
            if state.exhausted() && !was_exhausted {
                return Some(self.current_cleanup_event());
            }
            return None;
        }

        if correlated_handle.is_some() {
            self.retained_model_cleanup = Some(RetainedModelCleanup {
                resource,
                inspection: RetainedModelInspection::PendingSubmission { attempts: 0 },
            });
            if let Some(retained) = self.state.retained_model_mut() {
                retained.set_cleanup(ApplicationModelCleanupDisposition::Pending, None);
            }
            self.state.begin_retained_cleanup();
            return None;
        }

        let failure = ApplicationFailure::new(
            ApplicationFailureKind::RetainedCleanup,
            "retained cleanup inspection did not contain the expected owner or an explicit release record",
        );
        self.retained_model_cleanup = Some(RetainedModelCleanup {
            resource,
            inspection: RetainedModelInspection::CoordinationRetryAvailable {
                attempts: MAXIMUM_LOAD_CLEANUP_INSPECTION_SUBMISSION_ATTEMPTS,
            },
        });
        if let Some(retained) = self.state.retained_model_mut() {
            retained.set_cleanup(
                ApplicationModelCleanupDisposition::CoordinationRetryAvailable {
                    attempts: MAXIMUM_LOAD_CLEANUP_INSPECTION_SUBMISSION_ATTEMPTS,
                    maximum_attempts: MAXIMUM_LOAD_CLEANUP_INSPECTION_SUBMISSION_ATTEMPTS,
                },
                Some(failure),
            );
        }
        self.state.begin_retained_cleanup();
        Some(self.current_cleanup_event())
    }

    fn model_unload_correlation_exists(&self, handle: ModelHandle) -> bool {
        self.pending_unload
            .is_some_and(|transaction| transaction.handle == handle)
            || self
                .incompatible_model_cleanup
                .as_ref()
                .is_some_and(|cleanup| cleanup.handle == handle)
    }

    pub(super) fn begin_runtime_retention(
        &mut self,
        cleanup: CleanupRetryState,
        primary_override: Option<ApplicationFailure>,
    ) {
        let primary_failure = primary_override
            .or_else(|| self.retained_primary_failure(cleanup.resource))
            .unwrap_or_else(|| application_primary_failure(cleanup.failure));
        let (public_resource, public_ownership) = self.retained_model_evidence(cleanup);
        if cleanup.ownership.is_released() {
            self.retained_model_cleanup = Some(RetainedModelCleanup {
                resource: cleanup.resource,
                inspection: RetainedModelInspection::PendingSubmission { attempts: 0 },
            });
            self.state.set_retained_model(ApplicationRetainedModel::new(
                public_resource,
                ApplicationRetainedOwnership::Unknown,
                ApplicationModelCleanupDisposition::Pending,
                primary_failure,
                Some(ApplicationFailure::from_debug(
                    ApplicationFailureKind::RetainedCleanup,
                    "lower cleanup failure contradicted its released-ownership claim",
                    cleanup.failure,
                )),
            ));
            return;
        }
        let retained = ApplicationRetainedModel::new(
            public_resource,
            public_ownership,
            application_cleanup_disposition(cleanup),
            primary_failure,
            Some(application_cleanup_failure(cleanup.failure)),
        );
        let inspection = if cleanup.exhausted() {
            RetainedModelInspection::LowerExhausted
        } else {
            RetainedModelInspection::PendingSubmission { attempts: 0 }
        };
        self.retained_model_cleanup = Some(RetainedModelCleanup {
            resource: cleanup.resource,
            inspection,
        });
        self.state.set_retained_model(retained);
    }

    fn retained_model_evidence(
        &self,
        cleanup: CleanupRetryState,
    ) -> (
        ApplicationRetainedModelResource,
        ApplicationRetainedOwnership,
    ) {
        let CleanupResource::Sequence { handle, .. } = cleanup.resource else {
            return (
                application_cleanup_resource(cleanup.resource),
                application_retained_ownership(cleanup.ownership),
            );
        };
        if let Some(loaded) = self
            .state
            .loaded()
            .filter(|loaded| loaded.handle() == handle)
        {
            return (
                ApplicationRetainedModelResource::LoadedModel { handle },
                ApplicationRetainedOwnership::Exact(loaded.reserved_footprint()),
            );
        }
        if let Some(retained) = self.state.retained_model().filter(|retained| {
            retained.resource() == ApplicationRetainedModelResource::LoadedModel { handle }
        }) {
            return (retained.resource(), retained.ownership());
        }
        (
            ApplicationRetainedModelResource::UnconfirmedModel,
            ApplicationRetainedOwnership::Unknown,
        )
    }

    fn retained_primary_failure(&self, resource: CleanupResource) -> Option<ApplicationFailure> {
        if let CleanupResource::IncompatibleModel { handle } = resource
            && let Some(cleanup) = self
                .incompatible_model_cleanup
                .as_ref()
                .filter(|cleanup| cleanup.handle == handle)
        {
            return Some(cleanup.compatibility_failure.clone());
        }
        let public_resource = application_cleanup_resource(resource);
        self.state
            .retained_model()
            .filter(|retained| {
                retained.resource() == public_resource
                    || matches!(
                        resource,
                        CleanupResource::Sequence { handle, .. }
                            if retained.resource()
                                == ApplicationRetainedModelResource::LoadedModel { handle }
                    )
            })
            .map(|retained| retained.primary_failure().clone())
    }

    pub(super) fn reject_incompatible_model(
        &mut self,
        handle: ModelHandle,
        reserved_footprint: MemoryFootprint,
        failure: ApplicationFailure,
    ) -> ApplicationEvent {
        self.incompatible_model_cleanup = Some(IncompatibleModelCleanup {
            handle,
            compatibility_failure: failure.clone(),
            unload: IncompatibleModelUnload::PendingSubmission {
                attempts: 0,
                last_failure: None,
            },
        });
        self.state.set_retained_model(ApplicationRetainedModel::new(
            ApplicationRetainedModelResource::LoadedModel { handle },
            ApplicationRetainedOwnership::Exact(reserved_footprint.into()),
            ApplicationModelCleanupDisposition::Pending,
            failure.clone(),
            None,
        ));

        match self.try_submit_incompatible_model_unload() {
            Ok(()) => ApplicationEvent::ModelCompatibilityFailed { failure },
            Err(_) => self.current_cleanup_event(),
        }
    }

    pub(super) fn retry_incompatible_model_cleanup(&mut self) -> Option<ApplicationEvent> {
        if !matches!(
            self.incompatible_model_cleanup
                .as_ref()
                .map(|cleanup| &cleanup.unload),
            Some(IncompatibleModelUnload::PendingSubmission { .. })
        ) {
            return None;
        }
        match self.try_submit_incompatible_model_unload() {
            Ok(()) => None,
            Err(_) => Some(self.current_cleanup_event()),
        }
    }

    fn try_submit_incompatible_model_unload(&mut self) -> Result<(), ApplicationError> {
        let Some((handle, attempts)) =
            self.incompatible_model_cleanup
                .as_ref()
                .and_then(|cleanup| match &cleanup.unload {
                    IncompatibleModelUnload::PendingSubmission { attempts, .. } => {
                        Some((cleanup.handle, *attempts))
                    }
                    IncompatibleModelUnload::Submitted { .. }
                    | IncompatibleModelUnload::RetryExhausted { .. }
                    | IncompatibleModelUnload::WorkerDisconnected => None,
                })
        else {
            return Ok(());
        };
        let attempt = attempts.saturating_add(1);

        match self.submit_model_unload(handle, crate::ModelUnloadBehavior::Drain) {
            Ok(ticket) => {
                if let Some(cleanup) = self.incompatible_model_cleanup.as_mut() {
                    cleanup.unload = IncompatibleModelUnload::Submitted {
                        ticket,
                        attempts: attempt,
                        last_failure: None,
                    };
                }
                if let Some(retained) = self.state.retained_model_mut() {
                    retained.set_cleanup(ApplicationModelCleanupDisposition::Pending, None);
                }
                self.state.begin_retained_cleanup();
                Ok(())
            }
            Err(error) => {
                let failure = ApplicationFailure::new(
                    ApplicationFailureKind::RetainedCleanup,
                    format!("automatic retained-model unload submission failed: {error}"),
                );
                if let Some(cleanup) = self.incompatible_model_cleanup.as_mut() {
                    cleanup.unload = if matches!(error, ApplicationError::RuntimeDisconnected) {
                        IncompatibleModelUnload::WorkerDisconnected
                    } else if attempt >= MAXIMUM_INCOMPATIBLE_UNLOAD_SUBMISSION_ATTEMPTS {
                        IncompatibleModelUnload::RetryExhausted {
                            attempts: attempt,
                            last_failure: failure.clone(),
                        }
                    } else {
                        IncompatibleModelUnload::PendingSubmission {
                            attempts: attempt,
                            last_failure: Some(failure.clone()),
                        }
                    };
                }
                if !matches!(error, ApplicationError::RuntimeDisconnected) {
                    let disposition = if attempt >= MAXIMUM_INCOMPATIBLE_UNLOAD_SUBMISSION_ATTEMPTS
                    {
                        ApplicationModelCleanupDisposition::CoordinationRetryAvailable {
                            attempts: attempt,
                            maximum_attempts: MAXIMUM_INCOMPATIBLE_UNLOAD_SUBMISSION_ATTEMPTS,
                        }
                    } else {
                        ApplicationModelCleanupDisposition::Pending
                    };
                    if let Some(retained) = self.state.retained_model_mut() {
                        retained.set_cleanup(disposition, Some(failure));
                    }
                    self.state.begin_retained_cleanup();
                }
                Err(error)
            }
        }
    }

    pub(super) fn process_model_unload(
        &mut self,
        ticket: CommandTicket,
        result: Result<inference_runtime::UnloadReceipt, RuntimeError>,
    ) -> Option<ApplicationEvent> {
        let incompatible_ticket = self
            .incompatible_model_cleanup
            .as_ref()
            .and_then(|cleanup| match &cleanup.unload {
                IncompatibleModelUnload::Submitted { ticket, .. } => Some(*ticket),
                IncompatibleModelUnload::PendingSubmission { .. }
                | IncompatibleModelUnload::RetryExhausted { .. }
                | IncompatibleModelUnload::WorkerDisconnected => None,
            });
        if incompatible_ticket == Some(ticket) {
            return Some(self.process_incompatible_model_unload(ticket, result));
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
                    self.retained_model_cleanup = None;
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
                self.begin_runtime_retention(cleanup, None);
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
        self.incompatible_model_cleanup = Some(IncompatibleModelCleanup {
            handle,
            compatibility_failure: failure.clone(),
            unload: IncompatibleModelUnload::PendingSubmission {
                attempts: 0,
                last_failure: None,
            },
        });
        self.state.set_retained_model(ApplicationRetainedModel::new(
            ApplicationRetainedModelResource::LoadedModel { handle },
            ApplicationRetainedOwnership::Unknown,
            ApplicationModelCleanupDisposition::Pending,
            failure,
            None,
        ));
        let _submission = self.try_submit_incompatible_model_unload();
        self.current_cleanup_event()
    }

    fn process_incompatible_model_unload(
        &mut self,
        ticket: CommandTicket,
        result: Result<inference_runtime::UnloadReceipt, RuntimeError>,
    ) -> ApplicationEvent {
        let expected_handle = self
            .incompatible_model_cleanup
            .as_ref()
            .map(|cleanup| cleanup.handle);
        let attempts = self
            .incompatible_model_cleanup
            .as_ref()
            .and_then(|cleanup| match cleanup.unload {
                IncompatibleModelUnload::Submitted { attempts, .. } => Some(attempts),
                _ => None,
            })
            .unwrap_or(1);
        match result {
            Ok(receipt) if expected_handle != Some(receipt.handle) => self
                .record_incompatible_command_failure(
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
                    self.release_incompatible_model_cleanup();
                    self.retained_model_cleanup = None;
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
            ) => {
                let primary = self
                    .incompatible_model_cleanup
                    .as_ref()
                    .map(|cleanup| cleanup.compatibility_failure.clone());
                self.begin_runtime_retention(cleanup, primary);
                if let Some(incompatible) = self.incompatible_model_cleanup.as_mut() {
                    incompatible.unload = IncompatibleModelUnload::Submitted {
                        ticket,
                        attempts,
                        last_failure: self
                            .state
                            .retained_model()
                            .and_then(ApplicationRetainedModel::cleanup_failure)
                            .cloned(),
                    };
                }
                self.current_cleanup_event()
            }
            Err(error) => self.record_incompatible_command_failure(
                attempts,
                ApplicationFailure::from_debug(
                    ApplicationFailureKind::RetainedCleanup,
                    "automatic retained-model unload failed",
                    error,
                ),
            ),
        }
    }

    fn record_incompatible_command_failure(
        &mut self,
        attempts: u8,
        failure: ApplicationFailure,
    ) -> ApplicationEvent {
        if let Some(cleanup) = self.incompatible_model_cleanup.as_mut() {
            cleanup.unload = if attempts >= MAXIMUM_INCOMPATIBLE_UNLOAD_SUBMISSION_ATTEMPTS {
                IncompatibleModelUnload::RetryExhausted {
                    attempts,
                    last_failure: failure.clone(),
                }
            } else {
                IncompatibleModelUnload::PendingSubmission {
                    attempts,
                    last_failure: Some(failure.clone()),
                }
            };
        }
        let disposition = if attempts >= MAXIMUM_INCOMPATIBLE_UNLOAD_SUBMISSION_ATTEMPTS {
            ApplicationModelCleanupDisposition::CoordinationRetryAvailable {
                attempts,
                maximum_attempts: MAXIMUM_INCOMPATIBLE_UNLOAD_SUBMISSION_ATTEMPTS,
            }
        } else {
            ApplicationModelCleanupDisposition::Pending
        };
        if let Some(retained) = self.state.retained_model_mut() {
            retained.set_cleanup(disposition, Some(failure));
        }
        self.state.begin_retained_cleanup();
        self.current_cleanup_event()
    }

    pub(crate) fn mark_model_worker_disconnected(&mut self) {
        let disconnect_failure = ApplicationFailure::new(
            ApplicationFailureKind::Worker,
            "inference worker disconnected without proving model ownership release",
        );
        if let Some(retained) = self.state.retained_model_mut() {
            retained.set_cleanup(
                ApplicationModelCleanupDisposition::WorkerDisconnected,
                Some(disconnect_failure),
            );
        } else if let Some(loaded) = self.state.loaded().cloned() {
            self.state.set_retained_model(ApplicationRetainedModel::new(
                ApplicationRetainedModelResource::LoadedModel {
                    handle: loaded.handle(),
                },
                ApplicationRetainedOwnership::Exact(loaded.reserved_footprint()),
                ApplicationModelCleanupDisposition::WorkerDisconnected,
                disconnect_failure.clone(),
                None,
            ));
        } else if self.pending_load.is_some()
            || matches!(
                self.state.activity(),
                ApplicationActivity::Loading | ApplicationActivity::Unloading
            )
        {
            self.state.set_retained_model(ApplicationRetainedModel::new(
                ApplicationRetainedModelResource::UnconfirmedLoad,
                ApplicationRetainedOwnership::Unknown,
                ApplicationModelCleanupDisposition::WorkerDisconnected,
                disconnect_failure,
                None,
            ));
        }
        self.pending_load = None;
        self.pending_unload = None;
        if let Some(cleanup) = self.incompatible_model_cleanup.as_mut() {
            cleanup.unload = IncompatibleModelUnload::WorkerDisconnected;
        }
        if let Some(cleanup) = self.retained_model_cleanup.as_mut() {
            cleanup.inspection = RetainedModelInspection::WorkerDisconnected;
        }
    }

    pub(crate) fn mark_terminal_worker_failure(&mut self, error: RuntimeError) {
        let had_model_evidence = self.state.loaded().is_some()
            || self.state.retained_model().is_some()
            || self.pending_load.is_some()
            || self.pending_unload.is_some();
        if let RuntimeError::CleanupFailed(cleanup) | RuntimeError::CleanupRetryExhausted(cleanup) =
            error
        {
            self.begin_runtime_retention(cleanup, None);
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
        if let Some(retained) = self.state.retained_model_mut() {
            retained.set_cleanup(
                ApplicationModelCleanupDisposition::RetainedUntilProcessExit,
                Some(failure),
            );
        } else if had_model_evidence {
            self.state.set_retained_model(ApplicationRetainedModel::new(
                ApplicationRetainedModelResource::UnconfirmedModel,
                ApplicationRetainedOwnership::Unknown,
                ApplicationModelCleanupDisposition::RetainedUntilProcessExit,
                failure,
                None,
            ));
        }
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
        if let Some(retained) = self.state.retained_model_mut() {
            retained.set_cleanup(
                ApplicationModelCleanupDisposition::RetainedUntilProcessExit,
                None,
            );
            return;
        }
        let mut retained = if is_model_resource(first.resource) {
            application_retained_model(first, None)
        } else {
            ApplicationRetainedModel::new(
                ApplicationRetainedModelResource::UnconfirmedModel,
                ApplicationRetainedOwnership::Unknown,
                ApplicationModelCleanupDisposition::RetainedUntilProcessExit,
                ApplicationFailure::new(
                    ApplicationFailureKind::RetainedCleanup,
                    "terminal shutdown retained model ownership until process exit",
                ),
                None,
            )
        };
        retained.set_cleanup(
            ApplicationModelCleanupDisposition::RetainedUntilProcessExit,
            None,
        );
        self.state.set_retained_model(retained);
    }

    fn release_retained_model(&mut self, resource: CleanupResource) -> ApplicationEvent {
        let public_resource = self.state.retained_model().map_or_else(
            || application_cleanup_resource(resource),
            ApplicationRetainedModel::resource,
        );
        self.retained_model_cleanup = None;
        self.release_incompatible_model_cleanup();
        self.state.clear_retained_model();
        ApplicationEvent::ModelCleanupReleased {
            resource: public_resource,
        }
    }

    pub(super) fn current_cleanup_event(&self) -> ApplicationEvent {
        let cleanup = self.state.retained_model().cloned().unwrap_or_else(|| {
            ApplicationRetainedModel::new(
                ApplicationRetainedModelResource::UnconfirmedModel,
                ApplicationRetainedOwnership::Unknown,
                ApplicationModelCleanupDisposition::WorkerDisconnected,
                ApplicationFailure::new(
                    ApplicationFailureKind::RetainedCleanup,
                    "model ownership remains unresolved",
                ),
                None,
            )
        });
        ApplicationEvent::ModelCleanupPending { cleanup }
    }

    pub(crate) fn release_incompatible_model_cleanup(&mut self) {
        self.incompatible_model_cleanup = None;
    }

    pub(crate) fn confirm_runtime_shutdown_released(&mut self) {
        self.pending_load = None;
        self.pending_unload = None;
        self.retained_model_cleanup = None;
        self.release_incompatible_model_cleanup();
        self.generation.confirm_runtime_shutdown();
        self.state.confirm_runtime_shutdown_released();
    }
}

fn application_retained_model(
    cleanup: CleanupRetryState,
    primary_override: Option<ApplicationFailure>,
) -> ApplicationRetainedModel {
    let primary_failure =
        primary_override.unwrap_or_else(|| application_primary_failure(cleanup.failure));
    ApplicationRetainedModel::new(
        application_cleanup_resource(cleanup.resource),
        application_retained_ownership(cleanup.ownership),
        application_cleanup_disposition(cleanup),
        primary_failure,
        Some(application_cleanup_failure(cleanup.failure)),
    )
}

const fn cleanup_resource_handle(resource: CleanupResource) -> ModelHandle {
    match resource {
        CleanupResource::Model { handle }
        | CleanupResource::IncompatibleModel { handle }
        | CleanupResource::FailedLoad { handle }
        | CleanupResource::Sequence { handle, .. } => handle,
    }
}

fn cleanup_resource_belongs_to_model(
    resource: CleanupResource,
    expected_handle: ModelHandle,
) -> bool {
    match resource {
        CleanupResource::Model { handle } | CleanupResource::Sequence { handle, .. } => {
            handle == expected_handle
        }
        CleanupResource::IncompatibleModel { .. } | CleanupResource::FailedLoad { .. } => false,
    }
}

const fn is_model_resource(resource: CleanupResource) -> bool {
    matches!(
        resource,
        CleanupResource::Model { .. }
            | CleanupResource::IncompatibleModel { .. }
            | CleanupResource::FailedLoad { .. }
    )
}

const fn application_cleanup_resource(
    resource: CleanupResource,
) -> ApplicationRetainedModelResource {
    match resource {
        CleanupResource::Model { handle } => {
            ApplicationRetainedModelResource::LoadedModel { handle }
        }
        CleanupResource::IncompatibleModel { handle } => {
            ApplicationRetainedModelResource::IncompatibleModel { handle }
        }
        CleanupResource::FailedLoad { handle } => {
            ApplicationRetainedModelResource::FailedLoad { handle }
        }
        CleanupResource::Sequence { .. } => ApplicationRetainedModelResource::UnconfirmedModel,
    }
}

const fn application_cleanup_disposition(
    cleanup: CleanupRetryState,
) -> ApplicationModelCleanupDisposition {
    if cleanup.exhausted() {
        ApplicationModelCleanupDisposition::LowerExhausted {
            attempts: cleanup.attempts,
            maximum_attempts: cleanup.maximum_attempts,
        }
    } else {
        ApplicationModelCleanupDisposition::LowerRetryable {
            attempts: cleanup.attempts,
            maximum_attempts: cleanup.maximum_attempts,
        }
    }
}

fn application_retained_ownership(ownership: RetainedOwnership) -> ApplicationRetainedOwnership {
    match ownership {
        RetainedOwnership::Released => ApplicationRetainedOwnership::Unknown,
        RetainedOwnership::Exact(footprint) => {
            ApplicationRetainedOwnership::Exact(footprint.into())
        }
        RetainedOwnership::Unverified {
            accepted_loading_peak,
            reported_footprint,
            conservative_footprint,
        } => ApplicationRetainedOwnership::Unverified {
            accepted_loading_peak: accepted_loading_peak.into(),
            reported_footprint: reported_footprint.into(),
            conservative_footprint: match conservative_footprint {
                ConservativeFootprint::Known(footprint) => {
                    ApplicationConservativeFootprint::Known(footprint.into())
                }
                ConservativeFootprint::Overflow => ApplicationConservativeFootprint::Overflow,
            },
        },
    }
}

fn application_primary_failure(report: CleanupFailureReport) -> ApplicationFailure {
    let kind = match report.primary_failure {
        inference_runtime::FailureClass::Load => ApplicationFailureKind::ModelLoad,
        inference_runtime::FailureClass::Capacity => ApplicationFailureKind::MemoryAdmission,
        inference_runtime::FailureClass::BackendContract => {
            ApplicationFailureKind::IncompatibleReceipt
        }
        _ => ApplicationFailureKind::Inference,
    };
    ApplicationFailure::from_debug(
        kind,
        "model operation failed before retained cleanup",
        (report.primary_operation, report.primary_detail),
    )
}

fn application_cleanup_failure(report: CleanupFailureReport) -> ApplicationFailure {
    ApplicationFailure::from_debug(
        ApplicationFailureKind::RetainedCleanup,
        "explicit lower cleanup failed",
        (report.cleanup_operation, report.cleanup_detail),
    )
}
