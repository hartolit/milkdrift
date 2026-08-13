//! Frontend-neutral application state exposed by the orchestration engine.

use domain_contracts::{FinishReason, GenerationUsage, ModelHandle, RequestId};

use crate::{
    ApplicationDevice, ApplicationDeviceDiscoveryFailure, ApplicationDeviceSummary,
    ApplicationDeviceUnavailableReason, ApplicationFailure, ApplicationMemoryFootprint,
    ApplicationRetainedModel, ApplicationScalarType, ChatCompatibility, ImmutableModelIdentity,
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
    /// Lower model ownership remains retained or unconfirmed while normal admission is locked.
    RetainedCleanup,
    /// Worker shutdown has begun and no new work is accepted.
    ShuttingDown,
}

/// Validated immutable model selection available for loading.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolvedModel {
    selection: ModelSelection,
    identity: ImmutableModelIdentity,
    vocabulary_size: u32,
    configuration_declared_scalar_type: Option<ApplicationScalarType>,
    chat_profile: Option<crate::chat::PromptCompatibilityProfile>,
}

impl ResolvedModel {
    pub(crate) const fn new(
        selection: ModelSelection,
        identity: ImmutableModelIdentity,
        vocabulary_size: u32,
        configuration_declared_scalar_type: Option<ApplicationScalarType>,
        chat_profile: Option<crate::chat::PromptCompatibilityProfile>,
    ) -> Self {
        Self {
            selection,
            identity,
            vocabulary_size,
            configuration_declared_scalar_type,
            chat_profile,
        }
    }

    /// Returns the normalized selection represented by this resolution.
    #[must_use]
    pub const fn selection(&self) -> &ModelSelection {
        &self.selection
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

    /// Returns optional scalar metadata declared by immutable model configuration.
    ///
    /// This producer-intent evidence neither describes tensor-header homogeneity nor
    /// determines the scalar type selected later for execution.
    #[must_use]
    pub const fn configuration_declared_scalar_type(&self) -> Option<ApplicationScalarType> {
        self.configuration_declared_scalar_type
    }

    /// Returns explicit prompt-rendering and termination compatibility.
    #[must_use]
    pub const fn chat_compatibility(&self) -> ChatCompatibility {
        if self.chat_profile.is_some() {
            ChatCompatibility::Supported
        } else {
            ChatCompatibility::Unsupported
        }
    }

    pub(crate) const fn prompt_compatibility_profile(
        &self,
    ) -> Option<crate::chat::PromptCompatibilityProfile> {
        self.chat_profile
    }

    /// Returns whether a complete visible selection still addresses this resolution.
    #[must_use]
    pub fn matches_selection(&self, selection: &ModelSelection) -> bool {
        &self.selection == selection
    }
}

/// Stable generation mode supported by the resident application model.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ApplicationGenerationMode {
    /// No normal loaded model is available.
    #[default]
    Unavailable,
    /// Ordinary prompt completion is available without a verified chat profile.
    DirectCompletion,
    /// E1 owns a verified chat profile for the loaded artifact and tokenizer.
    Chat,
}

/// One model generation currently owned by the local inference runtime.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LoadedModel {
    handle: ModelHandle,
    selection: ModelSelection,
    identity: ImmutableModelIdentity,
    device: ApplicationDevice,
    execution_scalar_type: ApplicationScalarType,
    vocabulary_size: u32,
    maximum_context_tokens: u32,
    maximum_prefill_batch: u32,
    generation_mode: ApplicationGenerationMode,
    reserved_footprint: ApplicationMemoryFootprint,
}

impl LoadedModel {
    #[expect(
        clippy::too_many_arguments,
        reason = "the private constructor commits one complete verified load-receipt summary"
    )]
    pub(crate) const fn new(
        handle: ModelHandle,
        selection: ModelSelection,
        identity: ImmutableModelIdentity,
        device: ApplicationDevice,
        execution_scalar_type: ApplicationScalarType,
        vocabulary_size: u32,
        maximum_context_tokens: u32,
        maximum_prefill_batch: u32,
        generation_mode: ApplicationGenerationMode,
        reserved_footprint: ApplicationMemoryFootprint,
    ) -> Self {
        Self {
            handle,
            selection,
            identity,
            device,
            execution_scalar_type,
            vocabulary_size,
            maximum_context_tokens,
            maximum_prefill_batch,
            generation_mode,
            reserved_footprint,
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

    /// Returns the actual execution device verified by E0's load receipt.
    #[must_use]
    pub const fn device(&self) -> ApplicationDevice {
        self.device
    }

    /// Returns the immutable artifact identity loaded by E0.
    #[must_use]
    pub const fn identity(&self) -> &ImmutableModelIdentity {
        &self.identity
    }

    /// Returns the actual execution scalar type verified from E0's load receipt.
    #[must_use]
    pub const fn execution_scalar_type(&self) -> ApplicationScalarType {
        self.execution_scalar_type
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

    /// Returns the E1-owned generation mode for this loaded artifact and tokenizer.
    #[must_use]
    pub const fn generation_mode(&self) -> ApplicationGenerationMode {
        self.generation_mode
    }

    pub(crate) const fn reserved_footprint(&self) -> ApplicationMemoryFootprint {
        self.reserved_footprint
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
    devices: Vec<ApplicationDeviceSummary>,
    selected_device: ApplicationDevice,
    device_discovery_failures: Vec<ApplicationDeviceDiscoveryFailure>,
    accelerator_memory_budget_bytes: u64,
    resolved: Option<ResolvedModel>,
    loaded: Option<LoadedModel>,
    retained_model: Option<ApplicationRetainedModel>,
    generation: Option<GenerationSummary>,
    last_generation: Option<GenerationTerminal>,
    hub_available: bool,
    inference_available: bool,
}

impl Default for ApplicationState {
    fn default() -> Self {
        Self {
            activity: ApplicationActivity::Idle,
            devices: vec![ApplicationDeviceSummary::cpu()],
            selected_device: ApplicationDevice::Cpu,
            device_discovery_failures: Vec::new(),
            accelerator_memory_budget_bytes: 0,
            resolved: None,
            loaded: None,
            retained_model: None,
            generation: None,
            last_generation: None,
            hub_available: true,
            inference_available: true,
        }
    }
}

impl ApplicationState {
    pub(crate) fn with_devices(
        selected_device: ApplicationDevice,
        devices: Vec<ApplicationDeviceSummary>,
        device_discovery_failures: Vec<ApplicationDeviceDiscoveryFailure>,
        accelerator_memory_budget_bytes: u64,
    ) -> Self {
        let mut state = Self::default();
        state.devices.clear();
        for summary in devices {
            state.replace_device_summary(summary, None);
        }
        if !state
            .devices
            .iter()
            .any(|summary| summary.device() == ApplicationDevice::Cpu)
        {
            state.replace_device_summary(ApplicationDeviceSummary::cpu(), None);
        }
        if !state
            .devices
            .iter()
            .any(|summary| summary.device() == selected_device)
        {
            state.replace_device_summary(
                ApplicationDeviceSummary::unavailable(
                    selected_device,
                    ApplicationDeviceUnavailableReason::DiscoveryFailed,
                ),
                None,
            );
        }
        state.selected_device = selected_device;
        state.device_discovery_failures = device_discovery_failures;
        state.accelerator_memory_budget_bytes = accelerator_memory_budget_bytes;
        state
    }

    /// Returns the process catalogue of available devices plus an unavailable selected device.
    #[must_use]
    pub const fn devices(&self) -> &[ApplicationDeviceSummary] {
        self.devices.as_slice()
    }

    /// Returns the explicit device used for the next model load.
    #[must_use]
    pub const fn selected_device(&self) -> ApplicationDevice {
        self.selected_device
    }

    /// Returns the latest summary for the selected device.
    #[must_use]
    pub fn selected_device_summary(&self) -> Option<&ApplicationDeviceSummary> {
        self.devices
            .iter()
            .find(|summary| summary.device() == self.selected_device)
    }

    /// Returns whether the selected device passed its latest bounded probe.
    #[must_use]
    pub fn selected_device_available(&self) -> bool {
        self.selected_device_summary()
            .is_some_and(ApplicationDeviceSummary::available)
    }

    /// Returns why the selected device is unavailable, when known.
    #[must_use]
    pub fn selected_device_unavailable_reason(&self) -> Option<ApplicationDeviceUnavailableReason> {
        self.selected_device_summary()
            .and_then(ApplicationDeviceSummary::unavailable_reason)
    }

    /// Returns structured cold-path failures from the latest bounded probes.
    #[must_use]
    pub const fn device_discovery_failures(&self) -> &[ApplicationDeviceDiscoveryFailure] {
        self.device_discovery_failures.as_slice()
    }

    /// Returns whether explicit device selection may change without violating lifecycle ownership.
    #[must_use]
    pub const fn can_select_device(&self) -> bool {
        matches!(
            self.activity,
            ApplicationActivity::Idle | ApplicationActivity::Resolving
        ) && self.loaded.is_none()
            && self.retained_model.is_none()
            && self.generation.is_none()
    }

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

    /// Returns retained or unconfirmed lower model ownership, when present.
    #[must_use]
    pub const fn retained_model(&self) -> Option<&ApplicationRetainedModel> {
        self.retained_model.as_ref()
    }

    /// Returns the stable generation mode of the normal loaded model.
    #[must_use]
    pub const fn generation_mode(&self) -> ApplicationGenerationMode {
        match self.loaded.as_ref() {
            Some(model) => model.generation_mode(),
            None => ApplicationGenerationMode::Unavailable,
        }
    }

    /// Returns whether E1 coordination for retained cleanup may be explicitly retried.
    #[must_use]
    pub fn can_retry_model_cleanup(&self) -> bool {
        self.activity == ApplicationActivity::RetainedCleanup
            && self.inference_available
            && self.retained_model.as_ref().is_some_and(|model| {
                matches!(
                    model.cleanup(),
                    crate::ApplicationModelCleanupDisposition::CoordinationRetryAvailable { .. }
                )
            })
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
            && self.retained_model.is_none()
            && self.generation.is_none()
    }

    pub(crate) fn selected_device_memory_budget_available(&self) -> bool {
        match self.selected_device {
            ApplicationDevice::Cpu => true,
            ApplicationDevice::Cuda { .. } => {
                self.accelerator_memory_budget_bytes > 0
                    && self
                        .selected_device_summary()
                        .and_then(ApplicationDeviceSummary::total_memory_bytes)
                        .is_some_and(|total| self.accelerator_memory_budget_bytes <= total)
            }
        }
    }

    /// Returns whether a model may be loaded for the complete visible selection.
    #[must_use]
    pub fn can_load(&self, selection: &ModelSelection) -> bool {
        self.activity == ApplicationActivity::Idle
            && self.inference_available
            && self.selected_device_available()
            && self.selected_device_memory_budget_available()
            && self.loaded.is_none()
            && self.retained_model.is_none()
            && self.generation.is_none()
            && self
                .resolved
                .as_ref()
                .is_some_and(|resolved| resolved.matches_selection(selection))
    }

    /// Returns whether completion or compatible chat generation may start against the resident model.
    #[must_use]
    pub const fn can_start_generation(&self) -> bool {
        matches!(self.activity, ApplicationActivity::Idle)
            && self.inference_available
            && self.loaded.is_some()
            && self.retained_model.is_none()
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
            && self.retained_model.is_none()
    }

    pub(crate) fn set_selected_device(&mut self, device: ApplicationDevice) {
        self.selected_device = device;
    }

    pub(crate) fn replace_device_summary(
        &mut self,
        summary: ApplicationDeviceSummary,
        failure: Option<ApplicationDeviceDiscoveryFailure>,
    ) {
        let device = summary.device();
        self.device_discovery_failures
            .retain(|current| current.device() != device);
        if let Some(failure) = failure {
            self.device_discovery_failures.push(failure);
        }
        if let Some(current) = self
            .devices
            .iter_mut()
            .find(|current| current.device() == device)
        {
            *current = summary;
        } else {
            self.devices.push(summary);
            self.devices.sort_by_key(ApplicationDeviceSummary::device);
        }
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
        self.retained_model = None;
        self.activity = ApplicationActivity::Idle;
    }

    pub(crate) fn set_retained_model(&mut self, retained: ApplicationRetainedModel) {
        self.loaded = None;
        self.retained_model = Some(retained);
        if self.activity != ApplicationActivity::ShuttingDown {
            self.activity = ApplicationActivity::RetainedCleanup;
        }
    }

    pub(crate) fn clear_retained_model(&mut self) {
        self.retained_model = None;
        if self.activity == ApplicationActivity::RetainedCleanup {
            self.activity = ApplicationActivity::Idle;
        }
    }

    pub(crate) fn clear_loaded(&mut self) {
        self.loaded = None;
        self.activity = ApplicationActivity::Idle;
    }

    pub(crate) fn clear_normal_runtime_ownership_for_shutdown(&mut self) {
        self.loaded = None;
        self.generation = None;
    }

    pub(crate) fn confirm_runtime_shutdown_released(&mut self) {
        self.clear_normal_runtime_ownership_for_shutdown();
        self.retained_model = None;
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
