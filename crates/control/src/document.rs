use std::collections::BTreeSet;

use milkdrift_authority::ActorRef;
use milkdrift_blueprint::{ContentDigest, MutationBatch, RevisionId, WorkflowId};
use milkdrift_capability::{
    ArtifactReference, BoundedJson, CapabilityId, InvocationId, ProviderProfileRef,
};
use milkdrift_contracts::{CanonicalJsonError, JsonBoundKind, JsonLimits, canonical_json_bytes};
use milkdrift_model::{ModelResponse, StructuredOutput};
use milkdrift_persistence::{
    AttemptId, CorrelationKey, EvidenceReference, RunSequence, SignalDeliveryMode, SignalId,
    SignalTypeId,
};
use milkdrift_workspace::RunId;
use serde::{Deserialize, Serialize};

use crate::{ControlError, ProposalDigest, ProposalId};

/// Current untrusted workflow-proposal schema.
pub const PROPOSAL_SCHEMA_VERSION_V1: u32 = 1;
/// Maximum canonical bytes accepted for one workflow proposal document.
pub const MAX_PROPOSAL_DOCUMENT_BYTES: usize = 1_048_576;
const MAX_PROPOSAL_DEPTH: usize = 64;
const MAX_PROPOSAL_ITEMS: usize = 4_096;
const MAX_INLINE_TEXT_BYTES: usize = 2_048;
const MAX_NOTES: usize = 32;
const MAX_REFERENCES: usize = 64;
const PROPOSAL_JSON_LIMITS: JsonLimits = JsonLimits {
    maximum_depth: MAX_PROPOSAL_DEPTH,
    maximum_string_bytes: 65_536,
    maximum_key_bytes: 192,
    maximum_container_items: MAX_PROPOSAL_ITEMS,
};

/// Exact content-addressed artifact reference retained by a proposal.
pub type ProposalArtifactReference = ArtifactReference;

/// Producer-declared provenance references retained as untrusted proposal data.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case", tag = "type", deny_unknown_fields)]
pub enum ProposalProvenance {
    /// A human or importing service produced the document without a capability invocation.
    Direct,
    /// A process/tool capability produced the proposal artifact.
    Process {
        /// Exact producing capability.
        capability: CapabilityId,
        /// Exact invocation.
        invocation: InvocationId,
    },
    /// A model response produced structured proposal data.
    Model {
        /// Exact model capability.
        capability: CapabilityId,
        /// Exact model invocation.
        invocation: InvocationId,
        /// Credential-free model/provider profile reference.
        model_profile: ProviderProfileRef,
        /// Exact context manifest supplied to the model.
        context_manifest: ArtifactReference,
        /// Exact response artifact containing the structured output.
        response_artifact: ArtifactReference,
    },
}

/// Caller-requested policy; classification and authority may make it stricter.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProposalApplicationPolicy {
    /// Create and, for live runs, plan the proposal without applying it.
    ProposeOnly,
    /// Require an explicit approval even when the classifier reports low risk.
    RequireApproval,
    /// Apply only if classification is low risk and exact apply authority is granted.
    AutoApplyLowRisk,
}

/// Optional run-control action requested as untrusted proposal data.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case", tag = "type", deny_unknown_fields)]
pub enum RequestedRunAction {
    /// Pause new work.
    Pause,
    /// Resume a paused run.
    Resume,
    /// Request run cancellation.
    RequestCancellation,
    /// Retry exact retained or uncertain work through runtime resolution.
    RetryExternalWork {
        /// Attempt to resolve through the existing runtime command.
        attempt: AttemptId,
    },
    /// Deliver one exact typed signal.
    Signal {
        /// Stable delivery identity.
        signal: SignalId,
        /// Typed signal contract.
        signal_type: SignalTypeId,
        /// Optional correlation key.
        correlation: Option<CorrelationKey>,
        /// One-shot or broadcast delivery.
        mode: SignalDeliveryMode,
        /// Bounded signal payload.
        payload: BoundedJson,
    },
}

/// Model/controller completion claim retained as data and never treated as truth.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ClaimedStopCondition {
    /// The producer claims another bounded controller cycle is warranted.
    Continue,
    /// The producer claims the workflow objective is complete.
    Complete,
    /// The producer asks to wait for a human checkpoint.
    HumanCheckpoint,
}

/// Fully decoded bounded workflow proposal body.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct WorkflowProposal {
    identity: ProposalId,
    proposer: ActorRef,
    provenance: ProposalProvenance,
    workflow: WorkflowId,
    run: Option<RunId>,
    base_revision: RevisionId,
    base_digest: ContentDigest,
    observed_run_sequence: Option<RunSequence>,
    mutation: MutationBatch,
    rationale: String,
    rationale_artifact: Option<ArtifactReference>,
    risk_notes: Vec<String>,
    assumptions: Vec<String>,
    evidence: Vec<EvidenceReference>,
    artifacts: Vec<ArtifactReference>,
    application_policy: ProposalApplicationPolicy,
    requested_action: Option<RequestedRunAction>,
    claimed_stop: ClaimedStopCondition,
    digest: ProposalDigest,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WorkflowProposalWire {
    identity: ProposalId,
    proposer: ActorRef,
    provenance: ProposalProvenance,
    workflow: WorkflowId,
    run: Option<RunId>,
    base_revision: RevisionId,
    base_digest: ContentDigest,
    observed_run_sequence: Option<RunSequence>,
    mutation: MutationBatch,
    rationale: String,
    rationale_artifact: Option<ArtifactReference>,
    risk_notes: Vec<String>,
    assumptions: Vec<String>,
    evidence: Vec<EvidenceReference>,
    artifacts: Vec<ArtifactReference>,
    application_policy: ProposalApplicationPolicy,
    requested_action: Option<RequestedRunAction>,
    claimed_stop: ClaimedStopCondition,
    digest: ProposalDigest,
}

#[derive(Serialize)]
struct ProposalDigestInput<'a> {
    identity: &'a ProposalId,
    proposer: &'a ActorRef,
    provenance: &'a ProposalProvenance,
    workflow: &'a WorkflowId,
    run: &'a Option<RunId>,
    base_revision: &'a RevisionId,
    base_digest: &'a ContentDigest,
    observed_run_sequence: Option<RunSequence>,
    mutation: &'a MutationBatch,
    rationale: &'a str,
    rationale_artifact: &'a Option<ArtifactReference>,
    risk_notes: &'a [String],
    assumptions: &'a [String],
    evidence: &'a [EvidenceReference],
    artifacts: &'a [ArtifactReference],
    application_policy: ProposalApplicationPolicy,
    requested_action: &'a Option<RequestedRunAction>,
    claimed_stop: ClaimedStopCondition,
}

impl WorkflowProposal {
    /// Constructs a bounded proposal and derives its deterministic digest.
    #[allow(clippy::too_many_arguments)] // One validated control operation keeps its authority and optimistic facts explicit.
    pub fn new(
        identity: ProposalId,
        proposer: ActorRef,
        provenance: ProposalProvenance,
        workflow: WorkflowId,
        run: Option<RunId>,
        base_revision: RevisionId,
        base_digest: ContentDigest,
        observed_run_sequence: Option<RunSequence>,
        mutation: MutationBatch,
        rationale: impl Into<String>,
        rationale_artifact: Option<ArtifactReference>,
        risk_notes: Vec<String>,
        assumptions: Vec<String>,
        evidence: Vec<EvidenceReference>,
        artifacts: Vec<ArtifactReference>,
        application_policy: ProposalApplicationPolicy,
        requested_action: Option<RequestedRunAction>,
        claimed_stop: ClaimedStopCondition,
    ) -> Result<Self, ControlError> {
        let mut proposal = Self {
            identity,
            proposer,
            provenance,
            workflow,
            run,
            base_revision,
            base_digest,
            observed_run_sequence,
            mutation,
            rationale: rationale.into(),
            rationale_artifact,
            risk_notes,
            assumptions,
            evidence,
            artifacts,
            application_policy,
            requested_action,
            claimed_stop,
            digest: ProposalDigest::for_bytes(&[]),
        };
        proposal.validate_fields()?;
        proposal.digest = proposal.calculate_digest()?;
        Ok(proposal)
    }

    fn from_wire(wire: WorkflowProposalWire) -> Result<Self, ControlError> {
        let expected = wire.digest;
        let proposal = Self::new(
            wire.identity,
            wire.proposer,
            wire.provenance,
            wire.workflow,
            wire.run,
            wire.base_revision,
            wire.base_digest,
            wire.observed_run_sequence,
            wire.mutation,
            wire.rationale,
            wire.rationale_artifact,
            wire.risk_notes,
            wire.assumptions,
            wire.evidence,
            wire.artifacts,
            wire.application_policy,
            wire.requested_action,
            wire.claimed_stop,
        )?;
        if proposal.digest != expected {
            return Err(ControlError::InvalidContract(
                "proposal digest does not match its canonical body".to_owned(),
            ));
        }
        Ok(proposal)
    }

    fn validate_fields(&self) -> Result<(), ControlError> {
        if self.run.is_some() != self.observed_run_sequence.is_some() {
            return Err(ControlError::InvalidContract(
                "run and observed_run_sequence must be supplied together".to_owned(),
            ));
        }
        if self.requested_action.is_some() && self.run.is_none() {
            return Err(ControlError::InvalidContract(
                "a requested run action requires an exact live run".to_owned(),
            ));
        }
        if self.rationale.is_empty() || self.rationale.len() > MAX_INLINE_TEXT_BYTES {
            return Err(ControlError::Bounds {
                location: "proposal.rationale".to_owned(),
                reason: format!(
                    "inline rationale must contain 1..={MAX_INLINE_TEXT_BYTES} bytes; larger analysis must be artifact-backed"
                ),
            });
        }
        validate_notes("proposal.risk_notes", &self.risk_notes)?;
        validate_notes("proposal.assumptions", &self.assumptions)?;
        if self.evidence.len() > MAX_REFERENCES || self.artifacts.len() > MAX_REFERENCES {
            return Err(ControlError::Bounds {
                location: "proposal.references".to_owned(),
                reason: format!("each reference list is limited to {MAX_REFERENCES} items"),
            });
        }
        let evidence_ids: BTreeSet<_> = self.evidence.iter().map(|item| &item.id).collect();
        if evidence_ids.len() != self.evidence.len() {
            return Err(ControlError::InvalidContract(
                "proposal evidence identities must be distinct".to_owned(),
            ));
        }
        Ok(())
    }

    fn digest_input(&self) -> ProposalDigestInput<'_> {
        ProposalDigestInput {
            identity: &self.identity,
            proposer: &self.proposer,
            provenance: &self.provenance,
            workflow: &self.workflow,
            run: &self.run,
            base_revision: &self.base_revision,
            base_digest: &self.base_digest,
            observed_run_sequence: self.observed_run_sequence,
            mutation: &self.mutation,
            rationale: &self.rationale,
            rationale_artifact: &self.rationale_artifact,
            risk_notes: &self.risk_notes,
            assumptions: &self.assumptions,
            evidence: &self.evidence,
            artifacts: &self.artifacts,
            application_policy: self.application_policy,
            requested_action: &self.requested_action,
            claimed_stop: self.claimed_stop,
        }
    }

    fn calculate_digest(&self) -> Result<ProposalDigest, ControlError> {
        let bytes = canonical_json_bytes(&self.digest_input(), PROPOSAL_JSON_LIMITS)
            .map_err(map_canonical)?;
        Ok(ProposalDigest::for_bytes(&bytes))
    }

    /// Stable proposal identity.
    #[must_use]
    pub const fn identity(&self) -> &ProposalId {
        &self.identity
    }
    /// Actor claimed by the untrusted document and checked against caller context.
    #[must_use]
    pub const fn proposer(&self) -> &ActorRef {
        &self.proposer
    }
    /// Producing invocation facts.
    #[must_use]
    pub const fn provenance(&self) -> &ProposalProvenance {
        &self.provenance
    }
    /// Target workflow lineage.
    #[must_use]
    pub const fn workflow(&self) -> &WorkflowId {
        &self.workflow
    }
    /// Optional exact live run.
    #[must_use]
    pub const fn run(&self) -> Option<&RunId> {
        self.run.as_ref()
    }
    /// Exact immutable base revision.
    #[must_use]
    pub const fn base_revision(&self) -> &RevisionId {
        &self.base_revision
    }
    /// Exact semantic digest expected for the base.
    #[must_use]
    pub const fn base_digest(&self) -> &ContentDigest {
        &self.base_digest
    }
    /// Exact observed sequence for a live proposal.
    #[must_use]
    pub const fn observed_run_sequence(&self) -> Option<RunSequence> {
        self.observed_run_sequence
    }
    /// Closed atomic mutation batch.
    #[must_use]
    pub const fn mutation(&self) -> &MutationBatch {
        &self.mutation
    }
    /// Bounded rationale summary.
    #[must_use]
    pub fn rationale(&self) -> &str {
        &self.rationale
    }
    /// Optional artifact containing larger rationale.
    #[must_use]
    pub const fn rationale_artifact(&self) -> Option<&ArtifactReference> {
        self.rationale_artifact.as_ref()
    }
    /// Bounded producer-supplied risk notes; these are not policy decisions.
    #[must_use]
    pub fn risk_notes(&self) -> &[String] {
        &self.risk_notes
    }
    /// Bounded assumptions.
    #[must_use]
    pub fn assumptions(&self) -> &[String] {
        &self.assumptions
    }
    /// Supporting durable evidence references.
    #[must_use]
    pub fn evidence(&self) -> &[EvidenceReference] {
        &self.evidence
    }
    /// Supporting content-addressed artifacts.
    #[must_use]
    pub fn artifacts(&self) -> &[ArtifactReference] {
        &self.artifacts
    }
    /// Requested application behavior.
    #[must_use]
    pub const fn application_policy(&self) -> ProposalApplicationPolicy {
        self.application_policy
    }
    /// Optional requested run-control action.
    #[must_use]
    pub const fn requested_action(&self) -> Option<&RequestedRunAction> {
        self.requested_action.as_ref()
    }
    /// Untrusted producer stop/completion claim.
    #[must_use]
    pub const fn claimed_stop(&self) -> ClaimedStopCondition {
        self.claimed_stop
    }
    /// Deterministic canonical proposal digest.
    #[must_use]
    pub const fn digest(&self) -> &ProposalDigest {
        &self.digest
    }
}

impl<'de> Deserialize<'de> for WorkflowProposal {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        WorkflowProposal::from_wire(WorkflowProposalWire::deserialize(deserializer)?)
            .map_err(serde::de::Error::custom)
    }
}

/// Canonical versioned envelope for one untrusted workflow proposal.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct WorkflowProposalDocument {
    schema_version: u32,
    proposal: WorkflowProposal,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ProposalDocumentWire {
    schema_version: u32,
    proposal: WorkflowProposalWire,
}

impl WorkflowProposalDocument {
    /// Wraps one validated proposal in schema v1.
    #[must_use]
    pub const fn new(proposal: WorkflowProposal) -> Self {
        Self {
            schema_version: PROPOSAL_SCHEMA_VERSION_V1,
            proposal,
        }
    }

    /// Current proposal envelope schema.
    #[must_use]
    pub const fn schema_version(&self) -> u32 {
        self.schema_version
    }

    /// Validated proposal body.
    #[must_use]
    pub const fn proposal(&self) -> &WorkflowProposal {
        &self.proposal
    }

    /// Encodes deterministic compact key-sorted JSON.
    pub fn to_canonical_json(&self) -> Result<Vec<u8>, ControlError> {
        let bytes = canonical_json_bytes(self, PROPOSAL_JSON_LIMITS).map_err(map_canonical)?;
        if bytes.len() > MAX_PROPOSAL_DOCUMENT_BYTES {
            return Err(ControlError::Bounds {
                location: "$".to_owned(),
                reason: format!("document exceeds {MAX_PROPOSAL_DOCUMENT_BYTES} bytes"),
            });
        }
        Ok(bytes)
    }

    /// Performs lexical preflight, duplicate-safe decode, version validation, and digest checks.
    pub fn from_json(bytes: &[u8]) -> Result<Self, ControlError> {
        if bytes.len() > MAX_PROPOSAL_DOCUMENT_BYTES {
            return Err(ControlError::Bounds {
                location: "$".to_owned(),
                reason: format!("document exceeds {MAX_PROPOSAL_DOCUMENT_BYTES} bytes"),
            });
        }
        milkdrift_contracts::preflight_json_structure(bytes, PROPOSAL_JSON_LIMITS)
            .map_err(map_bound)?;
        let value = milkdrift_contracts::parse_json_without_duplicates(bytes)?;
        milkdrift_contracts::validate_json_value(&value, PROPOSAL_JSON_LIMITS)
            .map_err(map_bound)?;
        let version = value
            .get("schema_version")
            .and_then(serde_json::Value::as_u64)
            .and_then(|value| u32::try_from(value).ok())
            .ok_or_else(|| {
                ControlError::InvalidContract("missing numeric schema_version".to_owned())
            })?;
        if version != PROPOSAL_SCHEMA_VERSION_V1 {
            return Err(ControlError::UnsupportedVersion {
                document: "workflow_proposal",
                found: version,
                supported: PROPOSAL_SCHEMA_VERSION_V1,
            });
        }
        let wire: ProposalDocumentWire = serde_json::from_value(value)?;
        if wire.schema_version != PROPOSAL_SCHEMA_VERSION_V1 {
            return Err(ControlError::UnsupportedVersion {
                document: "workflow_proposal",
                found: wire.schema_version,
                supported: PROPOSAL_SCHEMA_VERSION_V1,
            });
        }
        Ok(Self::new(WorkflowProposal::from_wire(wire.proposal)?))
    }

    /// Reads only the strict structured result from a model response.
    ///
    /// Returned prose and tool calls remain ordinary model data and are never executed.
    pub fn from_model_response(response: &ModelResponse) -> Result<Self, ControlError> {
        let structured = response.structured().ok_or_else(|| {
            ControlError::InvalidContract(
                "model response did not contain workflow-proposal structured output".to_owned(),
            )
        })?;
        let object = structured.value().as_object().ok_or_else(|| {
            ControlError::InvalidContract(
                "workflow-proposal structured output must be an object".to_owned(),
            )
        })?;
        if object.len() != 1 {
            return Err(ControlError::InvalidContract(
                "workflow-proposal structured output contains unknown fields".to_owned(),
            ));
        }
        let document = object
            .get("proposal_document_json")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| {
                ControlError::InvalidContract(
                    "workflow-proposal structured output requires proposal_document_json"
                        .to_owned(),
                )
            })?;
        Self::from_json(document.as_bytes())
    }
}

impl<'de> Deserialize<'de> for WorkflowProposalDocument {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let wire = ProposalDocumentWire::deserialize(deserializer)?;
        if wire.schema_version != PROPOSAL_SCHEMA_VERSION_V1 {
            return Err(serde::de::Error::custom(
                "unsupported workflow proposal schema version",
            ));
        }
        WorkflowProposal::from_wire(wire.proposal)
            .map(Self::new)
            .map_err(serde::de::Error::custom)
    }
}

/// Builds the strict model structured-output declaration used for proposal generation tasks.
pub fn workflow_proposal_structured_output() -> Result<StructuredOutput, ControlError> {
    Ok(StructuredOutput::new(
        "milkdrift_workflow_proposal_v1",
        BoundedJson::new(serde_json::json!({
            "$schema": "https://json-schema.org/draft/2020-12/schema",
            "type": "object",
            "additionalProperties": false,
            "required": ["proposal_document_json"],
            "properties": {
                "proposal_document_json": {
                    "type": "string",
                    "minLength": 1,
                    "maxLength": MAX_PROPOSAL_DOCUMENT_BYTES,
                    "description": "Canonical schema-v1 WorkflowProposalDocument JSON. It is decoded again by milkdrift-control; code, scripts, SQL, credentials and event insertion have no fields."
                }
            }
        }))?,
        true,
    )?)
}

fn validate_notes(location: &'static str, notes: &[String]) -> Result<(), ControlError> {
    if notes.len() > MAX_NOTES
        || notes
            .iter()
            .any(|note| note.is_empty() || note.len() > MAX_INLINE_TEXT_BYTES)
    {
        return Err(ControlError::Bounds {
            location: location.to_owned(),
            reason: format!(
                "must contain at most {MAX_NOTES} nonempty entries of at most {MAX_INLINE_TEXT_BYTES} bytes"
            ),
        });
    }
    Ok(())
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
