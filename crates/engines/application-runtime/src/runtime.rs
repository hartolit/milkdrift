//! Frontend-neutral application orchestration over bounded host workers.

use candle_backend::CandleLlamaSource;
use domain_contracts::{DeviceId, DeviceKind, DrainTimeout, ModelHandle, ModelId, UnloadPolicy};
use hf_hub_adapter::{HubModelReference, ResolvedModelArtifacts};
use hf_tokenizer::HfTokenizer;
use host_runtime::{BoundedReceiver, BoundedSender, HostThread, TryReceiveError, TrySendError};
use inference_runtime::{
    CommandTicket, HostedRuntime, RuntimeCommand, RuntimeEvent, RuntimeThread, UnloadStatus,
};
use redb_storage::{ModelRecord, RedbStorage};
use tokenization::Tokenizer;

use crate::generation::GenerationBridge;
use crate::hub_worker::{HubCommand, HubEvent, HubWorker, start_hub_worker};
use crate::support::{
    application_preferences, candle_scalar_type, create_runtime, domain_scalar_type,
    hub_configuration, hub_failure, model_source_failure, storage_failure, stored_scalar_type,
    stored_settings, unix_milliseconds, validate_configuration, validate_preferences,
};
use crate::{
    ApplicationActivity, ApplicationBackend, ApplicationError, ApplicationEvent,
    ApplicationFailure, ApplicationFailureKind, ApplicationPreferences,
    ApplicationRuntimeConfiguration, ApplicationState, LoadedModel, ResolvedModel,
};

const MODEL_ID: ModelId = ModelId::new(1);
const CPU_DEVICE: DeviceId = DeviceId::new(0);
const INITIAL_COMMAND_TICKET: u64 = 1;

/// Frontend-neutral owner of model acquisition, persistence, lifecycle, and generation workers.
pub struct ApplicationRuntime {
    pub(crate) inference: HostedRuntime<CandleLlamaSource>,
    pub(crate) inference_thread: Option<RuntimeThread>,
    pub(crate) hub_commands: BoundedSender<HubCommand>,
    hub_results: BoundedReceiver<HubEvent>,
    pub(crate) hub_thread: Option<HostThread<()>>,
    storage: RedbStorage,
    preferences: ApplicationPreferences,
    pub(crate) configuration: ApplicationRuntimeConfiguration,
    pub(crate) state: ApplicationState,
    resolved_artifacts: Option<ResolvedModelArtifacts>,
    pub(crate) tokenizer: Option<HfTokenizer>,
    pub(crate) generation: GenerationBridge,
    next_ticket: u64,
}

impl ApplicationRuntime {
    /// Opens persistent state and starts the bounded Hub and inference workers.
    ///
    /// # Errors
    ///
    /// Returns an error when configuration or persisted preferences are invalid, bounded output
    /// storage cannot be allocated, storage cannot be opened or read, or either worker cannot be
    /// started.
    pub fn start(configuration: ApplicationRuntimeConfiguration) -> Result<Self, ApplicationError> {
        validate_configuration(&configuration)?;
        let generation = GenerationBridge::new(&configuration)?;
        let storage = RedbStorage::open(&configuration.database_path).map_err(storage_failure)?;
        let preferences = storage
            .load_settings()
            .map_err(storage_failure)?
            .map_or_else(|| configuration.defaults.clone(), application_preferences);
        validate_preferences(&preferences)?;

        let (inference, inference_thread) = create_runtime(&preferences, &configuration)?;
        let HubWorker {
            commands: hub_commands,
            events: hub_results,
            thread: hub_thread,
        } = start_hub_worker(
            hub_configuration(&configuration.hub),
            configuration.hub_channel_capacity,
            configuration.timing.hub_worker_poll,
            configuration.timing.hub_event_send_timeout,
        )?;

        Ok(Self {
            inference,
            inference_thread: Some(inference_thread),
            hub_commands,
            hub_results,
            hub_thread: Some(hub_thread),
            storage,
            preferences,
            configuration,
            state: ApplicationState::default(),
            resolved_artifacts: None,
            tokenizer: None,
            generation,
            next_ticket: INITIAL_COMMAND_TICKET,
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

    /// Starts immutable artifact and tokenizer resolution on the bounded Hub worker.
    ///
    /// # Errors
    ///
    /// Returns an error when another operation or model is active, the selection is invalid, or
    /// the Hub worker is busy or disconnected.
    pub fn resolve_model(
        &mut self,
        repository: impl Into<String>,
        revision: impl Into<String>,
    ) -> Result<(), ApplicationError> {
        self.require_idle()?;
        if self.state.loaded().is_some() {
            return Err(ApplicationError::ModelAlreadyLoaded);
        }
        if !self.state.hub_available() {
            return Err(ApplicationError::HubDisconnected);
        }
        let reference = HubModelReference::new(repository, revision).map_err(hub_failure)?;
        match self.hub_commands.try_send(HubCommand::Resolve(reference)) {
            Ok(()) => {}
            Err(TrySendError::Full(_)) => return Err(ApplicationError::HubBusy),
            Err(TrySendError::Disconnected(_)) => {
                self.state.disconnect_hub();
                return Err(ApplicationError::HubDisconnected);
            }
        }
        self.resolved_artifacts = None;
        self.tokenizer = None;
        self.state.begin_resolving();
        Ok(())
    }

    /// Loads the exact resolved repository and revision into the CPU inference runtime.
    ///
    /// # Errors
    ///
    /// Returns an error when loading is not currently valid, the resolved selection or scalar type
    /// is incompatible, or the inference worker cannot accept the command.
    pub fn load_model(&mut self, repository: &str, revision: &str) -> Result<(), ApplicationError> {
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
            .ok_or(ApplicationError::NoResolvedModel)?;
        if !resolved.matches_selection(repository, revision) {
            return Err(ApplicationError::SelectionChanged);
        }
        let scalar_type = resolved
            .scalar_type
            .and_then(candle_scalar_type)
            .ok_or(ApplicationError::UnknownScalarType)?;
        let artifacts = self
            .resolved_artifacts
            .as_ref()
            .ok_or(ApplicationError::NoResolvedModel)?;
        let source = CandleLlamaSource::new(
            artifacts.config_path.clone(),
            artifacts.weight_paths.clone(),
            scalar_type,
        )
        .map_err(model_source_failure)?;
        let command = RuntimeCommand::LoadModel {
            ticket: self.next_ticket()?,
            model_id: MODEL_ID,
            source,
            device: CPU_DEVICE,
            device_kind: DeviceKind::Cpu,
        };
        self.submit_inference(command)?;
        self.state.begin_loading();
        Ok(())
    }

    /// Requests bounded draining and deterministic release of the resident model.
    ///
    /// Active generation remains owned by E0 and may complete during the configured drain window;
    /// expiry escalates to safe-boundary cancellation.
    ///
    /// # Errors
    ///
    /// Returns an error when unloading is not currently valid, no model is loaded, or the
    /// inference worker cannot accept the command.
    pub fn unload_model(&mut self) -> Result<(), ApplicationError> {
        self.require_idle()?;
        let handle = self
            .state
            .loaded()
            .ok_or(ApplicationError::NoLoadedModel)?
            .handle;
        self.submit_unload(handle)
    }

    /// Processes at most one pending application, Hub, or inference event without blocking.
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
            match self.inference.try_receive() {
                Ok(event) => return self.process_runtime_event(&event),
                Err(inference_runtime::RuntimeReceiveError::Timeout) => {}
                Err(inference_runtime::RuntimeReceiveError::Disconnected) => {
                    self.state.disconnect_inference();
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
        self.pump_generation_event()
    }

    /// Cooperatively shuts down workers and waits only to configured hard deadlines.
    ///
    /// # Errors
    ///
    /// Returns the first worker command, timeout, join, or inference shutdown failure encountered.
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

    fn require_idle(&self) -> Result<(), ApplicationError> {
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
        match self.inference.try_submit(command) {
            Ok(()) => Ok(()),
            Err(inference_runtime::RuntimeSubmitError::Full(_)) => {
                Err(ApplicationError::RuntimeBusy)
            }
            Err(inference_runtime::RuntimeSubmitError::Disconnected(_)) => {
                self.state.disconnect_inference();
                Err(ApplicationError::RuntimeDisconnected)
            }
        }
    }

    fn submit_unload(&mut self, handle: ModelHandle) -> Result<(), ApplicationError> {
        let timeout = DrainTimeout::from_millis(self.preferences.drain_timeout_milliseconds)
            .map_err(|error| {
                ApplicationFailure::from_debug(
                    ApplicationFailureKind::Inference,
                    "invalid drain timeout",
                    error,
                )
            })?;
        let command = RuntimeCommand::UnloadModel {
            ticket: self.next_ticket()?,
            handle,
            policy: UnloadPolicy::Drain { timeout },
        };
        self.submit_inference(command)?;
        self.state.begin_unloading();
        Ok(())
    }

    fn process_hub_event(&mut self, event: HubEvent) -> ApplicationEvent {
        match event {
            HubEvent::Resolved(Ok(artifacts)) => self.accept_resolved_artifacts(artifacts),
            HubEvent::Resolved(Err(error)) => {
                self.state.set_idle();
                ApplicationEvent::ModelResolutionFailed {
                    failure: ApplicationFailure::new(ApplicationFailureKind::Hub, error),
                }
            }
        }
    }

    fn accept_resolved_artifacts(&mut self, artifacts: ResolvedModelArtifacts) -> ApplicationEvent {
        let tokenizer = match HfTokenizer::from_file(&artifacts.tokenizer_path) {
            Ok(tokenizer) => tokenizer,
            Err(error) => {
                self.state.set_idle();
                return ApplicationEvent::ModelResolutionFailed {
                    failure: ApplicationFailure::new(ApplicationFailureKind::Tokenizer, error),
                };
            }
        };
        let resolved = ResolvedModel {
            repository: artifacts.repository.clone(),
            revision: artifacts.revision.clone(),
            commit: artifacts.commit.clone(),
            vocabulary_size: tokenizer.vocabulary_size(),
            scalar_type: artifacts.declared_scalar_type.map(domain_scalar_type),
        };
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

    fn process_runtime_event(&mut self, event: &RuntimeEvent) -> Option<ApplicationEvent> {
        match event {
            RuntimeEvent::ModelLoaded { result, .. } => Some(self.process_model_loaded(*result)),
            RuntimeEvent::ModelUnload { result, .. } => Some(self.process_model_unload(*result)),
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
        result: Result<inference_runtime::LoadReceipt, inference_runtime::RuntimeError>,
    ) -> ApplicationEvent {
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
        let loaded = LoadedModel {
            handle: receipt.handle,
            vocabulary_size: receipt.descriptor.metadata.vocabulary_size,
            maximum_context_tokens: receipt.descriptor.capabilities.maximum_context_tokens,
            maximum_prefill_batch: receipt.descriptor.capabilities.maximum_prefill_batch,
            backend: ApplicationBackend::Candle,
            device: DeviceKind::Cpu,
        };
        self.state.set_loaded(loaded);
        let tokenizer_vocabulary = self.tokenizer.as_ref().map(Tokenizer::vocabulary_size);
        if tokenizer_vocabulary
            .is_some_and(|size| size != receipt.descriptor.metadata.vocabulary_size)
        {
            return self.reject_incompatible_model(receipt.handle);
        }
        ApplicationEvent::ModelLoaded { model: loaded }
    }

    fn reject_incompatible_model(&mut self, handle: ModelHandle) -> ApplicationEvent {
        self.state.clear_resolved();
        self.resolved_artifacts = None;
        self.tokenizer = None;
        let failure = ApplicationFailure {
            kind: ApplicationFailureKind::Inference,
            message:
                "tokenizer and model vocabulary sizes differ; deterministic unload was requested"
                    .to_owned(),
        };
        if let Err(error) = self.submit_unload(handle) {
            self.state.set_idle();
            return ApplicationEvent::ModelLoadFailed {
                failure: ApplicationFailure {
                    kind: ApplicationFailureKind::Inference,
                    message: format!("{failure}; automatic unload failed: {error}"),
                },
            };
        }
        ApplicationEvent::ModelCompatibilityFailed { failure }
    }

    fn process_model_unload(
        &mut self,
        result: Result<inference_runtime::UnloadReceipt, inference_runtime::RuntimeError>,
    ) -> ApplicationEvent {
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

    fn persist_resolved(
        &mut self,
        artifacts: &ResolvedModelArtifacts,
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
