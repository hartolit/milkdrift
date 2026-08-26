use std::collections::BTreeSet;

use milkdrift_authority::{
    ActorRef, AuthorityBudget, AuthorityOperation, AuthorityRequest, BoundaryTimeMillis,
    DecisionId, GrantId, RequestedResourceFacts,
};
use milkdrift_blueprint::{NodeId, RevisionId, WorkflowId};
use milkdrift_capability::{
    BoundedJson, CancellationAcknowledgement, InvocationEvent, InvocationTerminal,
};
use milkdrift_persistence::{
    AttemptId, AuthorityDecision, CommandId, CommandReceipt, CorrelationKey, EvidenceReference,
    LeaseId, MAX_COMMAND_DOCUMENT_BYTES, MAX_REPEAT_CONTINUATION_ADDITIONAL_ITERATIONS,
    NodeExecutionId, Reason, ReconciliationDecisionId, ReconciliationId, ReconciliationPlanId,
    ReconciliationPolicy, RepeatContinuationDecision, RepeatDecisionId, RunSequence,
    SignalDeliveryMode, SignalId, SignalTypeId, TimerId, TimestampMillis, WorkerId,
};
use milkdrift_workspace::{RunId, WorkspaceBudget, WorkspaceScope, WorkspaceValueEntry};
use serde::{Deserialize, Serialize};

use milkdrift_contracts::{CanonicalJsonError, JsonLimits};

use crate::RuntimeError;

/// Current closed runtime-command document schema.
pub const RUN_COMMAND_SCHEMA_VERSION_V1: u32 = 1;
/// Current authorization wrapper schema around an unchanged command-v1 document.
pub const AUTHORIZED_RUN_COMMAND_SCHEMA_VERSION_V1: u32 = 1;
/// Maximum events/references carried directly by one command.
pub const MAX_COMMAND_ITEMS: usize = 512;
const COMMAND_JSON_LIMITS: JsonLimits = JsonLimits {
    maximum_depth: 64,
    maximum_string_bytes: 65_536,
    maximum_key_bytes: 65_536,
    maximum_container_items: 4_096,
};

/// Explicit operator/controller action for retained or uncertain external work.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ExternalWorkAction {
    /// Query the external system using a separately authorized capability.
    Query,
    /// Create a new attempt when the side-effect/idempotency policy permits it.
    Retry,
    /// Create explicit compensating or remediation work.
    Compensate,
    /// Keep the obligation unresolved and visible.
    Retain,
    /// Resolve as succeeded from supplied evidence.
    ResolveSucceeded,
    /// Resolve as failed from supplied evidence.
    ResolveFailed,
}

/// Authenticated report submitted by an internal worker through runtime validation.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case", tag = "type", deny_unknown_fields)]
pub enum WorkerReport {
    /// Worker accepted a previously durable lease.
    LeaseAccepted {
        /// Exact lease.
        lease: LeaseId,
        /// Immutable attempt.
        attempt: AttemptId,
    },
    /// Extend a valid lease using a boundary-clock fact.
    Heartbeat {
        /// Exact lease.
        lease: LeaseId,
        /// New expiration.
        expires_at: TimestampMillis,
    },
    /// Executor admission/start was observed.
    Started {
        /// Immutable attempt.
        attempt: AttemptId,
    },
    /// One bounded, sequenced progress/output/terminal executor report.
    Invocation {
        /// Immutable attempt.
        attempt: AttemptId,
        /// Provider-neutral report.
        report: InvocationEvent,
    },
    /// Cancellation support/acceptance observation.
    Cancellation {
        /// Immutable attempt.
        attempt: AttemptId,
        /// Provider-neutral acknowledgement.
        acknowledgement: CancellationAcknowledgement,
    },
    /// Terminal report supplied directly by a worker boundary.
    Terminal {
        /// Immutable attempt.
        attempt: AttemptId,
        /// Monotonic worker report sequence.
        report_sequence: u64,
        /// Terminal observation.
        terminal: InvocationTerminal,
    },
}

/// Exact runtime-owned cause of one internal transition plan.
///
/// These values are durable audit facts. They are not accepted through the external
/// command planner; only trusted runtime services may commit them after deriving the
/// corresponding event family from authoritative state.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case", tag = "type", deny_unknown_fields)]
pub enum SystemTransition {
    /// A runnable execution was admitted and leased to one worker.
    ScheduleAndLease {
        /// Immutable attempt receiving the lease.
        attempt: AttemptId,
    },
    /// A previously entered external boundary lost a provable terminal result.
    DispatchOutcomeUncertain {
        /// Affected immutable attempt.
        attempt: AttemptId,
    },
    /// A deterministic immutable request failed before external dispatch.
    TerminalizePreDispatchFailure {
        /// Logical execution terminalized by the runtime.
        execution: NodeExecutionId,
    },
    /// The runtime advanced deterministic structured-control state.
    DriveStructuredProgress,
    /// A cancelled execution was restarted under an applied prospective revision.
    RestartReconciledExecution,
    /// A terminal child workflow was observed and its declared outputs were imported.
    ObserveChildTerminal,
    /// Nonterminal durable state was classified after startup or lease expiry.
    RecoverNonterminalRun,
    /// Durable cancellation intent was propagated through structured children.
    PropagateStructuredCancellation,
}

impl SystemTransition {
    /// Stable audit label used by tracing and command results.
    #[must_use]
    pub const fn label(&self) -> &'static str {
        match self {
            Self::ScheduleAndLease { .. } => "schedule_and_lease",
            Self::DispatchOutcomeUncertain { .. } => "dispatch_outcome_uncertain",
            Self::TerminalizePreDispatchFailure { .. } => "terminalize_pre_dispatch_failure",
            Self::DriveStructuredProgress => "drive_structured_progress",
            Self::RestartReconciledExecution => "restart_reconciled_execution",
            Self::ObserveChildTerminal => "observe_child_terminal",
            Self::RecoverNonterminalRun => "recover_nonterminal_run",
            Self::PropagateStructuredCancellation => "propagate_structured_cancellation",
        }
    }
}

/// Closed version-one set of requested run transitions.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case", tag = "type", deny_unknown_fields)]
pub enum RunCommand {
    /// Create a run pinned to one exact durable revision and bounded input values.
    CreateRun {
        /// Expected workflow lineage.
        workflow: WorkflowId,
        /// Exact immutable revision.
        revision: RevisionId,
        /// Validated run-root scope.
        root_scope: WorkspaceScope,
        /// Immutable workspace and artifact accounting limits.
        workspace_budget: WorkspaceBudget,
        /// Exact immutable initial workspace values.
        inputs: Vec<WorkspaceValueEntry>,
    },
    /// Start a created run.
    StartRun,
    /// Pause new admission/dispatch while retaining durable state.
    PauseRun,
    /// Resume a paused run.
    ResumeRun,
    /// Record durable run and structured-child cancellation intent.
    RequestCancellation,
    /// Deliver one typed, correlated, idempotent external signal.
    DeliverSignal {
        /// Stable delivery identity.
        signal: SignalId,
        /// Typed signal contract.
        signal_type: SignalTypeId,
        /// Optional matching key.
        correlation: Option<CorrelationKey>,
        /// One-shot or broadcast consumption shape.
        mode: SignalDeliveryMode,
        /// Bounded typed payload.
        payload: BoundedJson,
    },
    /// Observe that one durable timer is due.
    FireTimer {
        /// Timer identity.
        timer: TimerId,
    },
    /// Request a persisted prospective reconciliation plan.
    RequestRevisionAdoption {
        /// Stable request identity.
        reconciliation: ReconciliationId,
        /// Exact requested immutable revision.
        revision: RevisionId,
        /// Explicit prospective policy.
        policy: ReconciliationPolicy,
    },
    /// Record an authority decision over an immutable plan.
    DecideReconciliation {
        /// Immutable plan.
        plan: ReconciliationPlanId,
        /// Decision idempotency identity within the immutable plan.
        decision: ReconciliationDecisionId,
        /// Closed decision.
        outcome: AuthorityDecision,
    },
    /// Apply an approved/non-conflicting plan at its exact stale-safe boundary.
    ApplyReconciliation {
        /// Immutable plan.
        plan: ReconciliationPlanId,
    },
    /// Decide whether a repeat at an approval boundary may continue.
    DecideRepeatContinuation {
        /// Repeat execution awaiting authority.
        repeat_execution: NodeExecutionId,
        /// Decision idempotency identity within the repeat execution.
        decision: RepeatDecisionId,
        /// Closed approval/rejection outcome.
        outcome: RepeatContinuationDecision,
        /// Bounded additional iterations, present only for approval.
        approved_additional_iterations: Option<u32>,
    },
    /// Explicitly act on retained/uncertain external work.
    ResolveExternalWork {
        /// Attempt whose external result remains retained/uncertain.
        attempt: AttemptId,
        /// Decision idempotency identity within the retained attempt.
        decision: ReconciliationDecisionId,
        /// Authorized action.
        action: ExternalWorkAction,
        /// Exact remediation target, present only for compensation/remediation.
        remediation_node: Option<NodeId>,
    },
    /// Runtime-owned internal transition cause.
    ///
    /// External callers cannot submit this variant through the command planner.
    SystemTransition {
        /// Exact closed system transition.
        transition: SystemTransition,
    },
    /// Authenticated internal worker report; runtime remains the transition owner.
    WorkerReport {
        /// Exact worker/controller identity.
        worker: WorkerId,
        /// Closed report body.
        report: WorkerReport,
    },
}

/// Exact grant revision and revocation generation presented with an external command.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CommandAuthorityClaim {
    grant: GrantId,
    grant_revision: u64,
    revocation_generation: u64,
}

impl CommandAuthorityClaim {
    /// Constructs an exact nonzero grant revision claim.
    pub fn new(
        grant: GrantId,
        grant_revision: u64,
        revocation_generation: u64,
    ) -> Result<Self, RuntimeError> {
        if grant_revision == 0 {
            return Err(RuntimeError::InvalidCommand(
                "authority grant revision must be nonzero".to_owned(),
            ));
        }
        Ok(Self {
            grant,
            grant_revision,
            revocation_generation,
        })
    }

    /// Exact grant lineage.
    #[must_use]
    pub const fn grant(&self) -> &GrantId {
        &self.grant
    }

    /// Exact immutable grant revision.
    #[must_use]
    pub const fn grant_revision(&self) -> u64 {
        self.grant_revision
    }

    /// Exact revocation generation observed by the caller boundary.
    #[must_use]
    pub const fn revocation_generation(&self) -> u64 {
        self.revocation_generation
    }
}

/// Versioned command envelope binding intent to identity, actor, aggregate, guard, and clock fact.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RunCommandDocument {
    schema_version: u32,
    command_id: CommandId,
    run_id: RunId,
    actor: ActorRef,
    expected_sequence: RunSequence,
    issued_at: TimestampMillis,
    reason: Reason,
    evidence: Vec<EvidenceReference>,
    command: RunCommand,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RunCommandDocumentWire {
    schema_version: u32,
    command_id: CommandId,
    run_id: RunId,
    actor: ActorRef,
    expected_sequence: RunSequence,
    issued_at: TimestampMillis,
    reason: Reason,
    evidence: Vec<EvidenceReference>,
    command: RunCommand,
}

#[derive(Serialize)]
struct RunCommandIntent<'a> {
    schema_version: u32,
    reason: &'a Reason,
    evidence: &'a [EvidenceReference],
    command: &'a RunCommand,
}

#[derive(Serialize)]
struct AuthorizedCommandAudit<'a> {
    schema_version: u32,
    authority: &'a CommandAuthorityClaim,
    command: &'a RunCommandDocument,
}

#[derive(Serialize)]
struct AuthorizedCommandIntent<'a> {
    schema_version: u32,
    authority: &'a CommandAuthorityClaim,
    command: RunCommandIntent<'a>,
}

impl RunCommandDocument {
    /// Constructs and validates a complete command envelope.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        command_id: CommandId,
        run_id: RunId,
        actor: ActorRef,
        expected_sequence: RunSequence,
        issued_at: TimestampMillis,
        reason: Reason,
        evidence: Vec<EvidenceReference>,
        command: RunCommand,
    ) -> Result<Self, RuntimeError> {
        let document = Self {
            schema_version: RUN_COMMAND_SCHEMA_VERSION_V1,
            command_id,
            run_id,
            actor,
            expected_sequence,
            issued_at,
            reason,
            evidence,
            command,
        };
        document.validate()?;
        Ok(document)
    }

    /// Explicit command schema.
    #[must_use]
    pub const fn schema_version(&self) -> u32 {
        self.schema_version
    }

    /// Command/idempotency identity.
    #[must_use]
    pub const fn command_id(&self) -> &CommandId {
        &self.command_id
    }

    /// Exact owning aggregate.
    #[must_use]
    pub const fn run_id(&self) -> &RunId {
        &self.run_id
    }

    /// Issuing actor reference.
    #[must_use]
    pub const fn actor(&self) -> &ActorRef {
        &self.actor
    }

    /// Optimistic aggregate guard.
    #[must_use]
    pub const fn expected_sequence(&self) -> RunSequence {
        self.expected_sequence
    }

    /// Boundary-clock timestamp.
    #[must_use]
    pub const fn issued_at(&self) -> TimestampMillis {
        self.issued_at
    }

    /// Bounded structured transition rationale.
    #[must_use]
    pub const fn reason(&self) -> &Reason {
        &self.reason
    }

    /// Bounded supporting references.
    #[must_use]
    pub fn evidence(&self) -> &[EvidenceReference] {
        &self.evidence
    }

    /// Closed transition intent.
    #[must_use]
    pub const fn command(&self) -> &RunCommand {
        &self.command
    }

    /// Encodes deterministic compact JSON for durable idempotency evidence.
    pub fn to_canonical_json(&self) -> Result<Vec<u8>, RuntimeError> {
        canonical_json(self)
    }

    /// Bounds-checks and rejects unsupported future command schemas.
    pub fn from_json(bytes: &[u8]) -> Result<Self, RuntimeError> {
        if bytes.len() > MAX_COMMAND_DOCUMENT_BYTES {
            return Err(RuntimeError::InvalidCommand(format!(
                "command document exceeds {MAX_COMMAND_DOCUMENT_BYTES} bytes"
            )));
        }
        let value = milkdrift_contracts::parse_json_without_duplicates(bytes)?;
        let version = value
            .get("schema_version")
            .and_then(serde_json::Value::as_u64)
            .and_then(|value| u32::try_from(value).ok())
            .ok_or_else(|| {
                RuntimeError::InvalidCommand("missing numeric schema_version".to_owned())
            })?;
        if version != RUN_COMMAND_SCHEMA_VERSION_V1 {
            return Err(RuntimeError::UnsupportedCommandVersion {
                found: version,
                supported: RUN_COMMAND_SCHEMA_VERSION_V1,
            });
        }
        let wire: RunCommandDocumentWire = serde_json::from_value(value)?;
        let document = Self {
            schema_version: wire.schema_version,
            command_id: wire.command_id,
            run_id: wire.run_id,
            actor: wire.actor,
            expected_sequence: wire.expected_sequence,
            issued_at: wire.issued_at,
            reason: wire.reason,
            evidence: wire.evidence,
            command: wire.command,
        };
        document.validate()?;
        Ok(document)
    }

    /// Creates the persistence-owned receipt from exact audit bytes and semantic intent.
    ///
    /// Optimistic sequence and delivery timestamp remain in the retained document but are
    /// deliberately excluded from the idempotency fingerprint, allowing safe redelivery of
    /// the same command after an ordinary aggregate-head race.
    pub fn receipt(&self) -> Result<CommandReceipt, RuntimeError> {
        let intent = RunCommandIntent {
            schema_version: self.schema_version,
            reason: &self.reason,
            evidence: &self.evidence,
            command: &self.command,
        };
        Ok(CommandReceipt::new_idempotent(
            self.command_id.clone(),
            self.run_id.clone(),
            self.actor.clone(),
            self.expected_sequence,
            self.issued_at,
            self.to_canonical_json()?,
            canonical_json(&intent)?,
        )?)
    }

    /// Creates a receipt whose audit and idempotency bytes include the exact authority claim.
    pub fn authorized_receipt(
        &self,
        authority: &CommandAuthorityClaim,
    ) -> Result<CommandReceipt, RuntimeError> {
        let intent = RunCommandIntent {
            schema_version: self.schema_version,
            reason: &self.reason,
            evidence: &self.evidence,
            command: &self.command,
        };
        let audit = AuthorizedCommandAudit {
            schema_version: AUTHORIZED_RUN_COMMAND_SCHEMA_VERSION_V1,
            authority,
            command: self,
        };
        let authorized_intent = AuthorizedCommandIntent {
            schema_version: AUTHORIZED_RUN_COMMAND_SCHEMA_VERSION_V1,
            authority,
            command: intent,
        };
        Ok(CommandReceipt::new_idempotent(
            self.command_id.clone(),
            self.run_id.clone(),
            self.actor.clone(),
            self.expected_sequence,
            self.issued_at,
            canonical_json(&audit)?,
            canonical_json(&authorized_intent)?,
        )?)
    }

    /// Derives exact typed authorization facts from this closed external command.
    pub fn authority_request(
        &self,
        claim: &CommandAuthorityClaim,
    ) -> Result<AuthorityRequest, RuntimeError> {
        let (operation, workflow, budget) = match &self.command {
            RunCommand::CreateRun {
                workflow,
                workspace_budget,
                ..
            } => (
                AuthorityOperation::CreateRun,
                Some(workflow.clone()),
                AuthorityBudget {
                    artifact_bytes: Some(workspace_budget.max_total_artifact_bytes()),
                    ..AuthorityBudget::default()
                },
            ),
            RunCommand::StartRun => (
                AuthorityOperation::StartRun,
                None,
                AuthorityBudget::default(),
            ),
            RunCommand::PauseRun => (AuthorityOperation::Pause, None, AuthorityBudget::default()),
            RunCommand::ResumeRun => (AuthorityOperation::Resume, None, AuthorityBudget::default()),
            RunCommand::RequestCancellation => {
                (AuthorityOperation::Cancel, None, AuthorityBudget::default())
            }
            RunCommand::DeliverSignal { .. } => (
                AuthorityOperation::DeliverSignal,
                None,
                AuthorityBudget::default(),
            ),
            RunCommand::FireTimer { .. } => (
                AuthorityOperation::FireTimer,
                None,
                AuthorityBudget::default(),
            ),
            RunCommand::RequestRevisionAdoption { .. } => (
                AuthorityOperation::Propose,
                None,
                AuthorityBudget::default(),
            ),
            RunCommand::DecideReconciliation { .. }
            | RunCommand::DecideRepeatContinuation { .. } => (
                AuthorityOperation::Approve,
                None,
                AuthorityBudget::default(),
            ),
            RunCommand::ApplyReconciliation { .. } => {
                (AuthorityOperation::Apply, None, AuthorityBudget::default())
            }
            RunCommand::ResolveExternalWork { action, .. } => (
                match action {
                    ExternalWorkAction::Retry => AuthorityOperation::Retry,
                    ExternalWorkAction::Query => AuthorityOperation::Inspect,
                    ExternalWorkAction::Compensate => AuthorityOperation::Apply,
                    ExternalWorkAction::Retain => AuthorityOperation::Approve,
                    ExternalWorkAction::ResolveSucceeded | ExternalWorkAction::ResolveFailed => {
                        AuthorityOperation::Terminate
                    }
                },
                None,
                AuthorityBudget::default(),
            ),
            RunCommand::SystemTransition { .. } | RunCommand::WorkerReport { .. } => {
                return Err(RuntimeError::InvalidCommand(
                    "internal system transitions and worker reports cannot be submitted through the external command API"
                        .to_owned(),
                ));
            }
        };
        let mut resources = RequestedResourceFacts::empty();
        resources.workflow = workflow;
        resources.run = Some(self.run_id.clone());
        let digest = blake3::hash(self.command_id.as_str().as_bytes());
        Ok(AuthorityRequest {
            decision: DecisionId::new(format!("decision:{digest}"))?,
            actor: self.actor.clone(),
            grant: claim.grant.clone(),
            grant_revision: claim.grant_revision,
            revocation_generation: claim.revocation_generation,
            operation,
            resources,
            budget,
            evaluated_at: BoundaryTimeMillis::new(self.issued_at.get()),
        })
    }

    fn validate(&self) -> Result<(), RuntimeError> {
        if self.schema_version != RUN_COMMAND_SCHEMA_VERSION_V1 {
            return Err(RuntimeError::UnsupportedCommandVersion {
                found: self.schema_version,
                supported: RUN_COMMAND_SCHEMA_VERSION_V1,
            });
        }
        if self.evidence.len() > 32 {
            return Err(RuntimeError::InvalidCommand(
                "a command may carry at most 32 evidence references".to_owned(),
            ));
        }
        let mut evidence_ids = BTreeSet::new();
        if !self
            .evidence
            .iter()
            .all(|item| evidence_ids.insert(&item.id))
        {
            return Err(RuntimeError::InvalidCommand(
                "a command cannot repeat an evidence identity".to_owned(),
            ));
        }
        match &self.command {
            RunCommand::CreateRun {
                root_scope, inputs, ..
            } => {
                if self.expected_sequence != RunSequence::ZERO {
                    return Err(RuntimeError::InvalidCommand(
                        "create_run requires expected sequence zero".to_owned(),
                    ));
                }
                if root_scope.reference().run() != &self.run_id
                    || !root_scope.kind().is_run_root()
                    || root_scope.parent().is_some()
                {
                    return Err(RuntimeError::InvalidCommand(
                        "create_run root scope must be the parentless root of this run".to_owned(),
                    ));
                }
                if inputs.len() > MAX_COMMAND_ITEMS
                    || inputs.iter().any(|entry| {
                        entry.reference().scope() != root_scope.reference()
                            || entry.reference().scope().run() != &self.run_id
                    })
                {
                    return Err(RuntimeError::InvalidCommand(
                        "create_run inputs must be bounded values in this run's root scope"
                            .to_owned(),
                    ));
                }
            }
            RunCommand::WorkerReport {
                report:
                    WorkerReport::Terminal {
                        report_sequence: 0, ..
                    },
                ..
            } => {
                return Err(RuntimeError::InvalidCommand(
                    "worker report sequences begin at one".to_owned(),
                ));
            }
            RunCommand::DecideRepeatContinuation {
                outcome,
                approved_additional_iterations,
                ..
            } => {
                let valid = match (outcome, approved_additional_iterations) {
                    (RepeatContinuationDecision::Approved, Some(additional)) => {
                        (1..=MAX_REPEAT_CONTINUATION_ADDITIONAL_ITERATIONS).contains(additional)
                    }
                    (RepeatContinuationDecision::Rejected, None) => true,
                    (RepeatContinuationDecision::Approved, None)
                    | (RepeatContinuationDecision::Rejected, Some(_)) => false,
                };
                if !valid {
                    return Err(RuntimeError::InvalidCommand(format!(
                        "repeat approval requires 1..={MAX_REPEAT_CONTINUATION_ADDITIONAL_ITERATIONS} additional iterations and rejection forbids them"
                    )));
                }
            }
            RunCommand::ResolveExternalWork {
                action,
                remediation_node,
                ..
            } if (*action == ExternalWorkAction::Compensate) != remediation_node.is_some() => {
                return Err(RuntimeError::InvalidCommand(
                    "compensate requires exactly one remediation node and other external-work actions forbid it"
                        .to_owned(),
                ));
            }
            RunCommand::ResolveExternalWork {
                action: ExternalWorkAction::ResolveSucceeded | ExternalWorkAction::ResolveFailed,
                ..
            } if self.evidence.is_empty() => {
                return Err(RuntimeError::InvalidCommand(
                    "resolving uncertain external work as succeeded or failed requires durable evidence"
                        .to_owned(),
                ));
            }
            _ => {}
        }
        let _ = self.to_canonical_json()?;
        Ok(())
    }
}

fn canonical_json<T: Serialize>(value: &T) -> Result<Vec<u8>, RuntimeError> {
    let bytes =
        milkdrift_contracts::canonical_json_bytes(value, COMMAND_JSON_LIMITS).map_err(|error| {
            match error {
                CanonicalJsonError::Json(error) => RuntimeError::Json(error),
                CanonicalJsonError::Bounds(violation) => RuntimeError::InvalidCommand(format!(
                    "command JSON bound exceeded at {} for {:?} (maximum {})",
                    violation.path(),
                    violation.kind(),
                    violation.maximum()
                )),
            }
        })?;
    if bytes.len() > MAX_COMMAND_DOCUMENT_BYTES {
        return Err(RuntimeError::InvalidCommand(format!(
            "command document exceeds {MAX_COMMAND_DOCUMENT_BYTES} bytes"
        )));
    }
    Ok(bytes)
}

impl ExternalWorkAction {
    /// Returns the durable closed decision fact corresponding to this requested action.
    #[must_use]
    pub const fn authority_decision(self) -> AuthorityDecision {
        match self {
            Self::Retry => AuthorityDecision::Retry,
            Self::Compensate => AuthorityDecision::Compensate,
            Self::Retain => AuthorityDecision::Retain,
            Self::Query => AuthorityDecision::Query,
            Self::ResolveSucceeded => AuthorityDecision::ResolveSucceeded,
            Self::ResolveFailed => AuthorityDecision::ResolveFailed,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn repeat_document(
        outcome: RepeatContinuationDecision,
        approved_additional_iterations: Option<u32>,
    ) -> Result<RunCommandDocument, RuntimeError> {
        RunCommandDocument::new(
            CommandId::new("command-repeat")?,
            RunId::new("run-repeat")
                .map_err(|error| RuntimeError::InvalidCommand(error.to_string()))?,
            ActorRef::new("operator")?,
            RunSequence::new(7),
            TimestampMillis::new(1_000),
            Reason::new("repeat authority")?,
            Vec::new(),
            RunCommand::DecideRepeatContinuation {
                repeat_execution: NodeExecutionId::new("execution-repeat")?,
                decision: RepeatDecisionId::new("decision-repeat")?,
                outcome,
                approved_additional_iterations,
            },
        )
    }

    fn external_work_document(
        action: ExternalWorkAction,
        remediation_node: Option<&str>,
    ) -> Result<RunCommandDocument, RuntimeError> {
        let remediation_node = remediation_node
            .map(NodeId::new)
            .transpose()
            .map_err(|error| RuntimeError::InvalidCommand(error.to_string()))?;
        RunCommandDocument::new(
            CommandId::new("command-external-work")?,
            RunId::new("run-external-work")
                .map_err(|error| RuntimeError::InvalidCommand(error.to_string()))?,
            ActorRef::new("operator")?,
            RunSequence::new(7),
            TimestampMillis::new(1_000),
            Reason::new("external-work authority")?,
            Vec::new(),
            RunCommand::ResolveExternalWork {
                attempt: AttemptId::new("attempt-external-work")?,
                decision: ReconciliationDecisionId::new("decision-external-work")?,
                action,
                remediation_node,
            },
        )
    }

    #[test]
    fn rejects_duplicate_json_keys_before_value_decoding() {
        let duplicate = br#"{"schema_version":1,"schema_version":1}"#;
        assert!(matches!(
            RunCommandDocument::from_json(duplicate),
            Err(error) if error.to_string().contains("duplicate JSON object key")
        ));

        let nested = br#"{"schema_version":1,"command":{"type":"start_run","type":"start_run"}}"#;
        assert!(matches!(
            RunCommandDocument::from_json(nested),
            Err(error) if error.to_string().contains("duplicate JSON object key")
        ));
    }

    #[test]
    fn repeat_continuation_command_has_a_closed_bounded_shape() {
        assert!(repeat_document(RepeatContinuationDecision::Approved, Some(1)).is_ok());
        assert!(
            repeat_document(
                RepeatContinuationDecision::Approved,
                Some(MAX_REPEAT_CONTINUATION_ADDITIONAL_ITERATIONS)
            )
            .is_ok()
        );
        assert!(repeat_document(RepeatContinuationDecision::Rejected, None).is_ok());
        assert!(repeat_document(RepeatContinuationDecision::Approved, None).is_err());
        assert!(repeat_document(RepeatContinuationDecision::Rejected, Some(1)).is_err());
        assert!(
            repeat_document(
                RepeatContinuationDecision::Approved,
                Some(MAX_REPEAT_CONTINUATION_ADDITIONAL_ITERATIONS + 1)
            )
            .is_err()
        );
    }

    #[test]
    fn external_work_query_and_remediation_have_truthful_closed_shapes() {
        assert_eq!(
            ExternalWorkAction::Query.authority_decision(),
            AuthorityDecision::Query
        );
        assert!(external_work_document(ExternalWorkAction::Query, None).is_ok());
        assert!(external_work_document(ExternalWorkAction::Query, Some("query-node")).is_err());
        assert!(external_work_document(ExternalWorkAction::Compensate, None).is_err());
        assert!(
            external_work_document(ExternalWorkAction::Compensate, Some("remediation-node"))
                .is_ok()
        );
    }
}
