//! Retained ownership accounting for failed loads and incompatible load receipts.

use domain_contracts::{MemoryFootprint, ModelHandle};
use inference_runtime::{CommandTicket, RuntimeCommand, UnloadStatus};

use crate::{
    ApplicationActivity, ApplicationError, ApplicationEvent, ApplicationFailure,
    ApplicationFailureKind, ApplicationRuntime,
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
        last_failure: Option<ApplicationFailure>,
        retry_exhausted: bool,
    },
    RetryExhausted {
        attempts: u8,
        last_failure: ApplicationFailure,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum RetainedLoadCleanup {
    PendingInspection { submission_attempts: u8 },
    InspectionSubmitted { ticket: CommandTicket },
    Exhausted,
}

impl ApplicationRuntime {
    pub(super) fn retry_retained_load_cleanup_inspection(&mut self) -> Option<ApplicationEvent> {
        let submission_attempts = self
            .retained_load_cleanup
            .and_then(|cleanup| match cleanup {
                RetainedLoadCleanup::PendingInspection {
                    submission_attempts,
                } => Some(submission_attempts),
                RetainedLoadCleanup::InspectionSubmitted { .. }
                | RetainedLoadCleanup::Exhausted => None,
            })?;
        let attempt = submission_attempts.saturating_add(1);
        let ticket = match self.next_ticket() {
            Ok(ticket) => ticket,
            Err(error) => return Some(self.exhaust_load_cleanup_inspection(&error)),
        };
        match self.submit_inference(RuntimeCommand::Snapshot { ticket }) {
            Ok(()) => {
                self.retained_load_cleanup =
                    Some(RetainedLoadCleanup::InspectionSubmitted { ticket });
                None
            }
            Err(ApplicationError::RuntimeBusy)
                if attempt < MAXIMUM_LOAD_CLEANUP_INSPECTION_SUBMISSION_ATTEMPTS =>
            {
                self.retained_load_cleanup = Some(RetainedLoadCleanup::PendingInspection {
                    submission_attempts: attempt,
                });
                None
            }
            Err(ApplicationError::RuntimeDisconnected) => {
                self.retained_load_cleanup = None;
                if self.state.activity() == ApplicationActivity::Unloading {
                    self.state.set_idle();
                }
                Some(ApplicationEvent::RuntimeDisconnected)
            }
            Err(error) => Some(self.exhaust_load_cleanup_inspection(&error)),
        }
    }

    fn exhaust_load_cleanup_inspection(&mut self, error: &ApplicationError) -> ApplicationEvent {
        self.retained_load_cleanup = Some(RetainedLoadCleanup::Exhausted);
        self.state.begin_unloading();
        ApplicationEvent::ModelLoadFailed {
            failure: ApplicationFailure::new(
                ApplicationFailureKind::Inference,
                format!(
                    "retained model-load cleanup could not be inspected; device selection remains locked: {error}"
                ),
            ),
        }
    }

    pub(super) fn process_retained_load_cleanup_snapshot(
        &mut self,
        ticket: CommandTicket,
        snapshot: &inference_runtime::RuntimeSnapshot,
    ) {
        if !matches!(
            self.retained_load_cleanup,
            Some(RetainedLoadCleanup::InspectionSubmitted { ticket: expected }) if expected == ticket
        ) {
            return;
        }
        let ownership_released = snapshot.loaded_models == 0
            && snapshot.active_requests == 0
            && snapshot.generation_workspaces == 0
            && snapshot.pending_cleanup_models == 0
            && snapshot.pending_cleanup_sequences == 0
            && snapshot.reserved_footprint == MemoryFootprint::default()
            && snapshot.reserved_generation_workspace == MemoryFootprint::default();
        if ownership_released {
            self.retained_load_cleanup = None;
            if self.state.activity() == ApplicationActivity::Unloading {
                self.state.set_idle();
            }
        } else {
            self.retained_load_cleanup = Some(RetainedLoadCleanup::Exhausted);
            self.state.begin_unloading();
        }
    }

    pub(super) fn reject_incompatible_model(&mut self, handle: ModelHandle) -> ApplicationEvent {
        self.state.clear_resolved();
        self.resolved_artifacts = None;
        self.tokenizer = None;
        let failure = ApplicationFailure {
            kind: ApplicationFailureKind::Inference,
            message: "loaded-model compatibility failed because resolved identity, model handle, descriptor, tokenizer, source scalar, execution scalar, selected device, actual device, or reserved footprint evidence differs; deterministic unload was requested".to_owned(),
        };
        self.incompatible_model_cleanup = Some(IncompatibleModelCleanup {
            handle,
            compatibility_failure: failure.clone(),
            unload: IncompatibleModelUnload::PendingSubmission {
                attempts: 0,
                last_failure: None,
            },
        });
        self.state.begin_unloading();

        match self.try_submit_incompatible_model_unload() {
            Ok(()) => ApplicationEvent::ModelCompatibilityFailed { failure },
            Err(error) => {
                if self.incompatible_model_cleanup.is_none() {
                    self.state.set_idle();
                }
                ApplicationEvent::ModelLoadFailed {
                    failure: Self::incompatible_unload_submission_failure(
                        &failure,
                        &error,
                        self.incompatible_unload_retry_exhausted(),
                    ),
                }
            }
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
        let compatibility_failure = self
            .incompatible_model_cleanup
            .as_ref()
            .map(|cleanup| cleanup.compatibility_failure.clone())?;

        match self.try_submit_incompatible_model_unload() {
            Ok(()) => None,
            Err(_error) if self.incompatible_model_cleanup.is_none() => {
                if self.state.activity() == ApplicationActivity::Unloading {
                    self.state.set_idle();
                }
                Some(ApplicationEvent::RuntimeDisconnected)
            }
            Err(error) => Some(ApplicationEvent::ModelUnloadFailed {
                failure: Self::incompatible_unload_submission_failure(
                    &compatibility_failure,
                    &error,
                    self.incompatible_unload_retry_exhausted(),
                ),
            }),
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
                    | IncompatibleModelUnload::RetryExhausted { .. } => None,
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
                        last_failure: None,
                        retry_exhausted: false,
                    };
                }
                Ok(())
            }
            Err(error) => {
                if let Some(cleanup) = self.incompatible_model_cleanup.as_mut() {
                    let failure = ApplicationFailure {
                        kind: ApplicationFailureKind::Inference,
                        message: format!(
                            "automatic incompatible-model unload submission attempt {attempt}/{MAXIMUM_INCOMPATIBLE_UNLOAD_SUBMISSION_ATTEMPTS} failed: {error}"
                        ),
                    };
                    cleanup.unload = if attempt >= MAXIMUM_INCOMPATIBLE_UNLOAD_SUBMISSION_ATTEMPTS {
                        IncompatibleModelUnload::RetryExhausted {
                            attempts: attempt,
                            last_failure: failure,
                        }
                    } else {
                        IncompatibleModelUnload::PendingSubmission {
                            attempts: attempt,
                            last_failure: Some(failure),
                        }
                    };
                    self.state.begin_unloading();
                }
                Err(error)
            }
        }
    }

    fn incompatible_unload_retry_exhausted(&self) -> bool {
        matches!(
            self.incompatible_model_cleanup
                .as_ref()
                .map(|cleanup| &cleanup.unload),
            Some(IncompatibleModelUnload::RetryExhausted { .. })
        )
    }

    fn incompatible_unload_submission_failure(
        compatibility_failure: &ApplicationFailure,
        error: &ApplicationError,
        exhausted: bool,
    ) -> ApplicationFailure {
        let disposition = if exhausted {
            "automatic unload submission retries are exhausted; private model ownership remains retained"
        } else {
            "automatic unload submission will be retried"
        };
        ApplicationFailure {
            kind: ApplicationFailureKind::Inference,
            message: format!("{compatibility_failure}; {disposition}: {error}"),
        }
    }

    pub(super) fn process_model_unload(
        &mut self,
        ticket: CommandTicket,
        result: Result<inference_runtime::UnloadReceipt, inference_runtime::RuntimeError>,
    ) -> ApplicationEvent {
        let incompatible_ticket = self
            .incompatible_model_cleanup
            .as_ref()
            .and_then(|cleanup| match &cleanup.unload {
                IncompatibleModelUnload::Submitted { ticket, .. } => Some(*ticket),
                IncompatibleModelUnload::PendingSubmission { .. }
                | IncompatibleModelUnload::RetryExhausted { .. } => None,
            });
        if incompatible_ticket == Some(ticket) {
            return self.process_incompatible_model_unload(ticket, result);
        }

        match result {
            Ok(receipt) => match receipt.status {
                UnloadStatus::Draining => ApplicationEvent::ModelDraining {
                    handle: receipt.handle,
                },
                UnloadStatus::AlreadyAbsent | UnloadStatus::Unloaded => {
                    self.state.clear_loaded();
                    ApplicationEvent::ModelUnloaded {
                        handle: receipt.handle,
                        cancelled_requests: receipt.cancelled_requests,
                    }
                }
            },
            Err(error) => {
                self.state.set_idle();
                ApplicationEvent::ModelUnloadFailed {
                    failure: ApplicationFailure::from_debug(
                        ApplicationFailureKind::Inference,
                        "model unload failed",
                        error,
                    ),
                }
            }
        }
    }

    fn process_incompatible_model_unload(
        &mut self,
        ticket: CommandTicket,
        result: Result<inference_runtime::UnloadReceipt, inference_runtime::RuntimeError>,
    ) -> ApplicationEvent {
        let expected_handle = self
            .incompatible_model_cleanup
            .as_ref()
            .map(|cleanup| cleanup.handle);
        match result {
            Ok(receipt) if expected_handle != Some(receipt.handle) => {
                let failure = ApplicationFailure {
                    kind: ApplicationFailureKind::Inference,
                    message:
                        "automatic incompatible-model unload returned a different model handle"
                            .to_owned(),
                };
                self.record_incompatible_unload_failure(ticket, failure, false)
            }
            Ok(receipt) => match receipt.status {
                UnloadStatus::Draining => ApplicationEvent::ModelDraining {
                    handle: receipt.handle,
                },
                UnloadStatus::AlreadyAbsent | UnloadStatus::Unloaded => {
                    self.release_incompatible_model_cleanup();
                    self.state.clear_loaded();
                    ApplicationEvent::ModelUnloaded {
                        handle: receipt.handle,
                        cancelled_requests: receipt.cancelled_requests,
                    }
                }
            },
            Err(error) => {
                let retry_exhausted = matches!(
                    error,
                    inference_runtime::RuntimeError::CleanupRetryExhausted(_)
                );
                let failure = ApplicationFailure::from_debug(
                    ApplicationFailureKind::Inference,
                    "automatic incompatible-model unload failed",
                    error,
                );
                self.record_incompatible_unload_failure(ticket, failure, retry_exhausted)
            }
        }
    }

    fn record_incompatible_unload_failure(
        &mut self,
        ticket: CommandTicket,
        failure: ApplicationFailure,
        retry_exhausted: bool,
    ) -> ApplicationEvent {
        if let Some(cleanup) = self.incompatible_model_cleanup.as_mut() {
            cleanup.unload = IncompatibleModelUnload::Submitted {
                ticket,
                last_failure: Some(failure.clone()),
                retry_exhausted,
            };
            self.state.begin_unloading();
        }
        ApplicationEvent::ModelUnloadFailed { failure }
    }

    pub(crate) fn release_incompatible_model_cleanup(&mut self) {
        self.incompatible_model_cleanup = None;
    }
}
