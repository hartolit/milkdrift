//! Structured authoring form. The owner derives mutation/proposal digests before ordinary admission;
//! a draft grants no authoring, adaptation or application authority.
use super::{
    ClaimedStopCondition, PROPOSAL_SCHEMA_VERSION_V1, ProposalApplicationPolicy,
    ProposalProvenance, RequestedRunAction, WorkflowProposal, WorkflowProposalDocument,
};
use crate::{ControlError, ProposalId};
use milkdrift_authority::ActorRef;
use milkdrift_blueprint::{ContentDigest, Mutation, MutationBatch, RevisionId, WorkflowId};
use milkdrift_capability::ArtifactReference;
use milkdrift_persistence::{EvidenceReference, RunSequence};
use milkdrift_workspace::RunId;
use serde::Deserialize;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DraftDocument {
    schema_version: u32,
    draft: Draft,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Draft {
    identity: ProposalId,
    proposer: ActorRef,
    provenance: ProposalProvenance,
    workflow: WorkflowId,
    run: Option<RunId>,
    base_revision: RevisionId,
    base_digest: ContentDigest,
    observed_run_sequence: Option<RunSequence>,
    mutation: Vec<Mutation>,
    rationale: String,
    rationale_artifact: Option<ArtifactReference>,
    risk_notes: Vec<String>,
    assumptions: Vec<String>,
    evidence: Vec<EvidenceReference>,
    artifacts: Vec<ArtifactReference>,
    application_policy: ProposalApplicationPolicy,
    requested_action: Option<RequestedRunAction>,
    claimed_stop: ClaimedStopCondition,
}
pub(super) fn decode(value: serde_json::Value) -> Result<WorkflowProposalDocument, ControlError> {
    let document: DraftDocument = serde_json::from_value(value)?;
    if document.schema_version != PROPOSAL_SCHEMA_VERSION_V1 {
        return Err(ControlError::InvalidContract(
            "unsupported proposal draft version".to_owned(),
        ));
    }
    let d = document.draft;
    let mutation =
        MutationBatch::new(d.mutation).map_err(|e| ControlError::InvalidContract(e.to_string()))?;
    let proposal = WorkflowProposal::new(
        d.identity,
        d.proposer,
        d.provenance,
        d.workflow,
        d.run,
        d.base_revision,
        d.base_digest,
        d.observed_run_sequence,
        mutation,
        d.rationale,
        d.rationale_artifact,
        d.risk_notes,
        d.assumptions,
        d.evidence,
        d.artifacts,
        d.application_policy,
        d.requested_action,
        d.claimed_stop,
    )?;
    Ok(WorkflowProposalDocument::new(proposal))
}
