use super::{
    AcceptanceReason, CodingResult, ResultAcceptanceContract, ResultRequirement, VerifiedCheckpoint,
};
use milkdrift_capability::{ArtifactReference, BoundedJson};
use milkdrift_model::{FinishReason, ModelResponse, ModelResponseDocument, ToolCall, Usage};
use std::collections::{BTreeMap, BTreeSet};

type TestResult = Result<(), Box<dyn std::error::Error>>;

#[test]
fn acceptance_contract_fixture_roundtrips_and_refuses_extra_fields() -> TestResult {
    let bytes = include_bytes!("../../tests/fixtures/result-acceptance-v1.json");
    let contract: ResultAcceptanceContract = serde_json::from_slice(bytes)?;
    assert_eq!(serde_json::to_vec(&contract)?, bytes.trim_ascii());
    let mut value = serde_json::to_value(contract)?;
    value["thinking_disabled"] = true.into();
    assert!(serde_json::from_value::<ResultAcceptanceContract>(value).is_err());
    let bytes = include_bytes!("../../tests/fixtures/acceptance-rejected-v1.json");
    let report: super::ResultAcceptance = serde_json::from_slice(bytes)?;
    assert_eq!(serde_json::to_vec(&report)?, bytes.trim_ascii());
    let mut value = serde_json::to_value(report)?;
    value["schema_version"] = 2.into();
    assert!(serde_json::from_value::<super::ResultAcceptance>(value.clone()).is_err());
    value["schema_version"] = 1.into();
    value["accepted"] = true.into();
    assert!(serde_json::from_value::<super::ResultAcceptance>(value).is_err());
    Ok(())
}

fn usage() -> Usage {
    Usage {
        input_units: Some(8),
        output_units: Some(4),
        cached_input_units: None,
        cost_micros: None,
        currency: None,
    }
}
fn model(
    text: &str,
    structured: Option<serde_json::Value>,
    tools: bool,
    finish: FinishReason,
) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    Ok(ModelResponseDocument::new(ModelResponse::new(
        text.to_owned(),
        structured.map(BoundedJson::new).transpose()?,
        if tools {
            vec![ToolCall::new(
                "call-1",
                "inspect",
                BoundedJson::new(serde_json::json!({}))?,
            )?]
        } else {
            vec![]
        },
        finish,
        usage(),
        BTreeMap::new(),
    )?)
    .to_canonical_json()?)
}

#[test]
fn acceptance_distinguishes_final_prose_from_empty_tools_and_exhaustion() -> TestResult {
    let prose = ResultAcceptanceContract::new(ResultRequirement::ModelProse, BTreeSet::new())?;
    for (text, tools, finish, reason) in [
        (
            "Reviewed.",
            false,
            FinishReason::Stop,
            AcceptanceReason::Accepted,
        ),
        (
            "",
            false,
            FinishReason::Stop,
            AcceptanceReason::MissingRequiredOutput,
        ),
        (
            " \n\t",
            false,
            FinishReason::Stop,
            AcceptanceReason::MissingRequiredOutput,
        ),
        (
            "",
            true,
            FinishReason::ToolCalls,
            AcceptanceReason::IncompleteResult,
        ),
        (
            "Draft",
            false,
            FinishReason::Length,
            AcceptanceReason::OutputAllowanceExhausted,
        ),
        (
            "",
            false,
            FinishReason::Length,
            AcceptanceReason::OutputAllowanceExhausted,
        ),
        (
            "Looks good",
            false,
            FinishReason::Unknown,
            AcceptanceReason::IncompleteResult,
        ),
    ] {
        let bytes = model(text, None, tools, finish)?;
        let result = prose.evaluate(&bytes, &[]);
        assert_eq!(result.reason, reason);
        assert_eq!(result.accepted, reason == AcceptanceReason::Accepted);
        assert_eq!(
            result.model.as_ref().map(|value| &value.usage),
            Some(&usage())
        );
        assert_eq!(
            ModelResponseDocument::from_json(&bytes)?
                .body()
                .finish_reason(),
            finish
        );
    }
    let tools = ResultAcceptanceContract::new(
        ResultRequirement::ModelTools {
            names: BTreeSet::from(["inspect".to_owned()]),
        },
        BTreeSet::new(),
    )?;
    assert!(
        tools
            .evaluate(&model("", None, true, FinishReason::ToolCalls)?, &[])
            .accepted
    );
    assert!(
        !tools
            .evaluate(&model("", None, false, FinishReason::ToolCalls)?, &[])
            .accepted
    );
    Ok(())
}

#[test]
fn acceptance_validates_minimal_structured_decision_and_exact_evidence() -> TestResult {
    let evidence = ArtifactReference::new(
        "artifact:evidence",
        "a".repeat(64),
        Some("text/plain".to_owned()),
        Some(1),
    )?;
    let contract = ResultAcceptanceContract::new(
        ResultRequirement::ModelDecision,
        BTreeSet::from(["evidence".to_owned()]),
    )?;
    let bytes = model(
        "",
        Some(serde_json::json!({"accepted":true,"evidence":[evidence]})),
        false,
        FinishReason::Stop,
    )?;
    assert!(
        contract
            .evaluate(&bytes, std::slice::from_ref(&evidence))
            .accepted
    );
    for value in [
        serde_json::json!({"accepted":true}),
        serde_json::json!({"accepted":true,"evidence":[]}),
        serde_json::json!({"accepted":true,"evidence":[evidence],"confidence":1}),
    ] {
        assert!(
            !contract
                .evaluate(
                    &model("", Some(value), false, FinishReason::Stop)?,
                    std::slice::from_ref(&evidence)
                )
                .accepted
        );
    }
    assert!(
        !contract
            .evaluate(
                &model(
                    "",
                    Some(serde_json::json!({"accepted":false,"evidence":[evidence]})),
                    false,
                    FinishReason::Stop
                )?,
                &[evidence]
            )
            .accepted
    );
    Ok(())
}

#[test]
fn verification_binds_checks_to_checkpoint_and_requires_justified_no_change() -> TestResult {
    let contract = ResultAcceptanceContract::new(
        ResultRequirement::Verification {
            checks: BTreeSet::from(["tests".to_owned()]),
        },
        BTreeSet::new(),
    )?;
    let mut report = VerifiedCheckpoint {
        checkpoint: "checkpoint:exact".to_owned(),
        checked_checkpoint: "checkpoint:exact".to_owned(),
        checks: BTreeMap::from([("tests".to_owned(), true)]),
        coding: CodingResult::NoChange {
            justification: "Existing implementation satisfies the requested check.".to_owned(),
        },
    };
    assert!(
        contract
            .evaluate(&serde_json::to_vec(&report)?, &[])
            .accepted
    );
    report.coding = CodingResult::NoChange {
        justification: " \n".to_owned(),
    };
    assert_eq!(
        contract.evaluate(&serde_json::to_vec(&report)?, &[]).reason,
        AcceptanceReason::UnverifiedDecision
    );
    report.coding = CodingResult::Changed;
    report.checked_checkpoint = "checkpoint:stale".to_owned();
    assert_eq!(
        contract.evaluate(&serde_json::to_vec(&report)?, &[]).reason,
        AcceptanceReason::StaleCheckpoint
    );
    report.checked_checkpoint = report.checkpoint.clone();
    report.checks.clear();
    assert_eq!(
        contract.evaluate(&serde_json::to_vec(&report)?, &[]).reason,
        AcceptanceReason::VerificationFailed
    );
    Ok(())
}

#[test]
fn acceptance_contract_refuses_unknown_versions_and_unbounded_requirements() -> TestResult {
    let decision = ResultAcceptanceContract::new(
        ResultRequirement::ReviewDecision,
        BTreeSet::from(["evidence".to_owned()]),
    )?;
    assert_eq!(
        decision
            .evaluate(br#"{"accepted":false,"accepted":true,"evidence":[]}"#, &[])
            .reason,
        AcceptanceReason::InvalidStructure
    );
    let verification = ResultAcceptanceContract::new(
        ResultRequirement::Verification {
            checks: BTreeSet::from(["test".to_owned()]),
        },
        BTreeSet::new(),
    )?;
    assert_eq!(verification.evaluate(br#"{"checkpoint":"a","checked_checkpoint":"a","checks":{"test":false,"test":true},"coding":{"type":"changed"}}"#, &[]).reason,
        AcceptanceReason::InvalidStructure);
    assert!(
        ResultAcceptanceContract::new(ResultRequirement::ModelDecision, BTreeSet::new()).is_err()
    );
    assert!(
        ResultAcceptanceContract::new(
            ResultRequirement::ModelTools {
                names: BTreeSet::new()
            },
            BTreeSet::new()
        )
        .is_err()
    );
    let contract = ResultAcceptanceContract::new(ResultRequirement::ModelProse, BTreeSet::new())?;
    let mut value = serde_json::to_value(&contract)?;
    value["schema_version"] = 2.into();
    assert!(serde_json::from_value::<ResultAcceptanceContract>(value).is_err());
    assert_eq!(
        contract.evaluate(b"not JSON", &[]).reason,
        AcceptanceReason::InvalidStructure
    );
    Ok(())
}
