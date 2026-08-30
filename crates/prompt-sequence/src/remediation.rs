use milkdrift_authority::ActorRef;
use milkdrift_blueprint::{AuthorRef, BlueprintRevision};
use milkdrift_capability::ExtensionKey;
use milkdrift_control::{
    ClaimedStopCondition, ProposalApplicationPolicy, ProposalId, ProposalProvenance,
    WorkflowProposal, WorkflowProposalDocument,
};
use milkdrift_persistence::{EvidenceId, EvidenceKind, EvidenceReference, RunSequence};
use milkdrift_workspace::RunId;

use crate::{
    PromptSequenceDocument, PromptSequenceError, PromptSource, VerificationContract,
    compiler::{compile, remediation_mutation},
};

/// Exact live-run facts used to build a bounded prospective remediation proposal.
#[derive(Clone, Debug, PartialEq)]
pub struct RemediationProposalSpec {
    /// Exact live run.
    pub run: RunId,
    /// Run sequence observed through the authorized read plane.
    pub observed_sequence: RunSequence,
    /// Proposal identity selected by the caller.
    pub proposal: ProposalId,
    /// Server-authenticated actor identity expected by proposal submission.
    pub proposer: ActorRef,
    /// Failed imported stage.
    pub stage_id: String,
    /// Nonzero bounded remediation generation.
    pub generation: u16,
    /// Fresh remediation prompt/reference.
    pub prompt: PromptSource,
    /// Optional verifier replacement still subject to the frozen run grant and
    /// ordinary proposal risk/authority checks.
    pub verification_override: Option<VerificationContract>,
}

/// Builds a normal digest-bound workflow proposal that prospectively inserts
/// remediation, re-verification, re-review, and a renewed approval hold.
pub fn build_remediation_proposal(
    document: &PromptSequenceDocument,
    base: &BlueprintRevision,
    spec: RemediationProposalSpec,
) -> Result<WorkflowProposalDocument, PromptSequenceError> {
    if base.semantic().workflow().as_str() != document.sequence().workflow_id {
        return Err(PromptSequenceError::Invalid(
            "remediation base revision belongs to a different workflow".to_owned(),
        ));
    }
    let compiled = compile(
        document,
        AuthorRef::new("system:remediation-provenance")
            .map_err(|error| PromptSequenceError::Compilation(error.to_string()))?,
    )?;
    let metadata_key = ExtensionKey::new("org.milkdrift/prompt-sequence")
        .map_err(|error| PromptSequenceError::Compilation(error.to_string()))?;
    let metadata = base
        .semantic()
        .metadata()
        .extensions()
        .get(&metadata_key)
        .map(milkdrift_capability::BoundedJson::value)
        .ok_or_else(|| {
            PromptSequenceError::Invalid(
                "remediation base revision has no prompt-sequence provenance".to_owned(),
            )
        })?;
    let expected_stages = serde_json::to_value(compiled.stages())
        .map_err(|error| PromptSequenceError::Compilation(error.to_string()))?;
    let provenance_matches = metadata
        .get("schema_version")
        .and_then(serde_json::Value::as_u64)
        == Some(2)
        && metadata
            .get("sequence_id")
            .and_then(serde_json::Value::as_str)
            == Some(document.sequence().id.as_str())
        && metadata
            .get("import_digest")
            .and_then(serde_json::Value::as_str)
            == Some(compiled.import_digest())
        && metadata
            .get("repository_profile_digest")
            .and_then(serde_json::Value::as_str)
            == Some(compiled.repository_profile_digest())
        && metadata.get("stages") == Some(&expected_stages);
    if !provenance_matches {
        return Err(PromptSequenceError::Invalid(
            "remediation document does not exactly match the imported base sequence".to_owned(),
        ));
    }
    if spec.generation == 0 || spec.generation > document.sequence().budget.max_review_loops {
        return Err(PromptSequenceError::Invalid(format!(
            "remediation generation must be within 1..={}",
            document.sequence().budget.max_review_loops
        )));
    }
    let mut stage = document
        .sequence()
        .stages
        .iter()
        .find(|stage| stage.id == spec.stage_id)
        .cloned()
        .ok_or_else(|| PromptSequenceError::Invalid("remediation stage is absent".to_owned()))?;
    if let Some(verification) = spec.verification_override {
        stage.verification = verification;
    }
    let mutation = remediation_mutation(document, &stage, spec.generation, spec.prompt)?;
    base.revise(
        base.id(),
        mutation.clone(),
        AuthorRef::new(spec.proposer.as_str().to_owned())
            .map_err(|error| PromptSequenceError::Compilation(error.to_string()))?,
        format!(
            "prospective remediation {} for stage {}",
            spec.generation, spec.stage_id
        ),
    )
    .map_err(|error| PromptSequenceError::Compilation(format!("{error:?}")))?;

    let proposal = WorkflowProposal::new(
        spec.proposal,
        spec.proposer,
        ProposalProvenance::Direct,
        base.semantic().workflow().clone(),
        Some(spec.run),
        base.id().clone(),
        base.content_digest().clone(),
        Some(spec.observed_sequence),
        mutation,
        format!(
            "insert bounded remediation generation {} after failed stage {}",
            spec.generation, spec.stage_id
        ),
        None,
        vec![
            "completed coding and verification occurrences remain immutable".to_owned(),
            "proposal changes workflow semantics but cannot change the frozen authority grant"
                .to_owned(),
        ],
        vec!["selected failure branch is waiting at its shared-control approval gate".to_owned()],
        vec![EvidenceReference {
            id: EvidenceId::new(format!(
                "prompt-sequence-import:{}",
                compiled.import_digest()
            ))
            .map_err(|error| PromptSequenceError::Compilation(error.to_string()))?,
            kind: EvidenceKind::RecoveryObservation,
        }],
        Vec::new(),
        ProposalApplicationPolicy::RequireApproval,
        None,
        ClaimedStopCondition::Continue,
    )
    .map_err(|error| PromptSequenceError::Compilation(error.to_string()))?;
    Ok(WorkflowProposalDocument::new(proposal))
}
