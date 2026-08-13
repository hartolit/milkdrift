//! Frontend-neutral application orchestration over bounded host workers.

#[cfg(feature = "cuda-hardware-tests")]
pub(crate) mod cuda_hardware;
mod devices;
mod lifecycle;
mod model;
mod retained_cleanup;
mod startup;

#[cfg(test)]
use domain_contracts::ExecutionDevice;
use domain_contracts::MemoryBudget;
use hf_hub_adapter::ResolvedSafetensorsLlamaArtifacts;
use hf_tokenizer::HfTokenizer;
use host_runtime::{BoundedReceiver, BoundedSender, HostThread};
use redb_storage::RedbStorage;

use self::model::ModelLoadTransaction;
use self::retained_cleanup::ModelCleanupCoordinator;
use crate::conversation::ConversationState;
use crate::generation::GenerationBridge;
use crate::hub_worker::{HubCommand, HubEvent};
use crate::local::{DeviceProbe, LocalInference};
use crate::unload::ModelUnloadTransaction;
use crate::{
    ApplicationPreferences, ApplicationRuntimeConfiguration, ApplicationState, ContextDiagnostics,
    ModelSelection,
};

/// Frontend-neutral owner of model acquisition, persistence, lifecycle, and generation workers.
pub struct ApplicationRuntime {
    pub(crate) local: LocalInference,
    pub(crate) hub_commands: BoundedSender<HubCommand>,
    hub_results: BoundedReceiver<HubEvent>,
    pub(crate) hub_thread: Option<HostThread<()>>,
    storage: RedbStorage,
    preferences: ApplicationPreferences,
    memory_budget: MemoryBudget,
    device_probe: DeviceProbe,
    pub(crate) configuration: ApplicationRuntimeConfiguration,
    pub(crate) state: ApplicationState,
    resolved_artifacts: Option<ResolvedSafetensorsLlamaArtifacts>,
    pending_hub_selection: Option<ModelSelection>,
    pending_load: Option<ModelLoadTransaction>,
    pub(crate) pending_unload: Option<ModelUnloadTransaction>,
    pub(crate) tokenizer: Option<HfTokenizer>,
    pub(crate) generation: GenerationBridge,
    pub(crate) conversation: ConversationState,
    pub(crate) context_diagnostics: Option<ContextDiagnostics>,
    next_ticket: u64,
    pub(crate) shutdown_control: crate::shutdown::ShutdownControl,
    model_cleanup: Option<ModelCleanupCoordinator>,
    #[cfg(test)]
    forced_inference_busy_submissions: usize,
    #[cfg(test)]
    /// Injects an unsent-command disconnect result for transaction rollback tests only.
    /// It deliberately does not simulate the worker-lifecycle side effects tested elsewhere.
    forced_unsent_command_disconnects: usize,
    #[cfg(test)]
    last_submitted_load_device: Option<ExecutionDevice>,
}

#[cfg(test)]
mod tests;
