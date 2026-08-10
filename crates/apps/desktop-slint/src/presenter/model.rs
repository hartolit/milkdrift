use application_runtime::{
    ApplicationDevice, ApplicationGenerationMode, ApplicationModelCleanupDisposition,
    ApplicationRetainedModel, ApplicationRetainedModelResource, ApplicationRetainedOwnership,
    ApplicationRuntime, ApplicationScalarType, ChatCompatibility, LoadedModel, ModelSelection,
    ResolvedModel,
};

use crate::AppWindow;

pub(super) fn map_model_selection(repository: &str, revision: &str) -> ModelSelection {
    ModelSelection::new(repository, revision)
}

pub(super) fn selected_model(window: &AppWindow) -> ModelSelection {
    map_model_selection(
        window.get_repository().as_str(),
        window.get_revision().as_str(),
    )
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ComposerMode {
    Unavailable,
    Chat,
    DirectCompletion,
}

impl ComposerMode {
    pub(super) const fn label(self) -> &'static str {
        match self {
            Self::Unavailable => "Unavailable",
            Self::Chat => "Chat",
            Self::DirectCompletion => "Direct completion",
        }
    }

    pub(super) const fn guidance(self, retained_model: bool) -> &'static str {
        match self {
            Self::Unavailable if retained_model => {
                "Generation is unavailable while retained model resources await release."
            }
            Self::Unavailable => "Load a model to enable generation.",
            Self::Chat => {
                "Verified chat compatibility: E1 owns conversation history, prompt rendering, and regeneration."
            }
            Self::DirectCompletion => {
                "Chat compatibility is unsupported: input is submitted as an E1 direct-completion prompt without inferred history or template semantics."
            }
        }
    }

    pub(super) const fn input_label(self) -> &'static str {
        match self {
            Self::Unavailable | Self::Chat => "Message",
            Self::DirectCompletion => "Prompt",
        }
    }

    pub(super) const fn submit_label(self) -> &'static str {
        match self {
            Self::Unavailable | Self::Chat => "Send",
            Self::DirectCompletion => "Complete",
        }
    }
}

pub(super) const fn composer_mode_from_generation_mode(
    mode: ApplicationGenerationMode,
) -> ComposerMode {
    match mode {
        ApplicationGenerationMode::Unavailable => ComposerMode::Unavailable,
        ApplicationGenerationMode::DirectCompletion => ComposerMode::DirectCompletion,
        ApplicationGenerationMode::Chat => ComposerMode::Chat,
    }
}

pub(super) fn composer_mode(runtime: &ApplicationRuntime) -> ComposerMode {
    composer_mode_from_generation_mode(runtime.state().generation_mode())
}

pub(super) fn selected_model_summary(
    selection: &ModelSelection,
    selected_device: ApplicationDevice,
    selected_device_available: bool,
) -> String {
    format!(
        "Repository: {} • Revision: {} • Selected device: {} • Availability: {}",
        selection.repository(),
        selection.revision(),
        device_label(selected_device),
        device_availability_label(selected_device_available),
    )
}

pub(super) const UNRESOLVED_MODEL_SUMMARY: &str = "No resolved model facts.";
pub(super) const UNLOADED_MODEL_SUMMARY: &str = "No loaded or retained model resources.";

pub(super) fn resolved_model_summary(model: &ResolvedModel) -> String {
    resolved_model_facts_summary(
        model.configuration_declared_scalar_type(),
        model.chat_compatibility(),
    )
}

pub(super) fn resolved_model_facts_summary(
    configuration_declared_scalar_type: Option<ApplicationScalarType>,
    chat_compatibility: ChatCompatibility,
) -> String {
    let compatibility = chat_compatibility_label(chat_compatibility);
    configuration_declared_scalar_type.map_or_else(
        || format!("Resolved • Chat compatibility: {compatibility}"),
        |scalar_type| {
            format!(
                "Resolved • Recognized configuration scalar declaration: {} • Chat compatibility: {compatibility}",
                scalar_type_label(scalar_type)
            )
        },
    )
}

pub(super) fn loaded_model_summary(model: &LoadedModel) -> String {
    loaded_model_facts_summary(model.execution_scalar_type(), model.device())
}

pub(super) fn loaded_model_facts_summary(
    execution_scalar_type: ApplicationScalarType,
    execution_device: ApplicationDevice,
) -> String {
    format!(
        "Execution device: {} • Execution scalar: {}",
        device_label(execution_device),
        scalar_type_label(execution_scalar_type),
    )
}

pub(super) fn model_residency_summary(
    loaded: Option<&LoadedModel>,
    retained: Option<&ApplicationRetainedModel>,
) -> String {
    let loaded = loaded.map(loaded_model_summary);
    let retained = retained.map(retained_model_summary);
    model_residency_facts_summary(loaded.as_deref(), retained.as_deref())
}

pub(super) fn model_residency_facts_summary(
    loaded: Option<&str>,
    retained: Option<&str>,
) -> String {
    match (loaded, retained) {
        (Some(loaded), Some(retained)) => format!("{loaded} • {retained}"),
        (Some(loaded), None) => loaded.to_owned(),
        (_, Some(retained)) => retained.to_owned(),
        (None, None) => UNLOADED_MODEL_SUMMARY.to_owned(),
    }
}

pub(super) fn retained_model_summary(model: &ApplicationRetainedModel) -> String {
    retained_model_facts_summary(model.resource(), model.ownership(), model.cleanup())
}

pub(super) fn retained_model_facts_summary(
    resource: ApplicationRetainedModelResource,
    ownership: ApplicationRetainedOwnership,
    cleanup: ApplicationModelCleanupDisposition,
) -> String {
    format!(
        "Retained {} • Ownership certainty: {} • Cleanup disposition: {}",
        retained_model_resource_label(resource),
        retained_ownership_label(ownership),
        model_cleanup_disposition_label(cleanup),
    )
}

pub(super) const fn retained_model_resource_label(
    resource: ApplicationRetainedModelResource,
) -> &'static str {
    match resource {
        ApplicationRetainedModelResource::FailedLoad { .. } => "failed-load resources",
        ApplicationRetainedModelResource::LoadedModel { .. } => "loaded-model resources",
        ApplicationRetainedModelResource::IncompatibleModel { .. } => {
            "incompatible-model resources"
        }
        ApplicationRetainedModelResource::UnconfirmedLoad => "unconfirmed-load resources",
        ApplicationRetainedModelResource::UnconfirmedModel => "unconfirmed-model resources",
    }
}

const fn retained_ownership_label(ownership: ApplicationRetainedOwnership) -> &'static str {
    match ownership {
        ApplicationRetainedOwnership::Exact(_) => "exact",
        ApplicationRetainedOwnership::Unverified { .. } => "unverified",
        ApplicationRetainedOwnership::Unknown => "unknown",
    }
}

fn model_cleanup_disposition_label(cleanup: ApplicationModelCleanupDisposition) -> String {
    match cleanup {
        ApplicationModelCleanupDisposition::Pending => "pending".to_owned(),
        ApplicationModelCleanupDisposition::LowerRetryable {
            attempts,
            maximum_attempts,
        } => format!("lower cleanup retryable ({attempts}/{maximum_attempts} attempts)"),
        ApplicationModelCleanupDisposition::LowerExhausted {
            attempts,
            maximum_attempts,
        } => format!("lower cleanup exhausted ({attempts}/{maximum_attempts} attempts)"),
        ApplicationModelCleanupDisposition::CoordinationRetryAvailable {
            attempts,
            maximum_attempts,
        } => format!(
            "application coordination retry available ({attempts}/{maximum_attempts} attempts)"
        ),
        ApplicationModelCleanupDisposition::WorkerDisconnected => {
            "worker disconnected without confirmed release".to_owned()
        }
        ApplicationModelCleanupDisposition::RetainedUntilProcessExit => {
            "retained until process exit".to_owned()
        }
    }
}

const fn chat_compatibility_label(compatibility: ChatCompatibility) -> &'static str {
    match compatibility {
        ChatCompatibility::Supported => "supported",
        ChatCompatibility::Unsupported => "unsupported",
    }
}

pub(super) fn device_label(device: ApplicationDevice) -> String {
    match device {
        ApplicationDevice::Cpu => "CPU".to_owned(),
        ApplicationDevice::Cuda { ordinal } => format!("CUDA {ordinal}"),
    }
}

pub(super) const fn device_availability_label(available: bool) -> &'static str {
    if available {
        "available"
    } else {
        "unavailable"
    }
}

pub(super) const fn scalar_type_label(value: ApplicationScalarType) -> &'static str {
    match value {
        ApplicationScalarType::F32 => "F32",
        ApplicationScalarType::F16 => "F16",
        ApplicationScalarType::Bf16 => "BF16",
    }
}
