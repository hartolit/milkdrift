use super::{
    BTreeSet, Deserialize, LayoutDocument, MAX_EVIDENCE_ITEMS, MAX_REASON_BYTES, ProtocolError,
    ProtocolVersion, Serialize, Value, validate_identifier,
};

/// Reference to bounded external evidence retained elsewhere.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct EvidenceRef {
    /// Stable evidence identity.
    pub id: String,
    /// Stable evidence category.
    pub kind: String,
}

/// One client-owned request whose exact identity lets a caller recover a lost reply.
///
/// Keep the entire envelope when retrying: reason, evidence, guards, and body participate in
/// replay/conflict checks along with the authenticated actor and grant. Actor identity comes
/// from the daemon's credential mapping, so it cannot be supplied in this document.
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
    ///
    /// # Errors
    /// Rejects unsupported protocol versions, invalid command/evidence identities, empty or oversized
    /// reasons, excessive evidence, and repeated evidence identities. Command semantics and authority
    /// remain the daemon's responsibility.
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

/// Operations carried by the current external command envelope.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case", tag = "type", deny_unknown_fields)]
pub enum Command {
    /// Apply an optional ordinary edit and return the pending mutations. With `save`, validate
    /// the complete definition and store it. The envelope revision guard must match the base.
    AuthorBlueprint {
        /// Unsubmitted ordered mutations and their exact base revision.
        draft: crate::BlueprintDraft,
        /// Optional editor operation applied to the supplied draft.
        edit: Option<crate::BlueprintEdit>,
        /// Whether to validate and persist the complete resulting definition.
        save: bool,
    },
    /// Construct a canonical definition from existing mutations using server-owned identities.
    ConstructBlueprint {
        /// Unsubmitted ordered mutations and their exact base revision.
        draft: crate::BlueprintDraft,
        /// Whether to persist the constructed definition after validation.
        store: bool,
    },
    /// Save an independent definition from an exact authorized source. Identity-bound governing
    /// agreements refuse copying; neither run state nor authority is transferred.
    CopyBlueprint {
        /// Exact authorized immutable revision to copy.
        source_revision: String,
        /// New independent workflow identity.
        workflow_id: String,
        /// Display name for the new independent definition.
        name: String,
    },
    /// Build a normal approval-required proposal at the exact paused review hold. This
    /// prepares a document only; ordinary submit/approve/apply commands perform the change.
    PrepareModelRepair {
        /// Exact target run identity.
        run_id: String,
        /// New proposal identity for the prepared repair.
        proposal_id: String,
        /// Fresh repair step and explicit model selection.
        repair: crate::ModelRepair,
    },
    /// Select evidence, declare and compare a study, or promote through ordinary authority.
    Learning {
        /// Strict learning request for evidence selection, declaration, comparison, or promotion.
        document: Value,
    },
    /// Resolve reviewed choices into a publication document without publishing it.
    PrepareMethod {
        /// Reviewed publication choices; the owner derives agreement and service grant facts.
        document: Value,
    },
    /// Publish an exact reviewed method as a separate operation after preparation.
    PublishMethod {
        /// Reviewed publication definition produced by the method preparation owner.
        document: Value,
        /// Expected previous publication record version, or `None` for its first write.
        expected_previous_version: Option<u64>,
    },
    /// Inspect protected publication implementation details with administration authority.
    InspectMethod {
        /// Exact published capability identity.
        capability: String,
        /// Exact immutable descriptor generation.
        generation: u64,
    },
    /// Page retained publication definitions; public discovery uses the capability catalog.
    ListMethods {
        /// Last capability returned by the previous page, paired with its generation.
        after_capability: Option<String>,
        /// Last generation returned for `after_capability`.
        after_generation: Option<u64>,
        /// Maximum number of retained definitions to return.
        limit: u32,
    },
    /// Retire new selection while preserving accepted calls and their exact implementation.
    RetireMethod {
        /// Exact published capability identity.
        capability: String,
        /// Exact immutable descriptor generation.
        generation: u64,
        /// Expected current publication record version for this retirement.
        expected_version: u64,
    },
    /// Store a validated immutable blueprint document.
    ImportBlueprint {
        /// Immutable blueprint revision document, including its content-derived identity.
        document: Value,
    },
    /// Validate an immutable blueprint document without storing it.
    ValidateBlueprint {
        /// Immutable blueprint revision document whose identity and semantics must validate.
        document: Value,
    },
    /// Compile, validate, and store a bounded prompt-sequence as an ordinary blueprint revision.
    ImportPromptSequence {
        /// Versioned prompt-sequence definition to compile and store.
        document: Value,
    },
    /// Compile and validate a bounded prompt-sequence without storing its generated revision.
    ValidatePromptSequence {
        /// Versioned prompt-sequence definition to compile without persisting it.
        document: Value,
    },
    /// Create and start a run at one exact revision.
    StartRun {
        /// Exact target run identity.
        run_id: String,
        /// Workflow identity that must match the pinned revision.
        workflow_id: String,
        /// Exact immutable revision to pin when creating the run.
        revision_id: String,
        /// Named committed artifacts supplied to the workflow interface.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        inputs: Vec<RunInput>,
    },
    /// Pause new work for a run.
    PauseRun {
        /// Exact target run identity.
        run_id: String,
    },
    /// Resume a paused run.
    ResumeRun {
        /// Exact target run identity.
        run_id: String,
    },
    /// Request durable cancellation.
    CancelRun {
        /// Exact target run identity.
        run_id: String,
    },
    /// Deliver a typed signal with a bounded JSON payload.
    SignalRun {
        /// Exact target run identity.
        run_id: String,
        /// Stable signal identity used for durable deduplication.
        signal_id: String,
        /// Declared signal type to deliver.
        signal_type: String,
        /// Optional exact correlation key for matching waits.
        correlation: Option<String>,
        /// Whether the signal may satisfy all matching waits instead of one.
        broadcast: bool,
        /// Bounded JSON value interpreted by the declared signal contract.
        payload: Value,
    },
    /// Resolve retained/uncertain external work.
    ResolveWork {
        /// Exact target run identity.
        run_id: String,
        /// Exact retained attempt whose external outcome is being reconciled.
        attempt_id: String,
        /// Stable identity of the authorized durable decision.
        decision_id: String,
        /// Prospective resolution choice; runtime policy determines whether it is permitted.
        action: ResolveAction,
        /// Optional existing node selected for authorized compensation or remediation.
        remediation_node: Option<String>,
    },
    /// Inspect one exact durable controller occurrence through the shared control path.
    InspectController {
        /// Exact target run identity.
        run_id: String,
        /// Exact logical controller occurrence within the run.
        controller_execution: String,
    },
    /// Continue one exact durable controller checkpoint with ordinary approval authority.
    ContinueController {
        /// Exact target run identity.
        run_id: String,
        /// Exact logical controller occurrence within the run.
        controller_execution: String,
        /// Stable identity of the authorized durable decision.
        decision_id: String,
    },
    /// Submit a versioned workflow proposal document.
    SubmitProposal {
        /// Versioned proposal binding its base, mutation, provenance, evidence, and digest.
        document: Value,
    },
    /// Decide an exact proposal/reconciliation plan.
    DecideProposal {
        /// Exact target run identity.
        run_id: String,
        /// Exact retained proposal identity.
        proposal_id: String,
        /// Canonical digest binding the exact immutable proposal.
        proposal_digest: String,
        /// Exact candidate revision produced from that proposal.
        proposed_revision: String,
        /// Stable identity of the authorized durable decision.
        decision_id: String,
        /// Approval or rejection to retain over the exact plan.
        decision: ProposalDecision,
    },
    /// Apply an approved exact proposal.
    ApplyProposal {
        /// Exact target run identity.
        run_id: String,
        /// Exact retained proposal identity.
        proposal_id: String,
        /// Canonical digest binding the exact immutable proposal.
        proposal_digest: String,
        /// Exact candidate revision produced from that proposal.
        proposed_revision: String,
    },
    /// Optimistically replace presentation-only layout state.
    PutLayout {
        /// Sealed presentation document with its optimistic generation and revision association.
        layout: LayoutDocument,
    },
}

/// One named immutable artifact supplied to the pinned workflow interface.
///
/// Publish local file bytes through the input upload route first. The daemon resolves this
/// identity under the caller's content-read authority before creating the run; server paths
/// and arbitrary workspace references cannot be submitted here.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RunInput {
    /// Exact interface field name.
    pub name: String,
    /// Committed artifact identity, whose content and metadata are immutable.
    pub artifact_id: String,
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

/// Result of command acceptance, which may precede completion of the requested work.
///
/// For example, starting a run returns its accepted identity and sequence; run/attempt reads
/// establish whether its tasks later succeeded. `replayed` reports receipt recovery and does
/// not mean the daemon executed the operation again.
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
