use std::collections::BTreeMap;

use milkdrift_blueprint::{
    AuthorRef, BlueprintMetadata, BlueprintRevision, Condition, CostCurrencyCode, Edge, EdgeId,
    EdgeKind, Mutation, MutationBatch, Node, NodeId, NodeKind, PinnedSubworkflow, PortId,
    RepeatBudget, RepeatConfig, RepeatTermination, TerminalOutcome, WorkflowId,
};
use milkdrift_capability::{BoundedJson, ExtensionKey};
use serde::{Deserialize, Serialize};

use crate::ControlError;

/// Hard controller ceiling that stopped a cycle before hidden unbounded continuation.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ControllerBound {
    /// Controller/model invocation count.
    Invocations,
    /// New prospective revision count.
    Revisions,
    /// Mutations in the current proposal.
    MutationsPerProposal,
    /// Nodes added in the current proposal.
    NodesPerProposal,
    /// Elapsed controller time.
    ElapsedTime,
    /// Observed cost.
    Cost,
    /// Model/process input units.
    InputUnits,
    /// Model/process output units.
    OutputUnits,
    /// Artifact bytes.
    ArtifactBytes,
    /// Process invocation count.
    ProcessInvocations,
    /// Model invocation count.
    ModelInvocations,
    /// Failed cycle count.
    Failures,
    /// Rejected proposal count.
    Rejections,
    /// Repeat nesting depth.
    RepeatDepth,
    /// Child-workflow nesting depth.
    ChildDepth,
    /// Required human checkpoint interval.
    HumanCheckpoint,
}

/// Why a bounded controller must stop or wait.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case", tag = "type", deny_unknown_fields)]
pub enum ControllerStop {
    /// The controller may execute another cycle.
    Continue,
    /// A hard ceiling was reached.
    BoundReached {
        /// Exact limiting dimension.
        bound: ControllerBound,
    },
    /// The next cycle requires a recorded human continuation decision.
    HumanCheckpoint,
}

/// Immutable strict ceilings for a reusable controller pattern.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ControllerLimits {
    max_invocations: u32,
    max_revisions: u32,
    max_mutations_per_proposal: u16,
    max_nodes_per_proposal: u16,
    max_elapsed_ms: u64,
    max_cost_micros: u64,
    max_input_units: u64,
    max_output_units: u64,
    max_artifact_bytes: u64,
    max_process_invocations: u32,
    max_model_invocations: u32,
    max_failures: u16,
    max_rejections: u16,
    max_repeat_depth: u16,
    max_child_depth: u16,
    human_checkpoint_interval: Option<u32>,
}

impl ControllerLimits {
    /// Constructs nonzero hard limits for every controller dimension.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        max_invocations: u32,
        max_revisions: u32,
        max_mutations_per_proposal: u16,
        max_nodes_per_proposal: u16,
        max_elapsed_ms: u64,
        max_cost_micros: u64,
        max_input_units: u64,
        max_output_units: u64,
        max_artifact_bytes: u64,
        max_process_invocations: u32,
        max_model_invocations: u32,
        max_failures: u16,
        max_rejections: u16,
        max_repeat_depth: u16,
        max_child_depth: u16,
        human_checkpoint_interval: Option<u32>,
    ) -> Result<Self, ControlError> {
        if [
            u64::from(max_invocations),
            u64::from(max_revisions),
            u64::from(max_mutations_per_proposal),
            u64::from(max_nodes_per_proposal),
            max_elapsed_ms,
            max_cost_micros,
            max_input_units,
            max_output_units,
            max_artifact_bytes,
            u64::from(max_process_invocations),
            u64::from(max_model_invocations),
            u64::from(max_failures),
            u64::from(max_rejections),
            u64::from(max_repeat_depth),
            u64::from(max_child_depth),
        ]
        .contains(&0)
            || human_checkpoint_interval == Some(0)
        {
            return Err(ControlError::InvalidContract(
                "every controller ceiling and checkpoint interval must be nonzero".to_owned(),
            ));
        }
        Ok(Self {
            max_invocations,
            max_revisions,
            max_mutations_per_proposal,
            max_nodes_per_proposal,
            max_elapsed_ms,
            max_cost_micros,
            max_input_units,
            max_output_units,
            max_artifact_bytes,
            max_process_invocations,
            max_model_invocations,
            max_failures,
            max_rejections,
            max_repeat_depth,
            max_child_depth,
            human_checkpoint_interval,
        })
    }

    /// Maximum controller cycles.
    #[must_use]
    pub const fn max_invocations(&self) -> u32 {
        self.max_invocations
    }

    /// Maximum new prospective revisions.
    #[must_use]
    pub const fn max_revisions(&self) -> u32 {
        self.max_revisions
    }

    /// Maximum mutations in one proposal.
    #[must_use]
    pub const fn max_mutations_per_proposal(&self) -> u16 {
        self.max_mutations_per_proposal
    }

    /// Maximum newly added nodes in one proposal.
    #[must_use]
    pub const fn max_nodes_per_proposal(&self) -> u16 {
        self.max_nodes_per_proposal
    }

    /// Required human checkpoint interval, when configured.
    #[must_use]
    pub const fn human_checkpoint_interval(&self) -> Option<u32> {
        self.human_checkpoint_interval
    }

    /// Deterministically checks durable/accounted progress against every bound.
    #[must_use]
    pub fn assess(&self, progress: &ControllerProgress) -> ControllerStop {
        let checks = [
            (
                u64::from(progress.invocations) >= u64::from(self.max_invocations),
                ControllerBound::Invocations,
            ),
            (
                u64::from(progress.revisions) >= u64::from(self.max_revisions),
                ControllerBound::Revisions,
            ),
            (
                u64::from(progress.mutations_in_proposal)
                    > u64::from(self.max_mutations_per_proposal),
                ControllerBound::MutationsPerProposal,
            ),
            (
                u64::from(progress.nodes_in_proposal) > u64::from(self.max_nodes_per_proposal),
                ControllerBound::NodesPerProposal,
            ),
            (
                progress.elapsed_ms >= self.max_elapsed_ms,
                ControllerBound::ElapsedTime,
            ),
            (
                progress.cost_micros >= self.max_cost_micros,
                ControllerBound::Cost,
            ),
            (
                progress.input_units >= self.max_input_units,
                ControllerBound::InputUnits,
            ),
            (
                progress.output_units >= self.max_output_units,
                ControllerBound::OutputUnits,
            ),
            (
                progress.artifact_bytes >= self.max_artifact_bytes,
                ControllerBound::ArtifactBytes,
            ),
            (
                u64::from(progress.process_invocations) >= u64::from(self.max_process_invocations),
                ControllerBound::ProcessInvocations,
            ),
            (
                u64::from(progress.model_invocations) >= u64::from(self.max_model_invocations),
                ControllerBound::ModelInvocations,
            ),
            (
                u64::from(progress.failures) >= u64::from(self.max_failures),
                ControllerBound::Failures,
            ),
            (
                u64::from(progress.rejections) >= u64::from(self.max_rejections),
                ControllerBound::Rejections,
            ),
            (
                u64::from(progress.repeat_depth) > u64::from(self.max_repeat_depth),
                ControllerBound::RepeatDepth,
            ),
            (
                u64::from(progress.child_depth) > u64::from(self.max_child_depth),
                ControllerBound::ChildDepth,
            ),
        ];
        if let Some((_, bound)) = checks.into_iter().find(|(reached, _)| *reached) {
            return ControllerStop::BoundReached { bound };
        }
        if self.human_checkpoint_interval.is_some_and(|interval| {
            progress.invocations > 0 && progress.invocations.is_multiple_of(interval)
        }) {
            return ControllerStop::HumanCheckpoint;
        }
        ControllerStop::Continue
    }
}

/// Durable/accounted progress supplied to the pure controller-bound evaluator.
#[derive(Clone, Debug, Default, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ControllerProgress {
    /// Completed or attempted controller cycles.
    pub invocations: u32,
    /// Created prospective revisions.
    pub revisions: u32,
    /// Mutation count in the next proposal.
    pub mutations_in_proposal: u16,
    /// Added-node count in the next proposal.
    pub nodes_in_proposal: u16,
    /// Accounted elapsed time.
    pub elapsed_ms: u64,
    /// Accounted cost in millionths.
    pub cost_micros: u64,
    /// Accounted input units.
    pub input_units: u64,
    /// Accounted output units.
    pub output_units: u64,
    /// Accounted artifact bytes.
    pub artifact_bytes: u64,
    /// Process capability invocations.
    pub process_invocations: u32,
    /// Model capability invocations.
    pub model_invocations: u32,
    /// Failed cycles.
    pub failures: u16,
    /// Rejected proposals.
    pub rejections: u16,
    /// Current repeat nesting depth.
    pub repeat_depth: u16,
    /// Current child-workflow depth.
    pub child_depth: u16,
}

/// Inputs for constructing an ordinary bounded `Repeat` controller blueprint.
#[derive(Clone, Debug)]
pub struct ControllerBlueprintSpec {
    /// Workflow lineage for the controller wrapper.
    pub workflow: WorkflowId,
    /// Exact acyclic controller-cycle body.
    pub body: PinnedSubworkflow,
    /// Safe condition deciding whether another cycle is requested.
    pub continue_condition: Condition,
    /// Hard cross-cycle ceilings.
    pub limits: ControllerLimits,
    /// Revision author provenance.
    pub author: AuthorRef,
}

/// Builds an acyclic wrapper containing one explicit bounded `Repeat` and terminal.
pub fn build_controller_blueprint(
    spec: ControllerBlueprintSpec,
) -> Result<BlueprintRevision, ControlError> {
    let limits_metadata = BoundedJson::new(serde_json::to_value(&spec.limits)?)?;
    let iteration_limit = spec
        .limits
        .human_checkpoint_interval
        .unwrap_or(spec.limits.max_invocations)
        .min(spec.limits.max_invocations);
    let repeat = RepeatConfig::new(
        spec.body,
        spec.continue_condition,
        iteration_limit,
        RepeatBudget {
            max_duration_ms: Some(spec.limits.max_elapsed_ms),
            max_cost_micros: Some(spec.limits.max_cost_micros),
            max_cost_currency: Some(CostCurrencyCode::new("USD")?),
        },
        RepeatTermination::AwaitApproval,
    )?;
    let controller = Node::new(
        NodeId::new("controller-repeat")?,
        NodeKind::Repeat { config: repeat },
    )?
    .with_control_output(PortId::new("out")?)?;
    let terminal = Node::new(
        NodeId::new("controller-complete")?,
        NodeKind::Terminal {
            outcome: TerminalOutcome::Success,
        },
    )?
    .with_control_input(PortId::new("in")?)?;
    let edge = Edge::new(
        EdgeId::new("controller-finished")?,
        EdgeKind::Control,
        controller.id().clone(),
        PortId::new("out")?,
        terminal.id().clone(),
        PortId::new("in")?,
    );
    let batch = MutationBatch::new(vec![
        Mutation::SetMetadata {
            metadata: BlueprintMetadata::new(
                "bounded-audited-controller",
                "Explicit repeat controller; every non-repeat ceiling is enforced by the controller-cycle validator before proposal submission.",
                std::collections::BTreeSet::from(["bounded-controller".to_owned()]),
                BTreeMap::from([(
                    ExtensionKey::new("org.milkdrift/controller-limits")?,
                    limits_metadata,
                )]),
            )?,
        },
        Mutation::AddNode { node: controller },
        Mutation::AddNode { node: terminal },
        Mutation::AddEdge { edge },
    ])?;
    Ok(BlueprintRevision::genesis(
        spec.workflow,
        batch,
        spec.author,
        "bounded audited controller pattern",
    )?)
}
