//! Frontend-neutral application state exposed by the orchestration engine.

use domain_contracts::{FinishReason, GenerationUsage, ModelHandle, RequestId};

use crate::{
    ApplicationDevice, ApplicationEngine, ApplicationFailure, ApplicationModelFormat,
    ApplicationScalarType, ApplicationSource, ChatCompatibility, ImmutableModelIdentity,
    ModelSelection,
};

/// Long-running application operation currently in progress.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ApplicationActivity {
    /// No model-lifecycle command is awaiting completion.
    #[default]
    Idle,
    /// Immutable model artifacts are being resolved and validated.
    Resolving,
    /// Model resources are being loaded by the inference runtime.
    Loading,
    /// Active work is draining or the loaded model is being released.
    Unloading,
    /// Worker shutdown has begun and no new work is accepted.
    ShuttingDown,
}

/// Validated immutable model selection available for loading.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolvedModel {
    selection: ModelSelection,
    identity: ImmutableModelIdentity,
    vocabulary_size: u32,
    scalar_type: Option<ApplicationScalarType>,
    chat_compatibility: ChatCompatibility,
}

impl ResolvedModel {
    pub(crate) const fn new(
        selection: ModelSelection,
        identity: ImmutableModelIdentity,
        vocabulary_size: u32,
        scalar_type: Option<ApplicationScalarType>,
        chat_compatibility: ChatCompatibility,
    ) -> Self {
        Self {
            selection,
            identity,
            vocabulary_size,
            scalar_type,
            chat_compatibility,
        }
    }

    /// Returns the normalized selection represented by this resolution.
    #[must_use]
    pub const fn selection(&self) -> &ModelSelection {
        &self.selection
    }

    /// Returns the local execution engine.
    #[must_use]
    pub const fn engine(&self) -> ApplicationEngine {
        ApplicationEngine::Candle
    }

    /// Returns the artifact source category.
    #[must_use]
    pub const fn source(&self) -> ApplicationSource {
        ApplicationSource::HuggingFaceHub
    }

    /// Returns the execution device category.
    #[must_use]
    pub const fn device(&self) -> ApplicationDevice {
        ApplicationDevice::Cpu
    }

    /// Returns the model serialization format.
    #[must_use]
    pub const fn format(&self) -> ApplicationModelFormat {
        ApplicationModelFormat::Safetensors
    }

    /// Returns the immutable artifact identity.
    #[must_use]
    pub const fn identity(&self) -> &ImmutableModelIdentity {
        &self.identity
    }

    /// Returns the validated tokenizer vocabulary size.
    #[must_use]
    pub const fn vocabulary_size(&self) -> u32 {
        self.vocabulary_size
    }

    /// Returns the scalar type declared by immutable model configuration, when supported.
    #[must_use]
    pub const fn scalar_type(&self) -> Option<ApplicationScalarType> {
        self.scalar_type
    }

    /// Returns whether this resolution contains sufficient evidence for loading.
    #[must_use]
    pub const fn is_loadable(&self) -> bool {
        self.scalar_type.is_some()
    }

    /// Returns explicit prompt-rendering and termination compatibility.
    #[must_use]
    pub const fn chat_compatibility(&self) -> ChatCompatibility {
        self.chat_compatibility
    }

    /// Returns whether a complete visible selection still addresses this resolution.
    #[must_use]
    pub fn matches_selection(&self, selection: &ModelSelection) -> bool {
        &self.selection == selection
    }
}

/// One model generation currently owned by the local inference runtime.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LoadedModel {
    handle: ModelHandle,
    selection: ModelSelection,
    identity: ImmutableModelIdentity,
    scalar_type: ApplicationScalarType,
    vocabulary_size: u32,
    maximum_context_tokens: u32,
    maximum_prefill_batch: u32,
}

impl LoadedModel {
    pub(crate) const fn new(
        handle: ModelHandle,
        selection: ModelSelection,
        identity: ImmutableModelIdentity,
        scalar_type: ApplicationScalarType,
        vocabulary_size: u32,
        maximum_context_tokens: u32,
        maximum_prefill_batch: u32,
    ) -> Self {
        Self {
            handle,
            selection,
            identity,
            scalar_type,
            vocabulary_size,
            maximum_context_tokens,
            maximum_prefill_batch,
        }
    }

    /// Returns the generation-safe handle assigned by E0.
    #[must_use]
    pub const fn handle(&self) -> ModelHandle {
        self.handle
    }

    /// Returns the complete selection used for this loaded generation.
    #[must_use]
    pub const fn selection(&self) -> &ModelSelection {
        &self.selection
    }

    /// Returns the local execution engine.
    #[must_use]
    pub const fn engine(&self) -> ApplicationEngine {
        ApplicationEngine::Candle
    }

    /// Returns the artifact source category.
    #[must_use]
    pub const fn source(&self) -> ApplicationSource {
        ApplicationSource::HuggingFaceHub
    }

    /// Returns the execution device category.
    #[must_use]
    pub const fn device(&self) -> ApplicationDevice {
        ApplicationDevice::Cpu
    }

    /// Returns the model serialization format.
    #[must_use]
    pub const fn format(&self) -> ApplicationModelFormat {
        ApplicationModelFormat::Safetensors
    }

    /// Returns the immutable artifact identity loaded by E0.
    #[must_use]
    pub const fn identity(&self) -> &ImmutableModelIdentity {
        &self.identity
    }

    /// Returns the scalar type validated against the loaded E0 descriptor.
    #[must_use]
    pub const fn scalar_type(&self) -> ApplicationScalarType {
        self.scalar_type
    }

    /// Returns the loaded model vocabulary size.
    #[must_use]
    pub const fn vocabulary_size(&self) -> u32 {
        self.vocabulary_size
    }

    /// Returns maximum token positions supported by one sequence.
    #[must_use]
    pub const fn maximum_context_tokens(&self) -> u32 {
        self.maximum_context_tokens
    }

    /// Returns maximum prompt tokens accepted by one prefill operation.
    #[must_use]
    pub const fn maximum_prefill_batch(&self) -> u32 {
        self.maximum_prefill_batch
    }
}

/// Frontend-visible phase of one direct-completion request.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GenerationPhase {
    /// E1 submitted the complete request and awaits E0 admission.
    Starting,
    /// E0 admitted the request and generation may advance independently.
    Running,
    /// E1 requested cancellation and awaits a safe E0 terminal boundary.
    Cancelling,
    /// Generation is terminal and explicit sequence cleanup is in progress.
    Finishing,
    /// Sequence cleanup failed but remains retained for bounded retry.
    CleanupPending,
    /// Automatic cleanup attempts are exhausted and ownership remains retained.
    CleanupExhausted,
}

/// Current request identity and usage exposed to every frontend.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GenerationSummary {
    /// Active request identity.
    pub request_id: RequestId,
    /// Current frontend-visible phase.
    pub phase: GenerationPhase,
    /// Prompt/generated token accounting observed by E1.
    pub usage: GenerationUsage,
}

/// Final frontend-neutral generation result.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GenerationTerminalOutcome {
    /// E0 completed generation with a stable finish reason.
    Finished(FinishReason),
    /// E0 or E1 failed the request; diagnostic ownership remains in E1.
    Failed(ApplicationFailure),
}

/// Last terminal request summary retained after release or failed admission.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GenerationTerminal {
    /// Request identity.
    pub request_id: RequestId,
    /// Final completion or failure classification.
    pub outcome: GenerationTerminalOutcome,
    /// Prompt/generated usage observed before terminal release.
    pub usage: GenerationUsage,
}

/// Read-only application state shared by every frontend implementation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ApplicationState {
    activity: ApplicationActivity,
    resolved: Option<ResolvedModel>,
    loaded: Option<LoadedModel>,
    generation: Option<GenerationSummary>,
    last_generation: Option<GenerationTerminal>,
    hub_available: bool,
    inference_available: bool,
}

impl Default for ApplicationState {
    fn default() -> Self {
        Self {
            activity: ApplicationActivity::Idle,
            resolved: None,
            loaded: None,
            generation: None,
            last_generation: None,
            hub_available: true,
            inference_available: true,
        }
    }
}

impl ApplicationState {
    /// Returns the current long-running model-lifecycle operation.
    #[must_use]
    pub const fn activity(&self) -> ApplicationActivity {
        self.activity
    }

    /// Returns the immutable model resolution, when available.
    #[must_use]
    pub const fn resolved(&self) -> Option<&ResolvedModel> {
        self.resolved.as_ref()
    }

    /// Returns the loaded model generation, when present.
    #[must_use]
    pub const fn loaded(&self) -> Option<&LoadedModel> {
        self.loaded.as_ref()
    }

    /// Returns the active direct-completion request, when present.
    #[must_use]
    pub const fn active_generation(&self) -> Option<GenerationSummary> {
        self.generation
    }

    /// Returns the most recently terminal generation summary.
    #[must_use]
    pub const fn last_generation(&self) -> Option<&GenerationTerminal> {
        self.last_generation.as_ref()
    }

    /// Returns whether the Hub resolver worker can accept work.
    #[must_use]
    pub const fn hub_available(&self) -> bool {
        self.hub_available
    }

    /// Returns whether the inference worker can accept work.
    #[must_use]
    pub const fn inference_available(&self) -> bool {
        self.inference_available
    }

    /// Returns whether immutable artifact resolution may be started for a selection.
    #[must_use]
    pub const fn can_resolve(&self, _selection: &ModelSelection) -> bool {
        matches!(self.activity, ApplicationActivity::Idle)
            && self.hub_available
            && self.loaded.is_none()
            && self.generation.is_none()
    }

    /// Returns whether a model may be loaded for the complete visible selection.
    #[must_use]
    pub fn can_load(&self, selection: &ModelSelection) -> bool {
        self.activity == ApplicationActivity::Idle
            && self.inference_available
            && self.loaded.is_none()
            && self.generation.is_none()
            && self.resolved.as_ref().is_some_and(|resolved| {
                resolved.is_loadable() && resolved.matches_selection(selection)
            })
    }

    /// Returns whether completion or compatible chat generation may start against the resident model.
    #[must_use]
    pub const fn can_start_generation(&self) -> bool {
        matches!(self.activity, ApplicationActivity::Idle)
            && self.inference_available
            && self.loaded.is_some()
            && self.generation.is_none()
    }

    /// Returns whether the active request still accepts an explicit cancellation request.
    #[must_use]
    pub const fn can_cancel_generation(&self) -> bool {
        matches!(
            self.generation,
            Some(GenerationSummary {
                phase: GenerationPhase::Starting | GenerationPhase::Running,
                ..
            })
        )
    }

    /// Returns whether the resident model may be unloaded.
    #[must_use]
    pub const fn can_unload(&self) -> bool {
        matches!(self.activity, ApplicationActivity::Idle)
            && self.inference_available
            && self.loaded.is_some()
    }

    pub(crate) fn begin_resolving(&mut self) {
        self.activity = ApplicationActivity::Resolving;
        self.resolved = None;
    }

    pub(crate) const fn begin_loading(&mut self) {
        self.activity = ApplicationActivity::Loading;
    }

    pub(crate) const fn begin_unloading(&mut self) {
        self.activity = ApplicationActivity::Unloading;
    }

    pub(crate) const fn begin_shutdown(&mut self) {
        self.activity = ApplicationActivity::ShuttingDown;
    }

    pub(crate) const fn set_idle(&mut self) {
        self.activity = ApplicationActivity::Idle;
    }

    pub(crate) fn set_resolved(&mut self, resolved: ResolvedModel) {
        self.resolved = Some(resolved);
        self.activity = ApplicationActivity::Idle;
    }

    pub(crate) fn clear_resolved(&mut self) {
        self.resolved = None;
    }

    pub(crate) fn set_loaded(&mut self, loaded: LoadedModel) {
        self.loaded = Some(loaded);
        self.activity = ApplicationActivity::Idle;
    }

    pub(crate) fn clear_loaded(&mut self) {
        self.loaded = None;
        self.activity = ApplicationActivity::Idle;
    }

    pub(crate) fn begin_generation(&mut self, summary: GenerationSummary) {
        self.generation = Some(summary);
        self.last_generation = None;
    }

    pub(crate) const fn set_generation_phase(&mut self, phase: GenerationPhase) {
        if let Some(summary) = self.generation.as_mut() {
            summary.phase = phase;
        }
    }

    pub(crate) const fn increment_generated_tokens(&mut self) {
        if let Some(summary) = self.generation.as_mut() {
            summary.usage.generated_tokens = summary.usage.generated_tokens.saturating_add(1);
        }
    }

    pub(crate) fn finish_generation(&mut self, terminal: GenerationTerminal) {
        self.generation = None;
        self.last_generation = Some(terminal);
    }

    pub(crate) const fn disconnect_hub(&mut self) {
        self.hub_available = false;
    }

    pub(crate) const fn disconnect_inference(&mut self) {
        self.inference_available = false;
    }
}
