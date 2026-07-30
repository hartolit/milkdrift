//! Frontend-neutral application orchestration over bounded host workers.

use candle_backend::CandleLlamaSource;
use domain_contracts::{DeviceId, ModelId};
use hf_hub_adapter::{HubModelReference, ResolvedSafetensorsLlamaArtifacts};
use hf_tokenizer::HfTokenizer;
use host_runtime::{BoundedReceiver, BoundedSender, HostThread, TryReceiveError, TrySendError};
use inference_runtime::{CommandTicket, RuntimeEvent, UnloadStatus};
use redb_storage::{ModelRecord, RedbStorage};
use tokenization::Tokenizer;

use crate::conversation::ConversationState;
use crate::generation::GenerationBridge;
use crate::hub_worker::{HubCommand, HubEvent, HubWorker, start_hub_worker};
use crate::local::{
    LocalCommand, LocalInference, LocalModelSource, LocalSubmitError, LocalTokenizer,
    ResolvedGgufArtifacts,
};
use crate::support::{
    application_preferences, application_quantization, application_scalar_type, candle_scalar_type,
    create_runtime, domain_scalar_type, hub_configuration, hub_failure, model_source_failure,
    storage_failure, stored_scalar_type, stored_settings, unix_milliseconds,
    validate_configuration, validate_preferences,
};
use crate::{
    ApplicationActivity, ApplicationBackend, ApplicationError, ApplicationEvent,
    ApplicationFailure, ApplicationFailureKind, ApplicationPreferences,
    ApplicationRuntimeConfiguration, ApplicationState, ChatCompatibility, ContextDiagnostics,
    ImmutableModelIdentity, LoadedModel, ModelCompatibility, ModelSelection, ResolvedModel,
};

const MODEL_ID: ModelId = ModelId::new(1);
const CPU_DEVICE: DeviceId = DeviceId::new(0);
const INITIAL_COMMAND_TICKET: u64 = 1;

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
    resolved_artifacts: Option<ResolvedArtifacts>,
    pending_hub_selection: Option<ModelSelection>,
    pending_load: Option<LoadAdmission>,
    pending_event: Option<ApplicationEvent>,
    pub(crate) tokenizer: Option<LocalTokenizer>,
    pub(crate) generation: GenerationBridge,
    pub(crate) conversation: ConversationState,
    pub(crate) context_diagnostics: Option<ContextDiagnostics>,
    next_ticket: u64,
}

enum ResolvedArtifacts {
    Candle(ResolvedSafetensorsLlamaArtifacts),
    Gguf(ResolvedGgufArtifacts),
}

#[derive(Clone, Copy)]
enum LoadAdmission {
    Candle,
    Gguf { digest: gguf_backend::Sha256Digest },
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
        validate_configuration(&configuration)?;
        let generation = GenerationBridge::new(&configuration)?;
        let storage = RedbStorage::open(&configuration.database_path).map_err(storage_failure)?;
        let preferences = storage
            .load_settings()
            .map_err(storage_failure)?
            .map_or_else(|| configuration.defaults.clone(), application_preferences);
        validate_preferences(&preferences)?;

        let local = create_runtime(&preferences, &configuration)?;
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
            pending_event: None,
            tokenizer: None,
            generation,
            conversation: ConversationState::default(),
            context_diagnostics: None,
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

    /// Starts immutable artifact and tokenizer resolution for one complete local selection.
    ///
    /// Hub resolution remains asynchronous on its bounded worker. Local GGUF resolution is a
    /// synchronous cold-path inspection whose result is published by [`Self::poll_event`].
    ///
    /// # Errors
    ///
    /// Returns an error when another operation or model is active, a Hub selection is invalid,
    /// or the required worker is busy or disconnected.
    pub fn resolve_model(&mut self, selection: ModelSelection) -> Result<(), ApplicationError> {
        self.require_idle()?;
        if self.state.loaded().is_some() {
            return Err(ApplicationError::ModelAlreadyLoaded);
        }

        match selection {
            ModelSelection::HuggingFaceSafetensors {
                repository,
                revision,
            } => self.resolve_hugging_face(repository, revision),
            ModelSelection::LocalGguf { path } => {
                let inference_available = self.local.activate(ApplicationBackend::LlamaCpp);
                self.state.set_inference_available(inference_available);
                self.clear_resolution();
                self.state.begin_resolving();
                let result = self.local.resolve_gguf(
                    &path,
                    &self.configuration.gguf,
                    self.configuration.maximum_requests,
                );
                let event = match result {
                    Ok((artifacts, tokenizer)) => self.accept_resolved_gguf(artifacts, tokenizer),
                    Err(failure) => self.reject_resolution(failure),
                };
                self.pending_event = Some(event);
                Ok(())
            }
        }
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
        if self.local.active_backend() != Some(resolved.backend()) {
            return Err(ApplicationError::SelectionChanged);
        }

        let (source, admission) = match self
            .resolved_artifacts
            .as_ref()
            .ok_or(ApplicationError::NoResolvedModel)?
        {
            ResolvedArtifacts::Candle(artifacts)
                if resolved.backend() == ApplicationBackend::Candle =>
            {
                let scalar_type = resolved
                    .compatibility()
                    .scalar_type()
                    .and_then(candle_scalar_type)
                    .ok_or(ApplicationError::UnknownScalarType)?;
                let source = CandleLlamaSource::new(
                    artifacts.config_path.clone(),
                    artifacts.weight_paths.clone(),
                    scalar_type,
                )
                .map_err(model_source_failure)?;
                (LocalModelSource::Candle(source), LoadAdmission::Candle)
            }
            ResolvedArtifacts::Gguf(artifacts)
                if resolved.backend() == ApplicationBackend::LlamaCpp =>
            {
                (
                    LocalModelSource::Gguf(artifacts.source()),
                    LoadAdmission::Gguf {
                        digest: artifacts.digest(),
                    },
                )
            }
            ResolvedArtifacts::Candle(_) | ResolvedArtifacts::Gguf(_) => {
                return Err(ApplicationError::SelectionChanged);
            }
        };

        let command = LocalCommand::LoadModel {
            ticket: self.next_ticket()?,
            model_id: MODEL_ID,
            source,
            device: CPU_DEVICE,
        };
        self.submit_inference(command)?;
        self.pending_load = Some(admission);
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

    /// Processes at most one pending application, Hub, or active inference event without blocking.
    #[must_use]
    pub fn poll_event(&mut self) -> Option<ApplicationEvent> {
        if let Some(event) = self.pending_event.take() {
            return Some(event);
        }
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

    /// Cooperatively shuts down all workers and waits only to configured hard deadlines.
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
        command: LocalCommand,
    ) -> Result<(), ApplicationError> {
        match self.local.submit(command) {
            Ok(()) => Ok(()),
            Err(LocalSubmitError::Full) => Err(ApplicationError::RuntimeBusy),
            Err(LocalSubmitError::Disconnected) => {
                self.state.disconnect_inference();
                Err(ApplicationError::RuntimeDisconnected)
            }
            Err(LocalSubmitError::NoActiveBackend | LocalSubmitError::BackendMismatch) => {
                Err(ApplicationFailure::new(
                    ApplicationFailureKind::Worker,
                    "local inference dispatch did not match the active resolved product",
                )
                .into())
            }
        }
    }

    fn resolve_hugging_face(
        &mut self,
        repository: String,
        revision: String,
    ) -> Result<(), ApplicationError> {
        if !self.state.hub_available() {
            return Err(ApplicationError::HubDisconnected);
        }
        let reference = HubModelReference::new(repository, revision).map_err(hub_failure)?;
        let normalized =
            ModelSelection::hugging_face_safetensors(reference.repository(), reference.revision());
        match self.hub_commands.try_send(HubCommand::Resolve(reference)) {
            Ok(()) => {}
            Err(TrySendError::Full(_)) => return Err(ApplicationError::HubBusy),
            Err(TrySendError::Disconnected(_)) => {
                self.state.disconnect_hub();
                return Err(ApplicationError::HubDisconnected);
            }
        }
        let inference_available = self.local.activate(ApplicationBackend::Candle);
        self.state.set_inference_available(inference_available);
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
        let artifact_selection = ModelSelection::hugging_face_safetensors(
            artifacts.repository.clone(),
            artifacts.revision.clone(),
        );
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
            Ok(tokenizer) => LocalTokenizer::Hf(Box::new(tokenizer)),
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
        let compatibility = ModelCompatibility::CandleSafetensors {
            scalar_type: artifacts.declared_scalar_type.map(domain_scalar_type),
        };
        let resolved = ResolvedModel::new(
            artifact_selection,
            ImmutableModelIdentity::HuggingFaceCommit {
                repository: artifacts.repository.clone(),
                commit: artifacts.commit.clone(),
            },
            tokenizer.vocabulary_size(),
            compatibility,
            chat_compatibility,
        );
        let persistence_warning = self
            .persist_resolved(&artifacts)
            .err()
            .map(|error| ApplicationFailure::new(ApplicationFailureKind::Storage, error));
        self.resolved_artifacts = Some(ResolvedArtifacts::Candle(artifacts));
        self.tokenizer = Some(tokenizer);
        self.state.set_resolved(resolved.clone());
        ApplicationEvent::ModelResolved {
            model: resolved,
            persistence_warning,
        }
    }

    fn accept_resolved_gguf(
        &mut self,
        artifacts: ResolvedGgufArtifacts,
        tokenizer: LocalTokenizer,
    ) -> ApplicationEvent {
        let metadata = artifacts.metadata();
        if tokenizer.backend() != ApplicationBackend::LlamaCpp
            || tokenizer.vocabulary_size() != metadata.vocabulary_size()
            || tokenizer.gguf_digest() != Some(artifacts.digest())
        {
            return self.reject_resolution(ApplicationFailure::new(
                ApplicationFailureKind::Tokenizer,
                "GGUF metadata, tokenizer vocabulary, and exact content digest are incompatible",
            ));
        }
        let resolved = ResolvedModel::new(
            ModelSelection::local_gguf(artifacts.path()),
            ImmutableModelIdentity::GgufSha256 {
                digest: artifacts.digest().to_string(),
            },
            tokenizer.vocabulary_size(),
            ModelCompatibility::LlamaCppGguf {
                scalar_type: application_scalar_type(metadata.scalar_type()),
                quantization: application_quantization(metadata.quantization()),
            },
            ChatCompatibility::Unsupported,
        );
        self.resolved_artifacts = Some(ResolvedArtifacts::Gguf(artifacts));
        self.tokenizer = Some(tokenizer);
        self.state.set_resolved(resolved.clone());
        ApplicationEvent::ModelResolved {
            model: resolved,
            persistence_warning: None,
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
            self.state.set_idle();
            return ApplicationEvent::ModelLoadFailed {
                failure: ApplicationFailure::new(
                    ApplicationFailureKind::Inference,
                    "model load completed without retained immutable resolution evidence",
                ),
            };
        };

        let descriptor = receipt.descriptor;
        let loaded_compatibility = match resolved.backend() {
            ApplicationBackend::Candle => ModelCompatibility::CandleSafetensors {
                scalar_type: Some(application_scalar_type(descriptor.metadata.scalar_type)),
            },
            ApplicationBackend::LlamaCpp => ModelCompatibility::LlamaCppGguf {
                scalar_type: application_scalar_type(descriptor.metadata.scalar_type),
                quantization: application_quantization(descriptor.metadata.quantization),
            },
        };
        let loaded = LoadedModel::new(
            receipt.handle,
            resolved.selection().clone(),
            resolved.identity().clone(),
            loaded_compatibility,
            descriptor.metadata.vocabulary_size,
            descriptor.capabilities.maximum_context_tokens,
            descriptor.capabilities.maximum_prefill_batch,
        );
        self.state.set_loaded(loaded.clone());

        if !self.loaded_compatibility_matches(admission, &receipt, &resolved) {
            return self.reject_incompatible_model();
        }
        ApplicationEvent::ModelLoaded { model: loaded }
    }

    fn loaded_compatibility_matches(
        &self,
        admission: Option<LoadAdmission>,
        receipt: &inference_runtime::LoadReceipt,
        resolved: &ResolvedModel,
    ) -> bool {
        let descriptor = receipt.descriptor;
        let Some(tokenizer) = self.tokenizer.as_ref() else {
            return false;
        };
        if tokenizer.vocabulary_size() != descriptor.metadata.vocabulary_size
            || tokenizer.backend() != resolved.backend()
        {
            return false;
        }

        match (admission, self.resolved_artifacts.as_ref()) {
            (Some(LoadAdmission::Candle), Some(ResolvedArtifacts::Candle(_))) => true,
            (Some(LoadAdmission::Gguf { digest }), Some(ResolvedArtifacts::Gguf(artifacts))) => {
                let identity_matches = matches!(
                    resolved.identity(),
                    ImmutableModelIdentity::GgufSha256 { digest: resolved_digest }
                        if resolved_digest == &digest.to_string()
                );
                digest == artifacts.digest()
                    && tokenizer.gguf_digest() == Some(digest)
                    && identity_matches
                    && artifacts.matches_descriptor(&descriptor)
            }
            (None, _)
            | (Some(LoadAdmission::Candle), Some(ResolvedArtifacts::Gguf(_)))
            | (Some(LoadAdmission::Gguf { .. }), Some(ResolvedArtifacts::Candle(_)))
            | (_, None) => false,
        }
    }

    fn reject_incompatible_model(&mut self) -> ApplicationEvent {
        self.state.clear_resolved();
        self.resolved_artifacts = None;
        self.tokenizer = None;
        let failure = ApplicationFailure {
            kind: ApplicationFailureKind::Inference,
            message: "resolved identity, descriptor, and tokenizer compatibility evidence differ; deterministic unload was requested".to_owned(),
        };
        if let Err(error) = self.unload_model_with_behavior(crate::ModelUnloadBehavior::Drain) {
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
