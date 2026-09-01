use milkdrift_authority::ActorRef;
use milkdrift_blueprint::{RevisionId, WorkflowId};
use milkdrift_capability::BoundedJson;
use milkdrift_contracts::{CanonicalJsonError, JsonBoundKind, JsonLimits, canonical_json_bytes};
use milkdrift_persistence::{
    AttemptId, CorrelationKey, EvidenceReference, NodeExecutionId, PageSize,
    ReconciliationDecisionId, RepeatDecisionId, RunSequence, SignalDeliveryMode, SignalId,
    SignalTypeId, TimestampMillis,
};
use milkdrift_runtime::{CommandAuthorityClaim, ExternalWorkAction};
use milkdrift_workspace::{RunId, WorkspaceBudget, WorkspaceScope, WorkspaceValueEntry};
use serde::{Deserialize, Serialize};

use crate::{
    ControlError, ControlId, ControllerStatusRead, ProposalDigest, ProposalId, ProposalStatusRead,
    ProposalSubmission, RevisionInspection, RunInspection, TimelinePage, WorkflowProposalDocument,
};

/// Current versioned control-command schema.
pub const CONTROL_COMMAND_SCHEMA_VERSION_V1: u32 = 1;
const MAX_CONTROL_DOCUMENT_BYTES: usize = 1_310_720;
const CONTROL_JSON_LIMITS: JsonLimits = JsonLimits {
    maximum_depth: 72,
    maximum_string_bytes: 65_536,
    maximum_key_bytes: 192,
    maximum_container_items: 4_096,
};

/// Immutable authenticated actor/grant context supplied by a trusted caller boundary.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ActorAuthorityContext {
    actor: ActorRef,
    authority: CommandAuthorityClaim,
}

impl ActorAuthorityContext {
    /// Constructs exact actor and grant-revision context.
    #[must_use]
    pub const fn new(actor: ActorRef, authority: CommandAuthorityClaim) -> Self {
        Self { actor, authority }
    }

    /// Authenticated actor reference.
    #[must_use]
    pub const fn actor(&self) -> &ActorRef {
        &self.actor
    }

    /// Exact immutable authority claim.
    #[must_use]
    pub const fn authority(&self) -> &CommandAuthorityClaim {
        &self.authority
    }
}

/// Optimistic facts checked before any prospective state transition.
#[derive(Clone, Debug, Default, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct OptimisticGuard {
    /// Exact run sequence when a live run is involved.
    pub expected_run_sequence: Option<RunSequence>,
    /// Exact current/base revision when relevant.
    pub expected_revision: Option<RevisionId>,
    /// Exact proposal digest for approval/application/status commands.
    pub expected_proposal_digest: Option<ProposalDigest>,
}

/// Closed application-layer requests shared by human, service, and AI callers.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case", tag = "type", deny_unknown_fields)]
#[allow(clippy::large_enum_variant)] // Closed typed proposal data remains directly auditable.
pub enum ControlCommand {
    /// Inspect current bounded operational state.
    InspectRun {
        /// Exact run.
        run: RunId,
    },
    /// Inspect one exact immutable revision under workflow authority.
    InspectRevision {
        /// Exact immutable revision.
        revision: RevisionId,
    },
    /// Read one stable-cursor timeline page.
    InspectTimeline {
        /// Exact run.
        run: RunId,
        /// Inclusive starting sequence; absent begins at one.
        after: Option<RunSequence>,
        /// Bounded page size.
        limit: PageSize,
    },
    /// Inspect one exact durable controller occurrence.
    InspectController {
        /// Owning run.
        run: RunId,
        /// Exact logical controller execution.
        controller_execution: NodeExecutionId,
    },
    /// Parse, validate, authorize, classify, and store one proposal revision.
    SubmitProposal {
        /// Versioned untrusted proposal.
        proposal: WorkflowProposalDocument,
    },
    /// Record approval over the immutable plan for an exact proposed revision.
    ApproveProposal {
        /// Target run.
        run: RunId,
        /// Proposal identity.
        proposal: ProposalId,
        /// Exact proposal digest.
        proposal_digest: ProposalDigest,
        /// Exact proposed revision.
        proposed_revision: RevisionId,
        /// Idempotency identity scoped by the plan.
        decision: ReconciliationDecisionId,
    },
    /// Record rejection over the immutable plan for an exact proposed revision.
    RejectProposal {
        /// Target run.
        run: RunId,
        /// Proposal identity.
        proposal: ProposalId,
        /// Exact proposal digest.
        proposal_digest: ProposalDigest,
        /// Exact proposed revision.
        proposed_revision: RevisionId,
        /// Idempotency identity scoped by the plan.
        decision: ReconciliationDecisionId,
    },
    /// Apply an exact approved/non-conflicting plan through reconciliation.
    ApplyProposal {
        /// Target run.
        run: RunId,
        /// Proposal identity.
        proposal: ProposalId,
        /// Exact proposal digest.
        proposal_digest: ProposalDigest,
        /// Exact proposed revision.
        proposed_revision: RevisionId,
    },
    /// Query current proposal/reconciliation status.
    QueryProposal {
        /// Target run.
        run: RunId,
        /// Proposal identity.
        proposal: ProposalId,
        /// Exact proposed revision.
        proposed_revision: RevisionId,
    },
    /// Pause a running aggregate.
    PauseRun {
        /// Exact run.
        run: RunId,
    },
    /// Resume a paused aggregate.
    ResumeRun {
        /// Exact run.
        run: RunId,
    },
    /// Request durable run cancellation.
    RequestCancellation {
        /// Exact run.
        run: RunId,
    },
    /// Resolve retained/uncertain external work through the existing runtime command.
    ResolveExternalWork {
        /// Exact run.
        run: RunId,
        /// Exact attempt.
        attempt: AttemptId,
        /// Stable decision identity.
        decision: ReconciliationDecisionId,
        /// Closed existing runtime action.
        action: ExternalWorkAction,
        /// Optional remediation node for compensation.
        remediation_node: Option<milkdrift_blueprint::NodeId>,
    },
    /// Explicitly create a run pinned to one exact revision.
    CreateRun {
        /// New run aggregate.
        run: RunId,
        /// Expected workflow lineage.
        workflow: WorkflowId,
        /// Exact immutable revision.
        revision: RevisionId,
        /// Validated root scope.
        root_scope: WorkspaceScope,
        /// Immutable workspace budget.
        workspace_budget: WorkspaceBudget,
        /// Exact initial values.
        inputs: Vec<WorkspaceValueEntry>,
    },
    /// Start an explicitly created run.
    StartRun {
        /// Exact run.
        run: RunId,
    },
    /// Deliver one typed waiting-workflow signal.
    Signal {
        /// Exact run.
        run: RunId,
        /// Stable signal identity.
        signal: SignalId,
        /// Signal schema identity.
        signal_type: SignalTypeId,
        /// Optional correlation key.
        correlation: Option<CorrelationKey>,
        /// One-shot or broadcast mode.
        mode: SignalDeliveryMode,
        /// Bounded payload.
        payload: BoundedJson,
    },
    /// Continue one exact durable human checkpoint through ordinary approval authority.
    ContinueController {
        /// Owning run.
        run: RunId,
        /// Exact waiting controller occurrence.
        controller_execution: NodeExecutionId,
        /// Stable checkpoint decision identity.
        decision: RepeatDecisionId,
    },
}

/// Versioned command envelope retaining caller, idempotency, guard, reason, and evidence.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ControlCommandDocument {
    schema_version: u32,
    control_id: ControlId,
    context: ActorAuthorityContext,
    issued_at: TimestampMillis,
    guard: OptimisticGuard,
    reason: milkdrift_persistence::Reason,
    evidence: Vec<EvidenceReference>,
    command: ControlCommand,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ControlCommandWire {
    schema_version: u32,
    control_id: ControlId,
    context: ActorAuthorityContext,
    issued_at: TimestampMillis,
    guard: OptimisticGuard,
    reason: milkdrift_persistence::Reason,
    evidence: Vec<EvidenceReference>,
    command: ControlCommand,
}

impl ControlCommandDocument {
    /// Constructs a complete schema-v1 command envelope.
    #[allow(clippy::too_many_arguments)] // One validated control operation keeps its authority and optimistic facts explicit.
    pub fn new(
        control_id: ControlId,
        context: ActorAuthorityContext,
        issued_at: TimestampMillis,
        guard: OptimisticGuard,
        reason: milkdrift_persistence::Reason,
        evidence: Vec<EvidenceReference>,
        command: ControlCommand,
    ) -> Result<Self, ControlError> {
        let document = Self {
            schema_version: CONTROL_COMMAND_SCHEMA_VERSION_V1,
            control_id,
            context,
            issued_at,
            guard,
            reason,
            evidence,
            command,
        };
        document.validate()?;
        Ok(document)
    }

    fn validate(&self) -> Result<(), ControlError> {
        if self.schema_version != CONTROL_COMMAND_SCHEMA_VERSION_V1 {
            return Err(ControlError::UnsupportedVersion {
                document: "control_command",
                found: self.schema_version,
                supported: CONTROL_COMMAND_SCHEMA_VERSION_V1,
            });
        }
        if self.evidence.len() > 32 {
            return Err(ControlError::Bounds {
                location: "control.evidence".to_owned(),
                reason: "at most 32 references are allowed".to_owned(),
            });
        }
        let unique: std::collections::BTreeSet<_> =
            self.evidence.iter().map(|item| &item.id).collect();
        if unique.len() != self.evidence.len() {
            return Err(ControlError::InvalidContract(
                "control evidence identities must be distinct".to_owned(),
            ));
        }
        Ok(())
    }

    /// Current control schema.
    #[must_use]
    pub const fn schema_version(&self) -> u32 {
        self.schema_version
    }
    /// Caller-supplied idempotency identity.
    #[must_use]
    pub const fn control_id(&self) -> &ControlId {
        &self.control_id
    }
    /// Immutable actor/grant context.
    #[must_use]
    pub const fn context(&self) -> &ActorAuthorityContext {
        &self.context
    }
    /// Caller-supplied deterministic boundary time.
    #[must_use]
    pub const fn issued_at(&self) -> TimestampMillis {
        self.issued_at
    }
    pub(crate) fn with_trusted_issued_at(
        mut self,
        issued_at: TimestampMillis,
    ) -> Result<Self, ControlError> {
        self.issued_at = issued_at;
        self.validate()?;
        Ok(self)
    }
    /// Optimistic guards.
    #[must_use]
    pub const fn guard(&self) -> &OptimisticGuard {
        &self.guard
    }
    /// Bounded command reason.
    #[must_use]
    pub const fn reason(&self) -> &milkdrift_persistence::Reason {
        &self.reason
    }
    /// Bounded evidence references.
    #[must_use]
    pub fn evidence(&self) -> &[EvidenceReference] {
        &self.evidence
    }
    /// Closed command body.
    #[must_use]
    pub const fn command(&self) -> &ControlCommand {
        &self.command
    }

    /// Encodes deterministic canonical JSON.
    pub fn to_canonical_json(&self) -> Result<Vec<u8>, ControlError> {
        let bytes = canonical_json_bytes(self, CONTROL_JSON_LIMITS).map_err(map_canonical)?;
        if bytes.len() > MAX_CONTROL_DOCUMENT_BYTES {
            return Err(ControlError::Bounds {
                location: "$".to_owned(),
                reason: format!("document exceeds {MAX_CONTROL_DOCUMENT_BYTES} bytes"),
            });
        }
        Ok(bytes)
    }

    /// Strictly bounds, duplicate-checks, version-checks, and decodes a command.
    pub fn from_json(bytes: &[u8]) -> Result<Self, ControlError> {
        if bytes.len() > MAX_CONTROL_DOCUMENT_BYTES {
            return Err(ControlError::Bounds {
                location: "$".to_owned(),
                reason: format!("document exceeds {MAX_CONTROL_DOCUMENT_BYTES} bytes"),
            });
        }
        milkdrift_contracts::preflight_json_structure(bytes, CONTROL_JSON_LIMITS)
            .map_err(map_bound)?;
        let value = milkdrift_contracts::parse_json_without_duplicates(bytes)?;
        milkdrift_contracts::validate_json_value(&value, CONTROL_JSON_LIMITS).map_err(map_bound)?;
        let version = value
            .get("schema_version")
            .and_then(serde_json::Value::as_u64)
            .and_then(|value| u32::try_from(value).ok())
            .ok_or_else(|| {
                ControlError::InvalidContract("missing numeric schema_version".to_owned())
            })?;
        if version != CONTROL_COMMAND_SCHEMA_VERSION_V1 {
            return Err(ControlError::UnsupportedVersion {
                document: "control_command",
                found: version,
                supported: CONTROL_COMMAND_SCHEMA_VERSION_V1,
            });
        }
        let wire: ControlCommandWire = serde_json::from_value(value)?;
        if wire.schema_version != CONTROL_COMMAND_SCHEMA_VERSION_V1 {
            return Err(ControlError::UnsupportedVersion {
                document: "control_command",
                found: wire.schema_version,
                supported: CONTROL_COMMAND_SCHEMA_VERSION_V1,
            });
        }
        Self::new(
            wire.control_id,
            wire.context,
            wire.issued_at,
            wire.guard,
            wire.reason,
            wire.evidence,
            wire.command,
        )
    }
}

/// Stable application-layer output returned identically to every caller class.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "snake_case", tag = "type", deny_unknown_fields)]
pub enum ControlResult {
    /// Bounded run inspection.
    RunInspection {
        /// Authorization-filtered current state.
        value: RunInspection,
    },
    /// Immutable revision inspection.
    RevisionInspection {
        /// Bounded revision facts.
        value: RevisionInspection,
    },
    /// Bounded immutable timeline page.
    Timeline {
        /// Stable-cursor page.
        value: TimelinePage,
    },
    /// Proposed revision and policy result.
    ProposalSubmitted {
        /// Exact proposal outcome.
        value: ProposalSubmission,
    },
    /// Current proposal status.
    ProposalStatus {
        /// Current reconciliation-backed status.
        value: ProposalStatusRead,
    },
    /// Current controller lifecycle status.
    ControllerStatus {
        /// Authorization-filtered exact status.
        value: ControllerStatusRead,
    },
    /// A runtime command completed durably.
    RuntimeCommand {
        /// Authoritative resulting sequence.
        resulting_sequence: RunSequence,
    },
}

fn map_canonical(error: CanonicalJsonError) -> ControlError {
    match error {
        CanonicalJsonError::Json(error) => ControlError::Json(error),
        CanonicalJsonError::Bounds(bound) => map_bound(bound),
    }
}

fn map_bound(bound: milkdrift_contracts::JsonBoundViolation) -> ControlError {
    let name = match bound.kind() {
        JsonBoundKind::Depth => "depth",
        JsonBoundKind::String => "string bytes",
        JsonBoundKind::Key => "key bytes",
        JsonBoundKind::Array => "array items",
        JsonBoundKind::Object => "object entries",
    };
    ControlError::Bounds {
        location: bound.path().to_owned(),
        reason: format!("{name} exceed {}", bound.maximum()),
    }
}
