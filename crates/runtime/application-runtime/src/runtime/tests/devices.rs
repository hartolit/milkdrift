use std::sync::atomic::{AtomicU64, Ordering};

use domain_contracts::{ByteCount, DeviceId, DeviceKind, ExecutionDevice};
use redb_storage::{
    ApplicationSettings as StoredApplicationSettings, RedbStorage, StoredAcceleratorMemoryPolicy,
    StoredApplicationDevice,
};

use super::support::*;
use crate::{
    AcceleratorMemoryPolicy, ApplicationComputeCapability, ApplicationDevice,
    ApplicationDeviceDiscoveryFailure, ApplicationDeviceDiscoveryFailureKind,
    ApplicationDeviceSummary, ApplicationDeviceUnavailableReason, ApplicationError,
    ApplicationEvent,
};

static COUNTED_CUDA_PROBES: AtomicU64 = AtomicU64::new(0);
static SHRINKING_CUDA_THREE_PROBES: AtomicU64 = AtomicU64::new(0);

fn available_device_probe(device: ApplicationDevice) -> crate::local::DeviceProbeResult {
    match device {
        ApplicationDevice::Cpu => Ok(ApplicationDeviceSummary::cpu()),
        ApplicationDevice::Cuda { .. } => Ok(ApplicationDeviceSummary::discovered(
            device,
            Some("deterministic test device".to_owned()),
            Some(ByteCount::from_u64(TEST_CUDA_MEMORY_BYTES)),
            Some(ByteCount::from_u64(TEST_CUDA_MEMORY_BYTES / 2)),
            Some(ApplicationComputeCapability {
                major: 12,
                minor: 0,
            }),
        )),
    }
}

fn unavailable_cuda_probe(device: ApplicationDevice) -> crate::local::DeviceProbeResult {
    match device {
        ApplicationDevice::Cpu => Ok(ApplicationDeviceSummary::cpu()),
        ApplicationDevice::Cuda { .. } => Err(crate::local::DeviceProbeFailure::Discovery(
            ApplicationDeviceDiscoveryFailure::new(
                device,
                ApplicationDeviceDiscoveryFailureKind::Initialization,
                "deterministic device initialization failure".to_owned(),
            ),
        )),
    }
}

fn counted_unavailable_cuda_probe(device: ApplicationDevice) -> crate::local::DeviceProbeResult {
    if matches!(device, ApplicationDevice::Cuda { .. }) {
        COUNTED_CUDA_PROBES.fetch_add(1, Ordering::Relaxed);
    }
    unavailable_cuda_probe(device)
}

fn shrinking_cuda_probe(device: ApplicationDevice) -> crate::local::DeviceProbeResult {
    match device {
        ApplicationDevice::Cpu => Ok(ApplicationDeviceSummary::cpu()),
        ApplicationDevice::Cuda { ordinal } => {
            let probe = if ordinal == 3 {
                SHRINKING_CUDA_THREE_PROBES.fetch_add(1, Ordering::Relaxed)
            } else {
                0
            };
            let total_memory_bytes = if ordinal == 3 && probe > 0 {
                TEST_CUDA_MEMORY_BYTES / 2
            } else {
                TEST_CUDA_MEMORY_BYTES
            };
            Ok(ApplicationDeviceSummary::discovered(
                device,
                Some("shrinking deterministic test device".to_owned()),
                Some(ByteCount::from_u64(total_memory_bytes)),
                Some(ByteCount::from_u64(total_memory_bytes / 2)),
                None,
            ))
        }
    }
}

#[test]
fn fresh_configuration_selects_cpu_and_keeps_cpu_catalogue_identity() -> TestResult {
    with_runtime(default_test_configuration, |runtime| {
        assert_eq!(
            runtime.preferences().selected_device,
            ApplicationDevice::Cpu
        );
        assert_eq!(
            runtime.preferences().accelerator_memory_policy,
            AcceleratorMemoryPolicy::Automatic
        );
        assert_eq!(runtime.state().selected_device(), ApplicationDevice::Cpu);
        assert!(runtime.state().selected_device_available());
        assert!(runtime.state().can_select_device());
        let cpu = runtime
            .state()
            .devices()
            .first()
            .ok_or_else(|| "CPU was absent from the device catalogue".to_owned())?;
        assert_eq!(cpu.device(), ApplicationDevice::Cpu);
        assert_eq!(cpu.display_name(), None);
        assert!(cpu.available());
        #[cfg(not(feature = "cuda"))]
        assert_eq!(runtime.state().devices(), std::slice::from_ref(cpu));
        Ok(())
    })
}

#[test]
fn selected_cuda_and_memory_limit_persist_across_restart() -> TestResult {
    let database_path = unique_database_path();
    let limit = ByteCount::from_u64(TEST_CUDA_MEMORY_BYTES / 2);
    let result = with_runtime_at_with_probe(
        &database_path,
        |configuration| {
            default_test_configuration(configuration);
            configuration.defaults.accelerator_memory_policy =
                AcceleratorMemoryPolicy::Limit { bytes: limit };
        },
        available_device_probe,
        |runtime| {
            runtime
                .select_device(CUDA_ZERO)
                .map_err(application_error)?;
            assert_eq!(runtime.state().selected_device(), CUDA_ZERO);
            assert_eq!(runtime.memory_budget.device_bytes(), limit);
            Ok(())
        },
    )
    .and_then(|()| {
        with_runtime_at_with_probe(
            &database_path,
            default_test_configuration,
            available_device_probe,
            |runtime| {
                assert_eq!(runtime.preferences().selected_device, CUDA_ZERO);
                assert_eq!(runtime.state().selected_device(), CUDA_ZERO);
                assert!(runtime.state().selected_device_available());
                assert_eq!(
                    runtime.preferences().accelerator_memory_policy,
                    AcceleratorMemoryPolicy::Limit { bytes: limit }
                );
                assert_eq!(runtime.memory_budget.device_bytes(), limit);
                Ok(())
            },
        )
    })
    .and_then(|()| {
        with_runtime_at_with_probe(
            &database_path,
            default_test_configuration,
            unavailable_cuda_probe,
            |runtime| {
                assert_eq!(runtime.preferences().selected_device, CUDA_ZERO);
                assert_eq!(runtime.state().selected_device(), CUDA_ZERO);
                assert!(!runtime.state().selected_device_available());
                assert_eq!(
                    runtime.state().selected_device_unavailable_reason(),
                    Some(ApplicationDeviceUnavailableReason::DiscoveryFailed)
                );
                assert_eq!(runtime.state().device_discovery_failures().len(), 1);
                Ok(())
            },
        )
    });

    let cleanup_result = remove_database(&database_path);
    result.and(cleanup_result)
}

#[test]
fn persisted_nonzero_cuda_uses_exactly_two_bounded_startup_probes() -> TestResult {
    let database_path = unique_database_path();
    let storage = RedbStorage::open(&database_path).map_err(application_error)?;
    storage
        .save_settings(
            &StoredApplicationSettings::new(
                String::new(),
                "main".to_owned(),
                16 * 1024 * 1024 * 1024,
                StoredApplicationDevice::Cuda { ordinal: 3 },
                StoredAcceleratorMemoryPolicy::Automatic,
                2_000,
            )
            .map_err(application_error)?,
        )
        .map_err(application_error)?;
    drop(storage);
    COUNTED_CUDA_PROBES.store(0, Ordering::Relaxed);

    let result = with_runtime_at_with_probe(
        &database_path,
        default_test_configuration,
        counted_unavailable_cuda_probe,
        |runtime| {
            assert_eq!(COUNTED_CUDA_PROBES.load(Ordering::Relaxed), 2);
            assert_eq!(
                runtime.state().selected_device(),
                ApplicationDevice::Cuda { ordinal: 3 }
            );
            assert_eq!(runtime.state().devices().len(), 2);
            assert_eq!(
                runtime
                    .state()
                    .devices()
                    .first()
                    .map(ApplicationDeviceSummary::device),
                Some(ApplicationDevice::Cpu)
            );
            assert_eq!(
                runtime.state().selected_device_unavailable_reason(),
                Some(ApplicationDeviceUnavailableReason::DiscoveryFailed)
            );
            assert_eq!(runtime.state().device_discovery_failures().len(), 2);
            Ok(())
        },
    );

    let cleanup_result = remove_database(&database_path);
    result.and(cleanup_result)
}

#[test]
fn unavailable_selected_cuda_blocks_load_without_fallback() -> TestResult {
    with_runtime_and_probe(
        default_test_configuration,
        available_device_probe,
        |runtime| {
            runtime
                .select_device(CUDA_ZERO)
                .map_err(application_error)?;
            runtime.device_probe = unavailable_cuda_probe;
            let (selection, _resolved) =
                resolve_fixture_with(runtime, REPOSITORY, COMMIT, "tokenizer.json")?;

            assert_eq!(
                runtime.load_model(&selection),
                Err(ApplicationError::SelectedDeviceUnavailable {
                    device: CUDA_ZERO,
                    reason: ApplicationDeviceUnavailableReason::DiscoveryFailed,
                })
            );
            assert_eq!(runtime.state().selected_device(), CUDA_ZERO);
            assert!(!runtime.state().selected_device_available());
            assert!(!runtime.state().can_load(&selection));
            assert!(runtime.state().loaded().is_none());
            assert!(runtime.last_submitted_load_device.is_none());
            assert_eq!(runtime.state().device_discovery_failures().len(), 1);
            Ok(())
        },
    )
}

#[test]
fn selected_cuda_capacity_shrink_blocks_load_without_fallback() -> TestResult {
    let database_path = unique_database_path();
    let storage = RedbStorage::open(&database_path).map_err(application_error)?;
    storage
        .save_settings(
            &StoredApplicationSettings::new(
                String::new(),
                "main".to_owned(),
                16 * 1024 * 1024 * 1024,
                StoredApplicationDevice::Cuda { ordinal: 3 },
                StoredAcceleratorMemoryPolicy::Automatic,
                2_000,
            )
            .map_err(application_error)?,
        )
        .map_err(application_error)?;
    drop(storage);
    SHRINKING_CUDA_THREE_PROBES.store(0, Ordering::Relaxed);

    let selected_device = ApplicationDevice::Cuda { ordinal: 3 };
    let result = with_runtime_at_with_probe(
        &database_path,
        default_test_configuration,
        shrinking_cuda_probe,
        |runtime| {
            assert_eq!(runtime.state().selected_device(), selected_device);
            assert!(runtime.state().selected_device_available());
            assert_eq!(
                runtime.memory_budget.device_bytes(),
                ByteCount::from_u64(TEST_CUDA_MEMORY_BYTES)
            );
            let (selection, _resolved) =
                resolve_fixture_with(runtime, REPOSITORY, COMMIT, "tokenizer.json")?;

            assert!(matches!(
                runtime.load_model(&selection),
                Err(ApplicationError::SelectedDeviceMemoryBudgetUnavailable {
                    device,
                    budget_bytes,
                    total_memory_bytes,
                }) if device == selected_device
                    && budget_bytes == ByteCount::from_u64(TEST_CUDA_MEMORY_BYTES)
                    && total_memory_bytes
                        == Some(ByteCount::from_u64(TEST_CUDA_MEMORY_BYTES / 2))
            ));
            assert_eq!(runtime.state().selected_device(), selected_device);
            assert!(runtime.state().selected_device_available());
            assert!(!runtime.state().can_load(&selection));
            assert!(runtime.state().loaded().is_none());
            assert!(runtime.last_submitted_load_device.is_none());
            Ok(())
        },
    );

    let cleanup_result = remove_database(&database_path);
    result.and(cleanup_result)
}

#[test]
fn load_command_uses_the_exact_selected_cuda_device() -> TestResult {
    with_runtime_and_probe(
        default_test_configuration,
        available_device_probe,
        |runtime| {
            runtime
                .select_device(CUDA_ZERO)
                .map_err(application_error)?;
            let (selection, _resolved) =
                resolve_fixture_with(runtime, REPOSITORY, COMMIT, "tokenizer.json")?;
            runtime.load_model(&selection).map_err(application_error)?;

            assert_eq!(
                runtime.last_submitted_load_device,
                Some(ExecutionDevice::new(DeviceId::new(0), DeviceKind::Cuda))
            );
            assert_eq!(runtime.state().selected_device(), CUDA_ZERO);
            Ok(())
        },
    )
}

#[test]
fn device_selection_is_rejected_while_loading_or_unloading() -> TestResult {
    with_runtime_and_probe(
        default_test_configuration,
        available_device_probe,
        |runtime| {
            let (selection, _resolved) =
                resolve_fixture_with(runtime, REPOSITORY, COMMIT, "tokenizer.json")?;
            runtime.load_model(&selection).map_err(application_error)?;
            assert_eq!(
                runtime.select_device(CUDA_ZERO),
                Err(ApplicationError::DeviceSelectionLocked)
            );
            let (ticket, receipt) = receive_successful_load_receipt(runtime)?;
            let event = runtime.process_model_loaded(ticket, &Ok(receipt));
            if !matches!(event, Some(ApplicationEvent::ModelLoaded { .. })) {
                return Err(format!("load completion was not published: {event:?}"));
            }

            runtime.unload_model().map_err(application_error)?;
            assert_eq!(
                runtime.select_device(CUDA_ZERO),
                Err(ApplicationError::DeviceSelectionLocked)
            );
            Ok(())
        },
    )
}

#[test]
fn device_selection_is_rejected_while_loaded_generating_and_unloading() -> TestResult {
    with_loaded_runtime(default_test_configuration, |runtime, _loaded| {
        assert_eq!(
            runtime.select_device(CUDA_ZERO),
            Err(ApplicationError::DeviceSelectionLocked)
        );

        let request_id = runtime
            .start_generation("prompt seed", deterministic_settings(1))
            .map_err(application_error)?;
        assert_eq!(
            runtime.select_device(CUDA_ZERO),
            Err(ApplicationError::DeviceSelectionLocked)
        );
        wait_for_generation_started(runtime, request_id)?;
        let _generation = collect_generation(runtime, request_id)?;

        runtime.unload_model().map_err(application_error)?;
        assert_eq!(
            runtime.select_device(CUDA_ZERO),
            Err(ApplicationError::DeviceSelectionLocked)
        );
        let _unloaded = wait_for_event(runtime, |event| {
            matches!(event, ApplicationEvent::ModelUnloaded { .. })
        })?;
        assert_eq!(runtime.state().selected_device(), ApplicationDevice::Cpu);
        assert!(runtime.state().loaded().is_none());
        Ok(())
    })
}
