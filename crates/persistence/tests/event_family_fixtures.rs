//! Small golden examples spanning legacy and current durable run-event semantic families.

use std::collections::BTreeMap;

use milkdrift_authority::ActorRef;
use milkdrift_blueprint::{NodeId, RevisionId};
use milkdrift_capability::BoundedJson;
use milkdrift_persistence::{
    ControllerAssessmentBoundary, ControllerAssessmentOutcome, CurrencyCode, EventId,
    NodeExecutionId, ReconciliationId, ReconciliationPolicy, RunEventEnvelope, RunEventKind,
    RunOutcome, RunSequence, SubworkflowResourceUsage, TimestampMillis,
};
use milkdrift_workspace::{RunId, SubworkflowId};
use serde_json::json;

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

fn revision(digit: char) -> TestResult<RevisionId> {
    Ok(serde_json::from_value(json!(format!(
        "rev_{}",
        digit.to_string().repeat(64)
    )))?)
}

fn envelope(sequence: u64, kind: RunEventKind) -> TestResult<RunEventEnvelope> {
    Ok(RunEventEnvelope::new(
        EventId::new(format!("event-family-{sequence:02}"))?,
        RunId::new("run-family-fixture")?,
        RunSequence::new(sequence),
        TimestampMillis::new(1_700_000_000_000 + sequence),
        kind,
    )?)
}

fn legacy_fixtures() -> [(&'static str, &'static [u8]); 16] {
    [
        (
            "revision-control",
            include_bytes!("fixtures/run-event-revision-control-v1.json"),
        ),
        (
            "node-output",
            include_bytes!("fixtures/run-event-node-output-v1.json"),
        ),
        ("lease", include_bytes!("fixtures/run-event-lease-v1.json")),
        ("retry", include_bytes!("fixtures/run-event-retry-v1.json")),
        (
            "cancellation",
            include_bytes!("fixtures/run-event-cancellation-v1.json"),
        ),
        (
            "uncertainty",
            include_bytes!("fixtures/run-event-uncertainty-v1.json"),
        ),
        (
            "branch",
            include_bytes!("fixtures/run-event-branch-v1.json"),
        ),
        ("join", include_bytes!("fixtures/run-event-join-v1.json")),
        (
            "repeat",
            include_bytes!("fixtures/run-event-repeat-v1.json"),
        ),
        ("wait", include_bytes!("fixtures/run-event-wait-v1.json")),
        (
            "signal",
            include_bytes!("fixtures/run-event-signal-v1.json"),
        ),
        ("timer", include_bytes!("fixtures/run-event-timer-v1.json")),
        (
            "subworkflow",
            include_bytes!("fixtures/run-event-subworkflow-v1.json"),
        ),
        (
            "recovery",
            include_bytes!("fixtures/run-event-recovery-v1.json"),
        ),
        (
            "reconciliation",
            include_bytes!("fixtures/run-event-reconciliation-v1.json"),
        ),
        (
            "reconciliation-remediation",
            include_bytes!("fixtures/run-event-reconciliation-remediation-v1.json"),
        ),
    ]
}

fn current_fixtures() -> TestResult<[(&'static str, RunEventEnvelope, &'static [u8]); 3]> {
    let currency = CurrencyCode::new("USD")?;
    let usage = SubworkflowResourceUsage {
        input_units: Some(11),
        output_units: Some(13),
        artifact_bytes: 17,
        cost_micros: BTreeMap::from([(currency.clone(), 19)]),
        process_invocations: 2,
        model_invocations: 3,
        unknown_input_usage: 0,
        unknown_output_usage: 1,
        unknown_cost_usage: 0,
    };
    Ok([
        (
            "controller-assessment",
            envelope(
                17,
                RunEventKind::ControllerAssessmentRecorded {
                    controller_id: "controller-family".to_owned(),
                    policy_digest: "policy-family".to_owned(),
                    governing_revision: revision('1')?,
                    controller_node: NodeId::new("controller-node-family")?,
                    controller_execution: NodeExecutionId::new("controller-execution-family")?,
                    assessment_id: "assessment-family".to_owned(),
                    cycle_id: Some("cycle-family".to_owned()),
                    boundary: ControllerAssessmentBoundary::CycleEntry,
                    through_sequence: RunSequence::new(16),
                    progress: BoundedJson::new(json!({"completed_cycles": 1}))?,
                    outcome: ControllerAssessmentOutcome::Continue,
                },
            )?,
            include_bytes!("fixtures/run-event-controller-assessment-v2.json"),
        ),
        (
            "subworkflow-usage",
            envelope(
                18,
                RunEventKind::SubworkflowTerminal {
                    subworkflow: SubworkflowId::new("subworkflow-family")?,
                    child_run: RunId::new("run-child-family")?,
                    outcome: RunOutcome::Succeeded,
                    outputs: Vec::new(),
                    cost_micros: BTreeMap::from([(currency, 19)]),
                    usage,
                },
            )?,
            include_bytes!("fixtures/run-event-subworkflow-usage-v2.json"),
        ),
        (
            "attributed-reconciliation",
            envelope(
                19,
                RunEventKind::RevisionAdoptionRequested {
                    reconciliation: ReconciliationId::new("reconciliation-family")?,
                    requested_by: Some(ActorRef::new("ai:controller-family")?),
                    from_revision: revision('0')?,
                    to_revision: revision('1')?,
                    policy: ReconciliationPolicy::RequireAuthority,
                },
            )?,
            include_bytes!("fixtures/run-event-attributed-reconciliation-v2.json"),
        ),
    ])
}

#[test]
fn durable_event_families_retain_exact_v1_and_review_current_v2_goldens() -> TestResult {
    for (name, fixture) in legacy_fixtures() {
        let fixture = fixture.strip_suffix(b"\n").unwrap_or(fixture);
        let event = RunEventEnvelope::from_json(fixture)?;
        assert_eq!(event.schema_version(), 1, "legacy fixture {name}");
        assert_eq!(event.to_canonical_json()?, fixture, "legacy fixture {name}");
    }
    for (name, event, fixture) in current_fixtures()? {
        let fixture = fixture.strip_suffix(b"\n").unwrap_or(fixture);
        assert_eq!(event.schema_version(), 2, "current fixture {name}");
        assert_eq!(
            event.to_canonical_json()?,
            fixture,
            "current fixture {name}"
        );
        assert_eq!(RunEventEnvelope::from_json(fixture)?, event);
    }
    Ok(())
}
