//! Transactional worker startup, rollback, and retained startup cleanup.

use std::sync::{Mutex, MutexGuard};

use crate::generation::GenerationBridge;
use crate::hub_worker::{HubWorker, start_hub_worker};
use crate::local::{DeviceProbe, LocalInference, probe_application_device};
use crate::runtime::devices::{DeviceCatalogue, discover_device_catalogue};
use crate::support::{
    application_preferences, create_runtime, hub_configuration, runtime_memory_budget,
    storage_failure, validate_configuration, validate_preferences,
};
use crate::{
    ApplicationError, ApplicationFailure, ApplicationFailureKind, ApplicationRuntime,
    ApplicationRuntimeConfiguration, ApplicationState, ApplicationTiming, ApplicationWorker,
};
use redb_storage::RedbStorage;

const INITIAL_COMMAND_TICKET: u64 = 1;

type StartupInferenceRollback =
    fn(&mut LocalInference, ApplicationTiming) -> Result<(), ApplicationError>;

struct QuarantinedStartupInference {
    local: LocalInference,
    timing: ApplicationTiming,
}

static STARTUP_CLEANUP_QUARANTINE: Mutex<Vec<QuarantinedStartupInference>> = Mutex::new(Vec::new());

fn lock_startup_cleanup_quarantine() -> MutexGuard<'static, Vec<QuarantinedStartupInference>> {
    match STARTUP_CLEANUP_QUARANTINE.lock() {
        Ok(quarantine) => quarantine,
        Err(poisoned) => poisoned.into_inner(),
    }
}

fn quarantine_startup_inference(local: LocalInference, timing: ApplicationTiming) {
    lock_startup_cleanup_quarantine().push(QuarantinedStartupInference { local, timing });
}

pub(super) fn reap_startup_cleanup_quarantine() -> Option<Result<(), ApplicationError>> {
    let mut quarantined = lock_startup_cleanup_quarantine().pop()?;
    let mut result =
        crate::shutdown::rollback_started_inference(&mut quarantined.local, quarantined.timing);
    let unresolved = quarantined.local.thread_is_present();
    if unresolved && result.is_ok() {
        result = Err(ApplicationError::ShutdownTimeout(
            ApplicationWorker::Inference,
        ));
    }
    if unresolved {
        lock_startup_cleanup_quarantine().push(quarantined);
    }
    Some(result)
}

#[cfg(test)]
pub(super) fn startup_cleanup_quarantine_state() -> (usize, usize) {
    let quarantine = lock_startup_cleanup_quarantine();
    let retained_threads = quarantine
        .iter()
        .filter(|entry| entry.local.thread_is_present())
        .count();
    (quarantine.len(), retained_threads)
}

struct StartupRollbackGuard {
    local: Option<LocalInference>,
    timing: ApplicationTiming,
    rollback: StartupInferenceRollback,
}

impl StartupRollbackGuard {
    const fn new(
        local: LocalInference,
        timing: ApplicationTiming,
        rollback: StartupInferenceRollback,
    ) -> Self {
        Self {
            local: Some(local),
            timing,
            rollback,
        }
    }

    fn commit(mut self) -> Result<LocalInference, ApplicationError> {
        self.local.take().ok_or_else(|| {
            ApplicationFailure::new(
                ApplicationFailureKind::Worker,
                "inference startup rollback guard was already disarmed",
            )
            .into()
        })
    }

    fn rollback(mut self) -> Result<(), ApplicationError> {
        self.rollback_inner()
    }

    fn rollback_inner(&mut self) -> Result<(), ApplicationError> {
        let Some(local) = self.local.as_mut() else {
            return Ok(());
        };
        let mut result = (self.rollback)(local, self.timing);
        let unresolved = local.thread_is_present();
        if unresolved && result.is_ok() {
            result = Err(ApplicationError::ShutdownTimeout(
                ApplicationWorker::Inference,
            ));
        }

        if unresolved {
            if let Some(local) = self.local.take() {
                quarantine_startup_inference(local, self.timing);
            }
        } else {
            self.local = None;
        }
        result
    }
}

impl Drop for StartupRollbackGuard {
    fn drop(&mut self) {
        let _rollback_result = self.rollback_inner();
    }
}

pub(super) struct StartupFailure {
    pub(super) primary: ApplicationError,
    pub(super) inference_rollback: Option<Result<(), ApplicationError>>,
}

impl StartupFailure {
    fn into_primary(self) -> ApplicationError {
        let Self {
            primary,
            inference_rollback,
        } = self;
        drop(inference_rollback);
        primary
    }
}

impl From<ApplicationError> for StartupFailure {
    fn from(primary: ApplicationError) -> Self {
        Self {
            primary,
            inference_rollback: None,
        }
    }
}

impl ApplicationRuntime {
    /// Opens persistent state and starts the bounded Hub and local inference workers.
    ///
    /// # Errors
    ///
    /// Returns an error when configuration or persisted preferences are invalid, bounded output
    /// storage cannot be allocated, storage cannot be opened or read, or a worker cannot be
    /// started.
    pub fn start(configuration: ApplicationRuntimeConfiguration) -> Result<Self, ApplicationError> {
        #[cfg(not(test))]
        let _startup_cleanup_reap = reap_startup_cleanup_quarantine();

        Self::start_transaction(configuration, |configuration| {
            start_hub_worker(
                hub_configuration(&configuration.hub),
                configuration.hub_channel_capacity,
                configuration.timing.hub_worker_poll,
                configuration.timing.hub_event_send_timeout,
            )
        })
        .map_err(StartupFailure::into_primary)
    }

    #[cfg(any(test, feature = "cuda-hardware-tests"))]
    pub(super) fn start_with_device_probe(
        configuration: ApplicationRuntimeConfiguration,
        device_probe: DeviceProbe,
    ) -> Result<Self, ApplicationError> {
        Self::start_transaction_with_rollback(
            configuration,
            |configuration| {
                start_hub_worker(
                    hub_configuration(&configuration.hub),
                    configuration.hub_channel_capacity,
                    configuration.timing.hub_worker_poll,
                    configuration.timing.hub_event_send_timeout,
                )
            },
            crate::shutdown::rollback_started_inference,
            device_probe,
        )
        .map_err(StartupFailure::into_primary)
    }

    pub(super) fn start_transaction<F>(
        configuration: ApplicationRuntimeConfiguration,
        start_hub: F,
    ) -> Result<Self, StartupFailure>
    where
        F: FnOnce(&ApplicationRuntimeConfiguration) -> Result<HubWorker, ApplicationError>,
    {
        Self::start_transaction_with_rollback(
            configuration,
            start_hub,
            crate::shutdown::rollback_started_inference,
            probe_application_device,
        )
    }

    pub(super) fn start_transaction_with_rollback<F>(
        configuration: ApplicationRuntimeConfiguration,
        start_hub: F,
        rollback: StartupInferenceRollback,
        device_probe: DeviceProbe,
    ) -> Result<Self, StartupFailure>
    where
        F: FnOnce(&ApplicationRuntimeConfiguration) -> Result<HubWorker, ApplicationError>,
    {
        validate_configuration(&configuration)?;
        let generation = GenerationBridge::new(&configuration)?;
        let storage = RedbStorage::open(&configuration.database_path).map_err(storage_failure)?;
        let preferences = storage
            .load_settings()
            .map_err(storage_failure)?
            .map_or_else(|| configuration.defaults.clone(), application_preferences);
        validate_preferences(&preferences)?;
        let DeviceCatalogue {
            summaries,
            failures,
        } = discover_device_catalogue(preferences.selected_device, device_probe);
        let memory_budget = runtime_memory_budget(&preferences, &summaries);
        let state = ApplicationState::with_devices(
            preferences.selected_device,
            summaries,
            failures,
            memory_budget.device_bytes,
        );

        let local = create_runtime(memory_budget, &configuration)?;
        let local_guard = StartupRollbackGuard::new(local, configuration.timing, rollback);
        let HubWorker {
            commands: hub_commands,
            events: hub_results,
            thread: hub_thread,
        } = match start_hub(&configuration) {
            Ok(worker) => worker,
            Err(primary) => {
                let inference_rollback = Some(local_guard.rollback());
                return Err(StartupFailure {
                    primary,
                    inference_rollback,
                });
            }
        };
        let local = local_guard.commit()?;

        Ok(Self {
            local,
            hub_commands,
            hub_results,
            hub_thread: Some(hub_thread),
            storage,
            preferences,
            memory_budget,
            device_probe,
            configuration,
            state,
            resolved_artifacts: None,
            pending_hub_selection: None,
            pending_load: None,
            pending_unload: None,
            tokenizer: None,
            generation,
            conversation: crate::conversation::ConversationState::default(),
            context_diagnostics: None,
            next_ticket: INITIAL_COMMAND_TICKET,
            shutdown_control: crate::shutdown::ShutdownControl::default(),
            incompatible_model_cleanup: None,
            retained_model_cleanup: None,
            #[cfg(test)]
            forced_inference_busy_submissions: 0,
            #[cfg(test)]
            last_submitted_load_device: None,
        })
    }
}
