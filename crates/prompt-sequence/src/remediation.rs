use milkdrift_authority::ActorRef;
use milkdrift_blueprint::BlueprintRevision;
use milkdrift_control::{
    ClaimedStopCondition, ProposalApplicationPolicy, ProposalId, ProposalProvenance,
    WorkflowProposal, WorkflowProposalDocument,
};
use milkdrift_persistence::RunSequence;
use milkdrift_workspace::RunId;

use crate::{
    PromptSequenceDocument, PromptSequenceError, PromptSource, VerificationContract,
    compiler::remediation_mutation,
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
        milkdrift_blueprint::AuthorRef::new(spec.proposer.as_str().to_owned())
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
        Vec::new(),
        Vec::new(),
        ProposalApplicationPolicy::RequireApproval,
        None,
        ClaimedStopCondition::Continue,
    )
    .map_err(|error| PromptSequenceError::Compilation(error.to_string()))?;
    Ok(WorkflowProposalDocument::new(proposal))
}
