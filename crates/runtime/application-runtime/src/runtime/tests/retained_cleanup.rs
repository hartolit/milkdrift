use domain_contracts::{
    MemoryFootprint, ModelGeneration, ModelHandle, ModelId, RequestId, ScalarType, SequenceId,
};
use inference_runtime::{
    CleanupFailureReport, CleanupResource, CleanupRetryState, CommandTicket, ConservativeFootprint,
    FailureClass, RetainedModelSnapshot, RetainedOwnership, RuntimeCommand, RuntimeError,
    RuntimeEvent, RuntimeOperation, RuntimeSnapshot, UnloadReceipt, UnloadStatus,
};

use super::support::*;
use crate::runtime::retained_cleanup::{
    IncompatibleModelUnload, RetainedModelCleanup, RetainedModelInspection,
};
use crate::unload::ModelUnloadTransaction;
use crate::{
    ApplicationActivity, ApplicationConservativeFootprint, ApplicationError, ApplicationEvent,
    ApplicationFailure, ApplicationFailureKind, ApplicationMemoryFootprint,
    ApplicationModelCleanupDisposition, ApplicationRetainedModel, ApplicationRetainedModelResource,
    ApplicationRetainedOwnership, ApplicationRuntime, ApplicationRuntimeConfiguration,
    ApplicationWorker,
};

const RETAINED_HANDLE: ModelHandle = ModelHandle::new(ModelId::new(7), ModelGeneration::new(3));
const EXACT_FOOTPRINT: MemoryFootprint = MemoryFootprint {
    host_weight_bytes: 11,
    device_weight_bytes: 13,
    host_working_bytes: 17,
    device_working_bytes: 19,
};
const UPDATED_FOOTPRINT: MemoryFootprint = MemoryFootprint {
    host_weight_bytes: 23,
    device_weight_bytes: 29,
    host_working_bytes: 31,
    device_working_bytes: 37,
};
const ACCEPTED_LOADING_PEAK: MemoryFootprint = MemoryFootprint {
    host_weight_bytes: 41,
    device_weight_bytes: 43,
    host_working_bytes: 47,
    device_working_bytes: 53,
};
const REPORTED_FOOTPRINT: MemoryFootprint = MemoryFootprint {
    host_weight_bytes: 59,
    device_weight_bytes: 61,
    host_working_bytes: 67,
    device_working_bytes: 71,
};
const CONSERVATIVE_FOOTPRINT: MemoryFootprint = MemoryFootprint {
    host_weight_bytes: 59,
    device_weight_bytes: 61,
    host_working_bytes: 67,
    device_working_bytes: 71,
};

fn failed_preparation_cleanup(
    ownership: RetainedOwnership,
    attempts: u32,
    maximum_attempts: u32,
) -> CleanupRetryState {
    CleanupRetryState {
        resource: CleanupResource::FailedLoad {
            handle: RETAINED_HANDLE,
        },
        failure: CleanupFailureReport::new(
            RuntimeOperation::ModelLoad,
            FailureClass::Load,
            RuntimeOperation::FailedLoadCleanup,
            FailureClass::Synchronization,
        ),
        ownership,
        attempts,
        maximum_attempts,
    }
}

fn incompatible_lower_cleanup() -> CleanupRetryState {
    CleanupRetryState {
        resource: CleanupResource::IncompatibleModel {
            handle: RETAINED_HANDLE,
        },
        failure: CleanupFailureReport::new(
            RuntimeOperation::ModelLoad,
            FailureClass::BackendContract,
            RuntimeOperation::ModelUnload,
            FailureClass::Synchronization,
        ),
        ownership: RetainedOwnership::Unverified {
            accepted_loading_peak: ACCEPTED_LOADING_PEAK,
            reported_footprint: REPORTED_FOOTPRINT,
            conservative_footprint: ConservativeFootprint::Known(CONSERVATIVE_FOOTPRINT),
        },
        attempts: 1,
        maximum_attempts: 3,
    }
}

fn verified_unload_cleanup(handle: ModelHandle) -> CleanupRetryState {
    CleanupRetryState {
        resource: CleanupResource::Model { handle },
        failure: CleanupFailureReport::new(
            RuntimeOperation::ModelUnload,
            FailureClass::Completion,
            RuntimeOperation::ModelUnload,
            FailureClass::Synchronization,
        ),
        ownership: RetainedOwnership::Exact(EXACT_FOOTPRINT),
        attempts: 1,
        maximum_attempts: 3,
    }
}

fn sequence_cleanup(
    handle: ModelHandle,
    request_id: u64,
    sequence_id: u64,
    attempts: u32,
) -> CleanupRetryState {
    CleanupRetryState {
        resource: CleanupResource::Sequence {
            handle,
            request_id: RequestId::new(request_id),
            sequence_id: SequenceId::new(sequence_id),
        },
        failure: CleanupFailureReport::new(
            RuntimeOperation::Cancellation,
            FailureClass::Cancellation,
            RuntimeOperation::SequenceDestruction,
            FailureClass::Synchronization,
        ),
        ownership: RetainedOwnership::Exact(EXACT_FOOTPRINT),
        attempts,
        maximum_attempts: 3,
    }
}

fn retained_model(runtime: &ApplicationRuntime) -> TestResult<&ApplicationRetainedModel> {
    runtime
        .state()
        .retained_model()
        .ok_or_else(|| "expected retained model ownership".to_owned())
}

fn pending_cleanup(event: Option<ApplicationEvent>) -> TestResult<ApplicationRetainedModel> {
    match event {
        Some(ApplicationEvent::ModelCleanupPending { cleanup }) => Ok(cleanup),
        Some(event) => Err(format!("unexpected retained-cleanup event: {event:?}")),
        None => Err("retained cleanup did not publish an event".to_owned()),
    }
}

fn submit_inspection(
    runtime: &mut ApplicationRuntime,
    resource: CleanupResource,
    ticket: CommandTicket,
) {
    runtime.retained_model_cleanup = Some(RetainedModelCleanup {
        resource,
        inspection: RetainedModelInspection::Submitted { ticket },
    });
}

fn process_snapshot(
    application: &mut ApplicationRuntime,
    ticket: CommandTicket,
    aggregate: RuntimeSnapshot,
    retained: Vec<RetainedModelSnapshot>,
) -> TestResult<Option<ApplicationEvent>> {
    let event = RuntimeEvent::Snapshot {
        ticket,
        runtime: aggregate,
        models: Vec::new(),
        retained_models: retained,
    };
    let RuntimeEvent::Snapshot {
        ticket,
        runtime,
        retained_models,
        ..
    } = event
    else {
        return Err("expected a runtime snapshot event".to_owned());
    };
    Ok(application.process_retained_model_cleanup_snapshot(
        ticket,
        &runtime,
        retained_models.as_slice(),
    ))
}

fn stop_inference_worker(runtime: &mut ApplicationRuntime) -> TestResult {
    let ticket = runtime.next_ticket().map_err(application_error)?;
    runtime
        .submit_inference(RuntimeCommand::Shutdown { ticket })
        .map_err(application_error)?;
    let event = runtime
        .local
        .receive_timeout(TEST_TIMEOUT)
        .map_err(|error| format!("shutdown event failed: {error:?}"))?;
    match event {
        RuntimeEvent::Shutdown {
            ticket: event_ticket,
            result: Ok(_),
        } if event_ticket == ticket => {}
        _ => return Err("unexpected inference shutdown event".to_owned()),
    }
    let thread = runtime
        .local
        .take_thread()
        .ok_or_else(|| "inference thread was already absent".to_owned())?;
    thread
        .join()
        .map_err(|error| format!("inference worker join failed: {error:?}"))
}

#[test]
fn retryable_failed_preparation_retains_exact_ownership_separate_failures_and_locks() -> TestResult
{
    with_runtime(default_test_configuration, |runtime| {
        let (selection, _resolved) =
            resolve_fixture_with(runtime, REPOSITORY, COMMIT, "tokenizer.json")?;
        let lower = failed_preparation_cleanup(RetainedOwnership::Exact(EXACT_FOOTPRINT), 1, 3);

        runtime.begin_runtime_retention(lower, None);
        let cleanup = pending_cleanup(Some(runtime.current_cleanup_event()))?;

        assert_eq!(
            cleanup.resource(),
            ApplicationRetainedModelResource::FailedLoad {
                handle: RETAINED_HANDLE
            }
        );
        assert_eq!(
            cleanup.ownership(),
            ApplicationRetainedOwnership::Exact(ApplicationMemoryFootprint::from(EXACT_FOOTPRINT))
        );
        assert_eq!(
            cleanup.cleanup(),
            ApplicationModelCleanupDisposition::LowerRetryable {
                attempts: 1,
                maximum_attempts: 3,
            }
        );
        assert_eq!(
            cleanup.primary_failure().kind,
            ApplicationFailureKind::ModelLoad
        );
        let cleanup_failure = cleanup
            .cleanup_failure()
            .ok_or_else(|| "cleanup failure was not retained separately".to_owned())?;
        assert_eq!(
            cleanup_failure.kind,
            ApplicationFailureKind::RetainedCleanup
        );
        assert_ne!(cleanup.primary_failure(), cleanup_failure);
        assert_eq!(runtime.state().retained_model(), Some(&cleanup));
        assert!(runtime.state().loaded().is_none());
        assert_eq!(
            runtime.state().activity(),
            ApplicationActivity::RetainedCleanup
        );
        assert!(!runtime.state().can_select_device());
        assert!(!runtime.state().can_load(&selection));
        assert_eq!(
            runtime.select_device(CUDA_ZERO),
            Err(ApplicationError::DeviceSelectionLocked)
        );
        assert_eq!(
            runtime.load_model(&selection),
            Err(ApplicationError::Busy(ApplicationActivity::RetainedCleanup))
        );

        runtime.state.set_idle();
        assert_eq!(
            runtime.load_model(&selection),
            Err(ApplicationError::Busy(ApplicationActivity::RetainedCleanup))
        );
        runtime.state.begin_retained_cleanup();
        Ok(())
    })
}

#[test]
fn exhausted_failed_preparation_is_lower_exhausted_and_nonretryable() -> TestResult {
    with_runtime(default_test_configuration, |runtime| {
        let lower = failed_preparation_cleanup(RetainedOwnership::Exact(EXACT_FOOTPRINT), 3, 3);
        runtime.begin_runtime_retention(lower, None);
        let cleanup = pending_cleanup(Some(runtime.current_cleanup_event()))?;

        assert_eq!(
            cleanup.cleanup(),
            ApplicationModelCleanupDisposition::LowerExhausted {
                attempts: 3,
                maximum_attempts: 3,
            }
        );
        assert_eq!(
            runtime.retained_model_cleanup,
            Some(RetainedModelCleanup {
                resource: lower.resource,
                inspection: RetainedModelInspection::LowerExhausted,
            })
        );
        assert!(!runtime.state().can_retry_model_cleanup());
        assert_eq!(
            runtime.retry_model_cleanup(),
            Err(ApplicationError::ModelCleanupNotRetryable)
        );
        assert!(runtime.state().loaded().is_none());
        Ok(())
    })
}

#[test]
fn retryable_incompatible_lower_model_retains_unverified_ownership() -> TestResult {
    with_runtime(default_test_configuration, |runtime| {
        let lower = incompatible_lower_cleanup();
        runtime.begin_runtime_retention(lower, None);
        let cleanup = pending_cleanup(Some(runtime.current_cleanup_event()))?;

        assert_eq!(
            cleanup.resource(),
            ApplicationRetainedModelResource::IncompatibleModel {
                handle: RETAINED_HANDLE
            }
        );
        assert_eq!(
            cleanup.ownership(),
            ApplicationRetainedOwnership::Unverified {
                accepted_loading_peak: ApplicationMemoryFootprint::from(ACCEPTED_LOADING_PEAK),
                reported_footprint: ApplicationMemoryFootprint::from(REPORTED_FOOTPRINT),
                conservative_footprint: ApplicationConservativeFootprint::Known(
                    ApplicationMemoryFootprint::from(CONSERVATIVE_FOOTPRINT),
                ),
            }
        );
        assert_eq!(
            cleanup.cleanup(),
            ApplicationModelCleanupDisposition::LowerRetryable {
                attempts: 1,
                maximum_attempts: 3,
            }
        );
        assert_eq!(
            cleanup.primary_failure().kind,
            ApplicationFailureKind::IncompatibleReceipt
        );
        assert!(runtime.state().loaded().is_none());
        Ok(())
    })
}

#[test]
fn verified_unload_failure_retains_exact_owner_without_loaded_model() -> TestResult {
    with_loaded_runtime(default_test_configuration, |runtime, loaded| {
        let lower = verified_unload_cleanup(loaded.handle());
        let ticket = CommandTicket::new(73);
        runtime.pending_unload = Some(ModelUnloadTransaction {
            ticket,
            handle: loaded.handle(),
        });
        runtime.state.begin_unloading();

        let cleanup = pending_cleanup(
            runtime.process_model_unload(ticket, Err(RuntimeError::CleanupFailed(lower))),
        )?;

        assert_eq!(
            cleanup.resource(),
            ApplicationRetainedModelResource::LoadedModel {
                handle: loaded.handle()
            }
        );
        assert_eq!(
            cleanup.ownership(),
            ApplicationRetainedOwnership::Exact(ApplicationMemoryFootprint::from(EXACT_FOOTPRINT))
        );
        assert_eq!(runtime.state().retained_model(), Some(&cleanup));
        assert!(runtime.state().loaded().is_none());
        assert!(!runtime.state().can_start_generation());
        assert!(!runtime.state().can_select_device());
        Ok(())
    })
}

#[test]
fn sequence_cleanup_during_unload_keeps_model_locked_until_terminal_receipt() -> TestResult {
    with_loaded_runtime(default_test_configuration, |runtime, loaded| {
        let ticket = CommandTicket::new(74);
        runtime.pending_unload = Some(ModelUnloadTransaction {
            ticket,
            handle: loaded.handle(),
        });
        runtime.state.begin_unloading();
        let lower = sequence_cleanup(loaded.handle(), 9, 11, 1);

        let cleanup = pending_cleanup(
            runtime.process_model_unload(ticket, Err(RuntimeError::CleanupFailed(lower))),
        )?;
        assert_eq!(
            cleanup.resource(),
            ApplicationRetainedModelResource::LoadedModel {
                handle: loaded.handle(),
            }
        );
        assert_eq!(
            cleanup.ownership(),
            ApplicationRetainedOwnership::Exact(loaded.reserved_footprint())
        );
        assert_eq!(
            runtime.pending_unload.map(|pending| pending.ticket),
            Some(ticket)
        );
        assert!(runtime.state().loaded().is_none());

        let retry_ticket = CommandTicket::new(75);
        submit_inspection(runtime, lower.resource, retry_ticket);
        let retry = CleanupRetryState {
            attempts: 2,
            ..lower
        };
        assert_eq!(
            runtime.process_retained_model_cleanup_snapshot(
                retry_ticket,
                &RuntimeSnapshot {
                    last_cleanup: Some(retry),
                    ..RuntimeSnapshot::default()
                },
                &[],
            ),
            None
        );
        assert_eq!(
            retained_model(runtime)?.cleanup(),
            ApplicationModelCleanupDisposition::LowerRetryable {
                attempts: 2,
                maximum_attempts: 3,
            }
        );

        let release_ticket = CommandTicket::new(76);
        submit_inspection(runtime, lower.resource, release_ticket);
        let released = CleanupRetryState {
            ownership: RetainedOwnership::Released,
            attempts: 3,
            ..lower
        };
        let event = runtime.process_retained_model_cleanup_snapshot(
            release_ticket,
            &RuntimeSnapshot {
                last_cleanup: Some(released),
                ..RuntimeSnapshot::default()
            },
            &[],
        );
        let cleanup = pending_cleanup(event)?;
        assert_eq!(
            cleanup.cleanup(),
            ApplicationModelCleanupDisposition::Pending
        );
        assert!(matches!(
            runtime.retained_model_cleanup,
            Some(RetainedModelCleanup {
                resource,
                inspection: RetainedModelInspection::PendingSubmission { attempts: 0 },
            }) if resource == lower.resource
        ));
        assert!(runtime.pending_unload.is_some());

        let event = runtime.process_model_unload(
            ticket,
            Ok(UnloadReceipt {
                handle: loaded.handle(),
                status: UnloadStatus::Unloaded,
                cancelled_requests: 1,
            }),
        );
        assert!(matches!(
            event,
            Some(ApplicationEvent::ModelUnloaded {
                cancelled_requests: 1,
                ..
            })
        ));
        assert!(runtime.state().retained_model().is_none());
        assert!(runtime.state().loaded().is_none());
        assert!(runtime.pending_unload.is_none());
        Ok(())
    })
}

#[test]
fn correlated_unload_adopts_a_successor_cleanup_resource() -> TestResult {
    with_loaded_runtime(default_test_configuration, |runtime, loaded| {
        let unload_ticket = CommandTicket::new(77);
        runtime.pending_unload = Some(ModelUnloadTransaction {
            ticket: unload_ticket,
            handle: loaded.handle(),
        });
        runtime.state.begin_unloading();
        let original = sequence_cleanup(loaded.handle(), 9, 11, 1);
        runtime.begin_runtime_retention(original, None);

        let inspection_ticket = CommandTicket::new(78);
        submit_inspection(runtime, original.resource, inspection_ticket);
        let successor = sequence_cleanup(loaded.handle(), 37, 41, 3);
        let cleanup = pending_cleanup(runtime.process_retained_model_cleanup_snapshot(
            inspection_ticket,
            &RuntimeSnapshot {
                last_cleanup: Some(successor),
                ..RuntimeSnapshot::default()
            },
            &[],
        ))?;
        assert_eq!(
            cleanup.cleanup(),
            ApplicationModelCleanupDisposition::LowerExhausted {
                attempts: 3,
                maximum_attempts: 3,
            }
        );
        assert_eq!(
            runtime.retained_model_cleanup,
            Some(RetainedModelCleanup {
                resource: successor.resource,
                inspection: RetainedModelInspection::LowerExhausted,
            })
        );
        assert!(runtime.pending_unload.is_some());
        assert!(runtime.state().loaded().is_none());
        Ok(())
    })
}

#[test]
fn sequence_snapshot_exhaustion_remains_lower_exhaustion() -> TestResult {
    with_loaded_runtime(default_test_configuration, |runtime, loaded| {
        let unload_ticket = CommandTicket::new(77);
        runtime.pending_unload = Some(ModelUnloadTransaction {
            ticket: unload_ticket,
            handle: loaded.handle(),
        });
        runtime.state.begin_unloading();
        let lower = sequence_cleanup(loaded.handle(), 19, 23, 1);
        runtime.begin_runtime_retention(lower, None);

        let inspection_ticket = CommandTicket::new(78);
        submit_inspection(runtime, lower.resource, inspection_ticket);
        let exhausted = CleanupRetryState {
            attempts: 3,
            ..lower
        };
        let cleanup = pending_cleanup(runtime.process_retained_model_cleanup_snapshot(
            inspection_ticket,
            &RuntimeSnapshot {
                last_cleanup: Some(exhausted),
                ..RuntimeSnapshot::default()
            },
            &[],
        ))?;
        assert_eq!(
            cleanup.cleanup(),
            ApplicationModelCleanupDisposition::LowerExhausted {
                attempts: 3,
                maximum_attempts: 3,
            }
        );
        assert!(!runtime.state().can_retry_model_cleanup());
        assert!(runtime.pending_unload.is_some());
        assert!(runtime.state().loaded().is_none());
        Ok(())
    })
}

#[test]
fn sequence_release_preserves_incompatible_unload_correlation() -> TestResult {
    with_runtime(default_test_configuration, |runtime| {
        let primary = ApplicationFailure::new(
            ApplicationFailureKind::IncompatibleReceipt,
            "unload receipt identity was contradictory",
        );
        runtime.incompatible_model_cleanup =
            Some(crate::runtime::retained_cleanup::IncompatibleModelCleanup {
                handle: RETAINED_HANDLE,
                compatibility_failure: primary.clone(),
                unload: IncompatibleModelUnload::Submitted {
                    ticket: CommandTicket::new(79),
                    attempts: 1,
                    last_failure: None,
                },
            });
        runtime
            .state
            .set_retained_model(ApplicationRetainedModel::new(
                ApplicationRetainedModelResource::LoadedModel {
                    handle: RETAINED_HANDLE,
                },
                ApplicationRetainedOwnership::Unknown,
                ApplicationModelCleanupDisposition::Pending,
                primary,
                None,
            ));
        let lower = sequence_cleanup(RETAINED_HANDLE, 29, 31, 1);
        runtime.begin_runtime_retention(lower, None);

        let inspection_ticket = CommandTicket::new(80);
        submit_inspection(runtime, lower.resource, inspection_ticket);
        let released = CleanupRetryState {
            ownership: RetainedOwnership::Released,
            attempts: 2,
            ..lower
        };
        let cleanup = pending_cleanup(runtime.process_retained_model_cleanup_snapshot(
            inspection_ticket,
            &RuntimeSnapshot {
                last_cleanup: Some(released),
                ..RuntimeSnapshot::default()
            },
            &[],
        ))?;
        assert_eq!(
            cleanup.resource(),
            ApplicationRetainedModelResource::LoadedModel {
                handle: RETAINED_HANDLE,
            }
        );
        assert_eq!(cleanup.ownership(), ApplicationRetainedOwnership::Unknown);
        assert_eq!(
            cleanup.cleanup(),
            ApplicationModelCleanupDisposition::Pending
        );
        assert!(matches!(
            runtime.retained_model_cleanup,
            Some(RetainedModelCleanup {
                resource,
                inspection: RetainedModelInspection::PendingSubmission { attempts: 0 },
            }) if resource == lower.resource
        ));
        assert!(matches!(
            runtime.incompatible_model_cleanup,
            Some(crate::runtime::retained_cleanup::IncompatibleModelCleanup {
                unload: IncompatibleModelUnload::Submitted {
                    ticket,
                    ..
                },
                ..
            }) if ticket == CommandTicket::new(79)
        ));
        assert!(runtime.state().loaded().is_none());
        Ok(())
    })
}

#[test]
fn rejected_load_receipt_starts_exact_retained_loaded_model() -> TestResult {
    with_runtime(default_test_configuration, |runtime| {
        let (selection, _resolved) =
            resolve_fixture_with(runtime, REPOSITORY, COMMIT, "tokenizer.json")?;
        runtime.load_model(&selection).map_err(application_error)?;
        let (ticket, mut receipt) = receive_successful_load_receipt(runtime)?;
        let handle = receipt.handle;
        let footprint = receipt.reserved_footprint;
        receipt.execution_scalar_type = ScalarType::I8;

        let event = runtime.process_model_loaded(ticket, Ok(receipt));
        assert!(matches!(
            event,
            Some(ApplicationEvent::ModelCompatibilityFailed { .. })
        ));
        let cleanup = retained_model(runtime)?;
        assert_eq!(
            cleanup.resource(),
            ApplicationRetainedModelResource::LoadedModel { handle }
        );
        assert_eq!(
            cleanup.ownership(),
            ApplicationRetainedOwnership::Exact(ApplicationMemoryFootprint::from(footprint))
        );
        assert_eq!(
            cleanup.cleanup(),
            ApplicationModelCleanupDisposition::Pending
        );
        assert_eq!(
            cleanup.primary_failure().kind,
            ApplicationFailureKind::IncompatibleReceipt
        );
        assert!(cleanup.cleanup_failure().is_none());
        assert!(matches!(
            runtime
                .incompatible_model_cleanup
                .as_ref()
                .map(|cleanup| &cleanup.unload),
            Some(IncompatibleModelUnload::Submitted {
                attempts: 1,
                last_failure: None,
                ..
            })
        ));
        assert!(runtime.state().loaded().is_none());
        assert!(!runtime.state().can_load(&selection));
        assert!(!runtime.state().can_select_device());
        Ok(())
    })
}

#[test]
fn per_owner_snapshot_updates_retry_then_exhaustion_with_zero_aggregate() -> TestResult {
    with_runtime(default_test_configuration, |runtime| {
        let initial = failed_preparation_cleanup(RetainedOwnership::Exact(EXACT_FOOTPRINT), 1, 3);
        runtime.begin_runtime_retention(initial, None);

        let retry_ticket = CommandTicket::new(74);
        submit_inspection(runtime, initial.resource, retry_ticket);
        let retry = failed_preparation_cleanup(RetainedOwnership::Exact(UPDATED_FOOTPRINT), 2, 3);
        let event = process_snapshot(
            runtime,
            retry_ticket,
            RuntimeSnapshot::default(),
            vec![RetainedModelSnapshot {
                handle: RETAINED_HANDLE,
                cleanup: retry,
            }],
        )?;
        assert!(event.is_none());
        let retained = retained_model(runtime)?;
        assert_eq!(
            retained.ownership(),
            ApplicationRetainedOwnership::Exact(ApplicationMemoryFootprint::from(
                UPDATED_FOOTPRINT
            ))
        );
        assert_eq!(
            retained.cleanup(),
            ApplicationModelCleanupDisposition::LowerRetryable {
                attempts: 2,
                maximum_attempts: 3,
            }
        );
        assert!(matches!(
            runtime.retained_model_cleanup,
            Some(RetainedModelCleanup {
                inspection: RetainedModelInspection::PendingSubmission { attempts: 0 },
                ..
            })
        ));

        let exhausted_ticket = CommandTicket::new(75);
        submit_inspection(runtime, initial.resource, exhausted_ticket);
        let exhausted =
            failed_preparation_cleanup(RetainedOwnership::Exact(UPDATED_FOOTPRINT), 3, 3);
        let event = process_snapshot(
            runtime,
            exhausted_ticket,
            RuntimeSnapshot::default(),
            vec![RetainedModelSnapshot {
                handle: RETAINED_HANDLE,
                cleanup: exhausted,
            }],
        )?;
        let cleanup = pending_cleanup(event)?;
        assert_eq!(
            cleanup.cleanup(),
            ApplicationModelCleanupDisposition::LowerExhausted {
                attempts: 3,
                maximum_attempts: 3,
            }
        );
        assert_eq!(runtime.state().retained_model(), Some(&cleanup));
        assert_eq!(
            runtime.retained_model_cleanup,
            Some(RetainedModelCleanup {
                resource: exhausted.resource,
                inspection: RetainedModelInspection::LowerExhausted,
            })
        );
        assert!(runtime.state().loaded().is_none());
        Ok(())
    })
}

#[test]
fn explicit_released_snapshot_clears_and_emits_model_cleanup_released() -> TestResult {
    with_runtime(default_test_configuration, |runtime| {
        let retained = failed_preparation_cleanup(RetainedOwnership::Exact(EXACT_FOOTPRINT), 1, 3);
        runtime.begin_runtime_retention(retained, None);
        let ticket = CommandTicket::new(76);
        submit_inspection(runtime, retained.resource, ticket);
        let released = failed_preparation_cleanup(RetainedOwnership::Released, 2, 3);

        let event = process_snapshot(
            runtime,
            ticket,
            RuntimeSnapshot {
                last_cleanup: Some(released),
                ..RuntimeSnapshot::default()
            },
            Vec::new(),
        )?;

        assert_eq!(
            event,
            Some(ApplicationEvent::ModelCleanupReleased {
                resource: ApplicationRetainedModelResource::FailedLoad {
                    handle: RETAINED_HANDLE,
                },
            })
        );
        assert!(runtime.state().retained_model().is_none());
        assert!(runtime.retained_model_cleanup.is_none());
        assert!(runtime.incompatible_model_cleanup.is_none());
        assert_eq!(runtime.state().activity(), ApplicationActivity::Idle);
        assert!(runtime.state().can_select_device());
        Ok(())
    })
}

#[test]
fn inspection_submission_exhaustion_allows_e1_retry_but_zero_aggregate_does_not_release()
-> TestResult {
    with_runtime(default_test_configuration, |runtime| {
        let lower = failed_preparation_cleanup(RetainedOwnership::Exact(EXACT_FOOTPRINT), 1, 3);
        runtime.begin_runtime_retention(lower, None);
        runtime.forced_inference_busy_submissions = 3;

        assert!(runtime.retry_retained_model_cleanup_inspection().is_none());
        assert!(runtime.retry_retained_model_cleanup_inspection().is_none());
        let event = runtime.retry_retained_model_cleanup_inspection();
        let cleanup = pending_cleanup(event)?;
        assert_eq!(
            cleanup.cleanup(),
            ApplicationModelCleanupDisposition::CoordinationRetryAvailable {
                attempts: 3,
                maximum_attempts: 3,
            }
        );
        assert!(runtime.state().can_retry_model_cleanup());
        assert_eq!(
            runtime.retained_model_cleanup,
            Some(RetainedModelCleanup {
                resource: lower.resource,
                inspection: RetainedModelInspection::CoordinationRetryAvailable { attempts: 3 },
            })
        );

        runtime.retry_model_cleanup().map_err(application_error)?;
        assert_eq!(
            retained_model(runtime)?.cleanup(),
            ApplicationModelCleanupDisposition::Pending
        );
        assert_eq!(
            runtime.retained_model_cleanup,
            Some(RetainedModelCleanup {
                resource: lower.resource,
                inspection: RetainedModelInspection::PendingSubmission { attempts: 0 },
            })
        );
        assert!(runtime.retry_retained_model_cleanup_inspection().is_none());

        let snapshot_event = runtime
            .local
            .receive_timeout(TEST_TIMEOUT)
            .map_err(|error| format!("cleanup snapshot event failed: {error:?}"))?;
        let RuntimeEvent::Snapshot {
            ticket,
            runtime: aggregate,
            retained_models,
            ..
        } = snapshot_event
        else {
            return Err("unexpected cleanup inspection event".to_owned());
        };
        assert_eq!(aggregate.pending_cleanup_models, 0);
        assert!(aggregate.last_cleanup.is_none());
        assert!(retained_models.is_empty());

        let event = runtime.process_retained_model_cleanup_snapshot(
            ticket,
            &aggregate,
            retained_models.as_slice(),
        );
        let cleanup = pending_cleanup(event)?;
        assert_eq!(
            cleanup.cleanup(),
            ApplicationModelCleanupDisposition::CoordinationRetryAvailable {
                attempts: 3,
                maximum_attempts: 3,
            }
        );
        assert!(runtime.state().retained_model().is_some());
        assert!(runtime.state().loaded().is_none());
        assert_eq!(
            runtime.state().activity(),
            ApplicationActivity::RetainedCleanup
        );
        Ok(())
    })
}

#[test]
fn stale_unload_events_cannot_clear_a_loaded_or_retained_owner() -> TestResult {
    with_loaded_runtime(default_test_configuration, |runtime, loaded| {
        runtime.unload_model().map_err(application_error)?;
        let transaction = runtime
            .pending_unload
            .ok_or_else(|| "ordinary unload transaction was absent".to_owned())?;
        let event = runtime
            .local
            .receive_timeout(TEST_TIMEOUT)
            .map_err(|error| format!("model unload event failed: {error:?}"))?;
        let RuntimeEvent::ModelUnload {
            ticket,
            result: Ok(receipt),
        } = event
        else {
            return Err("unexpected model unload event".to_owned());
        };
        assert_eq!(ticket, transaction.ticket);

        let stale_ticket = CommandTicket::new(ticket.get().saturating_add(1));
        assert_eq!(
            runtime.process_model_unload(stale_ticket, Ok(receipt)),
            None
        );
        assert_eq!(runtime.state().loaded(), Some(&loaded));
        assert_eq!(runtime.pending_unload, Some(transaction));

        let event = runtime.process_model_unload(ticket, Ok(receipt));
        assert!(matches!(
            event,
            Some(ApplicationEvent::ModelUnloaded { .. })
        ));
        assert!(runtime.state().loaded().is_none());
        assert!(runtime.pending_unload.is_none());

        let lower = failed_preparation_cleanup(RetainedOwnership::Exact(EXACT_FOOTPRINT), 1, 3);
        runtime.begin_runtime_retention(lower, None);
        let retained = retained_model(runtime)?.clone();
        let stale_receipt = UnloadReceipt {
            handle: RETAINED_HANDLE,
            status: UnloadStatus::Unloaded,
            cancelled_requests: 0,
        };
        assert_eq!(
            runtime.process_model_unload(CommandTicket::new(999), Ok(stale_receipt)),
            None
        );
        assert_eq!(runtime.state().retained_model(), Some(&retained));
        assert!(runtime.state().loaded().is_none());
        Ok(())
    })
}

#[test]
fn mismatched_unload_handle_locks_unverified_lifecycle_until_cleanup() -> TestResult {
    with_loaded_runtime(default_test_configuration, |runtime, loaded| {
        runtime.unload_model().map_err(application_error)?;
        let transaction = runtime
            .pending_unload
            .ok_or_else(|| "ordinary unload transaction was absent".to_owned())?;
        let event = runtime
            .local
            .receive_timeout(TEST_TIMEOUT)
            .map_err(|error| format!("model unload event failed: {error:?}"))?;
        let RuntimeEvent::ModelUnload {
            ticket,
            result: Ok(mut receipt),
        } = event
        else {
            return Err("unexpected model unload event".to_owned());
        };
        assert_eq!(ticket, transaction.ticket);
        receipt.handle = RETAINED_HANDLE;

        let cleanup = pending_cleanup(runtime.process_model_unload(ticket, Ok(receipt)))?;
        assert_eq!(
            cleanup.resource(),
            ApplicationRetainedModelResource::LoadedModel {
                handle: loaded.handle(),
            }
        );
        assert_eq!(cleanup.ownership(), ApplicationRetainedOwnership::Unknown);
        assert_eq!(
            cleanup.primary_failure().kind,
            ApplicationFailureKind::IncompatibleReceipt
        );
        assert!(runtime.state().loaded().is_none());
        assert!(runtime.pending_unload.is_none());
        assert!(runtime.incompatible_model_cleanup.is_some());
        assert_eq!(
            runtime.state().activity(),
            ApplicationActivity::RetainedCleanup
        );
        Ok(())
    })
}

#[test]
fn retained_snapshot_preserves_the_original_compatibility_failure() -> TestResult {
    with_runtime(default_test_configuration, |runtime| {
        let primary = ApplicationFailure::new(
            ApplicationFailureKind::IncompatibleReceipt,
            "stable application compatibility failure",
        );
        let lower = incompatible_lower_cleanup();
        runtime.incompatible_model_cleanup =
            Some(crate::runtime::retained_cleanup::IncompatibleModelCleanup {
                handle: RETAINED_HANDLE,
                compatibility_failure: primary.clone(),
                unload: IncompatibleModelUnload::Submitted {
                    ticket: CommandTicket::new(80),
                    attempts: 1,
                    last_failure: None,
                },
            });
        runtime.begin_runtime_retention(lower, Some(primary.clone()));

        let ticket = CommandTicket::new(81);
        submit_inspection(runtime, lower.resource, ticket);
        let mut refreshed = lower;
        refreshed.attempts = 2;
        let _event = process_snapshot(
            runtime,
            ticket,
            RuntimeSnapshot::default(),
            vec![RetainedModelSnapshot {
                handle: RETAINED_HANDLE,
                cleanup: refreshed,
            }],
        )?;

        let retained = retained_model(runtime)?;
        assert_eq!(retained.primary_failure(), &primary);
        assert_eq!(
            retained.cleanup(),
            ApplicationModelCleanupDisposition::LowerRetryable {
                attempts: 2,
                maximum_attempts: 3,
            }
        );
        Ok(())
    })
}

#[test]
fn contradictory_released_cleanup_error_remains_unknown_until_explicit_proof() -> TestResult {
    with_runtime(default_test_configuration, |runtime| {
        let lower = failed_preparation_cleanup(RetainedOwnership::Released, 1, 3);
        runtime.begin_runtime_retention(lower, None);

        let retained = retained_model(runtime)?;
        assert_eq!(retained.ownership(), ApplicationRetainedOwnership::Unknown);
        assert_eq!(
            retained.cleanup(),
            ApplicationModelCleanupDisposition::Pending
        );
        assert_eq!(
            retained
                .cleanup_failure()
                .ok_or_else(|| "contract contradiction was not retained".to_owned())?
                .kind,
            ApplicationFailureKind::RetainedCleanup
        );
        assert!(matches!(
            runtime.retained_model_cleanup,
            Some(RetainedModelCleanup {
                inspection: RetainedModelInspection::PendingSubmission { .. },
                ..
            })
        ));
        assert_eq!(
            runtime.state().activity(),
            ApplicationActivity::RetainedCleanup
        );
        assert!(runtime.state().loaded().is_none());
        Ok(())
    })
}

#[test]
fn terminal_process_retention_cannot_be_reopened_by_stale_coordination_state() -> TestResult {
    with_runtime(default_test_configuration, |runtime| {
        let lower = failed_preparation_cleanup(RetainedOwnership::Exact(EXACT_FOOTPRINT), 1, 3);
        runtime.begin_runtime_retention(lower, None);
        runtime.retained_model_cleanup = Some(RetainedModelCleanup {
            resource: lower.resource,
            inspection: RetainedModelInspection::CoordinationRetryAvailable { attempts: 3 },
        });
        runtime
            .state
            .retained_model_mut()
            .ok_or_else(|| "retained model disappeared before terminal transition".to_owned())?
            .set_cleanup(
                ApplicationModelCleanupDisposition::CoordinationRetryAvailable {
                    attempts: 3,
                    maximum_attempts: 3,
                },
                None,
            );
        let RuntimeError::TerminalCleanupRetention { first, summary } = terminal_cleanup_failure()
        else {
            return Err("terminal cleanup fixture had the wrong shape".to_owned());
        };
        runtime.state.begin_shutdown();
        runtime.mark_terminal_process_retention(first, summary);

        assert_eq!(
            retained_model(runtime)?.cleanup(),
            ApplicationModelCleanupDisposition::RetainedUntilProcessExit
        );
        assert!(!runtime.state().can_retry_model_cleanup());
        assert_eq!(
            runtime.retry_model_cleanup(),
            Err(ApplicationError::ModelCleanupNotRetryable)
        );
        assert_eq!(
            runtime.state().activity(),
            ApplicationActivity::ShuttingDown
        );
        Ok(())
    })
}

#[test]
fn disconnect_preserves_retention_and_keeps_selection_locked() -> TestResult {
    let database_path = unique_database_path();
    let test_result = (|| {
        let mut configuration = ApplicationRuntimeConfiguration::new(&database_path);
        default_test_configuration(&mut configuration);
        let mut runtime = ApplicationRuntime::start(configuration).map_err(application_error)?;
        let lower = failed_preparation_cleanup(RetainedOwnership::Exact(EXACT_FOOTPRINT), 1, 3);
        runtime.begin_runtime_retention(lower, None);
        let retained_before_disconnect = retained_model(&runtime)?.clone();

        stop_inference_worker(&mut runtime)?;
        assert_eq!(
            runtime.poll_event(),
            Some(ApplicationEvent::RuntimeDisconnected)
        );

        let retained = retained_model(&runtime)?;
        assert_eq!(retained.resource(), retained_before_disconnect.resource());
        assert_eq!(retained.ownership(), retained_before_disconnect.ownership());
        assert_eq!(
            retained.primary_failure(),
            retained_before_disconnect.primary_failure()
        );
        assert_eq!(
            retained.cleanup(),
            ApplicationModelCleanupDisposition::WorkerDisconnected
        );
        assert_eq!(
            retained
                .cleanup_failure()
                .ok_or_else(|| "disconnect failure was not retained".to_owned())?
                .kind,
            ApplicationFailureKind::Worker
        );
        assert!(matches!(
            runtime.retained_model_cleanup,
            Some(RetainedModelCleanup {
                inspection: RetainedModelInspection::WorkerDisconnected,
                ..
            })
        ));
        assert!(!runtime.state().inference_available());
        assert!(!runtime.state().can_select_device());
        assert_eq!(
            runtime.select_device(CUDA_ZERO),
            Err(ApplicationError::DeviceSelectionLocked)
        );
        assert!(runtime.state().loaded().is_none());
        assert_eq!(
            runtime.retry_model_cleanup(),
            Err(ApplicationError::ModelCleanupNotRetryable)
        );
        assert_eq!(
            runtime.shutdown(),
            Err(ApplicationError::RuntimeDisconnected)
        );
        Ok(())
    })();

    let cleanup_result = remove_database(&database_path);
    test_result.and(cleanup_result)
}

#[test]
fn disconnect_during_load_creates_unknown_unconfirmed_load() -> TestResult {
    let database_path = unique_database_path();
    let test_result = (|| {
        let mut configuration = ApplicationRuntimeConfiguration::new(&database_path);
        default_test_configuration(&mut configuration);
        let mut runtime = ApplicationRuntime::start(configuration).map_err(application_error)?;
        let (selection, _resolved) =
            resolve_fixture_with(&mut runtime, REPOSITORY, COMMIT, "tokenizer.json")?;
        runtime.load_model(&selection).map_err(application_error)?;
        let (_ticket, _unobserved_receipt) = receive_successful_load_receipt(&mut runtime)?;
        assert_eq!(runtime.state().activity(), ApplicationActivity::Loading);

        stop_inference_worker(&mut runtime)?;
        assert_eq!(
            runtime.poll_event(),
            Some(ApplicationEvent::RuntimeDisconnected)
        );

        let retained = retained_model(&runtime)?;
        assert_eq!(
            retained.resource(),
            ApplicationRetainedModelResource::UnconfirmedLoad
        );
        assert_eq!(retained.ownership(), ApplicationRetainedOwnership::Unknown);
        assert_eq!(
            retained.cleanup(),
            ApplicationModelCleanupDisposition::WorkerDisconnected
        );
        assert_eq!(
            retained.primary_failure().kind,
            ApplicationFailureKind::Worker
        );
        assert!(retained.cleanup_failure().is_none());
        assert!(runtime.pending_load.is_none());
        assert!(runtime.state().loaded().is_none());
        assert!(!runtime.state().can_select_device());
        assert!(!runtime.state().can_load(&selection));
        assert_eq!(
            runtime.shutdown(),
            Err(ApplicationError::RuntimeDisconnected)
        );
        Ok(())
    })();

    let cleanup_result = remove_database(&database_path);
    test_result.and(cleanup_result)
}

#[test]
fn clean_shutdown_clears_loaded_state_before_an_independent_join_timeout() -> TestResult {
    with_loaded_runtime(default_test_configuration, |runtime, _loaded| {
        runtime.shutdown_control.forced_runtime_join_timeouts = 1;

        assert_eq!(
            runtime.shutdown(),
            Err(ApplicationError::ShutdownTimeout(
                ApplicationWorker::Inference
            ))
        );
        assert!(runtime.state().loaded().is_none());
        assert!(runtime.state().retained_model().is_none());
        assert!(runtime.pending_load.is_none());
        assert!(runtime.pending_unload.is_none());
        assert_eq!(
            runtime.state().activity(),
            ApplicationActivity::ShuttingDown
        );
        assert!(runtime.local.thread_is_present());

        runtime.shutdown().map_err(application_error)?;
        assert!(!runtime.local.thread_is_present());
        assert!(runtime.state().loaded().is_none());
        Ok(())
    })
}

#[test]
fn clean_shutdown_releases_retention_before_an_independent_join_timeout() -> TestResult {
    with_runtime(default_test_configuration, |runtime| {
        let lower = failed_preparation_cleanup(RetainedOwnership::Exact(EXACT_FOOTPRINT), 1, 3);
        runtime.begin_runtime_retention(lower, None);
        runtime.shutdown_control.forced_runtime_join_timeouts = 1;

        assert_eq!(
            runtime.shutdown(),
            Err(ApplicationError::ShutdownTimeout(
                ApplicationWorker::Inference
            ))
        );
        assert!(runtime.state().retained_model().is_none());
        assert!(runtime.retained_model_cleanup.is_none());
        assert!(runtime.incompatible_model_cleanup.is_none());
        assert!(runtime.state().loaded().is_none());
        assert!(runtime.local.thread_is_present());

        runtime.shutdown().map_err(application_error)?;
        assert!(!runtime.local.thread_is_present());
        assert!(runtime.state().retained_model().is_none());
        Ok(())
    })
}

#[test]
fn non_summary_terminal_shutdown_failure_never_leaves_normal_loaded_state() -> TestResult {
    let database_path = unique_database_path();
    let test_result = (|| {
        let mut configuration = ApplicationRuntimeConfiguration::new(&database_path);
        default_test_configuration(&mut configuration);
        let mut runtime = ApplicationRuntime::start(configuration).map_err(application_error)?;
        let loaded = load_fixture(&mut runtime)?;
        let cleanup = CleanupRetryState {
            resource: CleanupResource::Sequence {
                handle: loaded.handle(),
                request_id: RequestId::new(13),
                sequence_id: SequenceId::new(17),
            },
            failure: CleanupFailureReport::new(
                RuntimeOperation::Shutdown,
                FailureClass::Shutdown,
                RuntimeOperation::SequenceDestruction,
                FailureClass::Synchronization,
            ),
            ownership: RetainedOwnership::Exact(EXACT_FOOTPRINT),
            attempts: 1,
            maximum_attempts: 3,
        };
        runtime.shutdown_control.forced_runtime_shutdown_failure =
            Some(RuntimeError::CleanupFailed(cleanup));

        let first_error = runtime
            .shutdown()
            .err()
            .ok_or_else(|| "forced terminal cleanup error was reported as success".to_owned())?;
        let retained = retained_model(&runtime)?.clone();
        assert_eq!(
            retained.resource(),
            ApplicationRetainedModelResource::LoadedModel {
                handle: loaded.handle(),
            }
        );
        assert_eq!(
            retained.ownership(),
            ApplicationRetainedOwnership::Exact(loaded.reserved_footprint())
        );
        assert_eq!(
            retained.cleanup(),
            ApplicationModelCleanupDisposition::RetainedUntilProcessExit
        );
        assert!(runtime.state().loaded().is_none());
        assert!(runtime.state().active_generation().is_none());
        assert!(runtime.pending_load.is_none());
        assert!(runtime.pending_unload.is_none());
        assert!(!runtime.local.thread_is_present());
        assert_eq!(runtime.shutdown(), Err(first_error));
        assert_eq!(runtime.state().retained_model(), Some(&retained));
        Ok(())
    })();

    let cleanup_result = remove_database(&database_path);
    test_result.and(cleanup_result)
}

#[test]
fn terminal_retention_persists_after_worker_join() -> TestResult {
    let database_path = unique_database_path();
    let test_result = (|| {
        let mut configuration = ApplicationRuntimeConfiguration::new(&database_path);
        default_test_configuration(&mut configuration);
        let mut runtime = ApplicationRuntime::start(configuration).map_err(application_error)?;
        runtime.shutdown_control.forced_runtime_shutdown_failure = Some(terminal_cleanup_failure());
        runtime.shutdown_control.forced_runtime_join_timeouts = 1;

        let first_error = match runtime.shutdown() {
            Ok(()) => return Err("terminal cleanup retention was reported as success".to_owned()),
            Err(error) => error,
        };
        assert!(matches!(
            &first_error,
            ApplicationError::Failure(failure)
                if failure.kind == ApplicationFailureKind::Inference
                    && failure.message.contains("TerminalCleanupRetention")
        ));
        assert!(runtime.local.thread_is_present());
        let retained_before_join = retained_model(&runtime)?.clone();
        assert_eq!(
            retained_before_join.resource(),
            ApplicationRetainedModelResource::LoadedModel {
                handle: ModelHandle::new(ModelId::new(1), ModelGeneration::new(1)),
            }
        );
        assert_eq!(
            retained_before_join.ownership(),
            ApplicationRetainedOwnership::Exact(ApplicationMemoryFootprint::default())
        );
        assert_eq!(
            retained_before_join.cleanup(),
            ApplicationModelCleanupDisposition::RetainedUntilProcessExit
        );
        assert!(runtime.state().loaded().is_none());

        assert_eq!(runtime.shutdown(), Err(first_error.clone()));
        assert!(!runtime.local.thread_is_present());
        assert!(runtime.hub_thread.is_none());
        assert_eq!(
            runtime.state().retained_model(),
            Some(&retained_before_join)
        );
        assert_eq!(
            runtime.retry_model_cleanup(),
            Err(ApplicationError::ModelCleanupNotRetryable)
        );
        assert_eq!(runtime.shutdown(), Err(first_error));
        assert_eq!(
            runtime.state().retained_model(),
            Some(&retained_before_join)
        );
        Ok(())
    })();

    let cleanup_result = remove_database(&database_path);
    test_result.and(cleanup_result)
}
