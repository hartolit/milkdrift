//! Fixed criteria are challenged by changed identities, missing evidence and paired regressions.
use milkdrift_control::learning::*;
use milkdrift_persistence::{ControllerResourceBudget, ControllerResourceTotals};
use milkdrift_workspace::{
    ArtifactId, ArtifactReference, CandidateCheck, CandidateEvaluation, CandidateSubject,
    ContentDigest, MediaType,
};
use serde_json::json;
type Result<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

fn compare_methods(
    declaration: &LearningDeclaration,
    candidate: &milkdrift_blueprint::RevisionId,
    observations: &[(MethodEvaluation, MethodEvaluation)],
) -> Result<LearningComparison> {
    Ok(milkdrift_control::learning::compare_methods(
        declaration,
        candidate,
        &observations
            .iter()
            .map(|(a, b)| (Some(a.clone()), Some(b.clone())))
            .collect::<Vec<_>>(),
    )?)
}

#[test]
fn unaccepted_invocations_remain_inconclusive_without_fabricated_run_identities() -> Result {
    let (declaration, candidate, _) = fixture()?;
    let observations = vec![(None, None); declaration.pairs.len()];
    let result =
        milkdrift_control::learning::compare_methods(&declaration, &candidate, &observations)?;
    assert_eq!(result.outcome, LearningOutcome::Inconclusive);
    assert!(
        result
            .pairs
            .iter()
            .all(|p| p.baseline_repairs.is_none() && p.candidate_repairs.is_none())
    );
    Ok(())
}

fn artifact(name: &str) -> Result<ArtifactReference> {
    Ok(ArtifactReference::new(
        ArtifactId::new(name)?,
        ContentDigest::for_bytes(name.as_bytes()),
        MediaType::new("application/json")?,
        name.len() as u64,
    ))
}
fn digest(byte: char) -> String {
    format!("b3_{}", byte.to_string().repeat(64))
}
type Fixture = (
    LearningDeclaration,
    milkdrift_blueprint::RevisionId,
    Vec<(MethodEvaluation, MethodEvaluation)>,
);
fn fixture() -> Result<Fixture> {
    let baseline = serde_json::from_value(json!(format!("rev_{}", "a".repeat(64))))?;
    let candidate = serde_json::from_value(json!(format!("rev_{}", "b".repeat(64))))?;
    let allowance = ControllerResourceBudget::new(0, None, 200_000, 60_000, 268_435_456, 100, 20)?;
    let mut declaration = LearningDeclaration {
        schema_version: LEARNING_SCHEMA_VERSION,
        lane: LearningLane::SeededFixture,
        selection: LearningReceiptReference {
            actor: milkdrift_authority::ActorRef::new("human:evaluator")?,
            command: milkdrift_persistence::CommandId::new("selected")?,
        },
        baseline,
        proposal_run: milkdrift_workspace::RunId::new("learning-proposal")?,
        agreement: digest('a'),
        policy: digest('b'),
        verifier: digest('c'),
        checks: vec!["authenticated-mutation".into(), "durable-bookings".into()],
        input_field: milkdrift_workspace::ValueKey::new("product")?,
        candidate_output: milkdrift_workspace::ValueKey::new("candidate")?,
        pairs: vec![],
        allowance,
        maximum_duration_ms: 3_600_000,
        maximum_submissions: 3,
        minimum_repair_reduction: 2,
        provenance: vec![],
        unknowns: vec!["effective sampling settings unavailable".into()],
        publication: milkdrift_capability::CapabilityId::new("method:slotbook")?,
        generation: 2,
    };
    for name in ["loan-a", "loan-b", "class-a", "class-b"] {
        let slot = |label| -> Result<EvaluationSlot> {
            Ok(EvaluationSlot {
                invocation: EvaluationInvocation {
                    actor: milkdrift_authority::ActorRef::new("human:evaluator")?,
                    request: milkdrift_persistence::CommandId::new(format!("{name}-{label}"))?,
                },
                workspace: milkdrift_capability::managed::ManagedName::new(format!(
                    "{name}-{label}-work"
                ))?,
                workspace_configuration: digest('e'),
                workspace_generation: 1,
                target: milkdrift_capability::managed::ManagedName::new(format!(
                    "{name}-{label}-target"
                ))?,
                configuration: digest('d'),
                generation: 1,
            })
        };
        declaration.pairs.push(EvaluationPair {
            name: name.into(),
            input: artifact(name)?,
            baseline: slot("baseline")?,
            candidate: slot("candidate")?,
        });
    }
    let observations = declaration
        .pairs
        .iter()
        .map(|pair| {
            Ok((
                observed(
                    &declaration,
                    &pair.baseline,
                    &declaration.baseline,
                    &pair.input,
                    1,
                )?,
                observed(&declaration, &pair.candidate, &candidate, &pair.input, 0)?,
            ))
        })
        .collect::<Result<Vec<_>>>()?;
    Ok((declaration, candidate, observations))
}
fn observed(
    declaration: &LearningDeclaration,
    slot: &EvaluationSlot,
    method: &milkdrift_blueprint::RevisionId,
    input: &ArtifactReference,
    repairs: usize,
) -> Result<MethodEvaluation> {
    let submissions = (0..=repairs)
        .map(|i| {
            Ok(CandidateEvaluation {
                schema_version: 1,
                identity: format!(
                    "b3_{}",
                    blake3::hash(format!("{}-{i}", slot.invocation.request).as_bytes())
                ),
                subject: CandidateSubject {
                    artifact: artifact(&format!("{}-build-{i}", slot.invocation.request))?,
                    target: slot.target.clone(),
                    generation: slot.generation,
                    agreement: declaration.agreement.clone(),
                    configuration: slot.configuration.clone(),
                    policy: declaration.policy.clone(),
                    verifier: declaration.verifier.clone(),
                    producer: milkdrift_workspace::CausalId::new("trusted:verifier")?,
                },
                started_at: 10 + i as u64,
                expires_at: 1000,
                complete: true,
                checks: declaration
                    .checks
                    .iter()
                    .map(|name| CandidateCheck {
                        name: name.clone(),
                        passed: Some(i == repairs),
                        diagnostic: "finite fixture observation".into(),
                    })
                    .collect(),
            })
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(MethodEvaluation {
        invocation: slot.invocation.clone(),
        run: milkdrift_workspace::RunId::new(slot.invocation.request.as_str())?,
        method: method.clone(),
        input: input.clone(),
        history_complete: true,
        succeeded: true,
        violation: None,
        duration_ms: Some(500),
        usage: Some(ControllerResourceTotals::default()),
        allowance: Some(declaration.allowance.clone()),
        output: submissions
            .last()
            .map(|evaluation| evaluation.subject.artifact.clone()),
        submissions,
    })
}
#[test]
fn eligible_only_after_all_four_pairs_meet_the_unchanged_threshold() -> Result {
    let (declaration, candidate, observations) = fixture()?;
    let result = compare_methods(&declaration, &candidate, &observations)?;
    assert_eq!(result.outcome, LearningOutcome::Eligible);
    assert_eq!(result.lane, LearningLane::SeededFixture);
    assert!(
        result
            .pairs
            .iter()
            .all(|p| p.baseline_repairs == Some(1) && p.candidate_repairs == Some(0))
    );
    assert!(compare_methods(&declaration, &candidate, &observations[..3]).is_err());
    Ok(())
}
#[test]
fn one_regression_rejects_even_with_a_better_total_and_unknown_other_cost() -> Result {
    let (declaration, candidate, mut observations) = fixture()?;
    let pair = &declaration.pairs[0];
    observations[0].1 = observed(&declaration, &pair.candidate, &candidate, &pair.input, 2)?;
    observations[1].0.usage = None;
    assert_eq!(
        compare_methods(&declaration, &candidate, &observations)?.outcome,
        LearningOutcome::Rejected
    );
    Ok(())
}
#[test]
fn zero_baseline_rework_cannot_be_claimed_as_model_improvement() -> Result {
    let (declaration, candidate, mut observations) = fixture()?;
    for (pair, (baseline, _)) in declaration.pairs.iter().zip(&mut observations) {
        *baseline = observed(
            &declaration,
            &pair.baseline,
            &declaration.baseline,
            &pair.input,
            0,
        )?;
    }
    assert_eq!(
        compare_methods(&declaration, &candidate, &observations)?.outcome,
        LearningOutcome::Inconclusive
    );
    Ok(())
}
#[test]
fn incomplete_history_usage_or_verification_never_becomes_zero_failures() -> Result {
    let (declaration, candidate, observations) = fixture()?;
    for mutate in 0..4 {
        let mut values = observations.clone();
        let value = &mut values[0].1;
        match mutate {
            0 => value.history_complete = false,
            1 => value.usage = None,
            2 => {
                value.submissions[0].complete = false;
                for check in &mut value.submissions[0].checks {
                    check.passed = None;
                }
            }
            _ => value.duration_ms = None,
        }
        let compared = compare_methods(&declaration, &candidate, &values)?;
        assert_eq!(compared.outcome, LearningOutcome::Inconclusive);
        assert_eq!(compared.pairs[0].candidate_repairs, None);
    }
    Ok(())
}

#[test]
fn a_passing_verifier_cannot_qualify_a_different_or_missing_returned_product() -> Result {
    let (declaration, candidate, observations) = fixture()?;
    for output in [None, Some(artifact("unchecked-product")?)] {
        let mut values = observations.clone();
        values[0].1.output = output;
        assert_eq!(
            compare_methods(&declaration, &candidate, &values)?.outcome,
            LearningOutcome::Rejected
        );
        // Failure to read an output is missing evidence, not proof of a defective product.
        values[0].1.history_complete = false;
        assert_eq!(
            compare_methods(&declaration, &candidate, &values)?.outcome,
            LearningOutcome::Inconclusive
        );
    }
    let mut changed = declaration.clone();
    changed.input_field = milkdrift_workspace::ValueKey::new("unused-input")?;
    assert_ne!(changed.digest()?, declaration.digest()?);
    changed = declaration.clone();
    changed.candidate_output = milkdrift_workspace::ValueKey::new("unchecked-output")?;
    assert_ne!(changed.digest()?, declaration.digest()?);
    Ok(())
}
#[test]
fn changed_target_input_checkset_or_verifier_cannot_qualify() -> Result {
    let (declaration, candidate, observations) = fixture()?;
    for mutate in 0..5 {
        let mut values = observations.clone();
        let value = &mut values[0].1;
        match mutate {
            0 => value.input = artifact("other-input")?,
            1 => {
                value.submissions[0].subject.target = declaration.pairs[1].candidate.target.clone()
            }
            2 => value.submissions[0]
                .checks
                .pop()
                .map(|_| ())
                .ok_or("check absent")?,
            3 => value.submissions[0].subject.verifier = digest('d'),
            _ => value.submissions[0].subject.configuration = digest('e'),
        }
        assert!(compare_methods(&declaration, &candidate, &values).is_err());
    }
    Ok(())
}
#[test]
fn exceeded_bounds_failed_obligations_and_authority_violations_reject() -> Result {
    let (declaration, candidate, observations) = fixture()?;
    for mutate in 0..5 {
        let mut values = observations.clone();
        let value = &mut values[0].1;
        match mutate {
            0 => value.duration_ms = Some(declaration.maximum_duration_ms + 1),
            1 => value.submissions[0].checks[0].passed = Some(false),
            2 => value.violation = Some("forbidden publication".into()),
            3 => {
                value.allowance = Some(ControllerResourceBudget::new(
                    0,
                    None,
                    400_000,
                    60_000,
                    268_435_456,
                    100,
                    20,
                )?)
            }
            _ => {
                *value = observed(
                    &declaration,
                    &declaration.pairs[0].candidate,
                    &candidate,
                    &declaration.pairs[0].input,
                    3,
                )?;
            }
        }
        assert_eq!(
            compare_methods(&declaration, &candidate, &values)?.outcome,
            LearningOutcome::Rejected
        );
    }
    Ok(())
}
#[test]
fn shared_working_area_and_reused_inputs_are_refused_before_evaluation() -> Result {
    let (declaration, _, _) = fixture()?;
    let mut changed = declaration.clone();
    changed.pairs[1].candidate.workspace = changed.pairs[0].baseline.workspace.clone();
    assert!(changed.validate().is_err());
    changed = declaration.clone();
    changed.pairs[1].input = changed.pairs[0].input.clone();
    assert!(changed.validate().is_err());
    changed.pairs[1].input = ArtifactReference::new(
        ArtifactId::new("same-bytes-different-identity")?,
        changed.pairs[0].input.digest(),
        changed.pairs[0].input.media_type().clone(),
        changed.pairs[0].input.size_bytes(),
    );
    assert!(changed.validate().is_err());
    changed = declaration.clone();
    changed.minimum_repair_reduction = 1;
    assert_ne!(changed.digest()?, declaration.digest()?);
    Ok(())
}
