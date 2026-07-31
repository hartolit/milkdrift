//! Frontend-neutral application-state and model-selection contract tests.

use application_runtime::{
    ApplicationActivity, ApplicationState, GenerationPhase, GenerationSummary, ModelSelection,
};
use domain_contracts::{GenerationUsage, RequestId};

#[test]
fn selection_normalizes_repository_and_revision() {
    let selection = ModelSelection::new(" owner/model ", " main ");

    assert_eq!(selection.repository(), "owner/model");
    assert_eq!(selection.revision(), "main");
}

#[test]
fn default_state_admits_resolution_but_not_loading() {
    let state = ApplicationState::default();
    let selection = ModelSelection::new("owner/model", "main");

    assert_eq!(state.activity(), ApplicationActivity::Idle);
    assert!(state.can_resolve(&selection));
    assert!(!state.can_load(&selection));
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
