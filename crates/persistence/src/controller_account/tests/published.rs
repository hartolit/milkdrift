//! Composed reservations count internal work once, independently of public artifact copies.
use super::*;
use milkdrift_capability::{InvocationCounts, NestedWorkUsage};

#[test]
fn nested_allowance_survives_reopen_and_settles_without_double_charging() -> TestResult {
    let mut state = account(4, 4)?;
    let envelope = bounded_envelope(6)?.with_nested_invocations(InvocationCounts::new(3, 2));
    let (id, attempt) = reservation(&state, "published")?;
    assert!(matches!(
        state.admit(id.clone(), attempt, CapabilityCategory::Tool, &envelope)?,
        ControllerAdmissionOutcome::Reserved { .. }
    ));
    assert_eq!(state.settled().process_admissions(), 0);
    assert_eq!(state.outstanding().process_admissions(), 3);
    assert_eq!(state.outstanding().model_admissions(), 2);
    let bytes = serde_json::to_vec(&state)?;
    let mut state: ControllerAccountState = serde_json::from_slice(&bytes)?;
    state.validate()?;
    let (second, second_attempt) = reservation(&state, "exhausted")?;
    assert!(
        matches!(state.admit(second, second_attempt, CapabilityCategory::Tool,
        &InvocationAdmissionEnvelope::not_applicable().with_nested_invocations(InvocationCounts::new(2, 0)))?,
        ControllerAdmissionOutcome::Denied { reason: ControllerAdmissionDenial::Limit { dimension }, .. } if dimension == "process_admissions")
    );
    assert_eq!(
        state.charge_artifact(Some(&id), 2)?,
        ControllerArtifactChargeOutcome::Charged
    );
    let usage = AttemptUsage {
        input_units: Some(2),
        output_units: Some(3),
        duration_ms: None,
        cost: Some(MonetaryUsage {
            micros: 1,
            currency: CurrencyCode::new("USD")?,
        }),
        nested_work: Some(NestedWorkUsage::new(InvocationCounts::new(2, 1), 3)),
    };
    state.settle_terminal(&id, Some(&usage))?;
    assert!(state.blocked().is_none());
    assert!(state.reservations().is_empty());
    assert_eq!(state.settled().process_admissions(), 2);
    assert_eq!(state.settled().model_admissions(), 1);
    assert_eq!(state.settled().artifact_bytes(), 5);
    assert_eq!(state.outstanding(), ControllerResourceTotals::default());
    let prior = state.clone();
    assert!(state.settle_terminal(&id, Some(&usage)).is_err());
    assert_eq!(state, prior);
    state.validate()?;
    Ok(())
}

#[test]
fn missing_nested_usage_retains_counts_and_internal_bytes() -> TestResult {
    let mut state = account(3, 3)?;
    let (id, attempt) = reservation(&state, "unknown-published")?;
    let envelope = bounded_envelope(4)?.with_nested_invocations(InvocationCounts::new(2, 2));
    let _ = state.admit(id.clone(), attempt, CapabilityCategory::Tool, &envelope)?;
    let _ = state.charge_artifact(Some(&id), 1)?;
    state.settle_terminal(&id, None)?;
    assert!(state.blocked().is_some());
    assert_eq!(state.outstanding().process_admissions(), 2);
    assert_eq!(state.outstanding().model_admissions(), 2);
    assert_eq!(state.outstanding().artifact_bytes(), 3);
    assert_eq!(state.settled().artifact_bytes(), 1);
    state.validate()?;
    let recovered: ControllerAccountState = serde_json::from_slice(&serde_json::to_vec(&state)?)?;
    assert_eq!(state, recovered);
    Ok(())
}

#[test]
fn published_allowance_has_a_real_invocation_owner_and_stable_identity() -> TestResult {
    let original = account(3, 3)?;
    let declaration = ControllerAccountDeclaration::for_published_invocation(
        RunId::new("published:child")?,
        InvocationId::new("invocation:public")?,
        "method:digest",
        original.declaration().budget().clone(),
    )?;
    assert!(declaration.controller_execution().is_none());
    assert_eq!(
        declaration.published_invocation().map(InvocationId::as_str),
        Some("invocation:public")
    );
    declaration.validate()?;
    let decoded: ControllerAccountDeclaration =
        serde_json::from_slice(&serde_json::to_vec(&declaration)?)?;
    assert_eq!(decoded, declaration);
    assert_ne!(declaration.account(), original.declaration().account());
    Ok(())
}

#[test]
fn process_only_allowance_refuses_model_entry_and_preserves_zero_dimensions() -> TestResult {
    let budget = ControllerResourceBudget::new(0, None, 0, 0, 8, 2, 0)?;
    assert!(budget.fits_within(account(3, 3)?.declaration().budget()));
    let declaration = ControllerAccountDeclaration::for_published_invocation(
        RunId::new("published:process-only")?,
        InvocationId::new("public:process-only")?,
        "method:process-only",
        budget,
    )?;
    let mut state = ControllerAccountState::establish(declaration)?;
    let (reservation, attempt) = reservation(&state, "forbidden-model")?;
    let result = state.admit(
        reservation,
        attempt,
        CapabilityCategory::Model,
        &InvocationAdmissionEnvelope::not_applicable(),
    )?;
    assert!(matches!(result, ControllerAdmissionOutcome::Denied { .. }));
    assert_eq!(state.settled().model_admissions(), 0);
    assert!(state.reservations().is_empty());
    state.validate()?;
    Ok(())
}
