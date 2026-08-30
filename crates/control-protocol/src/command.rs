use super::*;

/// Reference to bounded external evidence retained elsewhere.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct EvidenceRef {
    /// Stable evidence identity.
    pub id: String,
    /// Stable evidence category.
    pub kind: String,
}

/// External mutating request. Actor identity is intentionally absent.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CommandRequest {
    /// Requested protocol version.
    pub protocol: ProtocolVersion,
    /// Stable client-owned idempotency identity.
    pub command_id: String,
    /// Optional aggregate sequence guard.
    pub expected_sequence: Option<u64>,
    /// Optional exact semantic revision guard.
    pub expected_revision: Option<String>,
    /// Bounded human/operator reason.
    pub reason: String,
    /// Bounded references, never inline evidence blobs.
    pub evidence: Vec<EvidenceRef>,
    /// Closed command body.
    pub command: Command,
}

impl CommandRequest {
    /// Validates common envelope bounds and version support.
    pub fn validate(&self) -> Result<(), ProtocolError> {
        self.protocol.negotiate()?;
        validate_identifier("command_id", &self.command_id, 192)?;
        if self.reason.is_empty() || self.reason.len() > MAX_REASON_BYTES {
            return Err(ProtocolError::Bounds(format!(
                "reason must contain 1..={MAX_REASON_BYTES} bytes"
            )));
        }
        if self.evidence.len() > MAX_EVIDENCE_ITEMS {
            return Err(ProtocolError::Bounds(format!(
                "at most {MAX_EVIDENCE_ITEMS} evidence references are allowed"
            )));
        }
        let mut identities = BTreeSet::new();
        for evidence in &self.evidence {
            validate_identifier("evidence.id", &evidence.id, 192)?;
            validate_identifier("evidence.kind", &evidence.kind, 64)?;
            if !identities.insert(&evidence.id) {
                return Err(ProtocolError::InvalidContract(
                    "evidence identities must be distinct".to_owned(),
                ));
            }
        }
        Ok(())
    }
}

/// Closed version-one mutation vocabulary.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case", tag = "type", deny_unknown_fields)]
#[allow(missing_docs)] // Variant prose documents each compact operation payload.
pub enum Command {
    /// Store a validated immutable blueprint document.
    ImportBlueprint { document: Value },
    /// Validate an immutable blueprint document without storing it.
    ValidateBlueprint { document: Value },
    /// Compile, validate, and store a bounded prompt-sequence as an ordinary blueprint revision.
    ImportPromptSequence { document: Value },
    /// Compile and validate a bounded prompt-sequence without storing its generated revision.
    ValidatePromptSequence { document: Value },
    /// Create and start a run at one exact revision.
    StartRun {
        run_id: String,
        workflow_id: String,
        revision_id: String,
    },
    /// Pause new work for a run.
    PauseRun { run_id: String },
    /// Resume a paused run.
    ResumeRun { run_id: String },
    /// Request durable cancellation.
    CancelRun { run_id: String },
    /// Deliver a typed signal with a bounded JSON payload.
    SignalRun {
        run_id: String,
        signal_id: String,
        signal_type: String,
        correlation: Option<String>,
        broadcast: bool,
        payload: Value,
    },
    /// Resolve retained/uncertain external work.
    ResolveWork {
        run_id: String,
        attempt_id: String,
        decision_id: String,
        action: ResolveAction,
        remediation_node: Option<String>,
    },
    /// Submit a versioned workflow proposal document.
    SubmitProposal { document: Value },
    /// Decide an exact proposal/reconciliation plan.
    DecideProposal {
        run_id: String,
        proposal_id: String,
        proposal_digest: String,
        proposed_revision: String,
        decision_id: String,
        decision: ProposalDecision,
    },
    /// Apply an approved exact proposal.
    ApplyProposal {
        run_id: String,
        proposal_id: String,
        proposal_digest: String,
        proposed_revision: String,
    },
    /// Optimistically replace presentation-only layout state.
    PutLayout { layout: LayoutDocument },
}

/// Public resolution choice for retained external work.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ResolveAction {
    /// Query external truth through a separately authorized capability.
    Query,
    /// Retry only under runtime idempotency policy.
    Retry,
    /// Create explicit compensation.
    Compensate,
    /// Keep the obligation visible.
    Retain,
    /// Resolve as succeeded from evidence.
    ResolveSucceeded,
    /// Resolve as failed from evidence.
    ResolveFailed,
}

/// Public proposal decision.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProposalDecision {
    /// Authorize application.
    Approve,
    /// Reject application.
    Reject,
}

/// Durable command acceptance response.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CommandAccepted {
    /// Echoed idempotency identity.
    pub command_id: String,
    /// True when a previously committed result was returned.
    pub replayed: bool,
    /// Resulting aggregate sequence, when a run was mutated.
    pub resulting_sequence: Option<u64>,
    /// Stable result category.
    pub result_type: String,
    /// Bounded operation-specific result.
    pub value: Value,
}
