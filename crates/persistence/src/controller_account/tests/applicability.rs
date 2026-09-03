use super::*;

#[test]
fn not_applicable_dimensions_reject_contradictory_positive_usage() -> TestResult {
    for (dimension, usage) in [
        (
            "input_units",
            AttemptUsage {
                input_units: Some(1),
                output_units: None,
                duration_ms: None,
                cost: None,
            },
        ),
        (
            "output_units",
            AttemptUsage {
                input_units: None,
                output_units: Some(1),
                duration_ms: None,
                cost: None,
            },
        ),
        (
            "monetary_cost",
            AttemptUsage {
                input_units: None,
                output_units: None,
                duration_ms: None,
                cost: Some(MonetaryUsage {
                    micros: 1,
                    currency: CurrencyCode::new("USD")?,
                }),
            },
        ),
    ] {
        let mut state = account(4, 4)?;
        let (reservation, attempt) = reservation(&state, &format!("not-applicable-{dimension}"))?;
        assert!(matches!(
            state.admit(
                reservation.clone(),
                attempt,
                CapabilityCategory::Tool,
                &InvocationAdmissionEnvelope::not_applicable(),
            )?,
            ControllerAdmissionOutcome::Reserved { .. }
        ));
        state.settle_terminal(&reservation, Some(&usage))?;
        assert!(matches!(
            state.blocked(),
            Some(ControllerAccountBlock::ContractViolation {
                dimension: blocked,
                observed: 1,
                reserved: 0,
                ..
            }) if blocked == dimension
        ));
        assert!(!state.reservations().contains_key(&reservation));
        state.validate()?;
    }

    let mut zero_artifact = account(4, 4)?;
    let (zero_reservation, zero_attempt) =
        reservation(&zero_artifact, "not-applicable-zero-artifact")?;
    zero_artifact.admit(
        zero_reservation.clone(),
        zero_attempt,
        CapabilityCategory::Tool,
        &InvocationAdmissionEnvelope::not_applicable(),
    )?;
    assert_eq!(
        zero_artifact.charge_artifact(Some(&zero_reservation), 0)?,
        ControllerArtifactChargeOutcome::Charged
    );
    assert!(zero_artifact.blocked().is_none());
    zero_artifact.validate()?;

    let mut artifact = account(4, 4)?;
    let (reservation, attempt) = reservation(&artifact, "not-applicable-artifact")?;
    artifact.admit(
        reservation.clone(),
        attempt,
        CapabilityCategory::Tool,
        &InvocationAdmissionEnvelope::not_applicable(),
    )?;
    assert_eq!(
        artifact.charge_artifact(Some(&reservation), 1)?,
        ControllerArtifactChargeOutcome::ContractViolation
    );
    assert!(matches!(
        artifact.blocked(),
        Some(ControllerAccountBlock::ContractViolation {
            dimension,
            observed: 1,
            reserved: 0,
            ..
        }) if dimension == "artifact_bytes"
    ));
    artifact.validate()?;
    Ok(())
}
