//! Small golden examples spanning the durable run-event semantic families.

use std::collections::BTreeMap;

use milkdrift_blueprint::{ContentDigest, RevisionId};
use milkdrift_capability::SideEffectClass;
use milkdrift_persistence::{
    AttemptId, BranchResultReference, EventId, JoinRule, LeaseId, NodeExecutionId,
    NodeExecutionMode, Reason, ReconciliationId, ReconciliationPlanId, ReconciliationPolicy,
    RunEventEnvelope, RunEventKind, RunOutcome, RunSequence, SignalDeliveryMode, SignalId,
    SignalTypeId, TimerId, TimestampMillis, WaitCondition, WorkerId,
};
use milkdrift_workspace::{
    BranchId, IterationId, RunId, ScopeId, ScopeReference, SubworkflowId, ValueKey, ValueVersion,
    WorkspaceValueReference,
};
use serde_json::json;

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

fn revision(digit: char) -> TestResult<RevisionId> {
    Ok(serde_json::from_value(json!(format!(
        "rev_{}",
        digit.to_string().repeat(64)
    )))?)
}

fn revision_digest(digit: char) -> TestResult<ContentDigest> {
    Ok(serde_json::from_value(json!(format!(
        "b3_{}",
        digit.to_string().repeat(64)
    )))?)
}

fn scope(run: &RunId, name: &str) -> TestResult<ScopeReference> {
    Ok(ScopeReference::new(run.clone(), ScopeId::new(name)?))
}

fn value(run: &RunId, name: &str) -> TestResult<WorkspaceValueReference> {
    Ok(WorkspaceValueReference::new(
        scope(run, "scope-fixture")?,
        ValueKey::new(name)?,
        ValueVersion::FIRST,
    ))
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

fn fixtures() -> TestResult<Vec<(&'static str, RunEventEnvelope)>> {
    let run = RunId::new("run-family-fixture")?;
    let branch = BranchId::new("branch-family")?;
    Ok(vec![
        (
            "revision-control",
            envelope(
                1,
                RunEventKind::RevisionPinned {
                    previous: revision('0')?,
                    revision: revision('1')?,
                    revision_digest: revision_digest('2')?,
                    plan: ReconciliationPlanId::new("plan-family")?,
                },
            )?,
        ),
        (
            "node-output",
            envelope(
                2,
                RunEventKind::NodeOutputPublished {
                    execution: NodeExecutionId::new("execution-family")?,
                    attempt: AttemptId::new("attempt-family")?,
                    report_sequence: 3,
                    value: value(&run, "result")?,
                    artifact: None,
                },
            )?,
        ),
        (
            "lease",
            envelope(
                3,
                RunEventKind::LeaseGranted {
                    lease: LeaseId::new("lease-family")?,
                    execution: NodeExecutionId::new("execution-family")?,
                    attempt: AttemptId::new("attempt-family")?,
                    worker: WorkerId::new("worker-family")?,
                    expires_at: TimestampMillis::new(1_700_000_001_000),
                },
            )?,
        ),
        (
            "retry",
            envelope(
                4,
                RunEventKind::NodeRetryScheduled {
                    execution: NodeExecutionId::new("execution-family")?,
                    previous_attempt: AttemptId::new("attempt-family")?,
                    next_attempt: AttemptId::new("attempt-family-2")?,
                    attempt_number: 2,
                    timer: TimerId::new("timer-retry-family")?,
                    fire_at: TimestampMillis::new(1_700_000_002_000),
                    error_class: milkdrift_capability::ErrorClass::Transport,
                    reason: Reason::new("bounded retry selected")?,
                },
            )?,
        ),
        (
            "cancellation",
            envelope(
                5,
                RunEventKind::RunCancellationRequested {
                    reason: Reason::new("operator requested cancellation")?,
                    evidence: Vec::new(),
                },
            )?,
        ),
        (
            "uncertainty",
            envelope(
                6,
                RunEventKind::ExternalOutcomeUncertain {
                    attempt: AttemptId::new("attempt-family")?,
                    report_sequence: 4,
                    side_effect: SideEffectClass::NonIdempotentWrite,
                    reason: Reason::new("provider outcome unavailable")?,
                    evidence: Vec::new(),
                },
            )?,
        ),
        (
            "branch",
            envelope(
                7,
                RunEventKind::BranchTerminal {
                    branch: branch.clone(),
                    outcome: RunOutcome::Succeeded,
                    outputs: vec![value(&run, "branch-result")?],
                },
            )?,
        ),
        (
            "join",
            envelope(
                8,
                RunEventKind::JoinSatisfied {
                    execution: NodeExecutionId::new("join-execution-family")?,
                    rule: JoinRule::All,
                    branches: vec![BranchResultReference {
                        branch: branch.clone(),
                        scope: scope(&run, "scope-branch-family")?,
                        outcome: RunOutcome::Succeeded,
                        outputs: vec![value(&run, "branch-result")?],
                    }],
                    retained_branches: Vec::new(),
                },
            )?,
        ),
        (
            "repeat",
            envelope(
                9,
                RunEventKind::RepeatConditionRecorded {
                    iteration: IterationId::new("iteration-family")?,
                    result: false,
                },
            )?,
        ),
        (
            "wait",
            envelope(
                10,
                RunEventKind::WaitRegistered {
                    execution: NodeExecutionId::new("wait-execution-family")?,
                    condition: WaitCondition::Signal {
                        signal_type: SignalTypeId::new("signal.type.family")?,
                        correlation: None,
                    },
                },
            )?,
        ),
        (
            "signal",
            envelope(
                11,
                RunEventKind::SignalReceived {
                    signal: SignalId::new("signal-family")?,
                    signal_type: SignalTypeId::new("signal.type.family")?,
                    correlation: None,
                    mode: SignalDeliveryMode::OneShot,
                    payload: milkdrift_capability::BoundedJson::new(json!({"approved": true}))?,
                },
            )?,
        ),
        (
            "timer",
            envelope(
                12,
                RunEventKind::TimerFired {
                    timer: TimerId::new("timer-family")?,
                    observed_at: TimestampMillis::new(1_700_000_003_000),
                },
            )?,
        ),
        (
            "subworkflow",
            envelope(
                13,
                RunEventKind::SubworkflowTerminal {
                    subworkflow: SubworkflowId::new("subworkflow-family")?,
                    child_run: RunId::new("run-child-family")?,
                    outcome: RunOutcome::Succeeded,
                    outputs: Vec::new(),
                    cost_micros: BTreeMap::new(),
                    usage: Default::default(),
                },
            )?,
        ),
        (
            "recovery",
            envelope(
                14,
                RunEventKind::RecoveryStarted {
                    controller: WorkerId::new("controller-family")?,
                    through_sequence: RunSequence::new(13),
                },
            )?,
        ),
        (
            "reconciliation",
            envelope(
                15,
                RunEventKind::RevisionAdoptionRequested {
                    reconciliation: ReconciliationId::new("reconciliation-family")?,
                    requested_by: None,
                    from_revision: revision('0')?,
                    to_revision: revision('1')?,
                    policy: ReconciliationPolicy::RequireAuthority,
                },
            )?,
        ),
        (
            "reconciliation-remediation",
            envelope(
                16,
                RunEventKind::ReconciliationRemediationCreated {
                    plan: ReconciliationPlanId::new("plan-family")?,
                    source_execution: NodeExecutionId::new("execution-family")?,
                    source_attempt: Some(AttemptId::new("attempt-family")?),
                    execution: NodeExecutionId::new("execution-remediation-family")?,
                    node: milkdrift_blueprint::NodeId::new("node-remediation-family")?,
                    scope: scope(&run, "scope-fixture")?,
                    mode: NodeExecutionMode::Executor,
                    reason: Reason::new("preserve prior truth with new work")?,
                },
            )?,
        ),
    ])
}

#[test]
fn durable_event_families_have_reviewable_golden_examples() -> TestResult {
    for (name, event) in fixtures()? {
        let fixture: &[u8] = match name {
            "revision-control" => include_bytes!("fixtures/run-event-revision-control-v1.json"),
            "node-output" => include_bytes!("fixtures/run-event-node-output-v1.json"),
            "lease" => include_bytes!("fixtures/run-event-lease-v1.json"),
            "retry" => include_bytes!("fixtures/run-event-retry-v1.json"),
            "cancellation" => include_bytes!("fixtures/run-event-cancellation-v1.json"),
            "uncertainty" => include_bytes!("fixtures/run-event-uncertainty-v1.json"),
            "branch" => include_bytes!("fixtures/run-event-branch-v1.json"),
            "join" => include_bytes!("fixtures/run-event-join-v1.json"),
            "repeat" => include_bytes!("fixtures/run-event-repeat-v1.json"),
            "wait" => include_bytes!("fixtures/run-event-wait-v1.json"),
            "signal" => include_bytes!("fixtures/run-event-signal-v1.json"),
            "timer" => include_bytes!("fixtures/run-event-timer-v1.json"),
            "subworkflow" => include_bytes!("fixtures/run-event-subworkflow-v1.json"),
            "recovery" => include_bytes!("fixtures/run-event-recovery-v1.json"),
            "reconciliation" => include_bytes!("fixtures/run-event-reconciliation-v1.json"),
            "reconciliation-remediation" => {
                include_bytes!("fixtures/run-event-reconciliation-remediation-v1.json")
            }
            _ => return Err(format!("missing fixture routing for {name}").into()),
        };
        let fixture = fixture.strip_suffix(b"\n").unwrap_or(fixture);
        assert_eq!(
            event.to_canonical_json()?,
            fixture,
            "fixture {name} changed"
        );
        assert_eq!(RunEventEnvelope::from_json(fixture)?, event);
    }
    Ok(())
}
