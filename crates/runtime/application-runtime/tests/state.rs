//! Frontend-neutral application-state and closed selection contract tests.

use std::path::PathBuf;

use application_runtime::{
    ApplicationActivity, ApplicationBackend, ApplicationDevice, ApplicationModelFormat,
    ApplicationQuantization, ApplicationScalarType, ApplicationSource, ApplicationState,
    GenerationPhase, GenerationSummary, LocalModelProduct, ModelCompatibility, ModelSelection,
};
use domain_contracts::{GenerationUsage, RequestId};

#[test]
fn selection_vocabulary_exposes_only_the_two_reviewed_local_products() {
    let hub = ModelSelection::hugging_face_safetensors(" owner/model ", " main ");
    assert_eq!(hub.hugging_face_reference(), Some(("owner/model", "main")));
    assert_eq!(
        hub.product(),
        LocalModelProduct::HuggingFaceCandleSafetensors
    );
    assert_eq!(hub.product().backend(), ApplicationBackend::Candle);
    assert_eq!(hub.product().source(), ApplicationSource::HuggingFaceHub);
    assert_eq!(hub.product().device(), ApplicationDevice::Cpu);
    assert_eq!(hub.product().format(), ApplicationModelFormat::Safetensors);

    let gguf = ModelSelection::local_gguf("model.gguf");
    assert_eq!(
        gguf.local_path(),
        Some(PathBuf::from("model.gguf").as_path())
    );
    assert_eq!(gguf.product(), LocalModelProduct::LocalLlamaCppGguf);
    assert_eq!(gguf.product().backend(), ApplicationBackend::LlamaCpp);
    assert_eq!(gguf.product().source(), ApplicationSource::LocalFile);
    assert_eq!(gguf.product().device(), ApplicationDevice::Cpu);
    assert_eq!(gguf.product().format(), ApplicationModelFormat::Gguf);
}

#[test]
fn default_state_admits_resolution_but_not_loading() {
    let state = ApplicationState::default();
    let hub = ModelSelection::hugging_face_safetensors("owner/model", "main");
    let gguf = ModelSelection::local_gguf("model.gguf");

    assert_eq!(state.activity(), ApplicationActivity::Idle);
    assert!(state.can_resolve(&hub));
    assert!(state.can_resolve(&gguf));
    assert!(!state.can_load(&hub));
    assert!(!state.can_load(&gguf));
}

#[test]
fn compatibility_summary_keeps_product_specific_scalar_and_quantization() {
    let candle = ModelCompatibility::CandleSafetensors {
        scalar_type: Some(ApplicationScalarType::F32),
    };
    assert!(candle.is_loadable());
    assert_eq!(candle.scalar_type(), Some(ApplicationScalarType::F32));
    assert_eq!(candle.quantization(), ApplicationQuantization::None);

    let gguf = ModelCompatibility::LlamaCppGguf {
        scalar_type: ApplicationScalarType::I8,
        quantization: ApplicationQuantization::Gguf(7),
    };
    assert!(gguf.is_loadable());
    assert_eq!(gguf.scalar_type(), Some(ApplicationScalarType::I8));
    assert_eq!(gguf.quantization(), ApplicationQuantization::Gguf(7));
}

#[test]
fn generation_summary_exposes_phase_and_usage() {
    let summary = GenerationSummary {
        request_id: RequestId::new(9),
        phase: GenerationPhase::Running,
        usage: GenerationUsage {
            prompt_tokens: 3,
            generated_tokens: 2,
        },
    };
    assert_eq!(summary.request_id.get(), 9);
    assert_eq!(summary.phase, GenerationPhase::Running);
    assert_eq!(summary.usage.generated_tokens, 2);
}
