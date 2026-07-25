//! Frontend-neutral application state exposed by the orchestration engine.

use domain_contracts::{
    DeviceKind, FinishReason, GenerationUsage, ModelHandle, RequestId, ScalarType,
};

use crate::ApplicationFailure;

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

/// Backend selected by the initial application product path.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ApplicationBackend {
    /// Candle Llama/Safetensors backend.
    Candle,
}

/// Validated immutable model selection available for loading.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolvedModel {
    /// Hugging Face repository requested by the user.
    pub repository: String,
    /// Branch, tag, reference, or commit requested by the user.
    pub revision: String,
    /// Immutable commit used for every cached artifact.
    pub commit: String,
    /// Vocabulary size reported by the validated tokenizer.
    pub vocabulary_size: u32,
    /// Scalar type declared by the model configuration when recognized.
    pub scalar_type: Option<ScalarType>,
}

impl ResolvedModel {
    /// Returns whether visible repository and revision values still address this resolution.
    #[must_use]
    pub fn matches_selection(&self, repository: &str, revision: &str) -> bool {
        repository.trim() == self.repository && revision.trim() == self.revision
    }
}

/// One model generation currently owned by the inference runtime.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LoadedModel {
    /// Generation-safe handle assigned by the inference runtime.
    pub handle: ModelHandle,
    /// Vocabulary size reported by the loaded model descriptor.
    pub vocabulary_size: u32,
    /// Maximum token positions supported by one sequence.
    pub maximum_context_tokens: u32,
    /// Maximum prompt tokens accepted by one prefill operation.
    pub maximum_prefill_batch: u32,
    /// Application-selected backend.
    pub backend: ApplicationBackend,
    /// Application-selected execution device category.
    pub device: DeviceKind,
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
    pub const fn loaded(&self) -> Option<LoadedModel> {
        self.loaded
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

    /// Returns whether immutable artifact resolution may be started.
    #[must_use]
    pub const fn can_resolve(&self) -> bool {
        matches!(self.activity, ApplicationActivity::Idle)
            && self.hub_available
            && self.loaded.is_none()
            && self.generation.is_none()
    }

    /// Returns whether a model may be loaded for the current visible selection.
    #[must_use]
    pub fn can_load(&self, repository: &str, revision: &str) -> bool {
        self.activity == ApplicationActivity::Idle
            && self.inference_available
            && self.loaded.is_none()
            && self.generation.is_none()
            && self.resolved.as_ref().is_some_and(|resolved| {
                resolved.scalar_type.is_some() && resolved.matches_selection(repository, revision)
            })
    }

    /// Returns whether direct completion may start against the resident model.
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

    pub(crate) const fn set_loaded(&mut self, loaded: LoadedModel) {
        self.loaded = Some(loaded);
        self.activity = ApplicationActivity::Idle;
    }

    pub(crate) const fn clear_loaded(&mut self) {
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
