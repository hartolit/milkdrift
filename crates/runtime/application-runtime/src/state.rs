//! Frontend-neutral application state exposed by the orchestration engine.

use domain_contracts::{
    ByteCount, FinishReason, GenerationUsage, MemoryFootprint, ModelHandle, RequestId,
};

use crate::{
    ApplicationDevice, ApplicationDeviceDiscoveryFailure, ApplicationDeviceSummary,
    ApplicationDeviceUnavailableReason, ApplicationFailure, ApplicationRetainedModel,
    ApplicationScalarType, ChatCompatibility, ImmutableModelIdentity, ModelSelection,
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
    reserved_footprint: MemoryFootprint,
}

/// One complete load publication assembled only after E1 validates the correlated E0 receipt.
pub(crate) struct LoadedModelCommit {
    pub(crate) handle: ModelHandle,
    pub(crate) selection: ModelSelection,
    pub(crate) identity: ImmutableModelIdentity,
    pub(crate) device: ApplicationDevice,
    pub(crate) execution_scalar_type: ApplicationScalarType,
    pub(crate) vocabulary_size: u32,
    pub(crate) maximum_context_tokens: u32,
    pub(crate) maximum_prefill_batch: u32,
    pub(crate) generation_mode: ApplicationGenerationMode,
    pub(crate) reserved_footprint: MemoryFootprint,
}

impl LoadedModel {
    pub(crate) fn commit(validated: LoadedModelCommit) -> Self {
        Self {
            handle: validated.handle,
            selection: validated.selection,
            identity: validated.identity,
            device: validated.device,
            execution_scalar_type: validated.execution_scalar_type,
            vocabulary_size: validated.vocabulary_size,
            maximum_context_tokens: validated.maximum_context_tokens,
            maximum_prefill_batch: validated.maximum_prefill_batch,
            generation_mode: validated.generation_mode,
            reserved_footprint: validated.reserved_footprint,
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

    pub(crate) const fn reserved_footprint(&self) -> MemoryFootprint {
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

#[derive(Clone, Debug, PartialEq, Eq)]
struct LoadedPhase {
    resolved: ResolvedModel,
    model: LoadedModel,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct RetainedPhase {
    resolved: Option<ResolvedModel>,
    model: ApplicationRetainedModel,
    status: RetainedCleanupStatus,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RetainedCleanupStatus {
    Available,
    Releasing,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum ShutdownOwnership {
    Released { resolved: Option<ResolvedModel> },
    Loading(ResolvedModel),
    Loaded(LoadedPhase),
    Unloading(LoadedPhase),
    Retained(RetainedPhase),
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum ModelLifecycle {
    Empty,
    Resolving,
    Resolved(ResolvedModel),
    Loading(ResolvedModel),
    Loaded(LoadedPhase),
    Unloading(LoadedPhase),
    RetainedCleanup(RetainedPhase),
    ShuttingDown(ShutdownOwnership),
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum GenerationLifecycle {
    #[default]
    Idle,
    Active(GenerationSummary),
}

/// Typed rejection from one private application lifecycle transition.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ApplicationTransitionError {
    InvalidPhase {
        operation: &'static str,
        actual: ApplicationActivity,
    },
    StaleGeneration {
        expected: RequestId,
        actual: RequestId,
    },
    InvalidGenerationPhase {
        request_id: RequestId,
        from: GenerationPhase,
        to: GenerationPhase,
    },
}

/// Read-only application state shared by every frontend implementation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ApplicationState {
    lifecycle: ModelLifecycle,
    generation: GenerationLifecycle,
    devices: Vec<ApplicationDeviceSummary>,
    selected_device: ApplicationDevice,
    device_discovery_failures: Vec<ApplicationDeviceDiscoveryFailure>,
    accelerator_memory_budget_bytes: ByteCount,
    last_generation: Option<GenerationTerminal>,
    hub_available: bool,
    inference_available: bool,
}

impl Default for ApplicationState {
    fn default() -> Self {
        Self {
            lifecycle: ModelLifecycle::Empty,
            generation: GenerationLifecycle::Idle,
            devices: vec![ApplicationDeviceSummary::cpu()],
            selected_device: ApplicationDevice::Cpu,
            device_discovery_failures: Vec::new(),
            accelerator_memory_budget_bytes: ByteCount::ZERO,
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
        accelerator_memory_budget_bytes: ByteCount,
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

    /// Returns the discovered execution-device catalogue.
    #[must_use]
    pub const fn devices(&self) -> &[ApplicationDeviceSummary] {
        self.devices.as_slice()
    }

    /// Returns the device selected for the next model load.
    #[must_use]
    pub const fn selected_device(&self) -> ApplicationDevice {
        self.selected_device
    }

    /// Returns discovery evidence for the selected device.
    #[must_use]
    pub fn selected_device_summary(&self) -> Option<&ApplicationDeviceSummary> {
        self.devices
            .iter()
            .find(|summary| summary.device() == self.selected_device)
    }

    /// Returns whether the selected device is currently available.
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

    /// Returns bounded device-discovery failures retained for diagnostics.
    #[must_use]
    pub const fn device_discovery_failures(&self) -> &[ApplicationDeviceDiscoveryFailure] {
        self.device_discovery_failures.as_slice()
    }

    /// Returns whether device selection may change without contradicting model ownership.
    #[must_use]
    pub const fn can_select_device(&self) -> bool {
        matches!(
            self.lifecycle,
            ModelLifecycle::Empty | ModelLifecycle::Resolving | ModelLifecycle::Resolved(_)
        )
    }

    /// Projects the private model lifecycle into the stable frontend activity category.
    #[must_use]
    pub const fn activity(&self) -> ApplicationActivity {
        match &self.lifecycle {
            ModelLifecycle::Empty | ModelLifecycle::Resolved(_) | ModelLifecycle::Loaded(_) => {
                ApplicationActivity::Idle
            }
            ModelLifecycle::Resolving => ApplicationActivity::Resolving,
            ModelLifecycle::Loading(_) => ApplicationActivity::Loading,
            ModelLifecycle::Unloading(_) => ApplicationActivity::Unloading,
            ModelLifecycle::RetainedCleanup(_) => ApplicationActivity::RetainedCleanup,
            ModelLifecycle::ShuttingDown(_) => ApplicationActivity::ShuttingDown,
        }
    }

    /// Returns the immutable resolved model carried by the current lifecycle phase.
    #[must_use]
    pub const fn resolved(&self) -> Option<&ResolvedModel> {
        match &self.lifecycle {
            ModelLifecycle::Resolved(resolved) | ModelLifecycle::Loading(resolved) => {
                Some(resolved)
            }
            ModelLifecycle::Loaded(phase) | ModelLifecycle::Unloading(phase) => {
                Some(&phase.resolved)
            }
            ModelLifecycle::RetainedCleanup(phase) => phase.resolved.as_ref(),
            ModelLifecycle::ShuttingDown(ownership) => shutdown_resolved_ref(ownership),
            ModelLifecycle::Empty | ModelLifecycle::Resolving => None,
        }
    }

    /// Returns the verified loaded model carried by the current lifecycle phase.
    #[must_use]
    pub const fn loaded(&self) -> Option<&LoadedModel> {
        match &self.lifecycle {
            ModelLifecycle::Loaded(phase) | ModelLifecycle::Unloading(phase) => Some(&phase.model),
            ModelLifecycle::ShuttingDown(
                ShutdownOwnership::Loaded(phase) | ShutdownOwnership::Unloading(phase),
            ) => Some(&phase.model),
            _ => None,
        }
    }

    /// Returns fail-closed retained model ownership when cleanup remains unresolved.
    #[must_use]
    pub const fn retained_model(&self) -> Option<&ApplicationRetainedModel> {
        match &self.lifecycle {
            ModelLifecycle::RetainedCleanup(phase)
            | ModelLifecycle::ShuttingDown(ShutdownOwnership::Retained(phase)) => {
                Some(&phase.model)
            }
            _ => None,
        }
    }

    /// Returns the generation mode derived from the current verified loaded model.
    #[must_use]
    pub const fn generation_mode(&self) -> ApplicationGenerationMode {
        match self.loaded() {
            Some(model) => model.generation_mode(),
            None => ApplicationGenerationMode::Unavailable,
        }
    }

    /// Returns whether the current retained model permits a fresh coordination round.
    #[must_use]
    pub fn can_retry_model_cleanup(&self) -> bool {
        self.inference_available
            && matches!(&self.lifecycle, ModelLifecycle::RetainedCleanup(phase) if phase.status == RetainedCleanupStatus::Available && matches!(phase.model.cleanup(), crate::ApplicationModelCleanupDisposition::CoordinationRetryAvailable { .. }))
    }

    /// Returns the independently active generation, if one exists.
    #[must_use]
    pub const fn active_generation(&self) -> Option<GenerationSummary> {
        match self.generation {
            GenerationLifecycle::Idle => None,
            GenerationLifecycle::Active(summary) => Some(summary),
        }
    }

    /// Returns the intentionally retained last terminal generation summary.
    #[must_use]
    pub const fn last_generation(&self) -> Option<&GenerationTerminal> {
        self.last_generation.as_ref()
    }

    /// Returns whether the artifact-resolution service can accept work.
    #[must_use]
    pub const fn hub_available(&self) -> bool {
        self.hub_available
    }

    /// Returns whether the inference service can accept work.
    #[must_use]
    pub const fn inference_available(&self) -> bool {
        self.inference_available
    }

    /// Returns whether immutable artifact resolution may begin.
    #[must_use]
    pub const fn can_resolve(&self, _selection: &ModelSelection) -> bool {
        self.hub_available
            && matches!(
                self.lifecycle,
                ModelLifecycle::Empty | ModelLifecycle::Resolved(_)
            )
    }

    pub(crate) fn selected_device_memory_budget_available(&self) -> bool {
        match self.selected_device {
            ApplicationDevice::Cpu => true,
            ApplicationDevice::Cuda { .. } => {
                !self.accelerator_memory_budget_bytes.is_zero()
                    && self
                        .selected_device_summary()
                        .and_then(ApplicationDeviceSummary::total_memory_bytes)
                        .is_some_and(|total| total.contains(self.accelerator_memory_budget_bytes))
            }
        }
    }

    /// Returns whether the exact resolved selection may be loaded now.
    #[must_use]
    pub fn can_load(&self, selection: &ModelSelection) -> bool {
        self.inference_available
            && self.selected_device_available()
            && self.selected_device_memory_budget_available()
            && matches!(&self.lifecycle, ModelLifecycle::Resolved(resolved) if resolved.matches_selection(selection))
    }

    /// Returns whether a generation may be admitted against the resident model.
    #[must_use]
    pub const fn can_start_generation(&self) -> bool {
        self.inference_available
            && matches!(self.lifecycle, ModelLifecycle::Loaded(_))
            && matches!(self.generation, GenerationLifecycle::Idle)
    }

    /// Returns whether the active generation is at a cancellable boundary.
    #[must_use]
    pub const fn can_cancel_generation(&self) -> bool {
        matches!(
            self.active_generation(),
            Some(GenerationSummary {
                phase: GenerationPhase::Starting | GenerationPhase::Running,
                ..
            })
        )
    }

    /// Returns whether deterministic model unload may be requested.
    #[must_use]
    pub const fn can_unload(&self) -> bool {
        self.inference_available && matches!(self.lifecycle, ModelLifecycle::Loaded(_))
    }

    /// Returns whether a model-lifecycle operation is in progress.
    #[must_use]
    pub const fn is_busy(&self) -> bool {
        !matches!(self.activity(), ApplicationActivity::Idle)
    }

    /// Returns whether the visible model selection may be edited.
    #[must_use]
    pub const fn can_edit_model_selection(&self) -> bool {
        matches!(
            self.lifecycle,
            ModelLifecycle::Empty | ModelLifecycle::Resolved(_)
        )
    }

    /// Returns whether conversation history can be cleared safely.
    #[must_use]
    pub const fn can_clear_conversation(&self) -> bool {
        self.active_generation().is_none()
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

    pub(crate) fn begin_resolving(&mut self) -> Result<(), ApplicationTransitionError> {
        self.validate_begin_resolving()?;
        self.lifecycle = ModelLifecycle::Resolving;
        Ok(())
    }

    pub(crate) fn validate_begin_resolving(&self) -> Result<(), ApplicationTransitionError> {
        if matches!(
            self.lifecycle,
            ModelLifecycle::Empty | ModelLifecycle::Resolved(_)
        ) {
            Ok(())
        } else {
            Err(self.invalid_transition("begin resolution"))
        }
    }

    pub(crate) fn complete_resolution(
        &mut self,
        resolved: ResolvedModel,
    ) -> Result<(), ApplicationTransitionError> {
        if !matches!(self.lifecycle, ModelLifecycle::Resolving) {
            return Err(self.invalid_transition("complete resolution"));
        }
        self.lifecycle = ModelLifecycle::Resolved(resolved);
        Ok(())
    }

    pub(crate) fn fail_resolution(&mut self) -> Result<(), ApplicationTransitionError> {
        if !matches!(self.lifecycle, ModelLifecycle::Resolving) {
            return Err(self.invalid_transition("fail resolution"));
        }
        self.lifecycle = ModelLifecycle::Empty;
        Ok(())
    }

    pub(crate) fn begin_loading(&mut self) -> Result<(), ApplicationTransitionError> {
        self.validate_begin_loading()?;
        let previous = std::mem::replace(&mut self.lifecycle, ModelLifecycle::Empty);
        match previous {
            ModelLifecycle::Resolved(resolved) => {
                self.lifecycle = ModelLifecycle::Loading(resolved);
                Ok(())
            }
            other => self.restore_invalid(other, "begin load"),
        }
    }

    pub(crate) fn validate_begin_loading(&self) -> Result<(), ApplicationTransitionError> {
        if matches!(self.lifecycle, ModelLifecycle::Resolved(_)) {
            Ok(())
        } else {
            Err(self.invalid_transition("begin load"))
        }
    }

    pub(crate) fn fail_loading(&mut self) -> Result<(), ApplicationTransitionError> {
        let previous = std::mem::replace(&mut self.lifecycle, ModelLifecycle::Empty);
        match previous {
            ModelLifecycle::Loading(resolved) => {
                self.lifecycle = ModelLifecycle::Resolved(resolved);
                Ok(())
            }
            other => self.restore_invalid(other, "fail load"),
        }
    }

    pub(crate) fn complete_loading(
        &mut self,
        model: LoadedModel,
    ) -> Result<(), ApplicationTransitionError> {
        let previous = std::mem::replace(&mut self.lifecycle, ModelLifecycle::Empty);
        match previous {
            ModelLifecycle::Loading(resolved) => {
                self.lifecycle = ModelLifecycle::Loaded(LoadedPhase { resolved, model });
                Ok(())
            }
            other => self.restore_invalid(other, "complete load"),
        }
    }

    pub(crate) fn begin_unloading(&mut self) -> Result<(), ApplicationTransitionError> {
        self.validate_begin_unloading()?;
        let previous = std::mem::replace(&mut self.lifecycle, ModelLifecycle::Empty);
        match previous {
            ModelLifecycle::Loaded(phase) => {
                self.lifecycle = ModelLifecycle::Unloading(phase);
                Ok(())
            }
            ModelLifecycle::RetainedCleanup(mut phase)
                if phase.status == RetainedCleanupStatus::Available =>
            {
                phase.status = RetainedCleanupStatus::Releasing;
                self.lifecycle = ModelLifecycle::RetainedCleanup(phase);
                Ok(())
            }
            other => self.restore_invalid(other, "begin unload"),
        }
    }

    pub(crate) fn validate_begin_unloading(&self) -> Result<(), ApplicationTransitionError> {
        if matches!(
            &self.lifecycle,
            ModelLifecycle::Loaded(_)
                | ModelLifecycle::RetainedCleanup(RetainedPhase {
                    status: RetainedCleanupStatus::Available,
                    ..
                })
        ) {
            Ok(())
        } else {
            Err(self.invalid_transition("begin unload"))
        }
    }

    pub(crate) fn fail_unloading(&mut self) -> Result<(), ApplicationTransitionError> {
        let previous = std::mem::replace(&mut self.lifecycle, ModelLifecycle::Empty);
        match previous {
            ModelLifecycle::Unloading(phase) => {
                self.lifecycle = ModelLifecycle::Loaded(phase);
                Ok(())
            }
            ModelLifecycle::RetainedCleanup(mut phase)
                if phase.status == RetainedCleanupStatus::Releasing =>
            {
                phase.status = RetainedCleanupStatus::Available;
                self.lifecycle = ModelLifecycle::RetainedCleanup(phase);
                Ok(())
            }
            other => self.restore_invalid(other, "fail unload"),
        }
    }

    pub(crate) fn complete_model_release(&mut self) -> Result<(), ApplicationTransitionError> {
        let previous = std::mem::replace(&mut self.lifecycle, ModelLifecycle::Empty);
        match previous {
            ModelLifecycle::Unloading(phase) => {
                self.lifecycle = ModelLifecycle::Resolved(phase.resolved);
                Ok(())
            }
            ModelLifecycle::RetainedCleanup(phase) => {
                self.lifecycle = phase
                    .resolved
                    .map_or(ModelLifecycle::Empty, ModelLifecycle::Resolved);
                Ok(())
            }
            ModelLifecycle::ShuttingDown(ownership) => {
                self.lifecycle = ModelLifecycle::ShuttingDown(ShutdownOwnership::Released {
                    resolved: shutdown_resolved(ownership),
                });
                Ok(())
            }
            other => self.restore_invalid(other, "complete model release"),
        }
    }

    pub(crate) fn retain_model(&mut self, model: ApplicationRetainedModel) {
        let previous = std::mem::replace(&mut self.lifecycle, ModelLifecycle::Empty);
        self.lifecycle = match previous {
            ModelLifecycle::ShuttingDown(ownership) => {
                ModelLifecycle::ShuttingDown(ShutdownOwnership::Retained(RetainedPhase {
                    resolved: shutdown_resolved(ownership),
                    model,
                    status: RetainedCleanupStatus::Available,
                }))
            }
            ModelLifecycle::RetainedCleanup(phase) => {
                ModelLifecycle::RetainedCleanup(RetainedPhase {
                    resolved: phase.resolved,
                    model,
                    status: phase.status,
                })
            }
            other => ModelLifecycle::RetainedCleanup(RetainedPhase {
                resolved: lifecycle_resolved(other),
                model,
                status: RetainedCleanupStatus::Available,
            }),
        };
    }

    pub(crate) fn begin_shutdown(&mut self) -> Result<(), ApplicationTransitionError> {
        if matches!(self.lifecycle, ModelLifecycle::ShuttingDown(_)) {
            return Err(self.invalid_transition("begin shutdown"));
        }
        let previous = std::mem::replace(&mut self.lifecycle, ModelLifecycle::Empty);
        let ownership = match previous {
            ModelLifecycle::Empty | ModelLifecycle::Resolving => {
                ShutdownOwnership::Released { resolved: None }
            }
            ModelLifecycle::Resolved(resolved) => ShutdownOwnership::Released {
                resolved: Some(resolved),
            },
            ModelLifecycle::Loading(resolved) => ShutdownOwnership::Loading(resolved),
            ModelLifecycle::Loaded(phase) => ShutdownOwnership::Loaded(phase),
            ModelLifecycle::Unloading(phase) => ShutdownOwnership::Unloading(phase),
            ModelLifecycle::RetainedCleanup(phase) => ShutdownOwnership::Retained(phase),
            ModelLifecycle::ShuttingDown(ownership) => {
                self.lifecycle = ModelLifecycle::ShuttingDown(ownership);
                return Err(self.invalid_transition("begin shutdown"));
            }
        };
        self.lifecycle = ModelLifecycle::ShuttingDown(ownership);
        Ok(())
    }

    pub(crate) fn clear_normal_runtime_ownership_for_shutdown(
        &mut self,
    ) -> Result<(), ApplicationTransitionError> {
        let previous = std::mem::replace(&mut self.lifecycle, ModelLifecycle::Empty);
        match previous {
            ModelLifecycle::ShuttingDown(ownership) => {
                self.lifecycle = ModelLifecycle::ShuttingDown(ShutdownOwnership::Released {
                    resolved: shutdown_resolved(ownership),
                });
                Ok(())
            }
            other => self.restore_invalid(other, "clear shutdown ownership"),
        }
    }

    pub(crate) fn confirm_runtime_shutdown_released(
        &mut self,
    ) -> Result<(), ApplicationTransitionError> {
        self.clear_normal_runtime_ownership_for_shutdown()
    }

    pub(crate) fn begin_generation(
        &mut self,
        summary: GenerationSummary,
    ) -> Result<(), ApplicationTransitionError> {
        if !matches!(self.lifecycle, ModelLifecycle::Loaded(_))
            || !matches!(self.generation, GenerationLifecycle::Idle)
        {
            let actual = self.activity();
            return Err(ApplicationTransitionError::InvalidPhase {
                operation: "begin generation",
                actual,
            });
        }
        self.generation = GenerationLifecycle::Active(summary);
        self.last_generation = None;
        Ok(())
    }

    pub(crate) fn transition_generation(
        &mut self,
        request_id: RequestId,
        next: GenerationPhase,
    ) -> Result<(), ApplicationTransitionError> {
        self.validate_generation_transition(request_id, next)?;
        let GenerationLifecycle::Active(summary) = &mut self.generation else {
            return Err(self.invalid_transition("advance generation"));
        };
        summary.phase = next;
        Ok(())
    }

    pub(crate) fn validate_generation_transition(
        &self,
        request_id: RequestId,
        next: GenerationPhase,
    ) -> Result<(), ApplicationTransitionError> {
        let summary = self.validate_generation_identity(request_id, "advance generation")?;
        if valid_generation_transition(summary.phase, next) {
            Ok(())
        } else {
            Err(ApplicationTransitionError::InvalidGenerationPhase {
                request_id,
                from: summary.phase,
                to: next,
            })
        }
    }

    pub(crate) fn validate_generation_identity(
        &self,
        request_id: RequestId,
        operation: &'static str,
    ) -> Result<GenerationSummary, ApplicationTransitionError> {
        let Some(summary) = self.active_generation() else {
            return Err(ApplicationTransitionError::InvalidPhase {
                operation,
                actual: self.activity(),
            });
        };
        if summary.request_id != request_id {
            return Err(ApplicationTransitionError::StaleGeneration {
                expected: summary.request_id,
                actual: request_id,
            });
        }
        Ok(summary)
    }

    pub(crate) fn abort_generation_start(
        &mut self,
        request_id: RequestId,
    ) -> Result<(), ApplicationTransitionError> {
        let actual = self.activity();
        let Some(summary) = self.active_generation() else {
            return Err(ApplicationTransitionError::InvalidPhase {
                operation: "abort generation submission",
                actual,
            });
        };
        if summary.request_id != request_id {
            return Err(ApplicationTransitionError::StaleGeneration {
                expected: summary.request_id,
                actual: request_id,
            });
        }
        if summary.phase != GenerationPhase::Starting {
            return Err(ApplicationTransitionError::InvalidGenerationPhase {
                request_id,
                from: summary.phase,
                to: GenerationPhase::Starting,
            });
        }
        self.generation = GenerationLifecycle::Idle;
        Ok(())
    }

    pub(crate) fn increment_generated_tokens(
        &mut self,
        request_id: RequestId,
    ) -> Result<(), ApplicationTransitionError> {
        let actual = self.activity();
        let GenerationLifecycle::Active(summary) = &mut self.generation else {
            return Err(ApplicationTransitionError::InvalidPhase {
                operation: "record generated token",
                actual,
            });
        };
        if summary.request_id != request_id {
            return Err(ApplicationTransitionError::StaleGeneration {
                expected: summary.request_id,
                actual: request_id,
            });
        }
        summary.usage.generated_tokens = summary.usage.generated_tokens.saturating_add(1);
        Ok(())
    }

    pub(crate) fn finish_generation(
        &mut self,
        terminal: GenerationTerminal,
    ) -> Result<(), ApplicationTransitionError> {
        let actual = self.activity();
        let Some(summary) = self.active_generation() else {
            return Err(ApplicationTransitionError::InvalidPhase {
                operation: "finish generation",
                actual,
            });
        };
        if summary.request_id != terminal.request_id {
            return Err(ApplicationTransitionError::StaleGeneration {
                expected: summary.request_id,
                actual: terminal.request_id,
            });
        }
        self.generation = GenerationLifecycle::Idle;
        self.last_generation = Some(terminal);
        Ok(())
    }

    pub(crate) const fn disconnect_hub(&mut self) {
        self.hub_available = false;
    }

    pub(crate) const fn disconnect_inference(&mut self) {
        self.inference_available = false;
    }

    fn invalid_transition(&self, operation: &'static str) -> ApplicationTransitionError {
        ApplicationTransitionError::InvalidPhase {
            operation,
            actual: self.activity(),
        }
    }

    fn restore_invalid(
        &mut self,
        previous: ModelLifecycle,
        operation: &'static str,
    ) -> Result<(), ApplicationTransitionError> {
        self.lifecycle = previous;
        Err(self.invalid_transition(operation))
    }
}

fn lifecycle_resolved(lifecycle: ModelLifecycle) -> Option<ResolvedModel> {
    match lifecycle {
        ModelLifecycle::Resolved(resolved) | ModelLifecycle::Loading(resolved) => Some(resolved),
        ModelLifecycle::Loaded(phase) | ModelLifecycle::Unloading(phase) => Some(phase.resolved),
        ModelLifecycle::RetainedCleanup(phase) => phase.resolved,
        ModelLifecycle::ShuttingDown(ownership) => shutdown_resolved(ownership),
        ModelLifecycle::Empty | ModelLifecycle::Resolving => None,
    }
}

const fn shutdown_resolved_ref(ownership: &ShutdownOwnership) -> Option<&ResolvedModel> {
    match ownership {
        ShutdownOwnership::Released { resolved } => resolved.as_ref(),
        ShutdownOwnership::Loading(resolved) => Some(resolved),
        ShutdownOwnership::Loaded(phase) | ShutdownOwnership::Unloading(phase) => {
            Some(&phase.resolved)
        }
        ShutdownOwnership::Retained(phase) => phase.resolved.as_ref(),
    }
}

fn shutdown_resolved(ownership: ShutdownOwnership) -> Option<ResolvedModel> {
    match ownership {
        ShutdownOwnership::Released { resolved } => resolved,
        ShutdownOwnership::Loading(resolved) => Some(resolved),
        ShutdownOwnership::Loaded(phase) | ShutdownOwnership::Unloading(phase) => {
            Some(phase.resolved)
        }
        ShutdownOwnership::Retained(phase) => phase.resolved,
    }
}

const fn valid_generation_transition(from: GenerationPhase, to: GenerationPhase) -> bool {
    matches!(
        (from, to),
        (
            GenerationPhase::Starting,
            GenerationPhase::Running
                | GenerationPhase::Cancelling
                | GenerationPhase::Finishing
                | GenerationPhase::CleanupPending
                | GenerationPhase::CleanupExhausted
        ) | (
            GenerationPhase::Running,
            GenerationPhase::Cancelling
                | GenerationPhase::Finishing
                | GenerationPhase::CleanupPending
                | GenerationPhase::CleanupExhausted
        ) | (
            GenerationPhase::Cancelling,
            GenerationPhase::Running
                | GenerationPhase::Finishing
                | GenerationPhase::CleanupPending
                | GenerationPhase::CleanupExhausted
        ) | (
            GenerationPhase::Finishing,
            GenerationPhase::CleanupPending | GenerationPhase::CleanupExhausted
        ) | (
            GenerationPhase::CleanupPending,
            GenerationPhase::CleanupExhausted
        )
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        ApplicationModelCleanupDisposition, ApplicationRetainedModelResource,
        ApplicationRetainedOwnership,
    };
    use domain_contracts::{GenerationUsage, ModelGeneration, ModelId};

    fn resolved_model() -> ResolvedModel {
        ResolvedModel::new(
            ModelSelection::new("owner/model", "main"),
            ImmutableModelIdentity::new("owner/model", "012345"),
            32,
            None,
            None,
        )
    }

    fn loaded_model() -> LoadedModel {
        LoadedModel::commit(LoadedModelCommit {
            handle: ModelHandle::new(ModelId::new(1), ModelGeneration::new(1)),
            selection: ModelSelection::new("owner/model", "main"),
            identity: ImmutableModelIdentity::new("owner/model", "012345"),
            device: ApplicationDevice::Cpu,
            execution_scalar_type: ApplicationScalarType::F32,
            vocabulary_size: 32,
            maximum_context_tokens: 128,
            maximum_prefill_batch: 64,
            generation_mode: ApplicationGenerationMode::DirectCompletion,
            reserved_footprint: MemoryFootprint::default(),
        })
    }

    fn loaded_state() -> ApplicationState {
        let mut state = ApplicationState::default();
        assert_eq!(state.begin_resolving(), Ok(()));
        assert_eq!(state.complete_resolution(resolved_model()), Ok(()));
        assert_eq!(state.begin_loading(), Ok(()));
        assert_eq!(state.complete_loading(loaded_model()), Ok(()));
        state
    }

    fn active_summary(request_id: RequestId) -> GenerationSummary {
        GenerationSummary {
            request_id,
            phase: GenerationPhase::Starting,
            usage: GenerationUsage {
                prompt_tokens: 4,
                generated_tokens: 0,
            },
        }
    }

    #[test]
    fn invalid_model_transitions_are_typed_and_leave_state_unchanged() {
        let mut state = ApplicationState::default();
        for rejected in [
            state.begin_loading(),
            state.fail_loading(),
            state.begin_unloading(),
            state.complete_model_release(),
        ] {
            assert!(matches!(
                rejected,
                Err(ApplicationTransitionError::InvalidPhase { .. })
            ));
            assert_eq!(state, ApplicationState::default());
        }
    }

    #[test]
    fn model_transition_matrix_derives_capabilities_from_each_phase() {
        let selection = ModelSelection::new("owner/model", "main");
        let mut state = ApplicationState::default();
        assert!(state.can_resolve(&selection));
        assert!(!state.can_load(&selection));

        assert_eq!(state.begin_resolving(), Ok(()));
        assert_eq!(state.activity(), ApplicationActivity::Resolving);
        assert!(!state.can_resolve(&selection));
        assert_eq!(state.fail_resolution(), Ok(()));
        assert_eq!(state, ApplicationState::default());

        assert_eq!(state.begin_resolving(), Ok(()));
        assert_eq!(state.complete_resolution(resolved_model()), Ok(()));
        assert!(state.can_load(&selection));
        assert_eq!(state.begin_loading(), Ok(()));
        assert_eq!(state.activity(), ApplicationActivity::Loading);
        assert_eq!(state.fail_loading(), Ok(()));
        assert!(state.can_load(&selection));

        assert_eq!(state.begin_loading(), Ok(()));
        assert_eq!(state.complete_loading(loaded_model()), Ok(()));
        assert!(state.can_start_generation());
        assert!(state.can_unload());
        assert_eq!(
            state.generation_mode(),
            ApplicationGenerationMode::DirectCompletion
        );

        assert_eq!(state.begin_unloading(), Ok(()));
        assert_eq!(state.activity(), ApplicationActivity::Unloading);
        assert_eq!(state.complete_model_release(), Ok(()));
        assert!(state.loaded().is_none());
        assert!(state.can_load(&selection));
    }

    #[test]
    fn stale_generation_identity_and_invalid_phase_do_not_mutate_state() {
        let request_id = RequestId::new(11);
        let mut state = loaded_state();
        assert_eq!(state.begin_generation(active_summary(request_id)), Ok(()));

        let before = state.clone();
        assert!(matches!(
            state.transition_generation(RequestId::new(12), GenerationPhase::Running),
            Err(ApplicationTransitionError::StaleGeneration { .. })
        ));
        assert_eq!(state, before);

        assert!(matches!(
            state.transition_generation(request_id, GenerationPhase::CleanupExhausted),
            Ok(())
        ));
        let before = state.clone();
        assert!(matches!(
            state.transition_generation(request_id, GenerationPhase::Running),
            Err(ApplicationTransitionError::InvalidGenerationPhase { .. })
        ));
        assert_eq!(state, before);
    }

    #[test]
    fn generation_ownership_remains_correlated_across_unload_and_retention() {
        let request_id = RequestId::new(21);
        let mut state = loaded_state();
        assert_eq!(state.begin_generation(active_summary(request_id)), Ok(()));
        assert_eq!(state.begin_unloading(), Ok(()));
        assert_eq!(state.complete_model_release(), Ok(()));
        assert_eq!(
            state.active_generation().map(|active| active.request_id),
            Some(request_id)
        );
        assert!(state.loaded().is_none());

        let terminal = GenerationTerminal {
            request_id,
            outcome: GenerationTerminalOutcome::Finished(FinishReason::TokenLimit),
            usage: active_summary(request_id).usage,
        };
        assert_eq!(state.finish_generation(terminal.clone()), Ok(()));
        assert!(state.active_generation().is_none());
        assert_eq!(state.last_generation(), Some(&terminal));
    }

    #[test]
    fn retained_cleanup_and_shutdown_preserve_independent_generation_identity() {
        let request_id = RequestId::new(31);
        let mut state = loaded_state();
        assert_eq!(state.begin_generation(active_summary(request_id)), Ok(()));
        let retained = ApplicationRetainedModel::new(
            ApplicationRetainedModelResource::LoadedModel {
                handle: loaded_model().handle(),
            },
            ApplicationRetainedOwnership::Exact(MemoryFootprint::default()),
            ApplicationModelCleanupDisposition::Pending,
            ApplicationFailure::new(
                crate::ApplicationFailureKind::RetainedCleanup,
                "cleanup pending",
            ),
            None,
        );
        state.retain_model(retained);
        assert_eq!(state.activity(), ApplicationActivity::RetainedCleanup);
        assert_eq!(
            state.active_generation().map(|active| active.request_id),
            Some(request_id)
        );

        assert_eq!(state.begin_shutdown(), Ok(()));
        assert_eq!(state.activity(), ApplicationActivity::ShuttingDown);
        assert_eq!(
            state.active_generation().map(|active| active.request_id),
            Some(request_id)
        );
        let before = state.clone();
        assert!(matches!(
            state.begin_shutdown(),
            Err(ApplicationTransitionError::InvalidPhase { .. })
        ));
        assert_eq!(state, before);
    }
}
