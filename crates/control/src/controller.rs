use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    sync::Arc,
};

use milkdrift_blueprint::{
    AuthorRef, BlueprintMetadata, BlueprintRevision, Condition, CostCurrencyCode, Edge, EdgeId,
    EdgeKind, Mutation, MutationBatch, Node, NodeId, NodeKind, PinnedSubworkflow, PortId,
    RepeatBudget, RepeatConfig, RepeatTermination, TerminalOutcome, WorkflowId,
};
use milkdrift_capability::{BoundedJson, CapabilityCategory, ExtensionKey};
use milkdrift_contracts::{JsonLimits, canonical_json_bytes};
use milkdrift_persistence::{
    ControllerAssessmentBoundary, ControllerAssessmentOutcome, CurrencyCode, NodeExecutionId,
    RevisionStore,
};
use milkdrift_runtime::{
    CONTROLLER_POLICY_EXTENSION_KEY, ControllerAssessment, ControllerAssessmentContext,
    ControllerLifecycle, RunProjection, RuntimeError,
};
use serde::{Deserialize, Serialize};

use crate::{ControlError, ControllerId, ControllerPolicyDigest};

/// Current strict controller-policy schema.
pub const CONTROLLER_POLICY_SCHEMA_VERSION_V1: u32 = 1;
const CONTROLLER_POLICY_JSON_LIMITS: JsonLimits = JsonLimits {
    maximum_depth: 48,
    maximum_string_bytes: 8_192,
    maximum_key_bytes: 192,
    maximum_container_items: 512,
};

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
    #[allow(clippy::too_many_arguments)] // One validated control operation keeps its authority and optimistic facts explicit.
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
        if max_invocations >= 10_000
            || human_checkpoint_interval.is_some_and(|value| value > max_invocations)
        {
            return Err(ControlError::InvalidContract(
                "controller invocations must be below the runtime structural ceiling and checkpoint interval cannot exceed them"
                    .to_owned(),
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

    /// Maximum elapsed controller lifetime in caller-clock milliseconds.
    #[must_use]
    pub const fn max_elapsed_ms(&self) -> u64 {
        self.max_elapsed_ms
    }

    /// Maximum cost in millionths of the policy currency.
    #[must_use]
    pub const fn max_cost_micros(&self) -> u64 {
        self.max_cost_micros
    }

    /// Maximum observed input units.
    #[must_use]
    pub const fn max_input_units(&self) -> u64 {
        self.max_input_units
    }

    /// Maximum observed output units.
    #[must_use]
    pub const fn max_output_units(&self) -> u64 {
        self.max_output_units
    }

    /// Maximum logical bytes published by controller cycles.
    #[must_use]
    pub const fn max_artifact_bytes(&self) -> u64 {
        self.max_artifact_bytes
    }

    /// Maximum exact process-category invocations.
    #[must_use]
    pub const fn max_process_invocations(&self) -> u32 {
        self.max_process_invocations
    }

    /// Maximum exact model-category invocations.
    #[must_use]
    pub const fn max_model_invocations(&self) -> u32 {
        self.max_model_invocations
    }

    /// Maximum failed controller cycles.
    #[must_use]
    pub const fn max_failures(&self) -> u16 {
        self.max_failures
    }

    /// Maximum rejected controller proposals.
    #[must_use]
    pub const fn max_rejections(&self) -> u16 {
        self.max_rejections
    }

    /// Maximum static repeat nesting depth.
    #[must_use]
    pub const fn max_repeat_depth(&self) -> u16 {
        self.max_repeat_depth
    }

    /// Maximum static subworkflow nesting depth.
    #[must_use]
    pub const fn max_child_depth(&self) -> u16 {
        self.max_child_depth
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
                progress.unknown_cost_observations > 0
                    || progress.cost_micros >= self.max_cost_micros,
                ControllerBound::Cost,
            ),
            (
                progress.unknown_input_observations > 0
                    || progress.input_units >= self.max_input_units,
                ControllerBound::InputUnits,
            ),
            (
                progress.unknown_output_observations > 0
                    || progress.output_units >= self.max_output_units,
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
            progress.invocations > progress.checkpoint_approved_invocations
                && progress.invocations.is_multiple_of(interval)
        }) {
            return ControllerStop::HumanCheckpoint;
        }
        ControllerStop::Continue
    }

    fn bound_fact(
        &self,
        progress: &ControllerProgress,
        bound: ControllerBound,
    ) -> (Option<u64>, u64, bool) {
        match bound {
            ControllerBound::Invocations => (
                Some(u64::from(progress.invocations)),
                u64::from(self.max_invocations),
                false,
            ),
            ControllerBound::Revisions => (
                Some(u64::from(progress.revisions)),
                u64::from(self.max_revisions),
                false,
            ),
            ControllerBound::MutationsPerProposal => (
                Some(u64::from(progress.mutations_in_proposal)),
                u64::from(self.max_mutations_per_proposal),
                false,
            ),
            ControllerBound::NodesPerProposal => (
                Some(u64::from(progress.nodes_in_proposal)),
                u64::from(self.max_nodes_per_proposal),
                false,
            ),
            ControllerBound::ElapsedTime => (Some(progress.elapsed_ms), self.max_elapsed_ms, false),
            ControllerBound::Cost if progress.unknown_cost_observations > 0 => {
                (None, self.max_cost_micros, true)
            }
            ControllerBound::Cost => (Some(progress.cost_micros), self.max_cost_micros, false),
            ControllerBound::InputUnits if progress.unknown_input_observations > 0 => {
                (None, self.max_input_units, true)
            }
            ControllerBound::InputUnits => {
                (Some(progress.input_units), self.max_input_units, false)
            }
            ControllerBound::OutputUnits if progress.unknown_output_observations > 0 => {
                (None, self.max_output_units, true)
            }
            ControllerBound::OutputUnits => {
                (Some(progress.output_units), self.max_output_units, false)
            }
            ControllerBound::ArtifactBytes => (
                Some(progress.artifact_bytes),
                self.max_artifact_bytes,
                false,
            ),
            ControllerBound::ProcessInvocations => (
                Some(u64::from(progress.process_invocations)),
                u64::from(self.max_process_invocations),
                false,
            ),
            ControllerBound::ModelInvocations => (
                Some(u64::from(progress.model_invocations)),
                u64::from(self.max_model_invocations),
                false,
            ),
            ControllerBound::Failures => (
                Some(u64::from(progress.failures)),
                u64::from(self.max_failures),
                false,
            ),
            ControllerBound::Rejections => (
                Some(u64::from(progress.rejections)),
                u64::from(self.max_rejections),
                false,
            ),
            ControllerBound::RepeatDepth => (
                Some(u64::from(progress.repeat_depth)),
                u64::from(self.max_repeat_depth),
                false,
            ),
            ControllerBound::ChildDepth => (
                Some(u64::from(progress.child_depth)),
                u64::from(self.max_child_depth),
                false,
            ),
            ControllerBound::HumanCheckpoint => (
                Some(u64::from(progress.invocations)),
                u64::from(
                    self.human_checkpoint_interval
                        .unwrap_or(self.max_invocations),
                ),
                false,
            ),
        }
    }
}

/// Durable/accounted progress supplied to the pure controller-bound evaluator.
#[derive(Clone, Debug, Default, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ControllerProgress {
    /// Accepted controller cycles whose attached body child reached any terminal outcome.
    /// Pre-entry assessment failures do not count because no body child was created.
    pub invocations: u32,
    /// Revision-adoption requests issued by the run's frozen controller actor.
    pub revisions: u32,
    /// Exact decoded mutation count in the proposal currently being validated.
    pub mutations_in_proposal: u16,
    /// Exact `AddNode`/`InstantiateSubworkflow` count in that validated proposal.
    pub nodes_in_proposal: u16,
    /// Caller-clock milliseconds since the durable execution/first-assessment start boundary.
    pub elapsed_ms: u64,
    /// Sum of durable attempt cost in millionths of the policy currency.
    pub cost_micros: u64,
    /// Sum of durable model/process input-unit observations.
    pub input_units: u64,
    /// Sum of durable model/process output-unit observations.
    pub output_units: u64,
    /// Sum of logical published artifact bytes from controller-owned child runs.
    pub artifact_bytes: u64,
    /// Admitted attempts whose frozen resolved descriptor category is `Process`.
    pub process_invocations: u32,
    /// Admitted attempts whose frozen resolved descriptor category is `Model`.
    pub model_invocations: u32,
    /// Terminal controller body children with failed or cancelled run outcomes.
    pub failures: u16,
    /// Rejected reconciliation plans for revision requests attributed to the run actor.
    pub rejections: u16,
    /// Maximum reachable repeat nesting depth in the exact pinned body graph.
    pub repeat_depth: u16,
    /// Maximum reachable subworkflow nesting depth in the exact pinned body graph.
    pub child_depth: u16,
    /// Model/process attempts whose cost observation is still unknown.
    pub unknown_cost_observations: u32,
    /// Model/process attempts whose input-unit observation is still unknown.
    pub unknown_input_observations: u32,
    /// Model/process attempts whose output-unit observation is still unknown.
    pub unknown_output_observations: u32,
    /// Completed-cycle frontier already continued through an authorized checkpoint.
    pub checkpoint_approved_invocations: u32,
}

/// Exact wrapper binding embedded without creating a content-address self-reference.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ControllerWrapperBinding {
    workflow: WorkflowId,
    node: NodeId,
    /// The containing immutable revision is the exact wrapper revision at read time.
    containing_revision: bool,
}

/// Explicit missing-usage behavior for hard controller resource bounds.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum UnknownUsagePolicy {
    /// Any missing model/process cost or unit observation stops before continuation.
    FailClosed,
}

/// Immutable behavior when a controller ceiling is reached.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ControllerStopBehavior {
    /// Deterministically fail the controller repeat without provider retry.
    FailController,
}

/// Ordinary authority operations required by controller control-plane transitions.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ControllerOperationRequirements {
    assessment_operation: String,
    proposal_operation: String,
    approval_operation: String,
    application_operation: String,
}

impl Default for ControllerOperationRequirements {
    fn default() -> Self {
        Self {
            assessment_operation: "controller.assess".to_owned(),
            proposal_operation: "workflow.propose_revision".to_owned(),
            approval_operation: "workflow.apply_proposal".to_owned(),
            application_operation: "workflow.apply_proposal".to_owned(),
        }
    }
}

/// Every executable field covered by one immutable controller-policy digest.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ControllerPolicy {
    identity: ControllerId,
    limits: ControllerLimits,
    wrapper: ControllerWrapperBinding,
    body: PinnedSubworkflow,
    checkpoint_interval: Option<u32>,
    stop_behavior: ControllerStopBehavior,
    unknown_usage: UnknownUsagePolicy,
    cost_currency: CostCurrencyCode,
    operations: ControllerOperationRequirements,
    labels: BTreeSet<String>,
    provenance: BTreeMap<String, String>,
}

impl ControllerPolicy {
    /// Stable policy lineage identity.
    #[must_use]
    pub const fn identity(&self) -> &ControllerId {
        &self.identity
    }

    /// Immutable cumulative limits.
    #[must_use]
    pub const fn limits(&self) -> &ControllerLimits {
        &self.limits
    }

    /// Exact pinned cycle body.
    #[must_use]
    pub const fn body(&self) -> &PinnedSubworkflow {
        &self.body
    }

    /// Exact checkpoint interval.
    #[must_use]
    pub const fn checkpoint_interval(&self) -> Option<u32> {
        self.checkpoint_interval
    }

    /// Immutable reached-bound behavior.
    #[must_use]
    pub const fn stop_behavior(&self) -> ControllerStopBehavior {
        self.stop_behavior
    }

    /// Exact cost ledger used for `max_cost_micros`.
    #[must_use]
    pub const fn cost_currency(&self) -> &CostCurrencyCode {
        &self.cost_currency
    }

    fn validate(&self) -> Result<(), ControlError> {
        if !self.wrapper.containing_revision
            || self.checkpoint_interval != self.limits.human_checkpoint_interval()
            || self.operations != ControllerOperationRequirements::default()
            || self.labels.len() > 32
            || self.provenance.len() > 32
            || self
                .labels
                .iter()
                .any(|value| value.is_empty() || value.len() > 96)
            || self.provenance.iter().any(|(key, value)| {
                key.is_empty() || key.len() > 96 || value.is_empty() || value.len() > 512
            })
        {
            return Err(ControlError::InvalidContract(
                "controller policy wrapper, checkpoint, labels, or provenance are invalid"
                    .to_owned(),
            ));
        }
        Ok(())
    }
}

/// Versioned strict controller-policy document embedded in semantic revision identity.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ControllerPolicyDocument {
    schema_version: u32,
    policy: ControllerPolicy,
    digest: ControllerPolicyDigest,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ControllerPolicyDocumentWire {
    schema_version: u32,
    policy: ControllerPolicy,
    digest: ControllerPolicyDigest,
}

impl ControllerPolicyDocument {
    fn new(policy: ControllerPolicy) -> Result<Self, ControlError> {
        policy.validate()?;
        let bytes = canonical_json_bytes(&policy, CONTROLLER_POLICY_JSON_LIMITS)
            .map_err(|error| ControlError::InvalidContract(format!("{error:?}")))?;
        Ok(Self {
            schema_version: CONTROLLER_POLICY_SCHEMA_VERSION_V1,
            policy,
            digest: ControllerPolicyDigest::for_bytes(&bytes),
        })
    }

    fn from_wire(wire: ControllerPolicyDocumentWire) -> Result<Self, ControlError> {
        if wire.schema_version != CONTROLLER_POLICY_SCHEMA_VERSION_V1 {
            return Err(ControlError::UnsupportedVersion {
                document: "controller_policy",
                found: wire.schema_version,
                supported: CONTROLLER_POLICY_SCHEMA_VERSION_V1,
            });
        }
        let expected = wire.digest;
        let document = Self::new(wire.policy)?;
        if document.digest != expected {
            return Err(ControlError::InvalidContract(
                "controller policy digest does not match every executable field".to_owned(),
            ));
        }
        Ok(document)
    }

    /// Parses and validates the one policy bound to an exact containing revision/node.
    pub fn from_revision(
        revision: &BlueprintRevision,
        controller_node: &NodeId,
    ) -> Result<Option<Self>, ControlError> {
        let key = ExtensionKey::new(CONTROLLER_POLICY_EXTENSION_KEY)?;
        let Some(value) = revision.semantic().metadata().extensions().get(&key) else {
            return Ok(None);
        };
        let wire: ControllerPolicyDocumentWire = serde_json::from_value(value.value().clone())?;
        let document = Self::from_wire(wire)?;
        if document.policy.wrapper.workflow != *revision.semantic().workflow()
            || document.policy.wrapper.node != *controller_node
        {
            return Err(ControlError::InvalidContract(
                "controller policy wrapper binding does not match its containing revision"
                    .to_owned(),
            ));
        }
        let node = revision
            .semantic()
            .nodes()
            .get(controller_node)
            .ok_or_else(|| {
                ControlError::InvalidContract(
                    "controller policy names a node absent from its containing revision".to_owned(),
                )
            })?;
        let NodeKind::Repeat { config } = node.kind() else {
            return Err(ControlError::InvalidContract(
                "controller policy must bind an ordinary repeat node".to_owned(),
            ));
        };
        if config.body() != &document.policy.body
            || config.termination() != RepeatTermination::AwaitApproval
            || config.maximum_iterations()
                != document.policy.limits.max_invocations().saturating_add(1)
        {
            return Err(ControlError::InvalidContract(
                "controller repeat configuration contradicts its immutable policy".to_owned(),
            ));
        }
        Ok(Some(document))
    }

    /// Reads the exact controller node named by a marked revision and validates the binding.
    pub fn from_controller_revision(
        revision: &BlueprintRevision,
    ) -> Result<Option<(NodeId, Self)>, ControlError> {
        let key = ExtensionKey::new(CONTROLLER_POLICY_EXTENSION_KEY)?;
        let Some(value) = revision.semantic().metadata().extensions().get(&key) else {
            return Ok(None);
        };
        let wire: ControllerPolicyDocumentWire = serde_json::from_value(value.value().clone())?;
        let node = wire.policy.wrapper.node.clone();
        Self::from_revision(revision, &node).map(|document| document.map(|value| (node, value)))
    }

    /// Current exact schema version.
    #[must_use]
    pub const fn schema_version(&self) -> u32 {
        self.schema_version
    }

    /// Validated policy body.
    #[must_use]
    pub const fn policy(&self) -> &ControllerPolicy {
        &self.policy
    }

    /// Digest binding every executable field.
    #[must_use]
    pub const fn digest(&self) -> &ControllerPolicyDigest {
        &self.digest
    }
}

impl<'de> Deserialize<'de> for ControllerPolicyDocument {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        Self::from_wire(ControllerPolicyDocumentWire::deserialize(deserializer)?)
            .map_err(serde::de::Error::custom)
    }
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
///
/// The returned immutable revision carries controller-policy schema 1 in semantic
/// metadata. Its repeat maximum is only a structural backstop; the installed
/// [`ControllerLifecycleOwner`] owns every cumulative ceiling and must assess a
/// marked controller before runtime can create a cycle.
///
/// # Example
///
/// ```no_run
/// use milkdrift_blueprint::{
///     AuthorRef, BlueprintRevision, Condition, Mutation, MutationBatch, Node, NodeId,
///     NodeKind, PinnedSubworkflow, TerminalOutcome, WorkflowId, WorkflowInterface,
/// };
/// use milkdrift_control::{
///     ControllerBlueprintSpec, ControllerLimits, build_controller_blueprint,
/// };
///
/// # fn build() -> Result<BlueprintRevision, Box<dyn std::error::Error>> {
/// let body = BlueprintRevision::genesis(
///     WorkflowId::new("review-cycle")?,
///     MutationBatch::new(vec![Mutation::AddNode {
///         node: Node::new(
///             NodeId::new("done")?,
///             NodeKind::Terminal { outcome: TerminalOutcome::Success },
///         )?,
///     }])?,
///     AuthorRef::new("human:operator")?,
///     "bounded controller body",
/// )?;
/// let body_ref = PinnedSubworkflow::new(
///     body.semantic().workflow().clone(),
///     body.id().clone(),
///     WorkflowInterface::new([], [])?,
/// );
/// let limits = ControllerLimits::new(
///     20, 4, 16, 8, 3_600_000, 2_000_000, 200_000, 200_000,
///     16_777_216, 20, 20, 4, 4, 3, 3, Some(5),
/// )?;
/// Ok(build_controller_blueprint(ControllerBlueprintSpec {
///     workflow: WorkflowId::new("bounded-review-controller")?,
///     body: body_ref,
///     continue_condition: Condition::Constant { value: true },
///     limits,
///     author: AuthorRef::new("human:operator")?,
/// })?)
/// # }
/// ```
pub fn build_controller_blueprint(
    spec: ControllerBlueprintSpec,
) -> Result<BlueprintRevision, ControlError> {
    let controller_node = NodeId::new("controller-repeat")?;
    let policy = ControllerPolicyDocument::new(ControllerPolicy {
        identity: ControllerId::new(format!("controller:{}", spec.workflow.as_str()))?,
        limits: spec.limits.clone(),
        wrapper: ControllerWrapperBinding {
            workflow: spec.workflow.clone(),
            node: controller_node.clone(),
            containing_revision: true,
        },
        body: spec.body.clone(),
        checkpoint_interval: spec.limits.human_checkpoint_interval(),
        stop_behavior: ControllerStopBehavior::FailController,
        unknown_usage: UnknownUsagePolicy::FailClosed,
        cost_currency: CostCurrencyCode::new("USD")?,
        operations: ControllerOperationRequirements::default(),
        labels: BTreeSet::from(["bounded-controller".to_owned()]),
        provenance: BTreeMap::from([(
            "builder".to_owned(),
            "milkdrift-control/controller-policy-v1".to_owned(),
        )]),
    })?;
    let policy_metadata = BoundedJson::new(serde_json::to_value(&policy)?)?;
    let repeat = RepeatConfig::new(
        spec.body,
        spec.continue_condition,
        spec.limits.max_invocations().saturating_add(1),
        RepeatBudget {
            max_duration_ms: None,
            max_cost_micros: None,
            max_cost_currency: None,
        },
        RepeatTermination::AwaitApproval,
    )?;
    let controller = Node::new(controller_node, NodeKind::Repeat { config: repeat })?
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
                "Ordinary repeat controller governed by a typed immutable policy and durable lifecycle assessments.",
                std::collections::BTreeSet::from(["bounded-controller".to_owned()]),
                BTreeMap::from([(
                    ExtensionKey::new(CONTROLLER_POLICY_EXTENSION_KEY)?,
                    policy_metadata,
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

/// Canonical production owner for typed controller parsing, accounting, and assessment.
pub struct ControllerLifecycleOwner {
    revisions: Arc<dyn RevisionStore>,
}

impl ControllerLifecycleOwner {
    /// Constructs the lifecycle over the same immutable revision owner used by runtime/control.
    #[must_use]
    pub fn new(revisions: Arc<dyn RevisionStore>) -> Self {
        Self { revisions }
    }

    /// Derives current progress solely from durable projection, usage, revision, and decision facts.
    pub fn progress(
        &self,
        document: &ControllerPolicyDocument,
        projection: &RunProjection,
        execution: &NodeExecutionId,
        observed_at_ms: u64,
    ) -> Result<ControllerProgress, ControlError> {
        let usage = projection.subworkflow_usage_for_execution(execution);
        let previous_assessment = projection.controller_assessment(execution);
        let started_at = projection
            .node_executions()
            .get(execution)
            .map(|execution| execution.created_at())
            .or_else(|| previous_assessment.map(|assessment| assessment.started_at()))
            .ok_or_else(|| {
                ControlError::InvalidContract(
                    "controller progress references neither an active execution nor a durable assessment"
                        .to_owned(),
                )
            })?;
        let mut progress = ControllerProgress {
            invocations: checked_u32(usage.map_or(0, |value| value.completed_children()))?,
            elapsed_ms: observed_at_ms.saturating_sub(started_at.get()),
            cost_micros: usage
                .and_then(|value| {
                    CurrencyCode::new(document.policy.cost_currency.as_str())
                        .ok()
                        .and_then(|currency| value.cost_micros().get(&currency).copied())
                })
                .unwrap_or(0),
            input_units: usage.and_then(|value| value.input_units()).unwrap_or(0),
            output_units: usage.and_then(|value| value.output_units()).unwrap_or(0),
            artifact_bytes: usage.map_or(0, |value| value.artifact_bytes()),
            process_invocations: checked_u32(usage.map_or(0, |value| value.process_invocations()))?,
            model_invocations: checked_u32(usage.map_or(0, |value| value.model_invocations()))?,
            failures: checked_u16(usage.map_or(0, |value| value.failed_children()))?,
            revisions: checked_u32(projection.run_actor_revision_requests())?,
            rejections: checked_u16(projection.run_actor_rejections())?,
            unknown_cost_observations: checked_u32(
                usage.map_or(0, |value| value.unknown_cost_usage()),
            )?,
            unknown_input_observations: checked_u32(
                usage.map_or(0, |value| value.unknown_input_usage()),
            )?,
            unknown_output_observations: checked_u32(
                usage.map_or(0, |value| value.unknown_output_usage()),
            )?,
            ..ControllerProgress::default()
        };
        if let Some(previous) = previous_assessment {
            let previous: ControllerProgress =
                serde_json::from_value(previous.progress().value().clone())?;
            progress.checkpoint_approved_invocations = previous
                .checkpoint_approved_invocations
                .min(progress.invocations);
        }
        let (repeat_depth, child_depth, _, _) = self.body_shape(document.policy.body())?;
        progress.repeat_depth = repeat_depth;
        progress.child_depth = child_depth;
        Ok(progress)
    }

    /// Builds the bounded controller read model for one exact logical occurrence.
    pub fn status(
        &self,
        run: &milkdrift_workspace::RunId,
        projection: &RunProjection,
        execution: &NodeExecutionId,
        observed_at_ms: u64,
    ) -> Result<crate::ControllerStatusRead, ControlError> {
        let latest = projection.controller_assessment(execution);
        let execution_view = projection.current_node_execution(execution);
        let (governing_revision, controller_node, execution_completed) =
            if let Some(view) = execution_view {
                (
                    view.revision().clone(),
                    view.node().clone(),
                    matches!(
                        view.state(),
                        milkdrift_runtime::NodeExecutionState::Terminal(_)
                            | milkdrift_runtime::NodeExecutionState::CancelledBeforeDispatch
                            | milkdrift_runtime::NodeExecutionState::RemovedProspectively(_)
                    ),
                )
            } else if let Some(assessment) = latest {
                (
                    assessment.governing_revision().clone(),
                    assessment.controller_node().clone(),
                    matches!(
                        projection.lifecycle(),
                        milkdrift_runtime::RunLifecycle::Terminal(_)
                    ),
                )
            } else {
                return Err(ControlError::InvalidContract(
                "controller status references neither a current execution nor a durable assessment"
                    .to_owned(),
            ));
            };
        let revision = self
            .revisions
            .revision(&governing_revision)?
            .ok_or(ControlError::BaseRevisionNotFound)?;
        let document = ControllerPolicyDocument::from_revision(&revision, &controller_node)?
            .ok_or_else(|| {
                ControlError::InvalidContract(
                    "requested execution is not governed by a controller policy".to_owned(),
                )
            })?;
        let progress = self.progress(&document, projection, execution, observed_at_ms)?;
        let current = document.policy.limits.assess(&progress);
        let (checkpoint_id, reached_bound) = match latest.map(|value| value.outcome()) {
            Some(ControllerAssessmentOutcome::HumanCheckpoint { checkpoint_id }) => {
                (Some(checkpoint_id.clone()), None)
            }
            Some(ControllerAssessmentOutcome::BoundReached { bound, .. }) => {
                (None, bound_from_name(bound))
            }
            Some(ControllerAssessmentOutcome::Continue) | None if execution_completed => {
                (None, None)
            }
            Some(ControllerAssessmentOutcome::Continue) | None => match current {
                ControllerStop::HumanCheckpoint => (
                    Some(stable_controller_identity(
                        "checkpoint",
                        document.digest.as_str(),
                        run.as_str(),
                        execution.as_str(),
                        progress.invocations,
                    )),
                    None,
                ),
                ControllerStop::BoundReached { bound } => (None, Some(bound)),
                ControllerStop::Continue => (None, None),
            },
        };
        let state = if reached_bound.is_some() {
            crate::ControllerLifecycleState::BoundReached
        } else if checkpoint_id.is_some() {
            crate::ControllerLifecycleState::AwaitingHumanCheckpoint
        } else if execution_completed {
            crate::ControllerLifecycleState::Completed
        } else {
            crate::ControllerLifecycleState::Eligible
        };
        Ok(crate::ControllerStatusRead {
            controller: document.policy.identity.clone(),
            policy_digest: document.digest.clone(),
            run: run.clone(),
            revision: revision.id().clone(),
            node: controller_node,
            execution: execution.clone(),
            state,
            progress,
            limits: document.policy.limits.clone(),
            last_assessment_sequence: latest.map(|value| value.recorded_sequence()),
            last_assessment_time: latest.map(|value| value.recorded_at().get()),
            last_assessment_id: latest.map(|value| value.assessment_id().to_owned()),
            checkpoint_id,
            reached_bound,
            cycle_eligible: state == crate::ControllerLifecycleState::Eligible,
        })
    }

    /// Assesses exact decoded proposal size before the candidate revision is persisted.
    pub fn assess_proposal(
        &self,
        run: &milkdrift_workspace::RunId,
        projection: &RunProjection,
        proposal: &crate::WorkflowProposal,
        observed_at_ms: u64,
    ) -> Result<(), ControlError> {
        if proposal.run() != Some(run) {
            return Err(ControlError::InvalidContract(
                "controller proposal assessment run does not match the proposal".to_owned(),
            ));
        }
        let controller_actor = projection.execution_authority().map(|value| value.actor());
        if controller_actor != Some(proposal.proposer()) {
            return Ok(());
        }
        let mut controllers = Vec::new();
        for execution in projection.node_executions().values() {
            let revision = self
                .revisions
                .revision(execution.revision())?
                .ok_or(ControlError::BaseRevisionNotFound)?;
            if ControllerPolicyDocument::from_revision(&revision, execution.node())?.is_some() {
                controllers.push(execution.execution().clone());
            }
        }
        if controllers.len() > 1 {
            return Err(ControlError::InvalidContract(
                "controller-generated proposals are unsupported when one run has multiple active controllers; use an exact controller-scoped operation"
                    .to_owned(),
            ));
        }
        let Some(execution) = controllers.first() else {
            return Ok(());
        };
        let execution_view = projection
            .node_executions()
            .get(execution)
            .ok_or_else(|| ControlError::InvalidContract("controller disappeared".to_owned()))?;
        let revision = self
            .revisions
            .revision(execution_view.revision())?
            .ok_or(ControlError::BaseRevisionNotFound)?;
        let document = ControllerPolicyDocument::from_revision(&revision, execution_view.node())?
            .ok_or_else(|| {
            ControlError::InvalidContract("controller policy disappeared".to_owned())
        })?;
        let mut progress = self.progress(&document, projection, execution, observed_at_ms)?;
        progress.mutations_in_proposal =
            u16::try_from(proposal.mutation().operations().len()).unwrap_or(u16::MAX);
        progress.nodes_in_proposal = u16::try_from(
            proposal
                .mutation()
                .operations()
                .iter()
                .filter(|mutation| {
                    matches!(
                        mutation,
                        Mutation::AddNode { .. } | Mutation::InstantiateSubworkflow { .. }
                    )
                })
                .count(),
        )
        .unwrap_or(u16::MAX);
        match document.policy.limits.assess(&progress) {
            ControllerStop::Continue => Ok(()),
            ControllerStop::HumanCheckpoint => Err(ControlError::ProposalState(
                "controller proposal requires its durable human checkpoint first".to_owned(),
            )),
            ControllerStop::BoundReached { bound } => Err(ControlError::Bounds {
                location: format!("controller.proposal.{}", bound_name(bound)),
                reason: "controller proposal exceeds its immutable cumulative policy".to_owned(),
            }),
        }
    }

    /// Reassesses cumulative controller limits before approving or applying one
    /// controller-authored prospective revision. The proposal's immutable revision
    /// reason binds its proposer, so no model-supplied counter or authority claim is
    /// consumed here.
    pub fn assess_proposal_transition(
        &self,
        run: &milkdrift_workspace::RunId,
        projection: &RunProjection,
        proposed_revision: &BlueprintRevision,
        boundary: ControllerAssessmentBoundary,
        observed_at_ms: u64,
    ) -> Result<(), ControlError> {
        if !matches!(
            boundary,
            ControllerAssessmentBoundary::ProposalApproval
                | ControllerAssessmentBoundary::ProposalApplication
        ) {
            return Err(ControlError::InvalidContract(
                "proposal transition assessment requires an approval/application boundary"
                    .to_owned(),
            ));
        }
        let Some(controller_actor) = projection.execution_authority().map(|value| value.actor())
        else {
            return Ok(());
        };
        if !proposed_revision
            .reason()
            .contains(&format!("proposer={controller_actor};"))
        {
            return Ok(());
        }
        let mut controllers = Vec::new();
        for execution in projection.node_executions().values() {
            let revision = self
                .revisions
                .revision(execution.revision())?
                .ok_or(ControlError::BaseRevisionNotFound)?;
            if ControllerPolicyDocument::from_revision(&revision, execution.node())?.is_some() {
                controllers.push((
                    execution.execution().clone(),
                    revision,
                    execution.node().clone(),
                ));
            }
        }
        let [(execution, revision, node)] = controllers.as_slice() else {
            return if controllers.is_empty() {
                Ok(())
            } else {
                Err(ControlError::InvalidContract(
                    "controller-authored proposal transitions are unsupported when one run has multiple active controllers"
                        .to_owned(),
                ))
            };
        };
        let document =
            ControllerPolicyDocument::from_revision(revision, node)?.ok_or_else(|| {
                ControlError::InvalidContract("controller policy disappeared".to_owned())
            })?;
        let progress = self.progress(&document, projection, execution, observed_at_ms)?;
        match self.outcome(&document, &progress, boundary, run, execution)? {
            ControllerAssessmentOutcome::Continue => Ok(()),
            ControllerAssessmentOutcome::HumanCheckpoint { .. } => {
                Err(ControlError::ProposalState(
                    "controller proposal transition requires its durable human checkpoint first"
                        .to_owned(),
                ))
            }
            ControllerAssessmentOutcome::BoundReached { bound, .. } => Err(ControlError::Bounds {
                location: format!("controller.proposal.{bound}"),
                reason: "controller proposal transition reached its immutable cumulative policy"
                    .to_owned(),
            }),
        }
    }

    fn body_shape(&self, body: &PinnedSubworkflow) -> Result<(u16, u16, u32, u32), ControlError> {
        let mut pending = VecDeque::from([(body.revision().clone(), 1_u16, 1_u16)]);
        let mut visited = BTreeSet::new();
        let mut repeat_depth = 1_u16;
        let mut child_depth = 1_u16;
        let mut process = 0_u32;
        let mut model = 0_u32;
        while let Some((revision_id, repeats, children)) = pending.pop_front() {
            if !visited.insert(revision_id.clone()) {
                continue;
            }
            if visited.len() > 512 {
                return Err(ControlError::Bounds {
                    location: "controller.body_revision_graph".to_owned(),
                    reason: "controller body graph exceeds 512 immutable revisions".to_owned(),
                });
            }
            let revision = self
                .revisions
                .revision(&revision_id)?
                .ok_or(ControlError::BaseRevisionNotFound)?;
            for node in revision.semantic().nodes().values() {
                match node.kind() {
                    NodeKind::Task { config } => {
                        let categories = config.requirement().categories();
                        // An unconstrained task can resolve to either resource-bearing
                        // category. Count it conservatively in both pre-entry ceilings;
                        // exact admitted attempts are later classified from the frozen
                        // resolved descriptor snapshot.
                        if categories.is_empty()
                            || categories.contains(&CapabilityCategory::Process)
                        {
                            process = process.checked_add(1).ok_or_else(|| {
                                ControlError::InvalidContract(
                                    "controller process shape overflow".to_owned(),
                                )
                            })?;
                        }
                        if categories.is_empty() || categories.contains(&CapabilityCategory::Model)
                        {
                            model = model.checked_add(1).ok_or_else(|| {
                                ControlError::InvalidContract(
                                    "controller model shape overflow".to_owned(),
                                )
                            })?;
                        }
                    }
                    NodeKind::Repeat { config } => {
                        let next = repeats.checked_add(1).ok_or_else(|| {
                            ControlError::InvalidContract(
                                "controller repeat depth overflow".to_owned(),
                            )
                        })?;
                        repeat_depth = repeat_depth.max(next);
                        pending.push_back((config.body().revision().clone(), next, children));
                    }
                    NodeKind::Subworkflow { reference } => {
                        let next = children.checked_add(1).ok_or_else(|| {
                            ControlError::InvalidContract(
                                "controller child depth overflow".to_owned(),
                            )
                        })?;
                        child_depth = child_depth.max(next);
                        pending.push_back((reference.revision().clone(), repeats, next));
                    }
                    NodeKind::Reducer { .. }
                    | NodeKind::Branch { .. }
                    | NodeKind::Fork { .. }
                    | NodeKind::Join { .. }
                    | NodeKind::Wait { .. }
                    | NodeKind::SignalWait { .. }
                    | NodeKind::Terminal { .. } => {}
                }
            }
        }
        Ok((repeat_depth, child_depth, process, model))
    }

    fn outcome(
        &self,
        document: &ControllerPolicyDocument,
        progress: &ControllerProgress,
        boundary: ControllerAssessmentBoundary,
        run: &milkdrift_workspace::RunId,
        execution: &NodeExecutionId,
    ) -> Result<ControllerAssessmentOutcome, ControlError> {
        let stop = if boundary == ControllerAssessmentBoundary::CheckpointContinuation {
            let mut continued = progress.clone();
            continued.checkpoint_approved_invocations = continued.invocations;
            document.policy.limits.assess(&continued)
        } else {
            document.policy.limits.assess(progress)
        };
        match stop {
            ControllerStop::Continue => {
                if matches!(
                    boundary,
                    ControllerAssessmentBoundary::Activation
                        | ControllerAssessmentBoundary::CycleEntry
                ) {
                    let (_, _, process, model) = self.body_shape(document.policy.body())?;
                    if progress
                        .process_invocations
                        .checked_add(process)
                        .is_none_or(|value| {
                            value > document.policy.limits.max_process_invocations()
                        })
                    {
                        return Ok(ControllerAssessmentOutcome::BoundReached {
                            bound: bound_name(ControllerBound::ProcessInvocations).to_owned(),
                            current: Some(
                                u64::from(progress.process_invocations)
                                    .saturating_add(u64::from(process)),
                            ),
                            limit: u64::from(document.policy.limits.max_process_invocations()),
                            unknown_usage: false,
                        });
                    }
                    if progress
                        .model_invocations
                        .checked_add(model)
                        .is_none_or(|value| value > document.policy.limits.max_model_invocations())
                    {
                        return Ok(ControllerAssessmentOutcome::BoundReached {
                            bound: bound_name(ControllerBound::ModelInvocations).to_owned(),
                            current: Some(
                                u64::from(progress.model_invocations)
                                    .saturating_add(u64::from(model)),
                            ),
                            limit: u64::from(document.policy.limits.max_model_invocations()),
                            unknown_usage: false,
                        });
                    }
                }
                Ok(ControllerAssessmentOutcome::Continue)
            }
            ControllerStop::HumanCheckpoint => Ok(ControllerAssessmentOutcome::HumanCheckpoint {
                checkpoint_id: stable_controller_identity(
                    "checkpoint",
                    document.digest.as_str(),
                    run.as_str(),
                    execution.as_str(),
                    progress.invocations,
                ),
            }),
            ControllerStop::BoundReached { bound } => {
                Ok(bound_outcome(&document.policy.limits, progress, bound))
            }
        }
    }
}

impl ControllerLifecycle for ControllerLifecycleOwner {
    fn assess(
        &self,
        context: &ControllerAssessmentContext<'_>,
    ) -> Result<Option<ControllerAssessment>, RuntimeError> {
        let document = ControllerPolicyDocument::from_revision(context.revision, context.node.id())
            .map_err(|error| RuntimeError::InvalidHistory(error.to_string()))?;
        let Some(document) = document else {
            return Ok(None);
        };
        let mut progress = self
            .progress(
                &document,
                context.projection,
                context.execution,
                context.observed_at.get(),
            )
            .map_err(|error| RuntimeError::InvalidHistory(error.to_string()))?;
        if context.boundary == ControllerAssessmentBoundary::CheckpointContinuation {
            progress.checkpoint_approved_invocations = progress.invocations;
        }
        let outcome = self
            .outcome(
                &document,
                &progress,
                context.boundary,
                context.run,
                context.execution,
            )
            .map_err(|error| RuntimeError::InvalidHistory(error.to_string()))?;
        let assessment_id = stable_controller_assessment_identity(
            document.digest.as_str(),
            context.run.as_str(),
            context.execution.as_str(),
            context.projection.sequence().get(),
            context.boundary,
            context.next_cycle,
        );
        let cycle_id = context.next_cycle.map(|cycle| {
            stable_controller_identity(
                "cycle",
                document.digest.as_str(),
                context.run.as_str(),
                context.execution.as_str(),
                cycle,
            )
        });
        let progress = BoundedJson::new(
            serde_json::to_value(progress)
                .map_err(|error| RuntimeError::InvalidHistory(error.to_string()))?,
        )
        .map_err(|error| RuntimeError::InvalidHistory(error.to_string()))?;
        Ok(Some(ControllerAssessment {
            controller_id: document.policy.identity.as_str().to_owned(),
            policy_digest: document.digest.as_str().to_owned(),
            assessment_id,
            cycle_id,
            progress,
            outcome,
        }))
    }
}

fn checked_u32(value: u64) -> Result<u32, ControlError> {
    u32::try_from(value).map_err(|_error| {
        ControlError::InvalidContract("controller u32 progress accounting overflow".to_owned())
    })
}

fn checked_u16(value: u64) -> Result<u16, ControlError> {
    u16::try_from(value).map_err(|_error| {
        ControlError::InvalidContract("controller u16 progress accounting overflow".to_owned())
    })
}

fn bound_outcome(
    limits: &ControllerLimits,
    progress: &ControllerProgress,
    bound: ControllerBound,
) -> ControllerAssessmentOutcome {
    let (current, limit, unknown_usage) = limits.bound_fact(progress, bound);
    ControllerAssessmentOutcome::BoundReached {
        bound: bound_name(bound).to_owned(),
        current,
        limit,
        unknown_usage,
    }
}

const fn bound_name(bound: ControllerBound) -> &'static str {
    match bound {
        ControllerBound::Invocations => "invocations",
        ControllerBound::Revisions => "revisions",
        ControllerBound::MutationsPerProposal => "mutations_per_proposal",
        ControllerBound::NodesPerProposal => "nodes_per_proposal",
        ControllerBound::ElapsedTime => "elapsed_time",
        ControllerBound::Cost => "cost",
        ControllerBound::InputUnits => "input_units",
        ControllerBound::OutputUnits => "output_units",
        ControllerBound::ArtifactBytes => "artifact_bytes",
        ControllerBound::ProcessInvocations => "process_invocations",
        ControllerBound::ModelInvocations => "model_invocations",
        ControllerBound::Failures => "failures",
        ControllerBound::Rejections => "rejections",
        ControllerBound::RepeatDepth => "repeat_depth",
        ControllerBound::ChildDepth => "child_depth",
        ControllerBound::HumanCheckpoint => "human_checkpoint",
    }
}

fn bound_from_name(value: &str) -> Option<ControllerBound> {
    Some(match value {
        "invocations" => ControllerBound::Invocations,
        "revisions" => ControllerBound::Revisions,
        "mutations_per_proposal" => ControllerBound::MutationsPerProposal,
        "nodes_per_proposal" => ControllerBound::NodesPerProposal,
        "elapsed_time" => ControllerBound::ElapsedTime,
        "cost" => ControllerBound::Cost,
        "input_units" => ControllerBound::InputUnits,
        "output_units" => ControllerBound::OutputUnits,
        "artifact_bytes" => ControllerBound::ArtifactBytes,
        "process_invocations" => ControllerBound::ProcessInvocations,
        "model_invocations" => ControllerBound::ModelInvocations,
        "failures" => ControllerBound::Failures,
        "rejections" => ControllerBound::Rejections,
        "repeat_depth" => ControllerBound::RepeatDepth,
        "child_depth" => ControllerBound::ChildDepth,
        "human_checkpoint" => ControllerBound::HumanCheckpoint,
        _ => return None,
    })
}

fn stable_controller_identity(
    domain: &str,
    policy_digest: &str,
    run: &str,
    execution: &str,
    number: impl Into<u64>,
) -> String {
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"milkdrift.controller-lifecycle.v1\0");
    controller_hash_field(&mut hasher, domain.as_bytes());
    controller_hash_field(&mut hasher, policy_digest.as_bytes());
    controller_hash_field(&mut hasher, run.as_bytes());
    controller_hash_field(&mut hasher, execution.as_bytes());
    hasher.update(&number.into().to_be_bytes());
    format!("controller-{}", &hasher.finalize().to_hex().as_str()[..40])
}

fn stable_controller_assessment_identity(
    policy_digest: &str,
    run: &str,
    execution: &str,
    through_sequence: u64,
    boundary: ControllerAssessmentBoundary,
    next_cycle: Option<u32>,
) -> String {
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"milkdrift.controller-assessment.v1\0");
    controller_hash_field(&mut hasher, policy_digest.as_bytes());
    controller_hash_field(&mut hasher, run.as_bytes());
    controller_hash_field(&mut hasher, execution.as_bytes());
    hasher.update(&through_sequence.to_be_bytes());
    controller_hash_field(&mut hasher, assessment_boundary_name(boundary).as_bytes());
    match next_cycle {
        Some(cycle) => {
            hasher.update(&[1]);
            hasher.update(&cycle.to_be_bytes());
        }
        None => {
            hasher.update(&[0]);
        }
    }
    format!("controller-{}", &hasher.finalize().to_hex().as_str()[..40])
}

fn controller_hash_field(hasher: &mut blake3::Hasher, bytes: &[u8]) {
    hasher.update(&u64::try_from(bytes.len()).unwrap_or(u64::MAX).to_be_bytes());
    hasher.update(bytes);
}

const fn assessment_boundary_name(boundary: ControllerAssessmentBoundary) -> &'static str {
    match boundary {
        ControllerAssessmentBoundary::Activation => "activation",
        ControllerAssessmentBoundary::CycleEntry => "cycle_entry",
        ControllerAssessmentBoundary::CheckpointContinuation => "checkpoint_continuation",
        ControllerAssessmentBoundary::ProposalAcceptance => "proposal_acceptance",
        ControllerAssessmentBoundary::ProposalApproval => "proposal_approval",
        ControllerAssessmentBoundary::ProposalApplication => "proposal_application",
    }
}
