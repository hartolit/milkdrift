//! Model resolution, load admission, receipt validation, and resolution persistence.

use candle_backend::{CandleLlamaSource, CandleShardIdentity, CandleWeightShard, SourceError};
use domain_contracts::{
    BackendFailureKind, CapabilitySet, ExecutionDevice, LoadError, MemoryBudget, MemoryFootprint,
    ModelArchitecture, ModelId, QuantizationFormat, ScalarTypeSet,
};
use hf_hub_adapter::{
    ArtifactContentIdentityAuthority, HubModelReference, ResolvedSafetensorsLlamaArtifacts,
};
use hf_tokenizer::HfTokenizer;
use host_runtime::{TrySendError, TrySendError::Disconnected};
use inference_runtime::{CommandTicket, RuntimeCommand};
use redb_storage::ModelRecord;
use tokenization::Tokenizer;

use crate::hub_worker::{HubCommand, HubEvent};
use crate::local::{CANDLE_BACKEND_ID, application_device, execution_device};
use crate::runtime::retained_cleanup::RetainedModelCleanup;
use crate::support::{
    application_configuration_declared_scalar_type, application_scalar_type, hub_failure,
    model_source_failure, stored_configuration_declared_scalar_type, stored_settings,
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
    pub(super) configuration_declared_scalar_type: Option<ApplicationScalarType>,
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
    /// Returns an error when loading is not currently valid, the complete selection or immutable
    /// declaration evidence changed, or the selected inference worker cannot accept the command.
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
        let configuration_declared_scalar_type = resolved.configuration_declared_scalar_type();
        let selected_device = self.state.selected_device();
        self.refresh_selected_device()?;
        let requested_execution_device = execution_device(selected_device);
        let artifacts = self
            .resolved_artifacts
            .as_ref()
            .ok_or(ApplicationError::NoResolvedModel)?;
        let source = CandleLlamaSource::new(
            artifacts.config_path.clone(),
            candle_weight_shards(artifacts).map_err(model_source_failure)?,
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
            configuration_declared_scalar_type,
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
                .configuration_declared_scalar_type
                .map(application_configuration_declared_scalar_type),
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
                let retained_cleanup_exhausted = match error {
                    inference_runtime::RuntimeError::CleanupFailed(_) => {
                        self.retained_model_cleanup =
                            Some(RetainedModelCleanup::PendingInspection {
                                submission_attempts: 0,
                            });
                        Some(false)
                    }
                    inference_runtime::RuntimeError::CleanupRetryExhausted(_) => {
                        self.retained_model_cleanup = Some(RetainedModelCleanup::Exhausted);
                        Some(true)
                    }
                    _ => None,
                };
                if let Some(exhausted) = retained_cleanup_exhausted {
                    self.state.begin_unloading();
                    return ApplicationEvent::ModelCleanupPending {
                        exhausted,
                        failure: retained_model_load_cleanup_failure(error, exhausted),
                    };
                }

                self.retained_model_cleanup = None;
                self.state.set_idle();
                return ApplicationEvent::ModelLoadFailed {
                    failure: model_load_failure(error),
                };
            }
        };
        let Some(resolved) = self.state.resolved().cloned() else {
            return self.reject_incompatible_model(receipt.handle);
        };
        let descriptor = receipt.descriptor;
        let Some(execution_scalar_type) = application_scalar_type(receipt.execution_scalar_type)
        else {
            return self.reject_incompatible_model(receipt.handle);
        };
        let Some(actual_device) = application_device(receipt.execution_device) else {
            return self.reject_incompatible_model(receipt.handle);
        };
        if !self.loaded_compatibility_matches(admission, ticket, &receipt, &resolved, actual_device)
        {
            return self.reject_incompatible_model(receipt.handle);
        }
        let loaded = LoadedModel::new(
            receipt.handle,
            resolved.selection().clone(),
            resolved.identity().clone(),
            actual_device,
            execution_scalar_type,
            descriptor.metadata.vocabulary_size,
            descriptor.capabilities.maximum_context_tokens,
            descriptor.capabilities.maximum_prefill_batch,
        );
        self.retained_model_cleanup = None;
        self.state.set_loaded(loaded.clone());
        ApplicationEvent::ModelLoaded { model: loaded }
    }

    fn loaded_compatibility_matches(
        &self,
        admission: Option<LoadAdmission>,
        ticket: CommandTicket,
        receipt: &inference_runtime::LoadReceipt,
        resolved: &ResolvedModel,
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
        let artifact_configuration_declared_scalar_type = artifacts
            .configuration_declared_scalar_type
            .map(application_configuration_declared_scalar_type);
        let descriptor_configuration_declared_scalar_type =
            match descriptor.metadata.configuration_declared_scalar_type {
                None => None,
                Some(value) => {
                    let Some(value) = application_scalar_type(value) else {
                        return false;
                    };
                    Some(value)
                }
            };
        let required_operations = CapabilitySet::PREFILL.union(CapabilitySet::INCREMENTAL_DECODE);

        admission.ticket == ticket
            && receipt.handle.id == MODEL_ID
            && admission.configuration_declared_scalar_type
                == descriptor_configuration_declared_scalar_type
            && admission.configuration_declared_scalar_type
                == resolved.configuration_declared_scalar_type()
            && admission.configuration_declared_scalar_type
                == artifact_configuration_declared_scalar_type
            && admission.selected_device == self.state.selected_device()
            && admission.selected_device == actual_device
            && admission.execution_device == receipt.execution_device
            && admission.memory_budget == self.memory_budget
            && Self::load_footprint_matches(admission, receipt.reserved_footprint)
            && observed_tensor_scalar_types_are_present(
                descriptor.metadata.observed_tensor_scalar_types,
            )
            && descriptor
                .capabilities
                .operations
                .contains(required_operations)
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
        let Some(host_bytes) = footprint.checked_host_bytes() else {
            return false;
        };
        let Some(device_bytes) = footprint.checked_device_bytes() else {
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
        self.storage.upsert_model(&ModelRecord {
            name: format!("{}@{}", artifacts.repository, artifacts.commit),
            repository: artifacts.repository.clone(),
            revision: artifacts.commit.clone(),
            configuration_declared_scalar_type: artifacts
                .configuration_declared_scalar_type
                .map(stored_configuration_declared_scalar_type),
            last_used_unix_milliseconds: unix_milliseconds(),
        })
    }
}

fn candle_weight_shards(
    artifacts: &ResolvedSafetensorsLlamaArtifacts,
) -> Result<Vec<CandleWeightShard>, SourceError> {
    let mut shards = Vec::new();
    shards
        .try_reserve_exact(artifacts.weight_shards.len())
        .map_err(|_| SourceError::Allocation)?;
    for shard in &artifacts.weight_shards {
        let identity = match shard.content_identity.authority {
            ArtifactContentIdentityAuthority::HuggingFaceLfs => {
                CandleShardIdentity::VerifiedImmutable {
                    byte_length: shard.content_identity.byte_length,
                    sha256: shard.content_identity.sha256,
                }
            }
            ArtifactContentIdentityAuthority::ProjectEstablished => {
                CandleShardIdentity::ProjectEstablished {
                    byte_length: shard.content_identity.byte_length,
                    sha256: shard.content_identity.sha256,
                }
            }
        };
        shards.push(CandleWeightShard::new(shard.path.clone(), identity));
    }
    Ok(shards)
}

const fn observed_tensor_scalar_types_are_present(observed: ScalarTypeSet) -> bool {
    !observed.is_empty()
}

fn model_load_failure(error: inference_runtime::RuntimeError) -> ApplicationFailure {
    let kind = model_load_failure_kind(error);
    let context = match kind {
        ApplicationFailureKind::UnsupportedArtifact => "model artifact or layout is unsupported",
        ApplicationFailureKind::MemoryAdmission => "model load exceeded memory admission",
        _ => "model preparation or materialization failed",
    };
    ApplicationFailure::from_debug(kind, context, error)
}

const fn model_load_failure_kind(error: inference_runtime::RuntimeError) -> ApplicationFailureKind {
    match error {
        inference_runtime::RuntimeError::Load(error) => load_error_failure_kind(error),
        inference_runtime::RuntimeError::InsufficientMemory { .. } => {
            ApplicationFailureKind::MemoryAdmission
        }
        inference_runtime::RuntimeError::CleanupFailed(_)
        | inference_runtime::RuntimeError::CleanupRetryExhausted(_) => {
            ApplicationFailureKind::RetainedCleanup
        }
        _ => ApplicationFailureKind::ModelLoad,
    }
}

const fn load_error_failure_kind(error: LoadError) -> ApplicationFailureKind {
    match error {
        LoadError::InvalidSource
        | LoadError::UnsupportedArchitecture
        | LoadError::UnsupportedFormat
        | LoadError::CapacityExhausted(_) => ApplicationFailureKind::UnsupportedArtifact,
        LoadError::InsufficientMemory { .. } => ApplicationFailureKind::MemoryAdmission,
        LoadError::Backend(failure) => match failure.kind {
            BackendFailureKind::InvalidModel | BackendFailureKind::Unsupported => {
                ApplicationFailureKind::UnsupportedArtifact
            }
            BackendFailureKind::HostMemory | BackendFailureKind::DeviceMemory => {
                ApplicationFailureKind::MemoryAdmission
            }
            _ => ApplicationFailureKind::ModelLoad,
        },
        _ => ApplicationFailureKind::ModelLoad,
    }
}

fn retained_model_load_cleanup_failure(
    error: inference_runtime::RuntimeError,
    exhausted: bool,
) -> ApplicationFailure {
    let context = if exhausted {
        "model load failed and retained cleanup is exhausted"
    } else {
        "model load failed and retained cleanup is pending"
    };
    ApplicationFailure::from_debug(ApplicationFailureKind::RetainedCleanup, context, error)
}
