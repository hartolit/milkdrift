use std::collections::{BTreeMap, BTreeSet};

use milkdrift_blueprint::{
    AuthorRef, BlueprintMetadata, BlueprintRevision, Condition, CostCurrencyCode, Edge, EdgeId,
    EdgeKind, Mutation, MutationBatch, Node, NodeId, NodeKind, PinnedSubworkflow, PortId,
    RepeatBudget, RepeatConfig, RepeatTermination, TerminalOutcome, WorkflowId,
};
use milkdrift_capability::{BoundedJson, ExtensionKey};
use milkdrift_contracts::{JsonLimits, canonical_json_bytes};
use milkdrift_persistence::ControllerAccountBlock;
use milkdrift_runtime::CONTROLLER_POLICY_EXTENSION_KEY;
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
    /// Controller-account integrity rather than a configured resource ceiling stopped progress.
    AccountIntegrity,
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
        if let Some(block) = progress.account_block.as_ref() {
            return ControllerStop::BoundReached {
                bound: account_block_bound(block),
            };
        }
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

    pub(super) fn bound_fact(
        &self,
        progress: &ControllerProgress,
        bound: ControllerBound,
    ) -> (Option<u64>, u64, bool) {
        if let Some(block) = progress.account_block.as_ref() {
            return match block {
                ControllerAccountBlock::UnknownUsage { .. } => {
                    let (_, limit, _) = self.bound_fact_without_account_block(progress, bound);
                    (None, limit, true)
                }
                ControllerAccountBlock::ContractViolation {
                    observed, reserved, ..
                } => (Some(*observed), *reserved, false),
                ControllerAccountBlock::Integrity { .. } => (None, 1, false),
            };
        }
        self.bound_fact_without_account_block(progress, bound)
    }

    fn bound_fact_without_account_block(
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
            ControllerBound::AccountIntegrity => (None, 1, false),
        }
    }
}

fn account_block_bound(block: &ControllerAccountBlock) -> ControllerBound {
    match block {
        ControllerAccountBlock::UnknownUsage { dimension, .. }
        | ControllerAccountBlock::ContractViolation { dimension, .. } => match dimension.as_str() {
            "input_units" => ControllerBound::InputUnits,
            "output_units" => ControllerBound::OutputUnits,
            "artifact_bytes" => ControllerBound::ArtifactBytes,
            "monetary_cost" => ControllerBound::Cost,
            // Stored account validation rejects other dimensions. Retain a fail-closed fallback
            // here so an in-memory implementation cannot accidentally permit continuation.
            _ => ControllerBound::AccountIntegrity,
        },
        ControllerAccountBlock::Integrity { .. } => ControllerBound::AccountIntegrity,
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
    /// Exact durable controller-account block, including its reservation and evidence.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account_block: Option<ControllerAccountBlock>,
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
/// [`crate::ControllerLifecycleOwner`] owns every cumulative ceiling and must assess a
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
