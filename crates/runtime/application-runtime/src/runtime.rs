//! Frontend-neutral application orchestration over bounded host workers.

use std::sync::{Mutex, MutexGuard};

use candle_backend::CandleLlamaSource;
use domain_contracts::{DeviceId, ModelHandle, ModelId, QuantizationFormat};
use hf_hub_adapter::{HubModelReference, ResolvedSafetensorsLlamaArtifacts};
use hf_tokenizer::HfTokenizer;
use host_runtime::{BoundedReceiver, BoundedSender, HostThread, TryReceiveError, TrySendError};
use inference_runtime::{CommandTicket, RuntimeCommand, RuntimeEvent, UnloadStatus};
use redb_storage::{ModelRecord, RedbStorage};
use tokenization::Tokenizer;

use crate::conversation::ConversationState;
use crate::generation::GenerationBridge;
use crate::hub_worker::{HubCommand, HubEvent, HubWorker, start_hub_worker};
use crate::local::{CANDLE_BACKEND_ID, LocalInference, LocalSubmitError};
use crate::support::{
    application_preferences, application_scalar_type, candle_scalar_type, create_runtime,
    domain_scalar_type, hub_configuration, hub_failure, model_source_failure, storage_failure,
    stored_scalar_type, stored_settings, unix_milliseconds, validate_configuration,
    validate_preferences,
};
use crate::{
    ApplicationActivity, ApplicationError, ApplicationEvent, ApplicationFailure,
    ApplicationFailureKind, ApplicationPreferences, ApplicationRuntimeConfiguration,
    ApplicationScalarType, ApplicationState, ApplicationTiming, ApplicationWorker,
    ContextDiagnostics, ImmutableModelIdentity, LoadedModel, ModelSelection, ResolvedModel,
};

const MODEL_ID: ModelId = ModelId::new(1);
const CPU_DEVICE: DeviceId = DeviceId::new(0);
const INITIAL_COMMAND_TICKET: u64 = 1;
const MAXIMUM_INCOMPATIBLE_UNLOAD_SUBMISSION_ATTEMPTS: u8 = 3;

/// Frontend-neutral owner of model acquisition, persistence, lifecycle, and generation workers.
pub struct ApplicationRuntime {
    pub(crate) local: LocalInference,
    pub(crate) hub_commands: BoundedSender<HubCommand>,
    hub_results: BoundedReceiver<HubEvent>,
    pub(crate) hub_thread: Option<HostThread<()>>,
    storage: RedbStorage,
    preferences: ApplicationPreferences,
    pub(crate) configuration: ApplicationRuntimeConfiguration,
    pub(crate) state: ApplicationState,
    resolved_artifacts: Option<ResolvedSafetensorsLlamaArtifacts>,
    pending_hub_selection: Option<ModelSelection>,
    pending_load: Option<LoadAdmission>,
    pub(crate) tokenizer: Option<HfTokenizer>,
    pub(crate) generation: GenerationBridge,
    pub(crate) conversation: ConversationState,
    pub(crate) context_diagnostics: Option<ContextDiagnostics>,
    next_ticket: u64,
    pub(crate) shutdown_control: crate::shutdown::ShutdownControl,
    incompatible_model_cleanup: Option<IncompatibleModelCleanup>,
    #[cfg(test)]
    forced_inference_busy_submissions: usize,
}

#[derive(Clone, Copy)]
struct LoadAdmission {
    ticket: CommandTicket,
    scalar_type: ApplicationScalarType,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct IncompatibleModelCleanup {
    handle: ModelHandle,
    compatibility_failure: ApplicationFailure,
    unload: IncompatibleModelUnload,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum IncompatibleModelUnload {
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

fn reap_startup_cleanup_quarantine() -> Option<Result<(), ApplicationError>> {
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
fn startup_cleanup_quarantine_state() -> (usize, usize) {
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

struct StartupFailure {
    primary: ApplicationError,
    inference_rollback: Option<Result<(), ApplicationError>>,
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

    fn start_transaction<F>(
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
        )
    }

    fn start_transaction_with_rollback<F>(
        configuration: ApplicationRuntimeConfiguration,
        start_hub: F,
        rollback: StartupInferenceRollback,
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

        let local = create_runtime(&preferences, &configuration)?;
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
            configuration,
            state: ApplicationState::default(),
            resolved_artifacts: None,
            pending_hub_selection: None,
            pending_load: None,
            tokenizer: None,
            generation,
            conversation: ConversationState::default(),
            context_diagnostics: None,
            next_ticket: INITIAL_COMMAND_TICKET,
            shutdown_control: crate::shutdown::ShutdownControl::default(),
            incompatible_model_cleanup: None,
            #[cfg(test)]
            forced_inference_busy_submissions: 0,
        })
    }

    /// Returns persisted settings or the configured defaults used for this process.
    #[must_use]
    pub const fn preferences(&self) -> &ApplicationPreferences {
        &self.preferences
    }

    /// Returns the current frontend-neutral state.
    #[must_use]
    pub const fn state(&self) -> &ApplicationState {
        &self.state
    }

    /// Starts immutable Hugging Face artifact and tokenizer resolution.
    ///
    /// Resolution remains asynchronous on the bounded Hub worker.
    ///
    /// # Errors
    ///
    /// Returns an error when another operation or model is active, the Hub selection is invalid,
    /// or the Hub worker is busy or disconnected.
    pub fn resolve_model(&mut self, selection: ModelSelection) -> Result<(), ApplicationError> {
        self.require_idle()?;
        if self.state.loaded().is_some() {
            return Err(ApplicationError::ModelAlreadyLoaded);
        }
        self.resolve_hugging_face(selection)
    }

    /// Loads the exact complete selection retained by immutable resolution.
    ///
    /// # Errors
    ///
    /// Returns an error when loading is not currently valid, the complete selection or scalar
    /// compatibility changed, or the selected inference worker cannot accept the command.
    pub fn load_model(&mut self, selection: &ModelSelection) -> Result<(), ApplicationError> {
        self.require_idle()?;
        if self.state.loaded().is_some() {
            return Err(ApplicationError::ModelAlreadyLoaded);
        }
        if !self.state.inference_available() {
            return Err(ApplicationError::RuntimeDisconnected);
        }
        let resolved = self
            .state
            .resolved()
            .cloned()
            .ok_or(ApplicationError::NoResolvedModel)?;
        if !resolved.matches_selection(selection) {
            return Err(ApplicationError::SelectionChanged);
        }
        let scalar_type = resolved
            .scalar_type()
            .ok_or(ApplicationError::UnknownScalarType)?;
        let artifacts = self
            .resolved_artifacts
            .as_ref()
            .ok_or(ApplicationError::NoResolvedModel)?;
        let source = CandleLlamaSource::new(
            artifacts.config_path.clone(),
            artifacts.weight_paths.clone(),
            candle_scalar_type(scalar_type),
        )
        .map_err(model_source_failure)?;
        let ticket = self.next_ticket()?;
        self.submit_inference(RuntimeCommand::LoadModel {
            ticket,
            model_id: MODEL_ID,
            source,
            device: CPU_DEVICE,
            device_kind: domain_contracts::DeviceKind::Cpu,
        })?;
        self.pending_load = Some(LoadAdmission {
            ticket,
            scalar_type,
        });
        self.state.begin_loading();
        Ok(())
    }

    /// Requests bounded draining and deterministic release of the resident model.
    ///
    /// This is the convenience form of
    /// [`ApplicationRuntime::unload_model_with_behavior`] using
    /// [`crate::ModelUnloadBehavior::Drain`].
    ///
    /// # Errors
    ///
    /// Returns an error when unloading is not currently valid, no model is loaded, the drain
    /// timeout is invalid, or the selected inference worker cannot accept the command.
    pub fn unload_model(&mut self) -> Result<(), ApplicationError> {
        self.unload_model_with_behavior(crate::ModelUnloadBehavior::Drain)
    }

    /// Processes at most one pending Hub or inference event without blocking.
    #[must_use]
    pub fn poll_event(&mut self) -> Option<ApplicationEvent> {
        if let Some(event) = self.take_generation_event() {
            return Some(event);
        }

        if self.state.hub_available() {
            match self.hub_results.try_receive() {
                Ok(event) => return Some(self.process_hub_event(event)),
                Err(TryReceiveError::Empty) => {}
                Err(TryReceiveError::Disconnected) => {
                    self.state.disconnect_hub();
                    if self.state.activity() == ApplicationActivity::Resolving {
                        self.state.set_idle();
                    }
                    return Some(ApplicationEvent::HubDisconnected);
                }
            }
        }

        if self.state.inference_available() {
            match self.local.try_receive() {
                Ok(event) => return self.process_runtime_event(&event),
                Err(inference_runtime::RuntimeReceiveError::Timeout) => {}
                Err(inference_runtime::RuntimeReceiveError::Disconnected) => {
                    self.shutdown_control.record_inference_disconnect();
                    self.state.disconnect_inference();
                    self.release_incompatible_model_cleanup();
                    self.handle_generation_runtime_disconnected();
                    if matches!(
                        self.state.activity(),
                        ApplicationActivity::Loading | ApplicationActivity::Unloading
                    ) {
                        self.state.set_idle();
                    }
                    return Some(ApplicationEvent::RuntimeDisconnected);
                }
            }
        }
        if let Some(event) = self.retry_incompatible_model_cleanup() {
            return Some(event);
        }
        self.pump_generation_event()
    }

    /// Cooperatively shuts down all workers and waits only to configured hard deadlines.
    ///
    /// # Errors
    ///
    /// Returns a retained terminal inference shutdown failure when present; otherwise returns the
    /// first worker command, timeout, or join failure encountered.
    pub fn shutdown(&mut self) -> Result<(), ApplicationError> {
        crate::shutdown::shutdown(self)
    }

    pub(crate) fn next_ticket(&mut self) -> Result<CommandTicket, ApplicationError> {
        let ticket = CommandTicket::new(self.next_ticket);
        self.next_ticket = self
            .next_ticket
            .checked_add(1)
            .ok_or(ApplicationError::TicketExhausted)?;
        Ok(ticket)
    }

    pub(crate) fn require_idle(&self) -> Result<(), ApplicationError> {
        let activity = self.state.activity();
        if activity == ApplicationActivity::Idle {
            Ok(())
        } else {
            Err(ApplicationError::Busy(activity))
        }
    }

    pub(crate) fn submit_inference(
        &mut self,
        command: RuntimeCommand<CandleLlamaSource>,
    ) -> Result<(), ApplicationError> {
        #[cfg(test)]
        if self.forced_inference_busy_submissions > 0 {
            self.forced_inference_busy_submissions -= 1;
            return Err(ApplicationError::RuntimeBusy);
        }

        match self.local.submit(command) {
            Ok(()) => Ok(()),
            Err(LocalSubmitError::Full) => Err(ApplicationError::RuntimeBusy),
            Err(LocalSubmitError::Disconnected) => {
                self.shutdown_control.record_inference_disconnect();
                self.state.disconnect_inference();
                self.release_incompatible_model_cleanup();
                Err(ApplicationError::RuntimeDisconnected)
            }
        }
    }

    fn resolve_hugging_face(&mut self, selection: ModelSelection) -> Result<(), ApplicationError> {
        if !self.state.hub_available() {
            return Err(ApplicationError::HubDisconnected);
        }
        let (repository, revision) = selection.into_parts();
        let reference = HubModelReference::new(repository, revision).map_err(hub_failure)?;
        let normalized = ModelSelection::new(reference.repository(), reference.revision());
        match self.hub_commands.try_send(HubCommand::Resolve(reference)) {
            Ok(()) => {}
            Err(TrySendError::Full(_)) => return Err(ApplicationError::HubBusy),
            Err(TrySendError::Disconnected(_)) => {
                self.state.disconnect_hub();
                return Err(ApplicationError::HubDisconnected);
            }
        }
        self.clear_resolution();
        self.pending_hub_selection = Some(normalized);
        self.state.begin_resolving();
        Ok(())
    }

    fn process_hub_event(&mut self, event: HubEvent) -> ApplicationEvent {
        match event {
            HubEvent::Resolved(Ok(artifacts)) => self.accept_resolved_artifacts(artifacts),
            HubEvent::Resolved(Err(error)) => {
                self.pending_hub_selection = None;
                self.state.set_idle();
                ApplicationEvent::ModelResolutionFailed {
                    failure: ApplicationFailure::new(ApplicationFailureKind::Hub, error),
                }
            }
        }
    }

    fn accept_resolved_artifacts(
        &mut self,
        artifacts: ResolvedSafetensorsLlamaArtifacts,
    ) -> ApplicationEvent {
        let artifact_selection =
            ModelSelection::new(artifacts.repository.clone(), artifacts.revision.clone());
        if self
            .pending_hub_selection
            .as_ref()
            .is_some_and(|selection| selection != &artifact_selection)
        {
            return self.reject_resolution(ApplicationFailure::new(
                ApplicationFailureKind::Hub,
                "Hub resolution returned artifacts for a different complete selection",
            ));
        }
        self.pending_hub_selection = None;

        let tokenizer = match HfTokenizer::from_file(&artifacts.tokenizer_path) {
            Ok(tokenizer) => tokenizer,
            Err(error) => {
                return self.reject_resolution(ApplicationFailure::new(
                    ApplicationFailureKind::Tokenizer,
                    error,
                ));
            }
        };
        let chat_compatibility = crate::chat::detect_chat_compatibility(
            artifacts.repository.as_str(),
            artifacts.commit.as_str(),
            &tokenizer,
        );
        let resolved = ResolvedModel::new(
            artifact_selection,
            ImmutableModelIdentity::new(artifacts.repository.clone(), artifacts.commit.clone()),
            tokenizer.vocabulary_size(),
            artifacts.declared_scalar_type.map(domain_scalar_type),
            chat_compatibility,
        );
        let persistence_warning = self
            .persist_resolved(&artifacts)
            .err()
            .map(|error| ApplicationFailure::new(ApplicationFailureKind::Storage, error));
        self.resolved_artifacts = Some(artifacts);
        self.tokenizer = Some(tokenizer);
        self.state.set_resolved(resolved.clone());
        ApplicationEvent::ModelResolved {
            model: resolved,
            persistence_warning,
        }
    }

    fn reject_resolution(&mut self, failure: ApplicationFailure) -> ApplicationEvent {
        self.clear_resolution();
        self.state.set_idle();
        ApplicationEvent::ModelResolutionFailed { failure }
    }

    fn clear_resolution(&mut self) {
        self.resolved_artifacts = None;
        self.pending_hub_selection = None;
        self.pending_load = None;
        self.tokenizer = None;
        self.state.clear_resolved();
    }

    fn process_runtime_event(&mut self, event: &RuntimeEvent) -> Option<ApplicationEvent> {
        match event {
            RuntimeEvent::ModelLoaded { ticket, result } => {
                Some(self.process_model_loaded(*ticket, *result))
            }
            RuntimeEvent::ModelUnload { ticket, result } => {
                Some(self.process_model_unload(*ticket, *result))
            }
            RuntimeEvent::GenerationAdmitted { .. }
            | RuntimeEvent::GenerationCancellationRequested { .. } => {
                self.process_generation_runtime_event(event)
            }
            RuntimeEvent::Shutdown { .. }
            | RuntimeEvent::RequestStarted { .. }
            | RuntimeEvent::PrefillCompleted { .. }
            | RuntimeEvent::DecodeCompleted { .. }
            | RuntimeEvent::RequestFinished { .. }
            | RuntimeEvent::Snapshot { .. } => None,
        }
    }

    fn process_model_loaded(
        &mut self,
        ticket: CommandTicket,
        result: Result<inference_runtime::LoadReceipt, inference_runtime::RuntimeError>,
    ) -> ApplicationEvent {
        let admission = self.pending_load.take();
        let receipt = match result {
            Ok(receipt) => receipt,
            Err(error) => {
                self.state.set_idle();
                return ApplicationEvent::ModelLoadFailed {
                    failure: ApplicationFailure::from_debug(
                        ApplicationFailureKind::Inference,
                        "model load failed",
                        error,
                    ),
                };
            }
        };
        let Some(resolved) = self.state.resolved().cloned() else {
            return self.reject_incompatible_model(receipt.handle);
        };
        let descriptor = receipt.descriptor;
        let Some(scalar_type) = application_scalar_type(descriptor.metadata.scalar_type) else {
            return self.reject_incompatible_model(receipt.handle);
        };
        if !self.loaded_compatibility_matches(admission, ticket, &receipt, &resolved, scalar_type) {
            return self.reject_incompatible_model(receipt.handle);
        }
        let loaded = LoadedModel::new(
            receipt.handle,
            resolved.selection().clone(),
            resolved.identity().clone(),
            scalar_type,
            descriptor.metadata.vocabulary_size,
            descriptor.capabilities.maximum_context_tokens,
            descriptor.capabilities.maximum_prefill_batch,
        );
        self.state.set_loaded(loaded.clone());
        ApplicationEvent::ModelLoaded { model: loaded }
    }

    fn loaded_compatibility_matches(
        &self,
        admission: Option<LoadAdmission>,
        ticket: CommandTicket,
        receipt: &inference_runtime::LoadReceipt,
        resolved: &ResolvedModel,
        scalar_type: ApplicationScalarType,
    ) -> bool {
        let Some(admission) = admission else {
            return false;
        };
        let Some(tokenizer) = self.tokenizer.as_ref() else {
            return false;
        };
        let Some(artifacts) = self.resolved_artifacts.as_ref() else {
            return false;
        };
        let descriptor = receipt.descriptor;
        let artifact_selection = ModelSelection::new(&artifacts.repository, &artifacts.revision);
        let artifact_scalar = artifacts.declared_scalar_type.map(domain_scalar_type);

        admission.ticket == ticket
            && admission.scalar_type == scalar_type
            && resolved.scalar_type() == Some(scalar_type)
            && artifact_scalar == Some(scalar_type)
            && resolved.selection() == &artifact_selection
            && resolved.identity().repository() == artifacts.repository
            && resolved.identity().commit() == artifacts.commit
            && descriptor.backend == CANDLE_BACKEND_ID
            && descriptor.metadata.quantization == QuantizationFormat::None
            && tokenizer.vocabulary_size() == descriptor.metadata.vocabulary_size
    }

    fn reject_incompatible_model(&mut self, handle: ModelHandle) -> ApplicationEvent {
        self.state.clear_resolved();
        self.resolved_artifacts = None;
        self.tokenizer = None;
        let failure = ApplicationFailure {
            kind: ApplicationFailureKind::Inference,
            message: "resolved identity, descriptor, and tokenizer compatibility evidence differ; deterministic unload was requested".to_owned(),
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

    fn retry_incompatible_model_cleanup(&mut self) -> Option<ApplicationEvent> {
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

    fn process_model_unload(
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

    fn persist_resolved(
        &mut self,
        artifacts: &ResolvedSafetensorsLlamaArtifacts,
    ) -> Result<(), redb_storage::StorageError> {
        self.preferences
            .default_repository
            .clone_from(&artifacts.repository);
        self.preferences
            .default_revision
            .clone_from(&artifacts.revision);
        self.storage
            .save_settings(&stored_settings(&self.preferences))?;
        let Some(scalar_type) = artifacts.declared_scalar_type else {
            return Ok(());
        };
        self.storage.upsert_model(&ModelRecord {
            name: format!("{}@{}", artifacts.repository, artifacts.commit),
            repository: artifacts.repository.clone(),
            revision: artifacts.commit.clone(),
            scalar_type: stored_scalar_type(scalar_type),
            last_used_unix_milliseconds: unix_milliseconds(),
        })
    }
}

#[cfg(test)]
mod tests;
