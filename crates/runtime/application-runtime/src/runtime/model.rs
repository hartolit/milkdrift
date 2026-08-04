//! Model resolution, load admission, receipt validation, and resolution persistence.

use candle_backend::CandleLlamaSource;
use domain_contracts::{
    ExecutionDevice, MemoryBudget, MemoryFootprint, ModelArchitecture, ModelId, QuantizationFormat,
};
use hf_hub_adapter::{HubModelReference, ResolvedSafetensorsLlamaArtifacts};
use hf_tokenizer::HfTokenizer;
use host_runtime::{TrySendError, TrySendError::Disconnected};
use inference_runtime::{CommandTicket, RuntimeCommand};
use redb_storage::ModelRecord;
use tokenization::Tokenizer;

use crate::hub_worker::{HubCommand, HubEvent};
use crate::local::{CANDLE_BACKEND_ID, application_device, execution_device};
use crate::runtime::retained_cleanup::RetainedLoadCleanup;
use crate::support::{
    application_scalar_type, application_source_scalar_type, candle_source_scalar_type,
    hub_failure, model_source_failure, stored_settings, stored_source_scalar_type,
    unix_milliseconds,
};
use crate::{
    ApplicationDevice, ApplicationError, ApplicationEvent, ApplicationFailure,
    ApplicationFailureKind, ApplicationRuntime, ApplicationScalarType, ImmutableModelIdentity,
    LoadedModel, ModelSelection, ResolvedModel,
};

const MODEL_ID: ModelId = ModelId::new(1);

#[derive(Clone, Copy)]
pub(super) struct LoadAdmission {
    pub(super) ticket: CommandTicket,
    pub(super) source_scalar_type: ApplicationScalarType,
    pub(super) selected_device: ApplicationDevice,
    pub(super) execution_device: ExecutionDevice,
    pub(super) memory_budget: MemoryBudget,
}

impl ApplicationRuntime {
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
    /// Returns an error when loading is not currently valid, the complete selection or source
    /// scalar evidence changed, or the selected inference worker cannot accept the command.
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
        let source_scalar_type = resolved
            .source_scalar_type()
            .ok_or(ApplicationError::UnknownScalarType)?;
        let selected_device = self.state.selected_device();
        self.refresh_selected_device()?;
        let requested_execution_device = execution_device(selected_device);
        let artifacts = self
            .resolved_artifacts
            .as_ref()
            .ok_or(ApplicationError::NoResolvedModel)?;
        let source = CandleLlamaSource::new(
            artifacts.config_path.clone(),
            artifacts.weight_paths.clone(),
            candle_source_scalar_type(source_scalar_type),
        )
        .map_err(model_source_failure)?;
        let ticket = self.next_ticket()?;
        self.submit_inference(RuntimeCommand::LoadModel {
            ticket,
            model_id: MODEL_ID,
            source,
            execution_device: requested_execution_device,
        })?;
        self.pending_load = Some(LoadAdmission {
            ticket,
            source_scalar_type,
            selected_device,
            execution_device: requested_execution_device,
            memory_budget: self.memory_budget,
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
            Err(Disconnected(_)) => {
                self.state.disconnect_hub();
                return Err(ApplicationError::HubDisconnected);
            }
        }
        self.clear_resolution();
        self.pending_hub_selection = Some(normalized);
        self.state.begin_resolving();
        Ok(())
    }

    pub(super) fn process_hub_event(&mut self, event: HubEvent) -> ApplicationEvent {
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

    pub(super) fn accept_resolved_artifacts(
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
            artifacts
                .declared_scalar_type
                .map(application_source_scalar_type),
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

    pub(super) fn process_model_loaded(
        &mut self,
        ticket: CommandTicket,
        result: Result<inference_runtime::LoadReceipt, inference_runtime::RuntimeError>,
    ) -> ApplicationEvent {
        let admission = self.pending_load.take();
        let receipt = match result {
            Ok(receipt) => receipt,
            Err(error) => {
                match &error {
                    inference_runtime::RuntimeError::CleanupFailed(_) => {
                        self.retained_load_cleanup = Some(RetainedLoadCleanup::PendingInspection {
                            submission_attempts: 0,
                        });
                        self.state.begin_unloading();
                    }
                    inference_runtime::RuntimeError::CleanupRetryExhausted(_) => {
                        self.retained_load_cleanup = Some(RetainedLoadCleanup::Exhausted);
                        self.state.begin_unloading();
                    }
                    _ => {
                        self.retained_load_cleanup = None;
                        self.state.set_idle();
                    }
                }
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
        let Some(source_scalar_type) = application_scalar_type(descriptor.metadata.scalar_type)
        else {
            return self.reject_incompatible_model(receipt.handle);
        };
        let Some(execution_scalar_type) = application_scalar_type(receipt.execution_scalar_type)
        else {
            return self.reject_incompatible_model(receipt.handle);
        };
        let Some(actual_device) = application_device(receipt.execution_device) else {
            return self.reject_incompatible_model(receipt.handle);
        };
        if !self.loaded_compatibility_matches(
            admission,
            ticket,
            &receipt,
            &resolved,
            source_scalar_type,
            actual_device,
        ) {
            return self.reject_incompatible_model(receipt.handle);
        }
        let loaded = LoadedModel::new(
            receipt.handle,
            resolved.selection().clone(),
            resolved.identity().clone(),
            actual_device,
            source_scalar_type,
            execution_scalar_type,
            descriptor.metadata.vocabulary_size,
            descriptor.capabilities.maximum_context_tokens,
            descriptor.capabilities.maximum_prefill_batch,
        );
        self.retained_load_cleanup = None;
        self.state.set_loaded(loaded.clone());
        ApplicationEvent::ModelLoaded { model: loaded }
    }

    fn loaded_compatibility_matches(
        &self,
        admission: Option<LoadAdmission>,
        ticket: CommandTicket,
        receipt: &inference_runtime::LoadReceipt,
        resolved: &ResolvedModel,
        source_scalar_type: ApplicationScalarType,
        actual_device: ApplicationDevice,
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
        let artifact_source_scalar_type = artifacts
            .declared_scalar_type
            .map(application_source_scalar_type);
        let Some(execution_scalar_type) = application_scalar_type(receipt.execution_scalar_type)
        else {
            return false;
        };

        admission.ticket == ticket
            && receipt.handle.id == MODEL_ID
            && admission.source_scalar_type == source_scalar_type
            && admission.selected_device == self.state.selected_device()
            && admission.selected_device == actual_device
            && admission.execution_device == receipt.execution_device
            && admission.memory_budget == self.memory_budget
            && Self::load_footprint_matches(admission, receipt.reserved_footprint)
            && resolved.source_scalar_type() == Some(source_scalar_type)
            && artifact_source_scalar_type == Some(source_scalar_type)
            && source_and_execution_scalars_are_coherent(source_scalar_type, execution_scalar_type)
            && resolved.selection() == &artifact_selection
            && resolved.identity().repository() == artifacts.repository
            && resolved.identity().commit() == artifacts.commit
            && descriptor.backend == CANDLE_BACKEND_ID
            && descriptor.metadata.architecture == ModelArchitecture::Llama
            && descriptor.metadata.quantization == QuantizationFormat::None
            && descriptor.metadata.context_length > 0
            && descriptor.capabilities.maximum_context_tokens == descriptor.metadata.context_length
            && descriptor.capabilities.maximum_prefill_batch > 0
            && descriptor.capabilities.maximum_prefill_batch
                <= descriptor.capabilities.maximum_context_tokens
            && descriptor.capabilities.maximum_sequences > 0
            && tokenizer.vocabulary_size() == descriptor.metadata.vocabulary_size
    }

    pub(super) fn load_footprint_matches(
        admission: LoadAdmission,
        footprint: MemoryFootprint,
    ) -> bool {
        let Some(host_bytes) = footprint
            .host_weight_bytes
            .checked_add(footprint.host_working_bytes)
        else {
            return false;
        };
        let Some(device_bytes) = footprint
            .device_weight_bytes
            .checked_add(footprint.device_working_bytes)
        else {
            return false;
        };
        if host_bytes > admission.memory_budget.host_bytes
            || device_bytes > admission.memory_budget.device_bytes
        {
            return false;
        }

        match admission.selected_device {
            ApplicationDevice::Cpu => {
                footprint.device_weight_bytes == 0 && footprint.device_working_bytes == 0
            }
            ApplicationDevice::Cuda { .. } => footprint.host_weight_bytes == 0,
        }
    }

    fn persist_resolved(
        &mut self,
        artifacts: &ResolvedSafetensorsLlamaArtifacts,
    ) -> Result<(), redb_storage::StorageError> {
        let mut candidate = self.preferences.clone();
        candidate
            .default_repository
            .clone_from(&artifacts.repository);
        candidate.default_revision.clone_from(&artifacts.revision);
        self.storage.save_settings(&stored_settings(&candidate))?;
        self.preferences = candidate;
        let Some(source_scalar_type) = artifacts.declared_scalar_type else {
            return Ok(());
        };
        self.storage.upsert_model(&ModelRecord {
            name: format!("{}@{}", artifacts.repository, artifacts.commit),
            repository: artifacts.repository.clone(),
            revision: artifacts.commit.clone(),
            scalar_type: stored_source_scalar_type(source_scalar_type),
            last_used_unix_milliseconds: unix_milliseconds(),
        })
    }
}

const fn source_and_execution_scalars_are_coherent(
    source_scalar_type: ApplicationScalarType,
    execution_scalar_type: ApplicationScalarType,
) -> bool {
    match source_scalar_type {
        ApplicationScalarType::F32 => {
            matches!(execution_scalar_type, ApplicationScalarType::F32)
        }
        ApplicationScalarType::F16 => {
            matches!(execution_scalar_type, ApplicationScalarType::F16)
        }
        ApplicationScalarType::Bf16 => matches!(
            execution_scalar_type,
            ApplicationScalarType::Bf16 | ApplicationScalarType::F32
        ),
    }
}
