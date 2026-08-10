//! Download-free CUDA hardware scenarios that require private E1 test seams.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use hf_hub_adapter::{
    ArtifactContentIdentity, ArtifactContentIdentityAuthority, ArtifactScalarType,
    ResolvedSafetensorsLlamaArtifacts, ResolvedSafetensorsShard,
};

use super::ApplicationRuntime;
use crate::local::{DeviceProbe, DeviceProbeFailure, DeviceProbeResult, probe_application_device};
use crate::{
    ApplicationActivity, ApplicationComputeCapability, ApplicationDevice,
    ApplicationDeviceDiscoveryFailure, ApplicationDeviceDiscoveryFailureKind,
    ApplicationDeviceSummary, ApplicationDeviceUnavailableReason, ApplicationError,
    ApplicationEvent, ApplicationRuntimeConfiguration, ApplicationScalarType, LoadedModel,
    ModelSelection,
};

const REPOSITORY: &str = "fixture/tiny-llama";
const REVISION: &str = "phase7";
const COMMIT: &str = "fixture";
const TEST_TIMEOUT: Duration = Duration::from_secs(10);
const TEST_POLL: Duration = Duration::from_millis(1);
const TEST_CUDA_MEMORY_BYTES: u64 = 8 * 1024 * 1024 * 1024;
const CUDA_ZERO: ApplicationDevice = ApplicationDevice::Cuda { ordinal: 0 };
const CANDLE_FIXTURE_WEIGHT_BYTES: u64 = 4_800;
const CANDLE_FIXTURE_WEIGHT_SHA256: [u8; 32] = [
    0xcc, 0x47, 0x98, 0xaf, 0x93, 0x48, 0x8b, 0x4f, 0xb2, 0xae, 0x05, 0x48, 0xc2, 0xb2, 0x8a, 0xce,
    0x60, 0x05, 0x21, 0x73, 0x2b, 0x52, 0x02, 0x3a, 0x77, 0x86, 0xc3, 0x22, 0x7d, 0x72, 0xd6, 0x72,
];

static NEXT_DATABASE_ID: AtomicU64 = AtomicU64::new(1);

type TestResult<T = ()> = Result<T, String>;

struct HardwareCase {
    name: &'static str,
    run: fn() -> TestResult,
}

macro_rules! hardware_cases {
    ($($case:ident),+ $(,)?) => {
        const HARDWARE_CASES: &[HardwareCase] = &[
            $(HardwareCase {
                name: stringify!($case),
                run: $case,
            }),+
        ];
    };
}

hardware_cases!(
    unavailable_selected_cuda_blocks_load_without_fallback,
    cuda_fixture_load_reports_selected_and_actual_e1_device,
);

pub(crate) fn run_hardware_suite() -> TestResult {
    require_cuda_opt_in()?;
    if HARDWARE_CASES.is_empty() {
        return Err("E1 CUDA hardware suite registered zero cases".to_owned());
    }

    let mut executed = 0_usize;
    for case in HARDWARE_CASES {
        executed = executed.saturating_add(1);
        eprintln!("running E1 CUDA case: {}", case.name);
        (case.run)().map_err(|error| format!("E1 CUDA case {} failed: {error}", case.name))?;
    }
    if executed != HARDWARE_CASES.len() {
        return Err(format!(
            "E1 CUDA suite executed {executed} of {} registered cases",
            HARDWARE_CASES.len()
        ));
    }
    eprintln!("E1 CUDA suite passed {executed} cases");
    Ok(())
}

fn require_cuda_opt_in() -> TestResult {
    if std::env::var("MILKDRIFT_CUDA_TEST").as_deref() == Ok("1") {
        Ok(())
    } else {
        Err("set MILKDRIFT_CUDA_TEST=1 to execute the E1 CUDA hardware suite".to_owned())
    }
}

#[expect(
    clippy::unnecessary_wraps,
    reason = "the deterministic probe conforms to the private fallible discovery seam"
)]
fn available_device_probe(device: ApplicationDevice) -> DeviceProbeResult {
    match device {
        ApplicationDevice::Cpu => Ok(ApplicationDeviceSummary::cpu()),
        ApplicationDevice::Cuda { .. } => Ok(ApplicationDeviceSummary::discovered(
            device,
            Some("deterministic CUDA hardware-suite device".to_owned()),
            Some(TEST_CUDA_MEMORY_BYTES),
            Some(TEST_CUDA_MEMORY_BYTES / 2),
            Some(ApplicationComputeCapability {
                major: 12,
                minor: 0,
            }),
        )),
    }
}

fn unavailable_cuda_probe(device: ApplicationDevice) -> DeviceProbeResult {
    match device {
        ApplicationDevice::Cpu => Ok(ApplicationDeviceSummary::cpu()),
        ApplicationDevice::Cuda { .. } => Err(DeviceProbeFailure::Discovery(
            ApplicationDeviceDiscoveryFailure::new(
                device,
                ApplicationDeviceDiscoveryFailureKind::Initialization,
                "deterministic device initialization failure".to_owned(),
            ),
        )),
    }
}

fn unavailable_selected_cuda_blocks_load_without_fallback() -> TestResult {
    with_runtime_and_probe(available_device_probe, |runtime| {
        runtime
            .select_device(CUDA_ZERO)
            .map_err(application_error)?;
        runtime.device_probe = unavailable_cuda_probe;
        let selection = accept_fixture(runtime)?;

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
        assert_eq!(runtime.state().device_discovery_failures().len(), 1);
        Ok(())
    })
}

fn cuda_fixture_load_reports_selected_and_actual_e1_device() -> TestResult {
    with_runtime_and_probe(probe_application_device, |runtime| {
        let cuda = runtime
            .state()
            .devices()
            .iter()
            .find(|summary| summary.device() == CUDA_ZERO && summary.available())
            .ok_or_else(|| "CUDA ordinal 0 was not discovered by E1".to_owned())?;
        assert!(cuda.display_name().is_some());
        eprintln!(
            "E1 discovered {:?} ({:?}) with total={:?} available={:?} compute={:?}",
            cuda.device(),
            cuda.display_name(),
            cuda.total_memory_bytes(),
            cuda.available_memory_bytes(),
            cuda.compute_capability(),
        );

        runtime
            .select_device(CUDA_ZERO)
            .map_err(application_error)?;
        assert_eq!(runtime.state().selected_device(), CUDA_ZERO);
        assert!(runtime.state().loaded().is_none());
        assert_eq!(runtime.state().activity(), ApplicationActivity::Idle);

        let loaded = load_fixture(runtime)?;
        assert_eq!(loaded.device(), CUDA_ZERO);
        assert_eq!(loaded.execution_scalar_type(), ApplicationScalarType::F32);
        assert_eq!(runtime.state().selected_device(), CUDA_ZERO);

        runtime.unload_model().map_err(application_error)?;
        let _unloaded = wait_for_event(runtime, |event| {
            matches!(event, ApplicationEvent::ModelUnloaded { .. })
        })?;
        assert!(runtime.state().loaded().is_none());
        assert_eq!(runtime.state().selected_device(), CUDA_ZERO);
        Ok(())
    })
}

fn with_runtime_and_probe<F>(device_probe: DeviceProbe, test: F) -> TestResult
where
    F: FnOnce(&mut ApplicationRuntime) -> TestResult,
{
    let database_path = unique_database_path();
    let mut configuration = ApplicationRuntimeConfiguration::new(&database_path);
    configuration.defaults.maximum_host_memory_bytes = u64::MAX;
    configuration.defaults.drain_timeout_milliseconds = 5_000;
    configuration.timing.runtime_poll = TEST_POLL;
    configuration.timing.hub_worker_poll = TEST_POLL;

    let result = match ApplicationRuntime::start_with_device_probe(configuration, device_probe) {
        Ok(mut runtime) => {
            let test_result = test(&mut runtime);
            let shutdown_result = runtime.shutdown().map_err(application_error);
            test_result.and(shutdown_result)
        }
        Err(error) => Err(application_error(error)),
    };
    let cleanup_result = remove_database(&database_path);
    result.and(cleanup_result)
}

fn accept_fixture(runtime: &mut ApplicationRuntime) -> TestResult<ModelSelection> {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let candle = manifest.join("../inference-runtime/tests/fixtures/candle-llama");
    let artifacts = ResolvedSafetensorsLlamaArtifacts {
        repository: REPOSITORY.to_owned(),
        revision: REVISION.to_owned(),
        commit: COMMIT.to_owned(),
        configuration_declared_scalar_type: Some(ArtifactScalarType::F32),
        config_path: canonical(candle.join("config.json"))?,
        tokenizer_path: canonical(manifest.join("tests/fixtures/tokenizer.json"))?,
        weight_shards: vec![ResolvedSafetensorsShard {
            path: canonical(candle.join("model.safetensors"))?,
            content_identity: ArtifactContentIdentity {
                byte_length: CANDLE_FIXTURE_WEIGHT_BYTES,
                sha256: CANDLE_FIXTURE_WEIGHT_SHA256,
                authority: ArtifactContentIdentityAuthority::ProjectEstablished,
            },
        }],
    };
    let selection = ModelSelection::new(REPOSITORY, REVISION);
    match runtime.accept_resolved_artifacts(artifacts) {
        ApplicationEvent::ModelResolved {
            model,
            persistence_warning,
        } => {
            assert!(persistence_warning.is_none());
            assert_eq!(model.selection(), &selection);
            assert_eq!(model.identity().repository(), REPOSITORY);
            assert_eq!(model.identity().commit(), COMMIT);
            Ok(selection)
        }
        event => Err(format!("unexpected fixture-resolution event: {event:?}")),
    }
}

fn load_fixture(runtime: &mut ApplicationRuntime) -> TestResult<LoadedModel> {
    let selection = accept_fixture(runtime)?;
    runtime.load_model(&selection).map_err(application_error)?;
    match wait_for_event(runtime, |event| {
        matches!(
            event,
            ApplicationEvent::ModelLoaded { .. }
                | ApplicationEvent::ModelLoadFailed { .. }
                | ApplicationEvent::ModelCleanupPending { .. }
                | ApplicationEvent::ModelCompatibilityFailed { .. }
        )
    })? {
        ApplicationEvent::ModelLoaded { model } => {
            assert_eq!(model.selection(), &selection);
            assert_eq!(model.device(), CUDA_ZERO);
            assert_eq!(model.identity().repository(), REPOSITORY);
            assert_eq!(model.identity().commit(), COMMIT);
            Ok(model)
        }
        event => Err(format!("fixture model did not load: {event:?}")),
    }
}

fn wait_for_event<F>(
    runtime: &mut ApplicationRuntime,
    mut matches: F,
) -> TestResult<ApplicationEvent>
where
    F: FnMut(&ApplicationEvent) -> bool,
{
    let deadline = deadline()?;
    loop {
        if let Some(event) = runtime.poll_event()
            && matches(&event)
        {
            return Ok(event);
        }
        if Instant::now() >= deadline {
            return Err("timed out waiting for an application event".to_owned());
        }
        std::thread::sleep(TEST_POLL);
    }
}

fn deadline() -> TestResult<Instant> {
    Instant::now()
        .checked_add(TEST_TIMEOUT)
        .ok_or_else(|| "test deadline overflow".to_owned())
}

fn unique_database_path() -> PathBuf {
    let identifier = NEXT_DATABASE_ID.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!(
        "milkdrift-e1-cuda-hardware-{}-{identifier}.redb",
        std::process::id()
    ))
}

fn remove_database(path: &Path) -> TestResult {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!("failed to remove test database: {error}")),
    }
}

fn canonical(path: impl AsRef<Path>) -> TestResult<PathBuf> {
    let path = path.as_ref();
    fs::canonicalize(path)
        .map_err(|error| format!("failed to resolve fixture path {}: {error}", path.display()))
}

fn application_error(error: impl std::fmt::Display) -> String {
    error.to_string()
}
