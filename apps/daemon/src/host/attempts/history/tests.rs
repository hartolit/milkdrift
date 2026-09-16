//! Historical inspection must keep an occurrence's original revision and a bounded page.

use super::*;
use milkdrift_blueprint::{ContentDigest, NodeId};
use milkdrift_capability::{
    CapabilityId, ErrorClass, InvocationId, InvocationRequest, OperationId,
};
use milkdrift_persistence::{EventId, NodeExecutionMode, NodeOutcome, Reason, TimestampMillis};
use milkdrift_workspace::{ScopeId, WorkspaceScope};

type TestValue<T> = Result<T, Box<dyn std::error::Error>>;
type TestResult = TestValue<()>;

fn revision(character: char) -> Result<RevisionId, serde_json::Error> {
    serde_json::from_str(&format!("\"rev_{}\"", character.to_string().repeat(64)))
}

fn event(sequence: u64, kind: RunEventKind) -> TestValue<RunEventEnvelope> {
    Ok(RunEventEnvelope::new(
        EventId::new(format!("history-{sequence}"))?,
        RunId::new("history-run")?,
        RunSequence::new(sequence),
        TimestampMillis::new(sequence),
        kind,
    )?)
}

fn eligible(sequence: u64, execution: &NodeExecutionId) -> TestValue<RunEventEnvelope> {
    event(
        sequence,
        RunEventKind::NodeBecameEligible {
            node: NodeId::new(if execution.as_str() == "original-execution" {
                "original-task"
            } else {
                "unrelated-task"
            })?,
            execution: execution.clone(),
            scope: WorkspaceScope::run_root(RunId::new("history-run")?, ScopeId::new("root")?)
                .reference()
                .clone(),
            mode: NodeExecutionMode::Executor,
        },
    )
}

fn pin(sequence: u64, revision: RevisionId) -> TestValue<RunEventEnvelope> {
    event(
        sequence,
        RunEventKind::RevisionPinned {
            previous: revision.clone(),
            revision,
            plan: milkdrift_persistence::ReconciliationPlanId::new("historical-plan")?,
            revision_digest: serde_json::from_str::<ContentDigest>(&format!(
                "\"b3_{}\"",
                "1".repeat(64)
            ))?,
        },
    )
}

#[test]
fn original_revision_survives_unrelated_history_and_retry_across_repin() -> TestResult {
    let attempt = AttemptId::new("retry-attempt")?;
    let execution = NodeExecutionId::new("original-execution")?;
    let mut full = HistoricalAttemptState::new(attempt.clone(), execution.clone());
    let mut anchored = HistoricalAttemptState::new(attempt.clone(), execution.clone());
    anchored.current_revision = Some(revision('a')?);
    full.fold(&pin(1, revision('a')?)?)
        .map_err(|error| error.message)?;
    for state in [&mut full, &mut anchored] {
        state
            .fold(&eligible(2, &execution)?)
            .map_err(|error| error.message)?;
        for number in 3..1_003 {
            state
                .fold(&eligible(
                    number,
                    &NodeExecutionId::new(format!("unrelated-{number}"))?,
                )?)
                .map_err(|error| error.message)?;
        }
        state
            .fold(&pin(1_003, revision('b')?)?)
            .map_err(|error| error.message)?;
        let timer = TimerId::new("retry-timer")?;
        state
            .fold(&event(
                1_004,
                RunEventKind::NodeRetryScheduled {
                    execution: execution.clone(),
                    previous_attempt: AttemptId::new("first-attempt")?,
                    next_attempt: attempt.clone(),
                    attempt_number: 2,
                    timer: timer.clone(),
                    fire_at: TimestampMillis::new(1_005),
                    error_class: ErrorClass::Transport,
                    reason: Reason::new("retry after transport failure")?,
                },
            )?)
            .map_err(|error| error.message)?;
        assert_eq!(
            state
                .located
                .as_ref()
                .ok_or("attempt not located")?
                .value
                .state,
            "awaiting_retry_timer"
        );
        let waiting = state.located.as_ref().ok_or("retry owner not located")?;
        assert_eq!(waiting.node_id, "original-task");
        assert_eq!(waiting.revision_id, revision('a')?.as_str());
        state
            .fold(&event(
                1_005,
                RunEventKind::TimerFired {
                    timer,
                    observed_at: TimestampMillis::new(1_005),
                },
            )?)
            .map_err(|error| error.message)?;
        assert_eq!(
            state
                .located
                .as_ref()
                .ok_or("attempt not located")?
                .value
                .state,
            "ready_to_schedule"
        );
        let invocation = InvocationId::new("retry-invocation")?;
        state
            .fold(&event(
                1_006,
                RunEventKind::NodeScheduled {
                    node: NodeId::new("original-task")?,
                    execution: execution.clone(),
                    attempt: attempt.clone(),
                    invocation: invocation.clone(),
                    idempotency_key: None,
                    request: InvocationRequest::new(
                        invocation,
                        CapabilityId::new("test-process")?,
                        OperationId::new("process.execute")?,
                        None,
                        None,
                        vec![],
                        std::collections::BTreeMap::new(),
                    )?,
                },
            )?)
            .map_err(|error| error.message)?;
        state
            .fold(&event(
                1_007,
                RunEventKind::NodeTerminal {
                    execution: execution.clone(),
                    attempt: attempt.clone(),
                    report_sequence: 1,
                    outcome: NodeOutcome::Succeeded,
                    error_class: None,
                    detail: None,
                },
            )?)
            .map_err(|error| error.message)?;
    }
    let full = full.finish().map_err(|error| error.message)?;
    let anchored = anchored.finish().map_err(|error| error.message)?;
    assert_eq!(full.revision_id, revision('a')?.as_str());
    assert_eq!(full.node_id, "original-task");
    assert_eq!(full.value.terminal.as_deref(), Some("succeeded"));
    assert_eq!(full.revision_id, anchored.revision_id);
    assert_eq!(full.value, anchored.value);
    Ok(())
}

#[test]
fn journal_scan_obeys_exact_prefix_and_rejects_missing_or_discontinuous_pages() -> TestResult {
    let run = RunId::new("history-run")?;
    let mut requested = Vec::new();
    let mut visited = Vec::new();
    scan_history(
        |query| {
            let start = query
                .cursor
                .as_ref()
                .ok_or_else(super::super::super::read_model::internal)?
                .next_sequence
                .get();
            requested.push(query.limit.get());
            Ok(EventPage {
                events: (start..start + u64::from(query.limit.get()))
                    .map(|sequence| event(sequence, RunEventKind::RunStarted))
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(|_| super::super::super::read_model::internal())?,
                next: None,
                observed_head: RunSequence::new(1_000),
            })
        },
        &run,
        RunSequence::new(17),
        RunSequence::new(529),
        |event| {
            visited.push(event.sequence().get());
            Ok(false)
        },
    )
    .map_err(|error| error.message)?;
    assert_eq!(requested, [256, 256, 1]);
    assert_eq!(visited, (17..=529).collect::<Vec<_>>());
    for events in [vec![], vec![event(18, RunEventKind::RunStarted)?]] {
        let error = scan_history(
            |_| {
                Ok(EventPage {
                    events: events.clone(),
                    next: None,
                    observed_head: RunSequence::new(529),
                })
            },
            &run,
            RunSequence::new(17),
            RunSequence::new(529),
            |_| Ok(false),
        )
        .err()
        .ok_or("invalid page was accepted")?;
        assert_eq!(
            error.code,
            milkdrift_control_protocol::ErrorCode::Corruption
        );
    }
    Ok(())
}

#[test]
fn remediation_attempts_keep_their_creation_revision_after_later_pins() -> TestResult {
    let attempt = AttemptId::new("remediation-attempt")?;
    let execution = NodeExecutionId::new("remediation-execution")?;
    let node = NodeId::new("remediation-task")?;
    let scope = WorkspaceScope::run_root(RunId::new("history-run")?, ScopeId::new("root")?)
        .reference()
        .clone();
    let recovery = RunEventKind::RemediationWorkCreated {
        source_attempt: AttemptId::new("source-attempt")?,
        execution: execution.clone(),
        node: node.clone(),
        scope: scope.clone(),
        mode: NodeExecutionMode::Executor,
        decision: milkdrift_persistence::ReconciliationDecisionId::new("recovery-decision")?,
        reason: Reason::new("authorized recovery remediation")?,
    };
    let reconciliation = RunEventKind::ReconciliationRemediationCreated {
        plan: milkdrift_persistence::ReconciliationPlanId::new("historical-plan")?,
        source_execution: NodeExecutionId::new("source-execution")?,
        source_attempt: Some(AttemptId::new("source-attempt")?),
        execution: execution.clone(),
        node: node.clone(),
        scope,
        mode: NodeExecutionMode::Executor,
        reason: Reason::new("authorized prospective remediation")?,
    };
    for (creation, expected_revision) in
        [(recovery, revision('a')?), (reconciliation, revision('b')?)]
    {
        for anchored in [false, true] {
            let mut state = HistoricalAttemptState::new(attempt.clone(), execution.clone());
            if anchored {
                state.current_revision = Some(expected_revision.clone());
            } else {
                state
                    .fold(&pin(1, revision('a')?)?)
                    .map_err(|error| error.message)?;
            }
            state
                .fold(&event(2, creation.clone())?)
                .map_err(|error| error.message)?;
            // Reconciliation creates remediation before its target revision is pinned.
            state
                .fold(&pin(3, revision('b')?)?)
                .map_err(|error| error.message)?;
            state
                .fold(&pin(4, revision('c')?)?)
                .map_err(|error| error.message)?;
            let invocation = InvocationId::new("remediation-invocation")?;
            state
                .fold(&event(
                    5,
                    RunEventKind::NodeScheduled {
                        node: node.clone(),
                        execution: execution.clone(),
                        attempt: attempt.clone(),
                        invocation: invocation.clone(),
                        idempotency_key: None,
                        request: InvocationRequest::new(
                            invocation,
                            CapabilityId::new("test-process")?,
                            OperationId::new("process.execute")?,
                            None,
                            None,
                            vec![],
                            std::collections::BTreeMap::new(),
                        )?,
                    },
                )?)
                .map_err(|error| error.message)?;
            let located = state.finish().map_err(|error| error.message)?;
            assert_eq!(located.node_id, node.as_str());
            assert_eq!(located.revision_id, expected_revision.as_str());
        }
    }
    Ok(())
}
