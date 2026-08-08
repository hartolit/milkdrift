use application_runtime::{
    ApplicationDevice, ApplicationEngine, ApplicationModelFormat, ApplicationRuntime,
    ApplicationScalarType, ApplicationSource, ChatCompatibility, ImmutableModelIdentity,
    LoadedModel, ModelSelection, ResolvedModel,
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
            Self::Unavailable => "No model loaded",
            Self::Chat => "Chat",
            Self::DirectCompletion => "Direct completion",
        }
    }

    pub(super) const fn guidance(self) -> &'static str {
        match self {
            Self::Unavailable => "Load a model to enable generation.",
            Self::Chat => {
                "Verified chat compatibility: E1 owns conversation history, prompt rendering, and regeneration."
            }
            Self::DirectCompletion => {
                "No verified chat profile: input is submitted as an E1 direct-completion prompt without inferred history or template semantics."
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

pub(super) const fn composer_mode_from_evidence(
    has_loaded_model: bool,
    has_verified_chat_compatibility: bool,
) -> ComposerMode {
    if !has_loaded_model {
        ComposerMode::Unavailable
    } else if has_verified_chat_compatibility {
        ComposerMode::Chat
    } else {
        ComposerMode::DirectCompletion
    }
}

pub(super) fn composer_mode(runtime: &ApplicationRuntime) -> ComposerMode {
    let state = runtime.state();
    let Some(loaded) = state.loaded() else {
        return ComposerMode::Unavailable;
    };
    let has_verified_chat_compatibility = state.resolved().is_some_and(|resolved| {
        resolved.selection() == loaded.selection()
            && resolved.identity() == loaded.identity()
            && matches!(
                resolved.chat_compatibility(),
                ChatCompatibility::Supported(_)
            )
    });
    composer_mode_from_evidence(true, has_verified_chat_compatibility)
}

pub(super) fn selected_model_summary(
    selection: &ModelSelection,
    selected_device: ApplicationDevice,
    selected_device_available: bool,
) -> String {
    format!(
        "{} • Repository: {} • Revision: {} • Selected device: {} • Availability: {} • Identity: pending resolution",
        current_artifact_target_label(),
        selection.repository(),
        selection.revision(),
        device_label(selected_device),
        device_availability_label(selected_device_available),
    )
}

pub(super) const UNRESOLVED_MODEL_SUMMARY: &str = "Not resolved.";
pub(super) const UNLOADED_MODEL_SUMMARY: &str = "Not loaded.";

pub(super) fn resolved_model_summary(model: &ResolvedModel) -> String {
    let target = artifact_target_label(model.engine(), model.source(), model.format());
    let identity = immutable_identity_label(model.identity());
    resolved_model_facts_summary(
        &target,
        model.configuration_declared_scalar_type(),
        &identity,
    )
}

pub(super) fn loaded_model_summary(model: &LoadedModel) -> String {
    let target = artifact_target_label(model.engine(), model.source(), model.format());
    let identity = immutable_identity_label(model.identity());
    loaded_model_facts_summary(
        &target,
        model.execution_scalar_type(),
        model.device(),
        &identity,
    )
}

pub(super) fn resolved_model_facts_summary(
    target: &str,
    configuration_declared_scalar_type: Option<ApplicationScalarType>,
    identity: &str,
) -> String {
    configuration_declared_scalar_type.map_or_else(
        || format!("{target} • Identity: {identity}"),
        |scalar_type| {
            format!(
                "{target} • Configuration-declared scalar: {} • Identity: {identity}",
                scalar_type_label(scalar_type)
            )
        },
    )
}

pub(super) fn loaded_model_facts_summary(
    target: &str,
    execution_scalar_type: ApplicationScalarType,
    execution_device: ApplicationDevice,
    identity: &str,
) -> String {
    format!(
        "{target} • Execution scalar: {} • Execution device: {} • Identity: {identity}",
        scalar_type_label(execution_scalar_type),
        device_label(execution_device),
    )
}

pub(super) fn current_artifact_target_label() -> String {
    artifact_target_label(
        ApplicationEngine::Candle,
        ApplicationSource::HuggingFaceHub,
        ApplicationModelFormat::Safetensors,
    )
}

pub(super) fn artifact_target_label(
    engine: ApplicationEngine,
    source: ApplicationSource,
    format: ApplicationModelFormat,
) -> String {
    format!(
        "Engine: {} • Source: {} • Format: {}",
        engine_label(engine),
        source_label(source),
        model_format_label(format)
    )
}

pub(super) const fn engine_label(engine: ApplicationEngine) -> &'static str {
    match engine {
        ApplicationEngine::Candle => "Candle",
    }
}

const fn source_label(source: ApplicationSource) -> &'static str {
    match source {
        ApplicationSource::HuggingFaceHub => "Hugging Face Hub",
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

const fn model_format_label(format: ApplicationModelFormat) -> &'static str {
    match format {
        ApplicationModelFormat::Safetensors => "Safetensors",
    }
}

pub(super) const fn scalar_type_label(value: ApplicationScalarType) -> &'static str {
    match value {
        ApplicationScalarType::F32 => "F32",
        ApplicationScalarType::F16 => "F16",
        ApplicationScalarType::Bf16 => "BF16",
    }
}

fn immutable_identity_label(identity: &ImmutableModelIdentity) -> String {
    format!(
        "Hub commit {} ({})",
        identity.commit(),
        identity.repository()
    )
}
