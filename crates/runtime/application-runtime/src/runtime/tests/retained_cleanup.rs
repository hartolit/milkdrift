use domain_contracts::{DeviceId, DeviceKind, ExecutionDevice, ScalarType};
use inference_runtime::RuntimeSnapshot;

use super::support::*;
use crate::runtime::retained_cleanup::{
    IncompatibleModelUnload, MAXIMUM_INCOMPATIBLE_UNLOAD_SUBMISSION_ATTEMPTS, RetainedLoadCleanup,
};
use crate::{
    ApplicationActivity, ApplicationDevice, ApplicationError, ApplicationEvent,
    ApplicationScalarType,
};

#[test]
fn incompatible_source_scalar_evidence_unloads_without_publishing_loaded_state() -> TestResult {
    with_runtime(default_test_configuration, |runtime| {
        let (selection, _resolved) =
            resolve_fixture_with(runtime, REPOSITORY, COMMIT, "tokenizer.json")?;
        runtime.load_model(&selection).map_err(application_error)?;
        runtime
            .pending_load
            .as_mut()
            .ok_or_else(|| "load admission evidence was not retained".to_owned())?
            .source_scalar_type = ApplicationScalarType::Bf16;

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
fn mismatched_supported_execution_scalar_receipt_uses_incompatible_model_cleanup() -> TestResult {
    with_runtime(default_test_configuration, |runtime| {
        let (selection, _resolved) =
            resolve_fixture_with(runtime, REPOSITORY, COMMIT, "tokenizer.json")?;
        runtime.load_model(&selection).map_err(application_error)?;
        let (ticket, mut receipt) = receive_successful_load_receipt(runtime)?;
        let handle = receipt.handle;
        receipt.execution_scalar_type = ScalarType::F16;

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
fn retained_load_cleanup_failure_keeps_device_selection_locked() -> TestResult {
    with_runtime(default_test_configuration, |runtime| {
        let (selection, _resolved) =
            resolve_fixture_with(runtime, REPOSITORY, COMMIT, "tokenizer.json")?;
        runtime.load_model(&selection).map_err(application_error)?;
        let (ticket, _receipt) = receive_successful_load_receipt(runtime)?;

        let event = runtime.process_model_loaded(ticket, Err(terminal_cleanup_failure()));
        assert!(matches!(event, ApplicationEvent::ModelLoadFailed { .. }));
        assert_eq!(
            runtime.retained_load_cleanup,
            Some(RetainedLoadCleanup::Exhausted)
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
fn retryable_load_cleanup_returns_idle_after_zero_ownership_snapshot() -> TestResult {
    with_runtime(default_test_configuration, |runtime| {
        let (selection, _resolved) =
            resolve_fixture_with(runtime, REPOSITORY, COMMIT, "tokenizer.json")?;
        runtime.load_model(&selection).map_err(application_error)?;
        let (ticket, _receipt) = receive_successful_load_receipt(runtime)?;

        let event = runtime.process_model_loaded(ticket, Err(retryable_cleanup_failure()));
        assert!(matches!(event, ApplicationEvent::ModelLoadFailed { .. }));
        assert!(matches!(
            runtime.retained_load_cleanup,
            Some(RetainedLoadCleanup::PendingInspection { .. })
        ));
        assert!(runtime.retry_retained_load_cleanup_inspection().is_none());
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

        runtime
            .process_retained_load_cleanup_snapshot(snapshot_ticket, &RuntimeSnapshot::default());
        assert!(runtime.retained_load_cleanup.is_none());
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
            .source_scalar_type = ApplicationScalarType::Bf16;
        runtime.forced_inference_busy_submissions = 1;

        let event = wait_for_event(runtime, |event| {
            matches!(event, ApplicationEvent::ModelLoadFailed { .. })
        })?;
        assert!(matches!(event, ApplicationEvent::ModelLoadFailed { .. }));
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
            .source_scalar_type = ApplicationScalarType::Bf16;
        runtime.forced_inference_busy_submissions =
            usize::from(MAXIMUM_INCOMPATIBLE_UNLOAD_SUBMISSION_ATTEMPTS);

        let _initial_failure = wait_for_event(runtime, |event| {
            matches!(event, ApplicationEvent::ModelLoadFailed { .. })
        })?;
        for _ in 1..MAXIMUM_INCOMPATIBLE_UNLOAD_SUBMISSION_ATTEMPTS {
            let _retry_failure = wait_for_event(runtime, |event| {
                matches!(event, ApplicationEvent::ModelUnloadFailed { .. })
            })?;
        }

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
