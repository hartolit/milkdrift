//! Narrow plug-in boundary for the control-owned durable controller lifecycle.
//!
//! The runtime remains a deterministic scheduler. It supplies immutable revision,
//! projection, identity, and caller-clock facts; `milkdrift-control` is the sole
//! parser and evaluator for the versioned controller policy extension.

use milkdrift_blueprint::{BlueprintRevision, Node};
use milkdrift_capability::BoundedJson;
use milkdrift_persistence::{
    ControllerAssessmentBoundary, ControllerAssessmentOutcome, NodeExecutionId, RunEventKind,
    TimestampMillis,
};
use milkdrift_workspace::RunId;

use crate::{RunProjection, RuntimeError};

/// Semantic extension key marking an enforced controller policy.
pub const CONTROLLER_POLICY_EXTENSION_KEY: &str = "org.milkdrift/controller-policy";

/// Immutable runtime facts supplied to the one control-owned lifecycle evaluator.
pub struct ControllerAssessmentContext<'a> {
    /// Owning controller run.
    pub run: &'a RunId,
    /// Exact governing revision.
    pub revision: &'a BlueprintRevision,
    /// Exact repeat node governed by the policy.
    pub node: &'a Node,
    /// Logical controller occurrence.
    pub execution: &'a NodeExecutionId,
    /// Verified projection through `through_sequence`.
    pub projection: &'a RunProjection,
    /// Caller-owned clock fact for elapsed-time assessment.
    pub observed_at: TimestampMillis,
    /// Boundary being considered.
    pub boundary: ControllerAssessmentBoundary,
    /// One-based next cycle number for activation/cycle entry.
    pub next_cycle: Option<u32>,
}

/// Complete durable result returned by the control-owned lifecycle evaluator.
#[derive(Clone, Debug, PartialEq)]
pub struct ControllerAssessment {
    /// Stable controller identity.
    pub controller_id: String,
    /// Digest binding every executable policy field.
    pub policy_digest: String,
    /// Stable assessment identity derived from canonical boundary facts.
    pub assessment_id: String,
    /// Stable considered-cycle identity, when applicable.
    pub cycle_id: Option<String>,
    /// Exact typed progress snapshot encoded by `milkdrift-control`.
    pub progress: BoundedJson,
    /// Closed runtime-consumed result.
    pub outcome: ControllerAssessmentOutcome,
}

impl ControllerAssessment {
    pub(crate) fn into_event(self, context: &ControllerAssessmentContext<'_>) -> RunEventKind {
        RunEventKind::ControllerAssessmentRecorded {
            controller_id: self.controller_id,
            policy_digest: self.policy_digest,
            governing_revision: context.revision.id().clone(),
            controller_node: context.node.id().clone(),
            controller_execution: context.execution.clone(),
            assessment_id: self.assessment_id,
            cycle_id: self.cycle_id,
            boundary: context.boundary,
            through_sequence: context.projection.sequence(),
            progress: self.progress,
            outcome: self.outcome,
        }
    }
}

/// Canonical controller lifecycle hook installed before runtime admission opens.
pub trait ControllerLifecycle: Send + Sync {
    /// Parses the exact policy and assesses one boundary from authoritative facts.
    ///
    /// `None` means the revision is an ordinary non-controller repeat. A revision
    /// carrying [`CONTROLLER_POLICY_EXTENSION_KEY`] must return `Some` or an error;
    /// the runtime rejects marked controllers when no lifecycle owner is installed.
    fn assess(
        &self,
        context: &ControllerAssessmentContext<'_>,
    ) -> Result<Option<ControllerAssessment>, RuntimeError>;
}
