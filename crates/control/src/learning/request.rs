//! Requests carry references to accepted facts. The authenticated caller never supplies scores.
use super::{LearningDeclaration, LearningReceiptReference};
use milkdrift_blueprint::RevisionId;
use milkdrift_capability::managed::ManagedName;
use milkdrift_persistence::published::PublishedMethod;
use milkdrift_workspace::{ArtifactReference, RunId};
use serde::{Deserialize, Serialize};

/// An explicit bounded page of one permitted source run, frozen at selection.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SourcePage {
    /// Source history; access to a public invocation result does not grant access here.
    pub run: RunId,
    /// Inclusive first event sequence.
    pub first: u64,
    /// Exact number of events required, at most 64 per selected page.
    pub count: u32,
}

/// A version of a setup's knowledge entry point and its explicitly selected evidence.
/// Editable files remain in the working area. This selection freezes the referenced bytes;
/// subsequent file edits or successor selections cannot change an earlier task's context.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct KnowledgeSelection {
    /// Method to which this guidance applies.
    pub method: RevisionId,
    /// Managed working setup whose ordinary documentation contains the discoverable entry point.
    pub workspace: ManagedName,
    /// Immutable exported entry point: purpose, operation, decisions and limitations.
    pub guidance: ArtifactReference,
    /// Exact supplementary source inputs, decisions, failed checks and repair artifacts.
    pub artifacts: Vec<ArtifactReference>,
    /// Explicit source history pages; no global history search is performed.
    pub pages: Vec<SourcePage>,
    /// Earlier selected version this one supersedes, without deleting it.
    pub supersedes: Option<LearningReceiptReference>,
    /// Promotion receipt approving learned guidance, when guidance claims approval.
    pub approval: Option<LearningReceiptReference>,
}

/// Versioned operation body shared by CLI/API and automated callers.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum LearningRequest {
    /// Freeze permitted knowledge and exact history pages in the command receipt.
    Select {
        /// Scope, immutable files and exact pages to select.
        selection: KnowledgeSelection,
    },
    /// Fix input pairs, comparison criteria and budgets before proposal generation.
    Declare {
        /// Independently authorized evaluator declaration.
        declaration: LearningDeclaration,
    },
    /// Parse a model-produced ordinary proposal against the source selection and fixed agreement.
    Candidate {
        /// Preexisting evaluator declaration; its private inputs are not returned to the proposer.
        declaration: LearningReceiptReference,
        /// Ordinary structured workflow proposal, including exact model output provenance.
        proposal: serde_json::Value,
        /// Predicted benefit, kept as an untrusted hypothesis.
        expected_benefit: String,
        /// Conditions under which the proposed method is useful.
        applicability: String,
        /// Evidence that would count against the hypothesis.
        counterevidence: String,
    },
    /// Read both methods' declared run/account/verifier journals and derive the comparison.
    Compare {
        /// Exact accepted declaration.
        declaration: LearningReceiptReference,
        /// Exact accepted candidate proposal receipt.
        candidate: LearningReceiptReference,
    },
    /// Fix the future publication envelope and executor before proposal generation.
    Preauthorize {
        /// Exact evaluator-owned criterion; a different study needs a different policy.
        declaration: LearningReceiptReference,
        /// Actor allowed to execute this conditional publication, without a general publish grant.
        executor: milkdrift_authority::ActorRef,
        /// Full publication template, naming the baseline revision. Only that revision may later
        /// be replaced with the eligible candidate; service, inputs, limits and agreement are fixed.
        method: Box<PublishedMethod>,
        /// Exact current publication record version compared on commit.
        expected_previous_version: u64,
    },
    /// Execute a separately authorized exact policy if its fixed comparison qualifies.
    AutoPromote {
        /// Preexisting operator authorization, rechecked against current authority on execution.
        policy: LearningReceiptReference,
        /// Host-derived eligible comparison under that policy's declaration.
        comparison: LearningReceiptReference,
    },
    /// Promote an eligible candidate with a separately authorized publication command.
    Promote {
        /// Immutable comparison receipt.
        comparison: LearningReceiptReference,
        /// Exact future publication definition; previous generations remain unchanged.
        method: Box<PublishedMethod>,
        /// Current generation's publication record version.
        expected_previous_version: u64,
    },
    /// Inspect an exact retained learning result through current read authority.
    Inspect {
        /// Receipt to inspect, including after restart/archival.
        receipt: LearningReceiptReference,
    },
}

impl LearningRequest {
    /// Operation name used by the ordinary capability scope within the actor's grant.
    #[must_use]
    pub const fn operation(&self) -> &'static str {
        match self {
            Self::Select { .. } => "learning.select",
            Self::Declare { .. } => "learning.declare",
            Self::Candidate { .. } => "learning.candidate",
            Self::Compare { .. } => "learning.compare",
            Self::Preauthorize { .. } => "learning.preauthorize",
            Self::AutoPromote { .. } => "learning.auto_promote",
            Self::Promote { .. } => "learning.promote",
            Self::Inspect { .. } => "learning.inspect",
        }
    }
}
