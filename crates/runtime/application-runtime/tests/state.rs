//! Frontend-neutral application-state contract tests.

use application_runtime::{
    ApplicationActivity, ApplicationBackend, ApplicationState, GenerationPhase, GenerationSummary,
    LoadedModel, ResolvedModel,
};
use domain_contracts::{
    DeviceKind, GenerationUsage, ModelGeneration, ModelHandle, ModelId, RequestId, ScalarType,
};

#[test]
fn resolved_selection_controls_load_admission() {
    let state = ApplicationState::default();
    assert_eq!(state.activity(), ApplicationActivity::Idle);
    assert!(!state.can_load("owner/model", "main"));

    let resolved = ResolvedModel {
        repository: "owner/model".to_owned(),
        revision: "main".to_owned(),
        commit: "immutable".to_owned(),
        vocabulary_size: 32,
        scalar_type: Some(ScalarType::F32),
    };
    assert!(resolved.matches_selection(" owner/model ", " main "));
}

#[test]
fn loaded_model_summary_retains_generation_safe_handle_and_target() {
    let loaded = LoadedModel {
        handle: ModelHandle::new(ModelId::new(7), ModelGeneration::new(3)),
        vocabulary_size: 128,
        maximum_context_tokens: 4_096,
        maximum_prefill_batch: 512,
        backend: ApplicationBackend::Candle,
        device: DeviceKind::Cpu,
    };
    assert_eq!(loaded.handle.generation.get(), 3);
    assert_eq!(loaded.vocabulary_size, 128);
    assert_eq!(loaded.maximum_context_tokens, 4_096);
    assert_eq!(loaded.backend, ApplicationBackend::Candle);
    assert_eq!(loaded.device, DeviceKind::Cpu);
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
