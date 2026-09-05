use milkdrift_capability::{
    AdmissionBound, AdmissionMonetaryBound, CapabilityCategory, InvocationAdmissionEnvelope,
};

use super::*;
use crate::{AttemptUsage, MonetaryUsage};

mod applicability;

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

fn account(process: u64, model: u64) -> TestResult<ControllerAccountState> {
    let budget =
        ControllerResourceBudget::new(8, CurrencyCode::new("USD")?, 8, 8, 8, process, model)?;
    Ok(ControllerAccountState::establish(
        ControllerAccountDeclaration::new(
            RunId::new("run-controller-account-test")?,
            NodeExecutionId::new("execution-controller-account-test")?,
            "policy:test-controller-account",
            budget,
        )?,
    )?)
}

fn reservation(
    state: &ControllerAccountState,
    suffix: &str,
) -> TestResult<(ControllerReservationId, AttemptId)> {
    let attempt = AttemptId::new(format!("attempt-{suffix}"))?;
    let reservation =
        ControllerReservationId::for_attempt(state.declaration().account(), &attempt)?;
    Ok((reservation, attempt))
}

fn bounded_envelope(maximum: u64) -> TestResult<InvocationAdmissionEnvelope> {
    Ok(InvocationAdmissionEnvelope::new(
        AdmissionBound::Bounded(maximum),
        AdmissionBound::Bounded(maximum),
        AdmissionBound::Bounded(maximum),
        AdmissionBound::Bounded(AdmissionMonetaryBound::new(maximum, "USD")?),
    ))
}

fn single_dimension_envelope(
    dimension: &str,
    maximum: u64,
) -> TestResult<InvocationAdmissionEnvelope> {
    let mut input = AdmissionBound::NotApplicable;
    let mut output = AdmissionBound::NotApplicable;
    let mut artifact = AdmissionBound::NotApplicable;
    let mut cost = AdmissionBound::NotApplicable;
    match dimension {
        "input_units" => input = AdmissionBound::Bounded(maximum),
        "output_units" => output = AdmissionBound::Bounded(maximum),
        "artifact_bytes" => artifact = AdmissionBound::Bounded(maximum),
        "monetary_cost" => {
            cost = AdmissionBound::Bounded(AdmissionMonetaryBound::new(maximum, "USD")?)
        }
        _ => return Err(format!("unsupported test dimension {dimension}").into()),
    }
    Ok(InvocationAdmissionEnvelope::new(
        input, output, artifact, cost,
    ))
}

#[test]
fn reservation_identity_is_derived_before_account_mutation() -> TestResult {
    let mut state = account(2, 2)?;
    let before = state.clone();
    let attempt = AttemptId::new("attempt-forged-reservation")?;
    let forged = ControllerReservationId::new("controller-reservation:forged")?;
    assert!(matches!(
        state.admit(
            forged,
            attempt,
            CapabilityCategory::Tool,
            &InvocationAdmissionEnvelope::not_applicable(),
        ),
        Err(PersistenceError::InvalidDocument(_))
    ));
    assert_eq!(state, before);
    Ok(())
}

#[test]
fn exact_process_ceiling_accepts_n_and_denies_n_plus_one() -> TestResult {
    let mut state = account(2, 2)?;
    for suffix in ["one", "two"] {
        let (reservation, attempt) = reservation(&state, suffix)?;
        assert!(matches!(
            state.admit(
                reservation,
                attempt,
                CapabilityCategory::Process,
                &InvocationAdmissionEnvelope::not_applicable(),
            )?,
            ControllerAdmissionOutcome::Reserved { .. }
        ));
    }
    let revision = state.revision();
    let (reservation, attempt) = reservation(&state, "three")?;
    assert!(matches!(
        state.admit(
            reservation,
            attempt,
            CapabilityCategory::Process,
            &InvocationAdmissionEnvelope::not_applicable(),
        )?,
        ControllerAdmissionOutcome::Denied {
            reason: ControllerAdmissionDenial::Limit { dimension },
            ..
        } if dimension == "process_admissions"
    ));
    assert_eq!(state.revision(), revision);
    assert_eq!(state.committed_totals()?.process_admissions(), 2);
    Ok(())
}

#[test]
fn every_resource_ceiling_accepts_exact_equality_and_denies_one_more() -> TestResult {
    for dimension in [
        "input_units",
        "output_units",
        "artifact_bytes",
        "monetary_cost",
    ] {
        let mut state = account(2, 2)?;
        let (exact, exact_attempt) = reservation(&state, &format!("{dimension}-exact"))?;
        assert!(matches!(
            state.admit(
                exact,
                exact_attempt,
                CapabilityCategory::Tool,
                &single_dimension_envelope(dimension, 8)?,
            )?,
            ControllerAdmissionOutcome::Reserved { .. }
        ));
        let (over, over_attempt) = reservation(&state, &format!("{dimension}-over"))?;
        assert!(matches!(
            state.admit(
                over,
                over_attempt,
                CapabilityCategory::Tool,
                &single_dimension_envelope(dimension, 1)?,
            )?,
            ControllerAdmissionOutcome::Denied {
                reason: ControllerAdmissionDenial::Limit {
                    dimension: denied,
                },
                ..
            } if denied == dimension
        ));
    }

    for (category, dimension) in [
        (CapabilityCategory::Process, "process_admissions"),
        (CapabilityCategory::Model, "model_admissions"),
    ] {
        let mut state = account(1, 1)?;
        let (exact, exact_attempt) = reservation(&state, &format!("{dimension}-exact"))?;
        assert!(matches!(
            state.admit(
                exact,
                exact_attempt,
                category.clone(),
                &InvocationAdmissionEnvelope::not_applicable(),
            )?,
            ControllerAdmissionOutcome::Reserved { .. }
        ));
        let (over, over_attempt) = reservation(&state, &format!("{dimension}-over"))?;
        assert!(matches!(
            state.admit(
                over,
                over_attempt,
                category,
                &InvocationAdmissionEnvelope::not_applicable(),
            )?,
            ControllerAdmissionOutcome::Denied {
                reason: ControllerAdmissionDenial::Limit {
                    dimension: denied,
                },
                ..
            } if denied == dimension
        ));
    }
    Ok(())
}

#[test]
fn unknown_currency_and_overflow_are_distinct_fail_closed_denials() -> TestResult {
    let mut state = account(2, 2)?;
    for (dimension, envelope) in [
        (
            "input_units",
            InvocationAdmissionEnvelope::new(
                AdmissionBound::Unknown,
                AdmissionBound::NotApplicable,
                AdmissionBound::NotApplicable,
                AdmissionBound::NotApplicable,
            ),
        ),
        (
            "output_units",
            InvocationAdmissionEnvelope::new(
                AdmissionBound::NotApplicable,
                AdmissionBound::Unknown,
                AdmissionBound::NotApplicable,
                AdmissionBound::NotApplicable,
            ),
        ),
        (
            "artifact_bytes",
            InvocationAdmissionEnvelope::new(
                AdmissionBound::NotApplicable,
                AdmissionBound::NotApplicable,
                AdmissionBound::Unknown,
                AdmissionBound::NotApplicable,
            ),
        ),
        (
            "monetary_cost",
            InvocationAdmissionEnvelope::new(
                AdmissionBound::NotApplicable,
                AdmissionBound::NotApplicable,
                AdmissionBound::NotApplicable,
                AdmissionBound::Unknown,
            ),
        ),
    ] {
        let (reservation, attempt) = reservation(&state, &format!("unknown-{dimension}"))?;
        assert!(matches!(
            state.admit(reservation, attempt, CapabilityCategory::Tool, &envelope)?,
            ControllerAdmissionOutcome::Denied {
                reason: ControllerAdmissionDenial::Unknown {
                    dimension: denied,
                },
                ..
            } if denied == dimension
        ));
    }
    let (currency_reservation, currency_attempt) = reservation(&state, "currency")?;
    let currency = InvocationAdmissionEnvelope::new(
        AdmissionBound::NotApplicable,
        AdmissionBound::NotApplicable,
        AdmissionBound::NotApplicable,
        AdmissionBound::Bounded(AdmissionMonetaryBound::new(1, "EUR")?),
    );
    assert!(matches!(
        state.admit(
            currency_reservation,
            currency_attempt,
            CapabilityCategory::Tool,
            &currency,
        )?,
        ControllerAdmissionOutcome::Denied {
            reason: ControllerAdmissionDenial::CurrencyMismatch,
            ..
        }
    ));

    let budget = ControllerResourceBudget::new(1, CurrencyCode::new("USD")?, u64::MAX, 1, 1, 1, 1)?;
    let mut overflow = ControllerAccountState::establish(ControllerAccountDeclaration::new(
        RunId::new("run-controller-overflow")?,
        NodeExecutionId::new("execution-controller-overflow")?,
        "policy:controller-overflow",
        budget,
    )?)?;
    let (exact, exact_attempt) = reservation(&overflow, "overflow-exact")?;
    assert!(matches!(
        overflow.admit(
            exact,
            exact_attempt,
            CapabilityCategory::Tool,
            &single_dimension_envelope("input_units", u64::MAX)?,
        )?,
        ControllerAdmissionOutcome::Reserved { .. }
    ));
    let (over, over_attempt) = reservation(&overflow, "overflow-over")?;
    assert!(matches!(
        overflow.admit(
            over,
            over_attempt,
            CapabilityCategory::Tool,
            &single_dimension_envelope("input_units", 1)?,
        )?,
        ControllerAdmissionOutcome::Denied {
            reason: ControllerAdmissionDenial::Overflow { dimension },
            ..
        } if dimension == "input_units"
    ));
    Ok(())
}

#[test]
fn uncertainty_retains_remainder_blocks_retry_and_roundtrips() -> TestResult {
    let mut state = account(4, 4)?;
    let (first, first_attempt) = reservation(&state, "uncertain-first")?;
    state.admit(
        first.clone(),
        first_attempt,
        CapabilityCategory::Tool,
        &bounded_envelope(4)?,
    )?;
    let (second, second_attempt) = reservation(&state, "uncertain-retry")?;
    state.admit(
        second,
        second_attempt,
        CapabilityCategory::Tool,
        &bounded_envelope(4)?,
    )?;
    let (third, third_attempt) = reservation(&state, "uncertain-over")?;
    assert!(matches!(
        state.admit(
            third,
            third_attempt,
            CapabilityCategory::Tool,
            &bounded_envelope(1)?,
        )?,
        ControllerAdmissionOutcome::Denied {
            reason: ControllerAdmissionDenial::Limit { .. },
            ..
        }
    ));
    state.settle_terminal(&first, None)?;
    assert!(matches!(
        state.blocked(),
        Some(ControllerAccountBlock::UnknownUsage { .. })
    ));
    assert_eq!(state.committed_totals()?.input_units(), 8);
    let stored = serde_json::to_vec(&state)?;
    let reopened: ControllerAccountState = serde_json::from_slice(&stored)?;
    reopened.validate()?;
    assert_eq!(reopened, state);
    Ok(())
}

#[test]
fn missing_usage_blocks_and_late_evidence_settles_the_original_reservation_once() -> TestResult {
    let mut state = account(2, 2)?;
    let (reservation, attempt) = reservation(&state, "late-usage")?;
    state.admit(
        reservation.clone(),
        attempt,
        CapabilityCategory::Tool,
        &single_dimension_envelope("input_units", 8)?,
    )?;
    state.settle_terminal(&reservation, None)?;
    assert!(matches!(
        state.blocked(),
        Some(ControllerAccountBlock::UnknownUsage { dimension, .. })
            if dimension == "input_units"
    ));
    assert_eq!(state.outstanding().input_units(), 8);

    state.settle_terminal(
        &reservation,
        Some(&AttemptUsage {
            input_units: Some(3),
            output_units: None,
            duration_ms: None,
            cost: None,
        }),
    )?;
    assert_eq!(state.outstanding().input_units(), 0);
    assert_eq!(state.settled().input_units(), 3);
    assert!(!state.reservations().contains_key(&reservation));
    assert!(matches!(
        state.settle_terminal(&reservation, None),
        Err(PersistenceError::NotFound { .. })
    ));
    state.validate()?;
    Ok(())
}

#[test]
fn terminal_cost_currency_and_partial_dimension_retention_are_exact() -> TestResult {
    let cost_envelope = InvocationAdmissionEnvelope::new(
        AdmissionBound::NotApplicable,
        AdmissionBound::NotApplicable,
        AdmissionBound::NotApplicable,
        AdmissionBound::Bounded(AdmissionMonetaryBound::new(4, "USD")?),
    );
    let mut matching = account(4, 4)?;
    let (matching_reservation, matching_attempt) = reservation(&matching, "matching-cost")?;
    matching.admit(
        matching_reservation.clone(),
        matching_attempt,
        CapabilityCategory::Tool,
        &cost_envelope,
    )?;
    matching.settle_terminal(
        &matching_reservation,
        Some(&AttemptUsage {
            input_units: None,
            output_units: None,
            duration_ms: None,
            cost: Some(MonetaryUsage {
                micros: 3,
                currency: CurrencyCode::new("USD")?,
            }),
        }),
    )?;
    assert_eq!(matching.settled().cost_micros(), 3);
    assert_eq!(matching.outstanding().cost_micros(), 0);
    assert!(matching.blocked().is_none());
    assert!(!matching.reservations().contains_key(&matching_reservation));

    let mut mismatching = account(4, 4)?;
    let (mismatching_reservation, mismatching_attempt) =
        reservation(&mismatching, "mismatching-cost")?;
    mismatching.admit(
        mismatching_reservation.clone(),
        mismatching_attempt,
        CapabilityCategory::Tool,
        &cost_envelope,
    )?;
    mismatching.settle_terminal(
        &mismatching_reservation,
        Some(&AttemptUsage {
            input_units: None,
            output_units: None,
            duration_ms: None,
            cost: Some(MonetaryUsage {
                micros: 3,
                currency: CurrencyCode::new("EUR")?,
            }),
        }),
    )?;
    assert!(matches!(
        mismatching.blocked(),
        Some(ControllerAccountBlock::Integrity { reason })
            if reason.contains("currency differs")
    ));
    assert_eq!(mismatching.settled().cost_micros(), 0);
    assert_eq!(mismatching.outstanding().cost_micros(), 4);
    assert!(
        mismatching
            .reservations()
            .contains_key(&mismatching_reservation)
    );

    let mut partial = account(4, 4)?;
    let (partial_reservation, partial_attempt) = reservation(&partial, "partial-usage")?;
    partial.admit(
        partial_reservation.clone(),
        partial_attempt,
        CapabilityCategory::Tool,
        &InvocationAdmissionEnvelope::new(
            AdmissionBound::Bounded(4),
            AdmissionBound::Bounded(4),
            AdmissionBound::NotApplicable,
            AdmissionBound::NotApplicable,
        ),
    )?;
    partial.settle_terminal(
        &partial_reservation,
        Some(&AttemptUsage {
            input_units: Some(2),
            output_units: None,
            duration_ms: None,
            cost: None,
        }),
    )?;
    assert_eq!(partial.settled().input_units(), 2);
    assert_eq!(partial.outstanding().input_units(), 0);
    assert_eq!(partial.outstanding().output_units(), 4);
    assert!(partial.reservations().contains_key(&partial_reservation));
    partial.validate()?;
    Ok(())
}

#[test]
fn account_mutating_transactions_require_the_exact_revision_guard() -> TestResult {
    let state = account(4, 4)?;
    let (reservation, attempt) = reservation(&state, "transaction-guard")?;
    let envelope = InvocationAdmissionEnvelope::not_applicable();
    let mut candidate = state.clone();
    let outcome = candidate.admit(
        reservation.clone(),
        attempt.clone(),
        CapabilityCategory::Process,
        &envelope,
    )?;
    let action = ControllerAccountAction::AdmitEntry {
        account: state.declaration().account().clone(),
        reservation,
        attempt,
        category: CapabilityCategory::Process,
        envelope,
        expected_outcome: outcome,
    };
    assert!(matches!(
        ControllerAccountTransaction::new(
            ControllerTransitionId::new("transition-controller-unguarded")?,
            None,
            vec![action.clone()],
        ),
        Err(PersistenceError::InvalidDocument(reason))
            if reason.contains("require the exact account revision guard")
    ));
    assert!(matches!(
        ControllerAccountTransaction::new(
            ControllerTransitionId::new("transition-controller-wrong-guard")?,
            Some((
                ControllerAccountId::new("controller-account:foreign")?,
                state.revision_digest().clone(),
            )),
            vec![action.clone()],
        ),
        Err(PersistenceError::InvalidDocument(reason))
            if reason.contains("require the exact account revision guard")
    ));
    let guarded = ControllerAccountTransaction::new(
        ControllerTransitionId::new("transition-controller-guarded")?,
        Some((
            state.declaration().account().clone(),
            state.revision_digest().clone(),
        )),
        vec![action],
    )?;
    assert_eq!(
        guarded
            .expected_account_revision()
            .map(|(account, _)| account),
        Some(state.declaration().account())
    );
    Ok(())
}

#[test]
fn over_contract_usage_records_actual_evidence_and_blocks() -> TestResult {
    let mut state = account(4, 4)?;
    let (reservation, attempt) = reservation(&state, "contract")?;
    state.admit(
        reservation.clone(),
        attempt,
        CapabilityCategory::Tool,
        &bounded_envelope(4)?,
    )?;
    state.settle_terminal(
        &reservation,
        Some(&AttemptUsage {
            input_units: Some(5),
            output_units: Some(4),
            duration_ms: None,
            cost: Some(MonetaryUsage {
                micros: 4,
                currency: CurrencyCode::new("USD")?,
            }),
        }),
    )?;
    assert!(matches!(
        state.blocked(),
        Some(ControllerAccountBlock::ContractViolation {
            dimension,
            observed: 5,
            reserved: 4,
            ..
        }) if dimension == "input_units"
    ));
    assert_eq!(state.settled().input_units(), 4);
    state.validate()?;
    Ok(())
}

#[test]
fn artifact_reservation_accepts_exact_boundary_and_refuses_one_more() -> TestResult {
    let mut state = account(4, 4)?;
    let (reservation, attempt) = reservation(&state, "artifact")?;
    state.admit(
        reservation.clone(),
        attempt,
        CapabilityCategory::Tool,
        &bounded_envelope(8)?,
    )?;
    state.charge_artifact(Some(&reservation), 8)?;
    assert_eq!(state.settled().artifact_bytes(), 8);
    assert!(matches!(
        state.charge_artifact(Some(&reservation), 1),
        Ok(ControllerArtifactChargeOutcome::ContractViolation)
    ));
    assert!(matches!(
        state.blocked(),
        Some(ControllerAccountBlock::ContractViolation {
            dimension,
            observed: 1,
            reserved: 0,
            ..
        }) if dimension == "artifact_bytes"
    ));
    state.validate()?;
    Ok(())
}

#[test]
fn controller_artifact_owner_wire_is_strict_and_preserves_run_binding_compatibility() -> TestResult
{
    let reservation = ControllerReservationId::new("controller-reservation:wire-owner")?;
    let owner = ControllerArtifactOwner::InvocationReservation(reservation.clone());
    let encoded = serde_json::to_vec(&owner)?;
    assert_eq!(
        encoded,
        br#"{"type":"invocation_reservation","reservation":"controller-reservation:wire-owner"}"#
    );
    assert_eq!(
        serde_json::from_slice::<ControllerArtifactOwner>(&encoded)?,
        owner
    );
    assert_eq!(
        serde_json::to_vec(&ControllerArtifactOwner::RunBinding)?,
        br#"{"type":"run_binding"}"#
    );
    assert!(
            serde_json::from_slice::<ControllerArtifactOwner>(
                br#"{"type":"invocation_reservation","reservation":"controller-reservation:wire-owner","extra":true}"#,
            )
            .is_err()
        );
    assert!(
        serde_json::from_slice::<ControllerArtifactOwner>(br#"{"type":"invocation_reservation"}"#,)
            .is_err()
    );
    Ok(())
}
