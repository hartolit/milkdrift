use domain_contracts::{
    CapabilitySet, DeviceId, DeviceKind, ExecutionDevice, ModelGeneration, ModelHandle, ModelId,
    ScalarType, ScalarTypeSet,
};
use inference_runtime::{RuntimeCommand, RuntimeEvent, RuntimeSnapshot};

use super::support::*;
use crate::runtime::retained_cleanup::{
    IncompatibleModelCleanup, IncompatibleModelUnload,
    MAXIMUM_INCOMPATIBLE_UNLOAD_SUBMISSION_ATTEMPTS, RetainedModelCleanup,
};
use crate::{
    ApplicationActivity, ApplicationDevice, ApplicationError, ApplicationEvent, ApplicationFailure,
    ApplicationFailureKind, ApplicationRuntime, ApplicationRuntimeConfiguration,
    ApplicationScalarType,
};

fn retain_submitted_incompatible_cleanup(
    runtime: &mut ApplicationRuntime,
    ticket: inference_runtime::CommandTicket,
) {
    runtime.incompatible_model_cleanup = Some(IncompatibleModelCleanup {
        handle: ModelHandle::new(ModelId::new(7), ModelGeneration::new(3)),
        compatibility_failure: ApplicationFailure::new(
            ApplicationFailureKind::IncompatibleReceipt,
            "fixture incompatible receipt",
        ),
        unload: IncompatibleModelUnload::Submitted {
            ticket,
            last_failure: None,
            retry_exhausted: false,
        },
    });
    runtime.state.begin_unloading();
}

#[test]
fn incompatible_configuration_declaration_evidence_unloads_without_publishing_loaded_state()
-> TestResult {
    with_runtime(default_test_configuration, |runtime| {
        let (selection, _resolved) =
            resolve_fixture_with(runtime, REPOSITORY, COMMIT, "tokenizer.json")?;
        runtime.load_model(&selection).map_err(application_error)?;
        runtime
            .pending_load
            .as_mut()
            .ok_or_else(|| "load admission evidence was not retained".to_owned())?
            .configuration_declared_scalar_type = Some(ApplicationScalarType::Bf16);

        let event = wait_for_event(runtime, |event| {
            matches!(event, ApplicationEvent::ModelCompatibilityFailed { .. })
        })?;
        assert!(matches!(
            event,
            ApplicationEvent::ModelCompatibilityFailed { .. }
        ));
        assert!(runtime.state().loaded().is_none());
        assert_eq!(runtime.state().activity(), ApplicationActivity::Unloading);
        assert_eq!(
            runtime.select_device(CUDA_ZERO),
            Err(ApplicationError::DeviceSelectionLocked)
        );

        let event = wait_for_event(runtime, |event| {
            matches!(event, ApplicationEvent::ModelUnloaded { .. })
        })?;
        assert!(matches!(
            event,
            ApplicationEvent::ModelUnloaded {
                cancelled_requests: 0,
                ..
            }
        ));
        assert!(runtime.state().loaded().is_none());
        assert_eq!(runtime.state().activity(), ApplicationActivity::Idle);
        Ok(())
    })
}

#[test]
fn unsupported_execution_scalar_receipt_uses_incompatible_model_cleanup() -> TestResult {
    with_runtime(default_test_configuration, |runtime| {
        let (selection, _resolved) =
            resolve_fixture_with(runtime, REPOSITORY, COMMIT, "tokenizer.json")?;
        runtime.load_model(&selection).map_err(application_error)?;
        let (ticket, mut receipt) = receive_successful_load_receipt(runtime)?;
        let handle = receipt.handle;
        receipt.execution_scalar_type = ScalarType::I8;

        let event = runtime.process_model_loaded(ticket, Ok(receipt));
        assert!(matches!(
            event,
            ApplicationEvent::ModelCompatibilityFailed { .. }
        ));
        assert!(runtime.state().loaded().is_none());
        assert_eq!(runtime.state().activity(), ApplicationActivity::Unloading);
        assert_eq!(
            runtime
                .incompatible_model_cleanup
                .as_ref()
                .map(|cleanup| cleanup.handle),
            Some(handle)
        );
        assert_eq!(
            runtime.select_device(CUDA_ZERO),
            Err(ApplicationError::DeviceSelectionLocked)
        );

        let event = wait_for_event(
            runtime,
            |event| matches!(event, ApplicationEvent::ModelUnloaded { handle: unloaded, .. } if *unloaded == handle),
        )?;
        assert!(matches!(event, ApplicationEvent::ModelUnloaded { .. }));
        assert!(runtime.incompatible_model_cleanup.is_none());
        assert_eq!(runtime.state().activity(), ApplicationActivity::Idle);
        Ok(())
    })
}

#[test]
fn unsupported_observed_scalar_classification_uses_incompatible_model_cleanup() -> TestResult {
    with_runtime(default_test_configuration, |runtime| {
        let (selection, _resolved) =
            resolve_fixture_with(runtime, REPOSITORY, COMMIT, "tokenizer.json")?;
        runtime.load_model(&selection).map_err(application_error)?;
        let (ticket, mut receipt) = receive_successful_load_receipt(runtime)?;
        let handle = receipt.handle;
        receipt.descriptor.metadata.observed_tensor_scalar_types =
            ScalarTypeSet::from_scalar(ScalarType::I8);

        let event = runtime.process_model_loaded(ticket, Ok(receipt));
        assert!(matches!(
            event,
            ApplicationEvent::ModelCompatibilityFailed { .. }
        ));
        assert!(runtime.state().loaded().is_none());
        assert_eq!(runtime.state().activity(), ApplicationActivity::Unloading);
        assert_eq!(
            runtime
                .incompatible_model_cleanup
                .as_ref()
                .map(|cleanup| cleanup.handle),
            Some(handle)
        );

        let event = wait_for_event(
            runtime,
            |event| matches!(event, ApplicationEvent::ModelUnloaded { handle: unloaded, .. } if *unloaded == handle),
        )?;
        assert!(matches!(event, ApplicationEvent::ModelUnloaded { .. }));
        assert!(runtime.incompatible_model_cleanup.is_none());
        assert_eq!(runtime.state().activity(), ApplicationActivity::Idle);
        Ok(())
    })
}

#[test]
fn missing_execution_capability_uses_incompatible_model_cleanup() -> TestResult {
    with_runtime(default_test_configuration, |runtime| {
        let (selection, _resolved) =
            resolve_fixture_with(runtime, REPOSITORY, COMMIT, "tokenizer.json")?;
        runtime.load_model(&selection).map_err(application_error)?;
        let (ticket, mut receipt) = receive_successful_load_receipt(runtime)?;
        let handle = receipt.handle;
        receipt.descriptor.capabilities.operations = CapabilitySet::PREFILL;

        let event = runtime.process_model_loaded(ticket, Ok(receipt));
        assert!(matches!(
            event,
            ApplicationEvent::ModelCompatibilityFailed { .. }
        ));
        assert!(runtime.state().loaded().is_none());
        assert_eq!(runtime.state().activity(), ApplicationActivity::Unloading);

        let event = wait_for_event(
            runtime,
            |event| matches!(event, ApplicationEvent::ModelUnloaded { handle: unloaded, .. } if *unloaded == handle),
        )?;
        assert!(matches!(event, ApplicationEvent::ModelUnloaded { .. }));
        assert!(runtime.incompatible_model_cleanup.is_none());
        Ok(())
    })
}

#[test]
fn malformed_load_receipt_footprint_uses_incompatible_model_cleanup() -> TestResult {
    with_runtime(default_test_configuration, |runtime| {
        let (selection, _resolved) =
            resolve_fixture_with(runtime, REPOSITORY, COMMIT, "tokenizer.json")?;
        runtime.load_model(&selection).map_err(application_error)?;
        let (ticket, mut receipt) = receive_successful_load_receipt(runtime)?;
        let handle = receipt.handle;
        receipt.reserved_footprint.device_weight_bytes = 1;

        let event = runtime.process_model_loaded(ticket, Ok(receipt));
        assert!(matches!(
            event,
            ApplicationEvent::ModelCompatibilityFailed { .. }
        ));
        assert!(runtime.state().loaded().is_none());
        assert_eq!(runtime.state().activity(), ApplicationActivity::Unloading);
        assert_eq!(
            runtime
                .incompatible_model_cleanup
                .as_ref()
                .map(|cleanup| cleanup.handle),
            Some(handle)
        );

        let event = wait_for_event(runtime, |event| {
            matches!(event, ApplicationEvent::ModelUnloaded { .. })
        })?;
        assert!(matches!(event, ApplicationEvent::ModelUnloaded { .. }));
        assert!(runtime.incompatible_model_cleanup.is_none());
        Ok(())
    })
}

#[test]
fn wrong_load_receipt_device_uses_incompatible_model_cleanup() -> TestResult {
    with_runtime(default_test_configuration, |runtime| {
        let (selection, _resolved) =
            resolve_fixture_with(runtime, REPOSITORY, COMMIT, "tokenizer.json")?;
        runtime.load_model(&selection).map_err(application_error)?;
        let (ticket, mut receipt) = receive_successful_load_receipt(runtime)?;
        receipt.execution_device = ExecutionDevice::new(DeviceId::new(0), DeviceKind::Cuda);

        let event = runtime.process_model_loaded(ticket, Ok(receipt));
        assert!(matches!(
            event,
            ApplicationEvent::ModelCompatibilityFailed { .. }
        ));
        assert!(runtime.state().loaded().is_none());
        assert_eq!(runtime.state().selected_device(), ApplicationDevice::Cpu);
        assert_eq!(runtime.state().activity(), ApplicationActivity::Unloading);
        assert!(runtime.incompatible_model_cleanup.is_some());
        assert_eq!(
            runtime.select_device(CUDA_ZERO),
            Err(ApplicationError::DeviceSelectionLocked)
        );

        let event = wait_for_event(runtime, |event| {
            matches!(event, ApplicationEvent::ModelUnloaded { .. })
        })?;
        assert!(matches!(event, ApplicationEvent::ModelUnloaded { .. }));
        assert!(runtime.incompatible_model_cleanup.is_none());
        assert_eq!(runtime.state().activity(), ApplicationActivity::Idle);
        assert_eq!(runtime.state().selected_device(), ApplicationDevice::Cpu);
        Ok(())
    })
}

#[test]
fn incompatible_receipt_disconnect_reports_disconnection_not_pending_cleanup() -> TestResult {
    let database_path = unique_database_path();
    let test_result = (|| {
        let mut configuration = ApplicationRuntimeConfiguration::desktop(&database_path);
        default_test_configuration(&mut configuration);
        let mut runtime = ApplicationRuntime::start(configuration).map_err(application_error)?;
        let (selection, _resolved) =
            resolve_fixture_with(&mut runtime, REPOSITORY, COMMIT, "tokenizer.json")?;
        runtime.load_model(&selection).map_err(application_error)?;
        let (load_ticket, mut receipt) = receive_successful_load_receipt(&mut runtime)?;

        let shutdown_ticket = runtime.next_ticket().map_err(application_error)?;
        runtime
            .submit_inference(RuntimeCommand::Shutdown {
                ticket: shutdown_ticket,
            })
            .map_err(application_error)?;
        let event = runtime
            .local
            .receive_timeout(TEST_TIMEOUT)
            .map_err(|error| format!("shutdown event failed: {error:?}"))?;
        assert!(matches!(
            event,
            RuntimeEvent::Shutdown {
                ticket,
                result: Ok(_),
            } if ticket == shutdown_ticket
        ));
        let thread = runtime
            .local
            .take_thread()
            .ok_or_else(|| "inference thread was already absent".to_owned())?;
        thread
            .join()
            .map_err(|error| format!("inference worker join failed: {error:?}"))?;

        receipt.execution_scalar_type = ScalarType::I8;
        let event = runtime.process_model_loaded(load_ticket, Ok(receipt));
        assert_eq!(event, ApplicationEvent::RuntimeDisconnected);
        assert!(runtime.incompatible_model_cleanup.is_none());
        assert!(runtime.retained_model_cleanup.is_none());
        assert!(runtime.state().loaded().is_none());
        assert_eq!(runtime.state().activity(), ApplicationActivity::Idle);
        assert!(!runtime.state().inference_available());
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
fn retained_load_cleanup_failure_keeps_device_selection_locked() -> TestResult {
    with_runtime(default_test_configuration, |runtime| {
        let (selection, _resolved) =
            resolve_fixture_with(runtime, REPOSITORY, COMMIT, "tokenizer.json")?;
        runtime.load_model(&selection).map_err(application_error)?;
        let (ticket, _receipt) = receive_successful_load_receipt(runtime)?;

        let event =
            runtime.process_model_loaded(ticket, Err(exhausted_failed_load_cleanup_failure()));
        assert!(matches!(
            event,
            ApplicationEvent::ModelCleanupPending {
                exhausted: true,
                failure: crate::ApplicationFailure {
                    kind: ApplicationFailureKind::RetainedCleanup,
                    ..
                }
            }
        ));
        assert_eq!(
            runtime.retained_model_cleanup,
            Some(RetainedModelCleanup::Exhausted)
        );
        assert!(runtime.state().loaded().is_none());
        assert_eq!(runtime.state().activity(), ApplicationActivity::Unloading);
        assert!(!runtime.state().can_select_device());
        assert!(!runtime.state().can_load(&selection));
        assert_eq!(
            runtime.select_device(CUDA_ZERO),
            Err(ApplicationError::DeviceSelectionLocked)
        );
        Ok(())
    })
}

#[test]
fn retained_unload_cleanup_clears_stale_loaded_state_and_locks_admission() -> TestResult {
    with_loaded_runtime(default_test_configuration, |runtime, _loaded| {
        runtime.state.begin_unloading();
        let event = runtime.process_model_unload(
            inference_runtime::CommandTicket::new(76),
            Err(retryable_model_unload_cleanup_failure()),
        );
        assert!(matches!(
            event,
            ApplicationEvent::ModelCleanupPending {
                exhausted: false,
                failure: crate::ApplicationFailure {
                    kind: ApplicationFailureKind::RetainedCleanup,
                    ..
                }
            }
        ));
        assert!(runtime.state().loaded().is_none());
        assert_eq!(runtime.state().activity(), ApplicationActivity::Unloading);
        assert!(!runtime.state().can_start_generation());
        assert!(!runtime.state().can_select_device());
        assert!(matches!(
            runtime.retained_model_cleanup,
            Some(RetainedModelCleanup::PendingInspection { .. })
        ));
        Ok(())
    })
}

#[test]
fn retryable_model_cleanup_returns_idle_after_zero_ownership_snapshot() -> TestResult {
    with_runtime(default_test_configuration, |runtime| {
        let (selection, _resolved) =
            resolve_fixture_with(runtime, REPOSITORY, COMMIT, "tokenizer.json")?;
        runtime.load_model(&selection).map_err(application_error)?;
        let (ticket, _receipt) = receive_successful_load_receipt(runtime)?;

        let event =
            runtime.process_model_loaded(ticket, Err(retryable_failed_load_cleanup_failure()));
        assert!(matches!(
            event,
            ApplicationEvent::ModelCleanupPending {
                exhausted: false,
                failure: crate::ApplicationFailure {
                    kind: ApplicationFailureKind::RetainedCleanup,
                    ..
                }
            }
        ));
        assert!(matches!(
            runtime.retained_model_cleanup,
            Some(RetainedModelCleanup::PendingInspection { .. })
        ));
        assert!(runtime.retry_retained_model_cleanup_inspection().is_none());
        let snapshot_event = runtime
            .local
            .receive_timeout(TEST_TIMEOUT)
            .map_err(|error| format!("cleanup snapshot event failed: {error:?}"))?;
        let inference_runtime::RuntimeEvent::Snapshot {
            ticket: snapshot_ticket,
            ..
        } = snapshot_event
        else {
            return Err("unexpected cleanup inspection event".to_owned());
        };

        let event = runtime
            .process_retained_model_cleanup_snapshot(snapshot_ticket, &RuntimeSnapshot::default());
        assert!(event.is_none());
        assert!(runtime.retained_model_cleanup.is_none());
        assert_eq!(runtime.state().activity(), ApplicationActivity::Idle);
        assert!(runtime.state().can_select_device());
        Ok(())
    })
}

#[test]
fn retryable_sequence_cleanup_snapshot_remains_pending_until_ownership_is_zero() -> TestResult {
    with_runtime(default_test_configuration, |runtime| {
        let ticket = inference_runtime::CommandTicket::new(77);
        runtime.retained_model_cleanup = Some(RetainedModelCleanup::InspectionSubmitted { ticket });
        runtime.state.begin_unloading();
        let snapshot = RuntimeSnapshot {
            loaded_models: 1,
            pending_cleanup_sequences: 1,
            reserved_footprint: domain_contracts::MemoryFootprint {
                host_working_bytes: 1,
                ..domain_contracts::MemoryFootprint::default()
            },
            ..RuntimeSnapshot::default()
        };

        let event = runtime.process_retained_model_cleanup_snapshot(ticket, &snapshot);
        assert!(event.is_none());
        assert!(matches!(
            runtime.retained_model_cleanup,
            Some(RetainedModelCleanup::PendingInspection { .. })
        ));
        assert_eq!(runtime.state().activity(), ApplicationActivity::Unloading);
        Ok(())
    })
}

#[test]
fn exhausted_model_cleanup_snapshot_is_public_and_keeps_selection_locked() -> TestResult {
    with_runtime(default_test_configuration, |runtime| {
        let ticket = inference_runtime::CommandTicket::new(78);
        runtime.retained_model_cleanup = Some(RetainedModelCleanup::InspectionSubmitted { ticket });
        runtime.state.begin_unloading();
        let snapshot = RuntimeSnapshot {
            pending_cleanup_models: 1,
            exhausted_cleanup_models: 1,
            reserved_footprint: domain_contracts::MemoryFootprint {
                host_working_bytes: 1,
                ..domain_contracts::MemoryFootprint::default()
            },
            ..RuntimeSnapshot::default()
        };

        let event = runtime.process_retained_model_cleanup_snapshot(ticket, &snapshot);
        assert!(matches!(
            event,
            Some(ApplicationEvent::ModelCleanupPending {
                exhausted: true,
                ..
            })
        ));
        assert_eq!(
            runtime.retained_model_cleanup,
            Some(RetainedModelCleanup::Exhausted)
        );
        assert_eq!(runtime.state().activity(), ApplicationActivity::Unloading);
        assert!(!runtime.state().can_select_device());
        Ok(())
    })
}

#[test]
fn incompatible_unload_cleanup_failure_is_inspected_until_lower_exhaustion_is_observed()
-> TestResult {
    with_runtime(default_test_configuration, |runtime| {
        let unload_ticket = inference_runtime::CommandTicket::new(79);
        retain_submitted_incompatible_cleanup(runtime, unload_ticket);

        let event = runtime
            .process_model_unload(unload_ticket, Err(retryable_model_unload_cleanup_failure()));
        assert!(matches!(
            event,
            ApplicationEvent::ModelCleanupPending {
                exhausted: false,
                failure: crate::ApplicationFailure {
                    kind: ApplicationFailureKind::RetainedCleanup,
                    ..
                }
            }
        ));
        assert!(matches!(
            runtime.retained_model_cleanup,
            Some(RetainedModelCleanup::PendingInspection { .. })
        ));
        assert!(matches!(
            runtime
                .incompatible_model_cleanup
                .as_ref()
                .map(|cleanup| &cleanup.unload),
            Some(IncompatibleModelUnload::Submitted {
                retry_exhausted: false,
                ..
            })
        ));

        assert!(runtime.retry_retained_model_cleanup_inspection().is_none());
        let snapshot_event = runtime
            .local
            .receive_timeout(TEST_TIMEOUT)
            .map_err(|error| format!("cleanup snapshot event failed: {error:?}"))?;
        let RuntimeEvent::Snapshot {
            ticket: snapshot_ticket,
            ..
        } = snapshot_event
        else {
            return Err("unexpected cleanup inspection event".to_owned());
        };
        let snapshot = RuntimeSnapshot {
            exhausted_cleanup_models: 1,
            ..RuntimeSnapshot::default()
        };

        let event = runtime.process_retained_model_cleanup_snapshot(snapshot_ticket, &snapshot);
        assert!(matches!(
            event,
            Some(ApplicationEvent::ModelCleanupPending {
                exhausted: true,
                failure: crate::ApplicationFailure {
                    kind: ApplicationFailureKind::RetainedCleanup,
                    ..
                }
            })
        ));
        assert_eq!(
            runtime.retained_model_cleanup,
            Some(RetainedModelCleanup::Exhausted)
        );
        assert!(runtime.incompatible_model_cleanup.is_some());
        assert_eq!(runtime.state().activity(), ApplicationActivity::Unloading);
        assert!(!runtime.state().can_select_device());
        Ok(())
    })
}

#[test]
fn incompatible_cleanup_zero_ownership_snapshot_releases_both_trackers() -> TestResult {
    with_runtime(default_test_configuration, |runtime| {
        let unload_ticket = inference_runtime::CommandTicket::new(80);
        retain_submitted_incompatible_cleanup(runtime, unload_ticket);

        let event = runtime
            .process_model_unload(unload_ticket, Err(retryable_model_unload_cleanup_failure()));
        assert!(matches!(
            event,
            ApplicationEvent::ModelCleanupPending {
                exhausted: false,
                ..
            }
        ));
        assert!(runtime.retry_retained_model_cleanup_inspection().is_none());
        let snapshot_event = runtime
            .local
            .receive_timeout(TEST_TIMEOUT)
            .map_err(|error| format!("cleanup snapshot event failed: {error:?}"))?;
        let RuntimeEvent::Snapshot {
            ticket: snapshot_ticket,
            ..
        } = snapshot_event
        else {
            return Err("unexpected cleanup inspection event".to_owned());
        };

        let event = runtime
            .process_retained_model_cleanup_snapshot(snapshot_ticket, &RuntimeSnapshot::default());
        assert!(event.is_none());
        assert!(runtime.retained_model_cleanup.is_none());
        assert!(runtime.incompatible_model_cleanup.is_none());
        assert_eq!(runtime.state().activity(), ApplicationActivity::Idle);
        assert!(runtime.state().can_select_device());
        Ok(())
    })
}

#[test]
fn incompatible_model_cleanup_retries_after_automatic_unload_submission_failure() -> TestResult {
    with_runtime(default_test_configuration, |runtime| {
        let (selection, _resolved) =
            resolve_fixture_with(runtime, REPOSITORY, COMMIT, "tokenizer.json")?;
        runtime.load_model(&selection).map_err(application_error)?;
        runtime
            .pending_load
            .as_mut()
            .ok_or_else(|| "load admission evidence was not retained".to_owned())?
            .configuration_declared_scalar_type = Some(ApplicationScalarType::Bf16);
        runtime.forced_inference_busy_submissions = 1;

        let event = wait_for_event(runtime, |event| {
            matches!(
                event,
                ApplicationEvent::ModelCleanupPending {
                    exhausted: false,
                    ..
                }
            )
        })?;
        assert!(matches!(
            event,
            ApplicationEvent::ModelCleanupPending {
                exhausted: false,
                failure: crate::ApplicationFailure {
                    kind: ApplicationFailureKind::RetainedCleanup,
                    ..
                }
            }
        ));
        let retained = runtime
            .incompatible_model_cleanup
            .as_ref()
            .ok_or_else(|| "incompatible model ownership was not retained".to_owned())?;
        let retained_handle = retained.handle;
        assert!(
            retained
                .compatibility_failure
                .message
                .contains("compatibility")
        );
        assert!(matches!(
            retained.unload,
            IncompatibleModelUnload::PendingSubmission { attempts: 1, .. }
        ));
        assert_eq!(runtime.state().activity(), ApplicationActivity::Unloading);
        assert!(runtime.state().loaded().is_none());

        let event = wait_for_event(runtime, |event| {
            matches!(
                event,
                ApplicationEvent::ModelUnloaded { handle, .. } if *handle == retained_handle
            )
        })?;
        assert!(matches!(event, ApplicationEvent::ModelUnloaded { .. }));
        assert!(runtime.incompatible_model_cleanup.is_none());
        assert_eq!(runtime.state().activity(), ApplicationActivity::Idle);
        Ok(())
    })
}

#[test]
fn incompatible_model_cleanup_exhaustion_remains_accounted() -> TestResult {
    with_runtime(default_test_configuration, |runtime| {
        let (selection, _resolved) =
            resolve_fixture_with(runtime, REPOSITORY, COMMIT, "tokenizer.json")?;
        runtime.load_model(&selection).map_err(application_error)?;
        runtime
            .pending_load
            .as_mut()
            .ok_or_else(|| "load admission evidence was not retained".to_owned())?
            .configuration_declared_scalar_type = Some(ApplicationScalarType::Bf16);
        runtime.forced_inference_busy_submissions =
            usize::from(MAXIMUM_INCOMPATIBLE_UNLOAD_SUBMISSION_ATTEMPTS);

        let _initial_failure = wait_for_event(runtime, |event| {
            matches!(
                event,
                ApplicationEvent::ModelCleanupPending {
                    exhausted: false,
                    ..
                }
            )
        })?;
        let mut saw_exhausted = false;
        for _ in 1..MAXIMUM_INCOMPATIBLE_UNLOAD_SUBMISSION_ATTEMPTS {
            let retry_failure = wait_for_event(runtime, |event| {
                matches!(event, ApplicationEvent::ModelCleanupPending { .. })
            })?;
            if matches!(
                retry_failure,
                ApplicationEvent::ModelCleanupPending {
                    exhausted: true,
                    ..
                }
            ) {
                saw_exhausted = true;
            }
        }
        assert!(saw_exhausted);

        let retained = runtime
            .incompatible_model_cleanup
            .as_ref()
            .ok_or_else(|| "exhausted incompatible model ownership was dropped".to_owned())?;
        assert!(
            retained
                .compatibility_failure
                .message
                .contains("compatibility")
        );
        assert!(matches!(
            retained.unload,
            IncompatibleModelUnload::RetryExhausted {
                attempts: MAXIMUM_INCOMPATIBLE_UNLOAD_SUBMISSION_ATTEMPTS,
                ..
            }
        ));
        assert_eq!(runtime.state().activity(), ApplicationActivity::Unloading);
        assert!(runtime.state().loaded().is_none());
        assert!(!runtime.state().can_select_device());
        Ok(())
    })
}
