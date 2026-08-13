//! Model resolution, correlated load admission, receipt validation, and persistence.

use candle_backend::{CandleLlamaSource, CandleShardIdentity, CandleWeightShard, SourceError};
use domain_contracts::{
    BackendFailureKind, CapabilitySet, ExecutionDevice, LoadError, MemoryBudget, MemoryFootprint,
    ModelArchitecture, ModelId, QuantizationFormat, ScalarTypeSet,
};
use hf_hub_adapter::{
    ArtifactContentIdentityAuthority, HubError, HubModelReference,
    ResolvedSafetensorsLlamaArtifacts,
};
use hf_tokenizer::{HfTokenizer, HfTokenizerLoadError};
use host_runtime::{TrySendError, TrySendError::Disconnected};
use inference_runtime::{CommandTicket, RuntimeCommand, RuntimeError};
use redb_storage::ModelRecord;
use tokenization::Tokenizer;

use crate::hub_worker::{HubCommand, HubEvent};
use crate::local::{CANDLE_BACKEND_ID, application_device, execution_device};
use crate::support::{
    application_configuration_declared_scalar_type, application_scalar_type, hub_failure,
    model_resolution_failure, model_source_failure, stored_configuration_declared_scalar_type,
    stored_settings, unix_milliseconds,
};
use crate::{
    ApplicationActivity, ApplicationDevice, ApplicationError, ApplicationEvent, ApplicationFailure,
    ApplicationFailureKind, ApplicationGenerationMode, ApplicationMemoryFootprint,
    ApplicationRuntime, ChatCompatibility, ImmutableModelIdentity, LoadedModel, ModelSelection,
    ResolvedModel,
};

const MODEL_ID: ModelId = ModelId::new(1);

#[derive(Clone, Copy)]
pub(super) struct LoadAdmission {
    pub(super) selected_device: ApplicationDevice,
    pub(super) execution_device: ExecutionDevice,
    pub(super) memory_budget: MemoryBudget,
}

#[derive(Clone)]
pub(super) struct ResolvedArtifactSnapshot {
    pub(super) model: ResolvedModel,
    pub(super) artifacts: ResolvedSafetensorsLlamaArtifacts,
    pub(super) tokenizer_vocabulary_size: u32,
}

pub(super) struct ModelLoadTransaction {
    pub(super) ticket: CommandTicket,
    pub(super) resolved: ResolvedArtifactSnapshot,
    pub(super) admission: LoadAdmission,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum LoadReceiptMismatch {
    MissingTransaction,
    Ticket,
    ModelIdentity,
    Declaration,
    ExecutionScalar,
    ExecutionDevice,
    SelectedDevice,
    MemoryBudget,
    FinalFootprint,
    ObservedEvidence,
    Capabilities,
    Composition,
    Limits,
    TokenizerVocabulary,
}

impl LoadReceiptMismatch {
    pub(super) const fn message(self) -> &'static str {
        match self {
            Self::MissingTransaction => "the load result had no pending load transaction",
            Self::Ticket => "the load result ticket did not match the pending load transaction",
            Self::ModelIdentity => "the load receipt model identity did not match",
            Self::Declaration => {
                "the independently parsed configuration declaration did not match resolution"
            }
            Self::ExecutionScalar => "the load receipt execution scalar is not representable by E1",
            Self::ExecutionDevice => "the load receipt execution device is not representable by E1",
            Self::SelectedDevice => "selected, requested, and actual execution devices disagreed",
            Self::MemoryBudget => "the startup-fixed load budget changed during the transaction",
            Self::FinalFootprint => {
                "the verified final footprint overflowed or exceeded the admitted budget"
            }
            Self::ObservedEvidence => {
                "the lower descriptor omitted complete observed scalar evidence"
            }
            Self::Capabilities => "the lower descriptor omitted required generation capabilities",
            Self::Composition => {
                "the lower descriptor did not match the supported local composition"
            }
            Self::Limits => "the lower descriptor reported incoherent model limits",
            Self::TokenizerVocabulary => {
                "the lower descriptor vocabulary did not match the resolved tokenizer"
            }
        }
    }
}

pub(super) struct ValidatedLoad {
    loaded: LoadedModel,
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

    /// Loads the immutable artifact snapshot retained by successful resolution.
    ///
    /// The transaction re-probes the selected device, constructs one source from
    /// the retained snapshot, submits one lower load, correlates its ticket, checks
    /// generic receipt invariants, and only then publishes [`LoadedModel`].
    ///
    /// # Errors
    ///
    /// Returns an error when loading is not currently valid, the visible selection
    /// changed, resolved evidence is internally inconsistent, or the inference worker
    /// cannot accept the command.
    pub fn load_model(&mut self, selection: &ModelSelection) -> Result<(), ApplicationError> {
        self.require_idle()?;
        if self.state.retained_model().is_some() {
            return Err(ApplicationError::Busy(ApplicationActivity::RetainedCleanup));
        }
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
        let snapshot = self.resolved_artifact_snapshot(resolved)?;

        let selected_device = self.state.selected_device();
        self.refresh_selected_device()?;
        let requested_execution_device = execution_device(selected_device);
        let config_bytes = snapshot
            .artifacts
            .config
            .read_verified_bytes()
            .map_err(|error| {
                resolved_content_verification_failure(
                    &error,
                    "resolved model configuration verification failed before load",
                )
            })?;
        let source = CandleLlamaSource::from_config_bytes(
            config_bytes,
            candle_weight_shards(&snapshot.artifacts).map_err(model_source_failure)?,
        )
        .map_err(model_source_failure)?;
        let ticket = self.next_ticket()?;
        self.submit_inference(RuntimeCommand::LoadModel {
            ticket,
            model_id: MODEL_ID,
            source,
            execution_device: requested_execution_device,
        })?;
        self.pending_load = Some(ModelLoadTransaction {
            ticket,
            resolved: snapshot,
            admission: LoadAdmission {
                selected_device,
                execution_device: requested_execution_device,
                memory_budget: self.memory_budget,
            },
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

    fn resolved_artifact_snapshot(
        &self,
        model: ResolvedModel,
    ) -> Result<ResolvedArtifactSnapshot, ApplicationError> {
        let artifacts = self
            .resolved_artifacts
            .clone()
            .ok_or(ApplicationError::NoResolvedModel)?;
        let tokenizer = self
            .tokenizer
            .as_ref()
            .ok_or(ApplicationError::NoTokenizer)?;
        let artifact_selection = ModelSelection::new(&artifacts.repository, &artifacts.revision);
        if model.selection() != &artifact_selection
            || model.identity().repository() != artifacts.repository
            || model.identity().commit() != artifacts.commit
            || model.vocabulary_size() != tokenizer.vocabulary_size()
        {
            return Err(ApplicationFailure::new(
                ApplicationFailureKind::IncompatibleReceipt,
                "resolved artifact state no longer matches its public immutable projection",
            )
            .into());
        }
        Ok(ResolvedArtifactSnapshot {
            model,
            artifacts,
            tokenizer_vocabulary_size: tokenizer.vocabulary_size(),
        })
    }

    fn resolve_hugging_face(&mut self, selection: ModelSelection) -> Result<(), ApplicationError> {
        if !self.state.hub_available() {
            return Err(ApplicationError::HubDisconnected);
        }
        let (repository, revision) = selection.into_parts();
        let reference =
            HubModelReference::new(repository, revision).map_err(|error| hub_failure(&error))?;
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
                    failure: model_resolution_failure(&error),
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
                ApplicationFailureKind::ArtifactResolution,
                "artifact resolution returned a different complete selection",
            ));
        }
        self.pending_hub_selection = None;

        let tokenizer_bytes = match artifacts.tokenizer.read_verified_bytes() {
            Ok(bytes) => bytes,
            Err(error) => {
                return self.reject_resolution(resolved_content_verification_failure(
                    &error,
                    "resolved tokenizer content verification failed",
                ));
            }
        };
        let tokenizer = match HfTokenizer::from_bytes(tokenizer_bytes.as_slice()) {
            Ok(tokenizer) => tokenizer,
            Err(error) => return self.reject_resolution(tokenizer_load_failure(&error)),
        };
        let chat_profile = crate::chat::detect_chat_profile(
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
            chat_profile,
        );
        let persistence_warning = self
            .persist_resolved(&artifacts, &resolved)
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
        result: &Result<inference_runtime::LoadReceipt, RuntimeError>,
    ) -> Option<ApplicationEvent> {
        let correlation_mismatch = match self.pending_load.as_ref() {
            None => Some(LoadReceiptMismatch::MissingTransaction),
            Some(transaction) if transaction.ticket != ticket => Some(LoadReceiptMismatch::Ticket),
            Some(_) => None,
        };
        if let Some(mismatch) = correlation_mismatch {
            return self.process_uncorrelated_model_load(result, mismatch);
        }

        let Some(transaction) = self.pending_load.take() else {
            return self
                .process_uncorrelated_model_load(result, LoadReceiptMismatch::MissingTransaction);
        };
        let receipt = match result {
            Ok(receipt) => *receipt,
            Err(
                RuntimeError::CleanupFailed(cleanup) | RuntimeError::CleanupRetryExhausted(cleanup),
            ) => {
                self.begin_runtime_retention(*cleanup, None);
                return Some(self.current_cleanup_event());
            }
            Err(error) => {
                self.state.set_idle();
                return Some(ApplicationEvent::ModelLoadFailed {
                    failure: model_load_failure(error),
                });
            }
        };

        match self.validate_load_receipt(&transaction, &receipt) {
            Ok(validated) => {
                self.model_cleanup = None;
                self.state.clear_retained_model();
                self.state.set_loaded(validated.loaded.clone());
                Some(ApplicationEvent::ModelLoaded {
                    model: validated.loaded,
                })
            }
            Err(mismatch) => Some(self.reject_incompatible_model(
                receipt.handle,
                receipt.reserved_footprint,
                incompatible_receipt_failure(mismatch),
            )),
        }
    }

    fn process_uncorrelated_model_load(
        &mut self,
        result: &Result<inference_runtime::LoadReceipt, RuntimeError>,
        mismatch: LoadReceiptMismatch,
    ) -> Option<ApplicationEvent> {
        match result {
            Ok(receipt) => {
                self.pending_load = None;
                Some(self.reject_incompatible_model(
                    receipt.handle,
                    receipt.reserved_footprint,
                    incompatible_receipt_failure(mismatch),
                ))
            }
            Err(
                RuntimeError::CleanupFailed(cleanup) | RuntimeError::CleanupRetryExhausted(cleanup),
            ) => {
                self.pending_load = None;
                self.begin_runtime_retention(
                    *cleanup,
                    Some(incompatible_receipt_failure(mismatch)),
                );
                Some(self.current_cleanup_event())
            }
            Err(_) => None,
        }
    }

    pub(super) fn validate_load_receipt(
        &self,
        transaction: &ModelLoadTransaction,
        receipt: &inference_runtime::LoadReceipt,
    ) -> Result<ValidatedLoad, LoadReceiptMismatch> {
        if receipt.handle.id != MODEL_ID {
            return Err(LoadReceiptMismatch::ModelIdentity);
        }
        let execution_scalar_type = application_scalar_type(receipt.execution_scalar_type)
            .ok_or(LoadReceiptMismatch::ExecutionScalar)?;
        let actual_device = application_device(receipt.execution_device)
            .ok_or(LoadReceiptMismatch::ExecutionDevice)?;
        let descriptor = receipt.descriptor;
        let descriptor_declaration = match descriptor.metadata.configuration_declared_scalar_type {
            None => None,
            Some(value) => {
                Some(application_scalar_type(value).ok_or(LoadReceiptMismatch::Declaration)?)
            }
        };
        if descriptor_declaration
            != transaction
                .resolved
                .model
                .configuration_declared_scalar_type()
        {
            return Err(LoadReceiptMismatch::Declaration);
        }
        if transaction.admission.selected_device != self.state.selected_device()
            || transaction.admission.selected_device != actual_device
            || transaction.admission.execution_device != receipt.execution_device
        {
            return Err(LoadReceiptMismatch::SelectedDevice);
        }
        if transaction.admission.memory_budget != self.memory_budget {
            return Err(LoadReceiptMismatch::MemoryBudget);
        }
        if !Self::load_footprint_matches(&transaction.admission, receipt.reserved_footprint) {
            return Err(LoadReceiptMismatch::FinalFootprint);
        }
        if !observed_tensor_scalar_types_are_present(
            descriptor.metadata.observed_tensor_scalar_types,
        ) {
            return Err(LoadReceiptMismatch::ObservedEvidence);
        }
        let required_operations = CapabilitySet::PREFILL.union(CapabilitySet::INCREMENTAL_DECODE);
        if !descriptor
            .capabilities
            .operations
            .contains(required_operations)
        {
            return Err(LoadReceiptMismatch::Capabilities);
        }
        if descriptor.backend != CANDLE_BACKEND_ID
            || descriptor.metadata.architecture != ModelArchitecture::Llama
            || descriptor.metadata.quantization != QuantizationFormat::None
        {
            return Err(LoadReceiptMismatch::Composition);
        }
        if descriptor.metadata.context_length == 0
            || descriptor.capabilities.maximum_context_tokens != descriptor.metadata.context_length
            || descriptor.capabilities.maximum_prefill_batch == 0
            || descriptor.capabilities.maximum_prefill_batch
                > descriptor.capabilities.maximum_context_tokens
            || descriptor.capabilities.maximum_sequences == 0
        {
            return Err(LoadReceiptMismatch::Limits);
        }
        if transaction.resolved.tokenizer_vocabulary_size != descriptor.metadata.vocabulary_size {
            return Err(LoadReceiptMismatch::TokenizerVocabulary);
        }

        let resolved = &transaction.resolved.model;
        let generation_mode = match resolved.chat_compatibility() {
            ChatCompatibility::Supported => ApplicationGenerationMode::Chat,
            ChatCompatibility::Unsupported => ApplicationGenerationMode::DirectCompletion,
        };
        let loaded = LoadedModel::new(
            receipt.handle,
            resolved.selection().clone(),
            resolved.identity().clone(),
            actual_device,
            execution_scalar_type,
            descriptor.metadata.vocabulary_size,
            descriptor.capabilities.maximum_context_tokens,
            descriptor.capabilities.maximum_prefill_batch,
            generation_mode,
            ApplicationMemoryFootprint::from(receipt.reserved_footprint),
        );
        Ok(ValidatedLoad { loaded })
    }

    pub(super) fn load_footprint_matches(
        admission: &LoadAdmission,
        footprint: MemoryFootprint,
    ) -> bool {
        footprint
            .checked_host_bytes()
            .zip(footprint.checked_device_bytes())
            .is_some_and(|(host_bytes, device_bytes)| {
                host_bytes <= admission.memory_budget.host_bytes
                    && device_bytes <= admission.memory_budget.device_bytes
            })
    }

    fn persist_resolved(
        &mut self,
        artifacts: &ResolvedSafetensorsLlamaArtifacts,
        resolved: &ResolvedModel,
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
            configuration_declared_scalar_type: resolved
                .configuration_declared_scalar_type()
                .map(stored_configuration_declared_scalar_type),
            last_resolved_unix_milliseconds: unix_milliseconds(),
        })
    }
}

fn resolved_content_verification_failure(error: &HubError, context: &str) -> ApplicationFailure {
    let normalized = model_resolution_failure(error);
    ApplicationFailure::new(
        normalized.kind,
        format!("{context}: {}", normalized.message),
    )
}

fn tokenizer_load_failure(error: &HfTokenizerLoadError) -> ApplicationFailure {
    let message = match error {
        HfTokenizerLoadError::InvalidTokenizer(_) => {
            "verified tokenizer bytes are not a valid supported tokenizer serialization"
        }
        HfTokenizerLoadError::VocabularyOverflow { .. } => {
            "verified tokenizer vocabulary exceeds the application token identifier range"
        }
    };
    ApplicationFailure::new(ApplicationFailureKind::Tokenizer, message)
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
            ArtifactContentIdentityAuthority::HuggingFaceLfs
            | ArtifactContentIdentityAuthority::HuggingFaceGitBlob => {
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

fn incompatible_receipt_failure(mismatch: LoadReceiptMismatch) -> ApplicationFailure {
    ApplicationFailure::new(
        ApplicationFailureKind::IncompatibleReceipt,
        mismatch.message(),
    )
}

pub(super) fn model_load_failure(error: &RuntimeError) -> ApplicationFailure {
    let kind = model_load_failure_kind(error);
    let context = match kind {
        ApplicationFailureKind::UnsupportedArtifact => "model artifact or layout is unsupported",
        ApplicationFailureKind::MemoryAdmission => "model load exceeded memory admission",
        _ => "model preparation or materialization failed",
    };
    ApplicationFailure::from_debug(kind, context, error)
}

const fn model_load_failure_kind(error: &RuntimeError) -> ApplicationFailureKind {
    match error {
        RuntimeError::Load(error) => load_error_failure_kind(*error),
        RuntimeError::InsufficientMemory { .. } => ApplicationFailureKind::MemoryAdmission,
        RuntimeError::CleanupFailed(_) | RuntimeError::CleanupRetryExhausted(_) => {
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
