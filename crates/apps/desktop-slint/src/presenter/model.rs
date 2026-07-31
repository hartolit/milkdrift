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

pub(super) fn selected_model_summary(selection: &ModelSelection) -> String {
    format!(
        "{} • Repository: {} • Revision: {} • Scalar: pending resolution • Identity: pending resolution",
        current_model_target_label(),
        selection.repository(),
        selection.revision(),
    )
}

pub(super) fn resolved_model_summary(model: &ResolvedModel) -> String {
    detailed_model_summary(
        model.engine(),
        model.source(),
        model.device(),
        model.format(),
        model.scalar_type(),
        model.identity(),
    )
}

pub(super) fn loaded_model_summary(model: &LoadedModel) -> String {
    detailed_model_summary(
        model.engine(),
        model.source(),
        model.device(),
        model.format(),
        Some(model.scalar_type()),
        model.identity(),
    )
}

fn detailed_model_summary(
    engine: ApplicationEngine,
    source: ApplicationSource,
    device: ApplicationDevice,
    format: ApplicationModelFormat,
    scalar_type: Option<ApplicationScalarType>,
    identity: &ImmutableModelIdentity,
) -> String {
    let scalar = scalar_type.map_or("Unknown", scalar_type_label);
    format!(
        "{} • Scalar: {scalar} • Identity: {}",
        model_target_label(engine, source, device, format),
        immutable_identity_label(identity)
    )
}

pub(super) fn current_model_target_label() -> String {
    model_target_label(
        ApplicationEngine::Candle,
        ApplicationSource::HuggingFaceHub,
        ApplicationDevice::Cpu,
        ApplicationModelFormat::Safetensors,
    )
}

pub(super) fn model_target_label(
    engine: ApplicationEngine,
    source: ApplicationSource,
    device: ApplicationDevice,
    format: ApplicationModelFormat,
) -> String {
    format!(
        "Engine: {} • Source: {} • Device: {} • Format: {}",
        engine_label(engine),
        source_label(source),
        device_label(device),
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

pub(super) const fn device_label(device: ApplicationDevice) -> &'static str {
    match device {
        ApplicationDevice::Cpu => "CPU",
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
