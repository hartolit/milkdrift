use domain_contracts::{
    MemoryFootprint, ModelGeneration, ModelHandle, ModelId, RequestId, ScalarType, SequenceId,
};
use inference_runtime::{
    CleanupFailureReport, CleanupResource, CleanupRetryState, CommandTicket, ConservativeFootprint,
    FailureClass, RetainedModelSnapshot, RetainedOwnership, RuntimeError, RuntimeOperation,
    RuntimeSnapshot, UnloadReceipt, UnloadStatus,
};

use super::support::*;
use crate::runtime::retained_cleanup::{
    CleanupCommand, ModelCleanupAction, ModelCleanupCoordinator,
};
use crate::unload::ModelUnloadTransaction;
use crate::{
    ApplicationConservativeFootprint, ApplicationError, ApplicationEvent, ApplicationFailure,
    ApplicationFailureKind, ApplicationModelCleanupDisposition, ApplicationRetainedModel,
    ApplicationRetainedModelResource, ApplicationRetainedOwnership, ApplicationRuntime,
    ApplicationRuntimeConfiguration, ApplicationState, ApplicationWorker,
};

const RETAINED_HANDLE: ModelHandle = ModelHandle::new(ModelId::new(7), ModelGeneration::new(3));
const UNRELATED_HANDLE: ModelHandle = ModelHandle::new(ModelId::new(11), ModelGeneration::new(5));
const EXACT_FOOTPRINT: MemoryFootprint = MemoryFootprint {
    host_weight_bytes: 11,
    device_weight_bytes: 13,
    host_working_bytes: 17,
    device_working_bytes: 19,
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

fn failed_load_cleanup(
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

fn incompatible_cleanup(attempts: u32) -> CleanupRetryState {
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
            accepted_footprint: ACCEPTED_LOADING_PEAK,
            reported_footprint: REPORTED_FOOTPRINT,
            conservative_footprint: ConservativeFootprint::Known(REPORTED_FOOTPRINT),
        },
        attempts,
        maximum_attempts: 3,
    }
}

fn verified_unload_cleanup(handle: ModelHandle, attempts: u32) -> CleanupRetryState {
    CleanupRetryState {
        resource: CleanupResource::Model { handle },
        failure: CleanupFailureReport::new(
            RuntimeOperation::ModelUnload,
            FailureClass::Completion,
            RuntimeOperation::ModelUnload,
            FailureClass::Synchronization,
        ),
        ownership: RetainedOwnership::Exact(EXACT_FOOTPRINT),
        attempts,
        maximum_attempts: 3,
    }
}

fn sequence_cleanup(handle: ModelHandle, attempts: u32) -> CleanupRetryState {
    CleanupRetryState {
        resource: CleanupResource::Sequence {
            handle,
            request_id: RequestId::new(17),
            sequence_id: SequenceId::new(19),
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

fn retained(runtime: &ApplicationRuntime) -> TestResult<&ApplicationRetainedModel> {
    runtime
        .state()
        .retained_model()
        .ok_or_else(|| "expected durable retained model state".to_owned())
}

fn cleanup_event(runtime: &ApplicationRuntime, event: Option<&ApplicationEvent>) -> TestResult {
    let Some(ApplicationEvent::ModelCleanupPending {
        resource,
        disposition,
    }) = event
    else {
        return Err(format!("expected cleanup transition event, got {event:?}"));
    };
    let current = retained(runtime)?;
    assert_eq!(*resource, current.resource());
    assert_eq!(*disposition, current.cleanup());
    Ok(())
}

fn submit_inspection(
    runtime: &mut ApplicationRuntime,
    resource: CleanupResource,
    ticket: CommandTicket,
) {
    runtime.install_submitted_cleanup_inspection(resource, ticket);
}

fn process_snapshot(
    runtime: &mut ApplicationRuntime,
    ticket: CommandTicket,
    snapshot: &RuntimeSnapshot,
    retained_models: &[RetainedModelSnapshot],
) -> Option<ApplicationEvent> {
    runtime.process_retained_model_cleanup_snapshot(ticket, snapshot, retained_models)
}

#[test]
fn failed_load_owner_keeps_primary_cleanup_and_lower_attempts_independent() -> TestResult {
    with_runtime(default_test_configuration, |runtime| {
        let (selection, _) = resolve_fixture_with(runtime, REPOSITORY, COMMIT, "tokenizer.json")?;
        let lower = failed_load_cleanup(RetainedOwnership::Exact(EXACT_FOOTPRINT), 1, 3);
        runtime.begin_runtime_retention(lower, None);
        cleanup_event(runtime, Some(&runtime.current_cleanup_event()))?;

        let cleanup = retained(runtime)?;
        assert_eq!(
            cleanup.resource(),
            ApplicationRetainedModelResource::FailedLoad {
                handle: RETAINED_HANDLE
            }
        );
        assert_eq!(
            cleanup.ownership(),
            ApplicationRetainedOwnership::Exact(EXACT_FOOTPRINT.into())
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
        assert_eq!(
            cleanup
                .cleanup_failure()
                .ok_or_else(|| "cleanup failure was not retained".to_owned())?
                .kind,
            ApplicationFailureKind::RetainedCleanup
        );
        assert!(!runtime.state().can_select_device());
        assert!(!runtime.state().can_load(&selection));
        assert!(!runtime.state().can_start_generation());
        Ok(())
    })
}

#[test]
fn lower_exhaustion_is_never_reopened_by_e1_retry() -> TestResult {
    with_runtime(default_test_configuration, |runtime| {
        runtime.begin_runtime_retention(
            failed_load_cleanup(RetainedOwnership::Exact(EXACT_FOOTPRINT), 3, 3),
            None,
        );
        assert!(matches!(
            runtime
                .model_cleanup
                .as_ref()
                .map(ModelCleanupCoordinator::action),
            Some(ModelCleanupAction::LowerExhausted {
                attempts: 3,
                maximum_attempts: 3,
                ..
            })
        ));
        assert_eq!(
            runtime.retry_model_cleanup(),
            Err(ApplicationError::ModelCleanupNotRetryable)
        );
        assert!(matches!(
            runtime
                .model_cleanup
                .as_ref()
                .map(ModelCleanupCoordinator::action),
            Some(ModelCleanupAction::LowerExhausted {
                attempts: 3,
                maximum_attempts: 3,
                ..
            })
        ));
        Ok(())
    })
}

#[test]
fn incompatible_lower_owner_preserves_unverified_evidence() -> TestResult {
    with_runtime(default_test_configuration, |runtime| {
        runtime.begin_runtime_retention(incompatible_cleanup(1), None);
        assert_eq!(
            retained(runtime)?.ownership(),
            ApplicationRetainedOwnership::Unverified {
                accepted_loading_peak: ACCEPTED_LOADING_PEAK.into(),
                reported_footprint: REPORTED_FOOTPRINT.into(),
                conservative_footprint: ApplicationConservativeFootprint::Known(
                    REPORTED_FOOTPRINT.into()
                ),
            }
        );
        assert_eq!(
            retained(runtime)?.primary_failure().kind,
            ApplicationFailureKind::IncompatibleReceipt
        );
        Ok(())
    })
}

#[test]
fn ordinary_unload_failure_enters_the_same_coordinator() -> TestResult {
    with_loaded_runtime(default_test_configuration, |runtime, loaded| {
        let ticket = CommandTicket::new(73);
        runtime.pending_unload = Some(ModelUnloadTransaction {
            ticket,
            handle: loaded.handle(),
        });
        runtime.state.begin_unloading();
        let result = Err(RuntimeError::CleanupFailed(verified_unload_cleanup(
            loaded.handle(),
            1,
        )));
        let event = runtime.process_model_unload(ticket, &result);
        cleanup_event(runtime, event.as_ref())?;
        assert_eq!(
            retained(runtime)?.resource(),
            ApplicationRetainedModelResource::LoadedModel {
                handle: loaded.handle()
            }
        );
        assert!(runtime.state().loaded().is_none());
        assert!(runtime.pending_unload.is_some());
        Ok(())
    })
}

#[test]
fn incompatible_receipt_submits_correlated_unload_and_releases_once() -> TestResult {
    with_runtime(default_test_configuration, |runtime| {
        let (selection, _) = resolve_fixture_with(runtime, REPOSITORY, COMMIT, "tokenizer.json")?;
        runtime.load_model(&selection).map_err(application_error)?;
        let (ticket, mut receipt) = receive_successful_load_receipt(runtime)?;
        receipt.execution_scalar_type = ScalarType::I8;

        let event = runtime.process_model_loaded(ticket, &Ok(receipt));
        assert!(matches!(
            event,
            Some(ApplicationEvent::ModelCompatibilityFailed { .. })
        ));
        assert!(matches!(
            runtime
                .model_cleanup
                .as_ref()
                .map(ModelCleanupCoordinator::action),
            Some(ModelCleanupAction::SubmittedCommand {
                command: CleanupCommand::UnloadIncompatibleModel { .. },
                attempts: 1,
                ..
            })
        ));

        let unloaded = wait_for_event(runtime, |event| {
            matches!(event, ApplicationEvent::ModelUnloaded { .. })
        })?;
        assert!(matches!(unloaded, ApplicationEvent::ModelUnloaded { .. }));
        assert!(runtime.model_cleanup.is_none());
        assert!(runtime.state().retained_model().is_none());
        assert!(runtime.poll_event().is_none());
        Ok(())
    })
}

#[test]
fn incompatible_unload_failure_transitions_to_lower_inspection_without_parallel_trackers()
-> TestResult {
    with_runtime(default_test_configuration, |runtime| {
        let primary = ApplicationFailure::new(
            ApplicationFailureKind::IncompatibleReceipt,
            "controlled incompatibility",
        );
        let ticket = CommandTicket::new(79);
        runtime.install_submitted_incompatible_cleanup(
            RETAINED_HANDLE,
            ApplicationRetainedOwnership::Exact(EXACT_FOOTPRINT.into()),
            primary.clone(),
            ticket,
            1,
        );
        let lower = incompatible_cleanup(1);
        let event = runtime.process_model_unload(ticket, &Err(RuntimeError::CleanupFailed(lower)));
        cleanup_event(runtime, event.as_ref())?;
        assert_eq!(retained(runtime)?.primary_failure(), &primary);
        assert!(matches!(
            runtime.model_cleanup.as_ref().map(ModelCleanupCoordinator::action),
            Some(ModelCleanupAction::WaitingForLowerRetry {
                resource,
                attempts: 1,
                ..
            }) if resource == lower.resource
        ));
        assert!(runtime.progress_model_cleanup_coordination().is_none());
        assert!(matches!(
            runtime.model_cleanup.as_ref().map(ModelCleanupCoordinator::action),
            Some(ModelCleanupAction::SubmittedCommand {
                command: CleanupCommand::InspectRetainedOwner { resource },
                ..
            }) if resource == lower.resource
        ));
        Ok(())
    })
}

#[test]
fn busy_submission_exhaustion_allows_only_a_fresh_e1_round() -> TestResult {
    with_runtime(default_test_configuration, |runtime| {
        let lower = failed_load_cleanup(RetainedOwnership::Exact(EXACT_FOOTPRINT), 1, 3);
        runtime.begin_runtime_retention(lower, None);
        runtime.forced_inference_busy_submissions = 3;
        assert!(runtime.progress_model_cleanup_coordination().is_none());
        assert!(runtime.progress_model_cleanup_coordination().is_none());
        let event = runtime.progress_model_cleanup_coordination();
        cleanup_event(runtime, event.as_ref())?;
        assert!(runtime.state().can_retry_model_cleanup());
        assert_eq!(
            retained(runtime)?.cleanup(),
            ApplicationModelCleanupDisposition::CoordinationRetryAvailable {
                attempts: 3,
                maximum_attempts: 3,
            }
        );

        runtime.retry_model_cleanup().map_err(application_error)?;
        let lower_attempts = runtime
            .model_cleanup
            .as_ref()
            .and_then(ModelCleanupCoordinator::lower_attempts)
            .ok_or_else(|| "lower attempt evidence was lost during E1 retry".to_owned())?;
        assert_eq!(lower_attempts.attempts, 1);
        assert_eq!(lower_attempts.maximum_attempts, 3);
        assert!(matches!(
            runtime.model_cleanup.as_ref().map(ModelCleanupCoordinator::action),
            Some(ModelCleanupAction::PendingCommandSubmission {
                command: CleanupCommand::InspectRetainedOwner { resource },
                attempts: 0,
            }) if resource == lower.resource
        ));
        assert_eq!(
            retained(runtime)?.cleanup(),
            ApplicationModelCleanupDisposition::Pending
        );
        Ok(())
    })
}

#[test]
fn explicit_release_clears_exact_and_unverified_owners_once() -> TestResult {
    for lower in [
        failed_load_cleanup(RetainedOwnership::Exact(EXACT_FOOTPRINT), 1, 3),
        incompatible_cleanup(1),
    ] {
        with_runtime(default_test_configuration, |runtime| {
            runtime.begin_runtime_retention(lower, None);
            let ticket = CommandTicket::new(83);
            submit_inspection(runtime, lower.resource, ticket);
            let released = CleanupRetryState {
                ownership: RetainedOwnership::Released,
                attempts: 2,
                ..lower
            };
            let event = process_snapshot(
                runtime,
                ticket,
                &RuntimeSnapshot {
                    last_cleanup: Some(released),
                    ..RuntimeSnapshot::default()
                },
                &[],
            );
            assert!(matches!(
                event,
                Some(ApplicationEvent::ModelCleanupReleased { resource })
                    if resource == application_resource(lower.resource)
            ));
            assert!(runtime.model_cleanup.is_none());
            assert!(runtime.state().retained_model().is_none());
            assert!(
                process_snapshot(
                    runtime,
                    ticket,
                    &RuntimeSnapshot {
                        last_cleanup: Some(released),
                        ..RuntimeSnapshot::default()
                    },
                    &[],
                )
                .is_none()
            );
            Ok(())
        })?;
    }
    Ok(())
}

#[test]
fn stale_ticket_and_wrong_resource_cannot_advance_cleanup() -> TestResult {
    with_runtime(default_test_configuration, |runtime| {
        let lower = failed_load_cleanup(RetainedOwnership::Exact(EXACT_FOOTPRINT), 1, 3);
        runtime.begin_runtime_retention(lower, None);
        let ticket = CommandTicket::new(89);
        submit_inspection(runtime, lower.resource, ticket);
        let before = retained(runtime)?.clone();

        assert!(
            process_snapshot(
                runtime,
                CommandTicket::new(90),
                &RuntimeSnapshot {
                    last_cleanup: Some(CleanupRetryState {
                        ownership: RetainedOwnership::Released,
                        ..lower
                    }),
                    ..RuntimeSnapshot::default()
                },
                &[],
            )
            .is_none()
        );
        assert_eq!(retained(runtime)?, &before);

        let unrelated = verified_unload_cleanup(UNRELATED_HANDLE, 1);
        let event = process_snapshot(
            runtime,
            ticket,
            &RuntimeSnapshot {
                last_cleanup: Some(unrelated),
                ..RuntimeSnapshot::default()
            },
            &[],
        );
        cleanup_event(runtime, event.as_ref())?;
        assert_eq!(retained(runtime)?.resource(), before.resource());
        assert!(matches!(
            retained(runtime)?.cleanup(),
            ApplicationModelCleanupDisposition::CoordinationRetryAvailable { .. }
        ));
        Ok(())
    })
}

#[test]
fn live_owner_overrides_contradictory_release_and_remains_unknown() -> TestResult {
    with_runtime(default_test_configuration, |runtime| {
        let lower = failed_load_cleanup(RetainedOwnership::Exact(EXACT_FOOTPRINT), 1, 3);
        runtime.begin_runtime_retention(lower, None);
        let ticket = CommandTicket::new(97);
        submit_inspection(runtime, lower.resource, ticket);
        let released = CleanupRetryState {
            ownership: RetainedOwnership::Released,
            attempts: 2,
            ..lower
        };
        let event = process_snapshot(
            runtime,
            ticket,
            &RuntimeSnapshot {
                last_cleanup: Some(released),
                ..RuntimeSnapshot::default()
            },
            &[RetainedModelSnapshot {
                handle: RETAINED_HANDLE,
                cleanup: lower,
            }],
        );
        cleanup_event(runtime, event.as_ref())?;
        assert_eq!(
            retained(runtime)?.ownership(),
            ApplicationRetainedOwnership::Unknown
        );
        assert!(runtime.model_cleanup.is_some());
        Ok(())
    })
}

#[test]
fn sequence_release_does_not_publish_model_release_during_correlated_unload() -> TestResult {
    with_loaded_runtime(default_test_configuration, |runtime, loaded| {
        let unload_ticket = CommandTicket::new(101);
        runtime.pending_unload = Some(ModelUnloadTransaction {
            ticket: unload_ticket,
            handle: loaded.handle(),
        });
        runtime.state.begin_unloading();
        let lower = sequence_cleanup(loaded.handle(), 1);
        runtime.begin_runtime_retention(lower, None);
        let ticket = CommandTicket::new(103);
        submit_inspection(runtime, lower.resource, ticket);
        let released = CleanupRetryState {
            ownership: RetainedOwnership::Released,
            attempts: 2,
            ..lower
        };
        let event = process_snapshot(
            runtime,
            ticket,
            &RuntimeSnapshot {
                last_cleanup: Some(released),
                ..RuntimeSnapshot::default()
            },
            &[],
        );
        cleanup_event(runtime, event.as_ref())?;
        assert!(runtime.pending_unload.is_some());
        assert!(runtime.state().retained_model().is_some());
        Ok(())
    })
}

#[test]
fn mismatched_unload_receipt_enters_unknown_single_owner_state() -> TestResult {
    with_loaded_runtime(default_test_configuration, |runtime, loaded| {
        let ticket = CommandTicket::new(107);
        runtime.pending_unload = Some(ModelUnloadTransaction {
            ticket,
            handle: loaded.handle(),
        });
        runtime.state.begin_unloading();
        let receipt = UnloadReceipt {
            handle: UNRELATED_HANDLE,
            status: UnloadStatus::Unloaded,
            cancelled_requests: 0,
        };
        let event = runtime.process_model_unload(ticket, &Ok(receipt));
        cleanup_event(runtime, event.as_ref())?;
        assert_eq!(
            retained(runtime)?.ownership(),
            ApplicationRetainedOwnership::Unknown
        );
        assert!(runtime.state().loaded().is_none());
        assert!(runtime.pending_unload.is_none());
        assert!(runtime.model_cleanup.is_some());
        Ok(())
    })
}

#[test]
fn disconnection_never_implies_release_and_remains_nonretryable() -> TestResult {
    let database_path = unique_database_path();
    let result = (|| {
        let mut configuration = ApplicationRuntimeConfiguration::new(&database_path);
        default_test_configuration(&mut configuration);
        let mut runtime = ApplicationRuntime::start(configuration).map_err(application_error)?;
        runtime.begin_runtime_retention(
            failed_load_cleanup(RetainedOwnership::Exact(EXACT_FOOTPRINT), 1, 3),
            None,
        );
        let before = retained(&runtime)?.clone();
        let shutdown_ticket = runtime.next_ticket().map_err(application_error)?;
        runtime
            .submit_inference(inference_runtime::RuntimeCommand::Shutdown {
                ticket: shutdown_ticket,
            })
            .map_err(application_error)?;
        let _shutdown = runtime
            .local
            .receive_timeout(TEST_TIMEOUT)
            .map_err(|error| format!("shutdown receive failed: {error:?}"))?;
        let thread = runtime
            .local
            .take_thread()
            .ok_or_else(|| "worker thread was absent".to_owned())?;
        thread
            .join()
            .map_err(|error| format!("worker join failed: {error:?}"))?;

        assert_eq!(
            runtime.poll_event(),
            Some(ApplicationEvent::RuntimeDisconnected)
        );
        assert_eq!(retained(&runtime)?.resource(), before.resource());
        assert_eq!(retained(&runtime)?.ownership(), before.ownership());
        assert_eq!(
            retained(&runtime)?.cleanup(),
            ApplicationModelCleanupDisposition::WorkerDisconnected
        );
        assert_eq!(
            runtime.retry_model_cleanup(),
            Err(ApplicationError::ModelCleanupNotRetryable)
        );
        assert!(!runtime.state().can_select_device());
        Ok(())
    })();
    let cleanup = remove_database(&database_path);
    result.and(cleanup)
}

#[test]
fn disconnection_during_unconfirmed_load_is_unknown() -> TestResult {
    let database_path = unique_database_path();
    let result = (|| {
        let mut configuration = ApplicationRuntimeConfiguration::new(&database_path);
        default_test_configuration(&mut configuration);
        let mut runtime = ApplicationRuntime::start(configuration).map_err(application_error)?;
        let (selection, _) =
            resolve_fixture_with(&mut runtime, REPOSITORY, COMMIT, "tokenizer.json")?;
        runtime.load_model(&selection).map_err(application_error)?;
        let (_ticket, _receipt) = receive_successful_load_receipt(&mut runtime)?;
        runtime.mark_model_worker_disconnected();
        assert_eq!(
            retained(&runtime)?.resource(),
            ApplicationRetainedModelResource::UnconfirmedLoad
        );
        assert_eq!(
            retained(&runtime)?.ownership(),
            ApplicationRetainedOwnership::Unknown
        );
        Ok(())
    })();
    let cleanup = remove_database(&database_path);
    result.and(cleanup)
}

#[test]
fn clean_shutdown_clears_cleanup_before_independent_join_retry() -> TestResult {
    with_runtime(default_test_configuration, |runtime| {
        runtime.begin_runtime_retention(
            failed_load_cleanup(RetainedOwnership::Exact(EXACT_FOOTPRINT), 1, 3),
            None,
        );
        runtime.shutdown_control.forced_runtime_join_timeouts = 1;
        assert_eq!(
            runtime.shutdown(),
            Err(ApplicationError::ShutdownTimeout(
                ApplicationWorker::Inference
            ))
        );
        assert!(runtime.model_cleanup.is_none());
        assert!(runtime.state().retained_model().is_none());
        runtime.shutdown().map_err(application_error)
    })
}

#[test]
fn terminal_retention_survives_worker_join_and_cannot_be_retried() -> TestResult {
    let database_path = unique_database_path();
    let result = (|| {
        let mut configuration = ApplicationRuntimeConfiguration::new(&database_path);
        default_test_configuration(&mut configuration);
        let mut runtime = ApplicationRuntime::start(configuration).map_err(application_error)?;
        let _loaded = load_fixture(&mut runtime)?;
        runtime.shutdown_control.forced_runtime_shutdown_failure = Some(terminal_cleanup_failure());
        let first = runtime
            .shutdown()
            .err()
            .ok_or_else(|| "terminal cleanup failure was reported as success".to_owned())?;
        assert!(!runtime.local.thread_is_present());
        assert_eq!(
            retained(&runtime)?.cleanup(),
            ApplicationModelCleanupDisposition::RetainedUntilProcessExit
        );
        assert!(runtime.state().loaded().is_none());
        assert_eq!(runtime.shutdown(), Err(first));
        assert_eq!(
            runtime.retry_model_cleanup(),
            Err(ApplicationError::ModelCleanupNotRetryable)
        );
        Ok(())
    })();
    let cleanup = remove_database(&database_path);
    result.and(cleanup)
}

#[test]
fn public_cleanup_event_is_compact_and_detailed_evidence_stays_in_state() {
    assert!(std::mem::size_of::<ApplicationEvent>() <= 192);
    assert!(std::mem::size_of::<ApplicationError>() <= 64);
    assert!(std::mem::size_of::<ApplicationRetainedModel>() <= 256);
    assert!(std::mem::size_of::<ApplicationState>() <= 768);
}

const fn application_resource(resource: CleanupResource) -> ApplicationRetainedModelResource {
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
