use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use thiserror::Error;

use milkdrift_capability::{
    BoundedJson, CapabilityRequirement, ExtensionKey, OperationId, SchemaId,
};

use crate::{
    BlueprintId, Condition, FieldId, NodeId, PathSelector, PortId, RevisionId, WorkflowId,
};

pub(crate) const MAX_NODES: usize = 1_024;
pub(crate) const MAX_EDGES: usize = 4_096;
const MAX_PORTS_PER_NODE: usize = 256;
const MAX_INTERFACE_FIELDS: usize = 256;
const MAX_METADATA_ENTRIES: usize = 64;
const MAX_REPEAT_ITERATIONS: u32 = 10_000;

/// Error returned when constructing a locally invalid semantic component.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
#[error("invalid blueprint model at {location}: {reason}")]
pub struct ModelError {
    location: String,
    reason: String,
}

impl ModelError {
    pub(crate) fn new(location: impl Into<String>, reason: impl Into<String>) -> Self {
        Self {
            location: location.into(),
            reason: reason.into(),
        }
    }

    pub(crate) fn location(&self) -> &str {
        &self.location
    }
}

/// Exact schema identity/version used for compatibility checks.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SchemaRef {
    id: SchemaId,
    version: u32,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SchemaRefWire {
    id: SchemaId,
    version: u32,
}

impl<'de> Deserialize<'de> for SchemaRef {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let wire = SchemaRefWire::deserialize(deserializer)?;
        Self::new(wire.id, wire.version).map_err(serde::de::Error::custom)
    }
}

impl SchemaRef {
    /// Creates a reference to a nonzero schema version.
    pub fn new(id: SchemaId, version: u32) -> Result<Self, ModelError> {
        if version == 0 {
            return Err(ModelError::new("schema.version", "must be nonzero"));
        }
        Ok(Self { id, version })
    }

    /// Schema identity.
    #[must_use]
    pub const fn id(&self) -> &SchemaId {
        &self.id
    }

    /// Schema version.
    #[must_use]
    pub const fn version(&self) -> u32 {
        self.version
    }

    pub(crate) fn compatible_with(&self, other: &Self) -> bool {
        self == other
    }
}

/// One declared workflow interface field.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct InterfaceField {
    schema: SchemaRef,
    required: bool,
}

impl InterfaceField {
    /// Creates a required interface field.
    #[must_use]
    pub const fn required(schema: SchemaRef) -> Self {
        Self {
            schema,
            required: true,
        }
    }

    /// Creates an optional interface field.
    #[must_use]
    pub const fn optional(schema: SchemaRef) -> Self {
        Self {
            schema,
            required: false,
        }
    }

    /// Field schema.
    #[must_use]
    pub const fn schema(&self) -> &SchemaRef {
        &self.schema
    }

    /// Whether a caller must supply the field.
    #[must_use]
    pub const fn is_required(&self) -> bool {
        self.required
    }
}

/// Declared input and output contract of a workflow or subworkflow.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct WorkflowInterface {
    inputs: BTreeMap<FieldId, InterfaceField>,
    outputs: BTreeMap<FieldId, InterfaceField>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WorkflowInterfaceWire {
    inputs: BTreeMap<FieldId, InterfaceField>,
    outputs: BTreeMap<FieldId, InterfaceField>,
}

impl<'de> Deserialize<'de> for WorkflowInterface {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let wire = WorkflowInterfaceWire::deserialize(deserializer)?;
        Self::new(wire.inputs, wire.outputs).map_err(serde::de::Error::custom)
    }
}

impl WorkflowInterface {
    /// Constructs a bounded workflow interface.
    pub fn new(
        inputs: impl IntoIterator<Item = (FieldId, InterfaceField)>,
        outputs: impl IntoIterator<Item = (FieldId, InterfaceField)>,
    ) -> Result<Self, ModelError> {
        let inputs: BTreeMap<_, _> = inputs.into_iter().collect();
        let outputs: BTreeMap<_, _> = outputs.into_iter().collect();
        if inputs.len() > MAX_INTERFACE_FIELDS || outputs.len() > MAX_INTERFACE_FIELDS {
            return Err(ModelError::new(
                "interface",
                format!("at most {MAX_INTERFACE_FIELDS} inputs and outputs are allowed"),
            ));
        }
        Ok(Self { inputs, outputs })
    }

    /// Input fields.
    #[must_use]
    pub const fn inputs(&self) -> &BTreeMap<FieldId, InterfaceField> {
        &self.inputs
    }

    /// Output fields.
    #[must_use]
    pub const fn outputs(&self) -> &BTreeMap<FieldId, InterfaceField> {
        &self.outputs
    }
}

/// Bounded descriptive metadata excluded from execution state but included in semantic identity.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct BlueprintMetadata {
    name: String,
    description: String,
    labels: BTreeSet<String>,
    extensions: BTreeMap<ExtensionKey, BoundedJson>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct BlueprintMetadataWire {
    name: String,
    description: String,
    labels: BTreeSet<String>,
    extensions: BTreeMap<ExtensionKey, BoundedJson>,
}

impl<'de> Deserialize<'de> for BlueprintMetadata {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let wire = BlueprintMetadataWire::deserialize(deserializer)?;
        Self::new(wire.name, wire.description, wire.labels, wire.extensions)
            .map_err(serde::de::Error::custom)
    }
}

impl BlueprintMetadata {
    /// Creates bounded package metadata.
    pub fn new(
        name: impl Into<String>,
        description: impl Into<String>,
        labels: BTreeSet<String>,
        extensions: BTreeMap<ExtensionKey, BoundedJson>,
    ) -> Result<Self, ModelError> {
        let name = name.into();
        let description = description.into();
        if name.is_empty() || name.len() > 160 || description.len() > 4_096 {
            return Err(ModelError::new(
                "metadata",
                "name must contain 1..=160 bytes and description at most 4096 bytes",
            ));
        }
        if labels.len() > MAX_METADATA_ENTRIES
            || labels
                .iter()
                .any(|label| label.is_empty() || label.len() > 96)
            || extensions.len() > MAX_METADATA_ENTRIES
        {
            return Err(ModelError::new(
                "metadata",
                format!("labels/extensions are bounded to {MAX_METADATA_ENTRIES} entries"),
            ));
        }
        let extension_bytes = serde_json::to_vec(&extensions)
            .map_err(|error| ModelError::new("metadata.extensions", error.to_string()))?;
        if extension_bytes.len() > 65_536 {
            return Err(ModelError::new(
                "metadata.extensions",
                "serialized extensions exceed 65536 bytes",
            ));
        }
        Ok(Self {
            name,
            description,
            labels,
            extensions,
        })
    }

    /// Human-facing blueprint name.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Bounded human-facing description.
    #[must_use]
    pub fn description(&self) -> &str {
        &self.description
    }

    /// Deterministically ordered labels.
    #[must_use]
    pub const fn labels(&self) -> &BTreeSet<String> {
        &self.labels
    }

    /// Bounded namespaced semantic extensions.
    #[must_use]
    pub const fn extensions(&self) -> &BTreeMap<ExtensionKey, BoundedJson> {
        &self.extensions
    }

    pub(crate) fn default_for(workflow: &WorkflowId) -> Result<Self, ModelError> {
        Self::new(workflow.as_str(), "", BTreeSet::new(), BTreeMap::new())
    }
}

/// Source bound to a node data input or safe condition operand.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case", tag = "type", deny_unknown_fields)]
pub enum BindingSource {
    /// Small literal structured value.
    Literal {
        /// Bounded structured literal.
        value: BoundedJson,
    },
    /// Declared workflow input.
    WorkflowInput {
        /// Interface input identity.
        field: FieldId,
    },
    /// Output of a causally upstream node with an optional safe path.
    NodeOutput {
        /// Source node.
        node: NodeId,
        /// Source data port.
        port: PortId,
        /// Safe selector within the output value.
        path: PathSelector,
    },
    /// Durable branch-local workspace value contract.
    WorkspaceValue {
        /// Opaque durable value reference.
        reference: String,
        /// Exact declared value schema.
        contract: SchemaRef,
    },
    /// Durable artifact contract reference.
    Artifact {
        /// Opaque durable artifact reference.
        reference: String,
        /// Exact declared artifact value schema.
        contract: SchemaRef,
    },
    /// Parameter supplied to an explicitly pinned subworkflow.
    SubworkflowParameter {
        /// Pinned subworkflow parameter identity.
        field: FieldId,
    },
}

impl BindingSource {
    pub(crate) fn validate(&self) -> Result<(), ModelError> {
        match self {
            Self::WorkspaceValue { reference, .. } | Self::Artifact { reference, .. }
                if reference.is_empty() || reference.len() > 256 =>
            {
                Err(ModelError::new(
                    "binding.reference",
                    "must contain 1..=256 bytes",
                ))
            }
            _ => Ok(()),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
enum PortDirection {
    Input,
    Output,
}

/// Declared data port; constructors prevent input/output contradictions.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DataPort {
    schema: SchemaRef,
    required: bool,
    binding: Option<BindingSource>,
    direction: PortDirection,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DataPortWire {
    schema: SchemaRef,
    required: bool,
    binding: Option<BindingSource>,
    direction: PortDirection,
}

impl<'de> Deserialize<'de> for DataPort {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let wire = DataPortWire::deserialize(deserializer)?;
        match wire.direction {
            PortDirection::Input => Self::input(wire.schema, wire.required, wire.binding),
            PortDirection::Output if !wire.required && wire.binding.is_none() => {
                Ok(Self::output(wire.schema))
            }
            PortDirection::Output => Err(ModelError::new(
                "port.direction",
                "an output cannot be required or carry an input binding",
            )),
        }
        .map_err(serde::de::Error::custom)
    }
}

impl DataPort {
    /// Creates an input port with an optional explicit binding.
    pub fn input(
        schema: SchemaRef,
        required: bool,
        binding: Option<BindingSource>,
    ) -> Result<Self, ModelError> {
        if let Some(binding) = &binding {
            binding.validate()?;
        }
        Ok(Self {
            schema,
            required,
            binding,
            direction: PortDirection::Input,
        })
    }

    /// Creates an output port. Outputs never carry input bindings.
    #[must_use]
    pub const fn output(schema: SchemaRef) -> Self {
        Self {
            schema,
            required: false,
            binding: None,
            direction: PortDirection::Output,
        }
    }

    /// Port schema.
    #[must_use]
    pub const fn schema(&self) -> &SchemaRef {
        &self.schema
    }

    /// Input binding, when this is an input.
    #[must_use]
    pub const fn binding(&self) -> Option<&BindingSource> {
        self.binding.as_ref()
    }

    /// Whether this input must resolve before its owning node may execute.
    ///
    /// Output ports always return `false` because their constructor cannot carry
    /// an input requirement.
    #[must_use]
    pub const fn is_required(&self) -> bool {
        self.required
    }

    fn ensure_direction(&self, expected: PortDirection) -> Result<(), ModelError> {
        if self.direction != expected {
            return Err(ModelError::new(
                "port.direction",
                "input and output port constructors cannot be interchanged",
            ));
        }
        Ok(())
    }
}

/// Typed conditional branch configuration keyed by outgoing control port.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct BranchConfig {
    arms: BTreeMap<PortId, Condition>,
    fallback: Option<PortId>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct BranchConfigWire {
    arms: BTreeMap<PortId, Condition>,
    fallback: Option<PortId>,
}

impl<'de> Deserialize<'de> for BranchConfig {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let wire = BranchConfigWire::deserialize(deserializer)?;
        Self::new(wire.arms, wire.fallback).map_err(serde::de::Error::custom)
    }
}

impl BranchConfig {
    /// Constructs a branch with one or more condition arms and an optional fallback.
    pub fn new(
        arms: BTreeMap<PortId, Condition>,
        fallback: Option<PortId>,
    ) -> Result<Self, ModelError> {
        if arms.is_empty()
            || arms.len() > 64
            || fallback.as_ref().is_some_and(|id| arms.contains_key(id))
        {
            return Err(ModelError::new(
                "branch",
                "branch needs 1..=64 condition arms and a distinct optional fallback",
            ));
        }
        for condition in arms.values() {
            condition
                .validate()
                .map_err(|error| ModelError::new("branch.condition", error.to_string()))?;
        }
        Ok(Self { arms, fallback })
    }

    /// Condition arms keyed by their outgoing control ports.
    #[must_use]
    pub const fn arms(&self) -> &BTreeMap<PortId, Condition> {
        &self.arms
    }

    /// Fallback outgoing control port, when declared.
    #[must_use]
    pub const fn fallback(&self) -> Option<&PortId> {
        self.fallback.as_ref()
    }

    pub(crate) fn ports(&self) -> BTreeSet<PortId> {
        self.arms
            .keys()
            .cloned()
            .chain(self.fallback.iter().cloned())
            .collect()
    }
}

/// Structured fork configuration keyed by isolated branch control ports.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ForkConfig {
    branches: BTreeSet<PortId>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ForkConfigWire {
    branches: BTreeSet<PortId>,
}

impl<'de> Deserialize<'de> for ForkConfig {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let wire = ForkConfigWire::deserialize(deserializer)?;
        Self::new(wire.branches).map_err(serde::de::Error::custom)
    }
}

impl ForkConfig {
    /// Constructs a fork with at least two isolated branches.
    pub fn new(branches: BTreeSet<PortId>) -> Result<Self, ModelError> {
        if !(2..=64).contains(&branches.len()) {
            return Err(ModelError::new(
                "fork.branches",
                "a fork must contain 2..=64 branches",
            ));
        }
        Ok(Self { branches })
    }

    /// Declared branch control ports.
    #[must_use]
    pub const fn branches(&self) -> &BTreeSet<PortId> {
        &self.branches
    }
}

/// Synchronization policy for a structured join.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case", tag = "type", content = "quorum")]
pub enum JoinPolicy {
    /// Wait for every owned branch.
    All,
    /// Continue after the first branch reaches any terminal outcome.
    Any,
    /// Continue after the first successful branch and cancel unfinished losers.
    FirstSuccess,
    /// Legacy spelling of `first_success`, retained for schema-v1 compatibility.
    #[deprecated(note = "use FirstSuccess for new revisions")]
    AnySuccessful,
    /// Continue after at least the bounded number of successful branches.
    Quorum(u16),
}

/// Join configuration. Reduction is represented by a separate reducer node.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct JoinConfig {
    fork: NodeId,
    policy: JoinPolicy,
}

impl JoinConfig {
    /// Creates a join owned by one fork.
    #[must_use]
    pub const fn new(fork: NodeId, policy: JoinPolicy) -> Self {
        Self { fork, policy }
    }

    /// Fork whose child branches this join owns and synchronizes.
    #[must_use]
    pub const fn fork(&self) -> &NodeId {
        &self.fork
    }

    /// Declared synchronization policy.
    #[must_use]
    pub const fn policy(&self) -> JoinPolicy {
        self.policy
    }
}

/// Provider-neutral reducer strategy, separate from join synchronization.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case", tag = "type", content = "operation")]
pub enum ReducerStrategy {
    /// Collect branch values without provider execution.
    Collect,
    /// Select the first value in deterministic branch order.
    First,
    /// Invoke an explicitly namespaced compositor operation.
    Capability(OperationId),
}

/// Reducer input shape and strategy.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ReducerConfig {
    input_port: PortId,
    item_schema: SchemaRef,
    minimum_items: u16,
    strategy: ReducerStrategy,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ReducerConfigWire {
    input_port: PortId,
    item_schema: SchemaRef,
    minimum_items: u16,
    strategy: ReducerStrategy,
}

impl<'de> Deserialize<'de> for ReducerConfig {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let wire = ReducerConfigWire::deserialize(deserializer)?;
        Self::new(
            wire.input_port,
            wire.item_schema,
            wire.minimum_items,
            wire.strategy,
        )
        .map_err(serde::de::Error::custom)
    }
}

impl ReducerConfig {
    /// Constructs a reducer requiring at least one input item.
    pub fn new(
        input_port: PortId,
        item_schema: SchemaRef,
        minimum_items: u16,
        strategy: ReducerStrategy,
    ) -> Result<Self, ModelError> {
        if minimum_items == 0 {
            return Err(ModelError::new("reducer.minimum_items", "must be nonzero"));
        }
        Ok(Self {
            input_port,
            item_schema,
            minimum_items,
            strategy,
        })
    }

    /// Data input receiving explicit branch result references.
    #[must_use]
    pub const fn input_port(&self) -> &PortId {
        &self.input_port
    }

    /// Exact schema of each collected item.
    #[must_use]
    pub const fn item_schema(&self) -> &SchemaRef {
        &self.item_schema
    }

    /// Minimum number of items needed before reduction.
    #[must_use]
    pub const fn minimum_items(&self) -> u16 {
        self.minimum_items
    }

    /// Explicit deterministic or capability-backed reduction strategy.
    #[must_use]
    pub const fn strategy(&self) -> &ReducerStrategy {
        &self.strategy
    }
}

/// Exact immutable subworkflow target and expected interface.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PinnedSubworkflow {
    workflow: WorkflowId,
    revision: RevisionId,
    interface: WorkflowInterface,
}

impl PinnedSubworkflow {
    /// Constructs an exact pinned subworkflow reference.
    #[must_use]
    pub const fn new(
        workflow: WorkflowId,
        revision: RevisionId,
        interface: WorkflowInterface,
    ) -> Self {
        Self {
            workflow,
            revision,
            interface,
        }
    }

    /// Pinned revision.
    #[must_use]
    pub const fn revision(&self) -> &RevisionId {
        &self.revision
    }

    /// Workflow lineage owning the pinned revision.
    #[must_use]
    pub const fn workflow(&self) -> &WorkflowId {
        &self.workflow
    }

    /// Exact interface expected from the pinned revision.
    #[must_use]
    pub const fn interface(&self) -> &WorkflowInterface {
        &self.interface
    }
}

/// Validated currency ledger used by a repeat cost budget.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct CostCurrencyCode(String);

impl CostCurrencyCode {
    /// Validates an uppercase three-letter currency code.
    pub fn new(value: impl Into<String>) -> Result<Self, ModelError> {
        let value = value.into();
        if value.len() != 3 || !value.bytes().all(|byte| byte.is_ascii_uppercase()) {
            return Err(ModelError::new(
                "repeat.budget.cost_currency",
                "must contain exactly three uppercase ASCII letters",
            ));
        }
        Ok(Self(value))
    }

    /// Returns the validated currency code.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Serialize for CostCurrencyCode {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for CostCurrencyCode {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        Self::new(String::deserialize(deserializer)?).map_err(serde::de::Error::custom)
    }
}

/// Additional hard limits for a bounded repeat.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RepeatBudget {
    /// Maximum elapsed runtime enforced by the owning runtime.
    pub max_duration_ms: Option<u64>,
    /// Maximum observed cost in millionths enforced by the owning runtime.
    pub max_cost_micros: Option<u64>,
    /// Exact currency ledger governed by `max_cost_micros`.
    #[serde(default)]
    pub max_cost_currency: Option<CostCurrencyCode>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RepeatBudgetWire {
    max_duration_ms: Option<u64>,
    max_cost_micros: Option<u64>,
    #[serde(default)]
    max_cost_currency: Option<CostCurrencyCode>,
}

impl<'de> Deserialize<'de> for RepeatBudget {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let wire = RepeatBudgetWire::deserialize(deserializer)?;
        let budget = Self {
            max_duration_ms: wire.max_duration_ms,
            max_cost_micros: wire.max_cost_micros,
            max_cost_currency: wire.max_cost_currency,
        };
        budget.validate().map_err(serde::de::Error::custom)?;
        Ok(budget)
    }
}

impl RepeatBudget {
    fn validate(&self) -> Result<(), ModelError> {
        if self.max_duration_ms == Some(0)
            || self.max_cost_micros == Some(0)
            || self.max_cost_micros.is_some() != self.max_cost_currency.is_some()
        {
            return Err(ModelError::new(
                "repeat.budget",
                "duration must be nonzero and cost micros/currency must be supplied together",
            ));
        }
        Ok(())
    }
}

/// Behavior when a repeat reaches a hard bound.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RepeatTermination {
    /// Produce the latest completed iteration as success.
    SucceedWithLatest,
    /// End the node execution as failure.
    Fail,
    /// Require an external approval before continuing.
    AwaitApproval,
}

/// Explicit repetition of a pinned acyclic body.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RepeatConfig {
    body: PinnedSubworkflow,
    condition: Condition,
    maximum_iterations: u32,
    budget: RepeatBudget,
    termination: RepeatTermination,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RepeatConfigWire {
    body: PinnedSubworkflow,
    condition: Condition,
    maximum_iterations: u32,
    budget: RepeatBudget,
    termination: RepeatTermination,
}

impl<'de> Deserialize<'de> for RepeatConfig {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let wire = RepeatConfigWire::deserialize(deserializer)?;
        Self::new(
            wire.body,
            wire.condition,
            wire.maximum_iterations,
            wire.budget,
            wire.termination,
        )
        .map_err(serde::de::Error::custom)
    }
}

impl RepeatConfig {
    /// Constructs a repeat with a hard iteration limit and optional tighter budgets.
    pub fn new(
        body: PinnedSubworkflow,
        condition: Condition,
        maximum_iterations: u32,
        budget: RepeatBudget,
        termination: RepeatTermination,
    ) -> Result<Self, ModelError> {
        if maximum_iterations == 0 || maximum_iterations > MAX_REPEAT_ITERATIONS {
            return Err(ModelError::new(
                "repeat.maximum_iterations",
                format!("must be between 1 and {MAX_REPEAT_ITERATIONS}"),
            ));
        }
        budget.validate()?;
        condition
            .validate()
            .map_err(|error| ModelError::new("repeat.condition", error.to_string()))?;
        Ok(Self {
            body,
            condition,
            maximum_iterations,
            budget,
            termination,
        })
    }

    /// Exact acyclic body invoked for each iteration.
    #[must_use]
    pub const fn body(&self) -> &PinnedSubworkflow {
        &self.body
    }

    /// Condition recorded after each completed iteration.
    #[must_use]
    pub const fn condition(&self) -> &Condition {
        &self.condition
    }

    /// Hard maximum number of iterations.
    #[must_use]
    pub const fn maximum_iterations(&self) -> u32 {
        self.maximum_iterations
    }

    /// Additional runtime-enforced budget hooks.
    #[must_use]
    pub const fn budget(&self) -> &RepeatBudget {
        &self.budget
    }

    /// Behavior when a hard bound is reached.
    #[must_use]
    pub const fn termination(&self) -> RepeatTermination {
        self.termination
    }
}

/// Explicit terminal result of a workflow.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TerminalOutcome {
    /// Successful workflow result.
    Success,
    /// Failed workflow result.
    Failure,
    /// Cancelled workflow result.
    Cancelled,
}

/// Complete semantic behavior of one definition-time node.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "snake_case", tag = "type", deny_unknown_fields)]
pub enum NodeKind {
    /// Invoke an operation through capability selection.
    Task {
        /// Requirement matched later by a registry.
        requirement: CapabilityRequirement,
    },
    /// Select one typed control arm.
    Branch {
        /// Typed arms and optional fallback.
        config: BranchConfig,
    },
    /// Start isolated structured-concurrency branches.
    Fork {
        /// Named isolated branch ports.
        config: ForkConfig,
    },
    /// Synchronize branches owned by one fork without reducing values.
    Join {
        /// Owning fork and synchronization policy.
        config: JoinConfig,
    },
    /// Reduce or compose branch outputs separately from synchronization.
    Reducer {
        /// Input shape and reduction strategy.
        config: ReducerConfig,
    },
    /// Execute a pinned acyclic body repeatedly under hard bounds.
    Repeat {
        /// Pinned body, condition, bounds, and limit policy.
        config: RepeatConfig,
    },
    /// Durable timer definition.
    Wait {
        /// Nonzero durable timer duration.
        duration_ms: u64,
    },
    /// Durable external signal wait.
    SignalWait {
        /// Namespaced external signal contract.
        signal: OperationId,
    },
    /// Invoke an exact immutable subworkflow revision.
    Subworkflow {
        /// Exact target revision and expected interface.
        reference: PinnedSubworkflow,
    },
    /// Explicit workflow terminal.
    Terminal {
        /// Declared workflow result.
        outcome: TerminalOutcome,
    },
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case", tag = "type", deny_unknown_fields)]
enum NodeKindWire {
    Task {
        requirement: CapabilityRequirement,
        // Pass 1 encoded the operation twice. Accept that legacy v1 spelling only
        // when it agrees, then migrate to the requirement-owned representation.
        #[serde(default)]
        operation: Option<OperationId>,
    },
    Branch {
        config: BranchConfig,
    },
    Fork {
        config: ForkConfig,
    },
    Join {
        config: JoinConfig,
    },
    Reducer {
        config: ReducerConfig,
    },
    Repeat {
        config: RepeatConfig,
    },
    Wait {
        duration_ms: u64,
    },
    SignalWait {
        signal: OperationId,
    },
    Subworkflow {
        reference: PinnedSubworkflow,
    },
    Terminal {
        outcome: TerminalOutcome,
    },
}

impl<'de> Deserialize<'de> for NodeKind {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let wire = NodeKindWire::deserialize(deserializer)?;
        match wire {
            NodeKindWire::Task {
                requirement,
                operation,
            } => {
                if operation
                    .as_ref()
                    .is_some_and(|legacy| legacy != requirement.operation())
                {
                    return Err(serde::de::Error::custom(
                        "legacy task operation conflicts with its capability requirement",
                    ));
                }
                Ok(Self::Task { requirement })
            }
            NodeKindWire::Branch { config } => Ok(Self::Branch { config }),
            NodeKindWire::Fork { config } => Ok(Self::Fork { config }),
            NodeKindWire::Join { config } => Ok(Self::Join { config }),
            NodeKindWire::Reducer { config } => Ok(Self::Reducer { config }),
            NodeKindWire::Repeat { config } => Ok(Self::Repeat { config }),
            NodeKindWire::Wait { duration_ms } => Ok(Self::Wait { duration_ms }),
            NodeKindWire::SignalWait { signal } => Ok(Self::SignalWait { signal }),
            NodeKindWire::Subworkflow { reference } => Ok(Self::Subworkflow { reference }),
            NodeKindWire::Terminal { outcome } => Ok(Self::Terminal { outcome }),
        }
    }
}

/// Definition-time node with declared control/data ports and immutable configuration.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Node {
    id: NodeId,
    kind: NodeKind,
    control_inputs: BTreeSet<PortId>,
    control_outputs: BTreeSet<PortId>,
    data_inputs: BTreeMap<PortId, DataPort>,
    data_outputs: BTreeMap<PortId, DataPort>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NodeWire {
    id: NodeId,
    kind: NodeKind,
    control_inputs: BTreeSet<PortId>,
    control_outputs: BTreeSet<PortId>,
    data_inputs: BTreeMap<PortId, DataPort>,
    data_outputs: BTreeMap<PortId, DataPort>,
}

impl<'de> Deserialize<'de> for Node {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let wire = NodeWire::deserialize(deserializer)?;
        let node = Self {
            id: wire.id,
            kind: wire.kind,
            control_inputs: wire.control_inputs,
            control_outputs: wire.control_outputs,
            data_inputs: wire.data_inputs,
            data_outputs: wire.data_outputs,
        };
        node.validate_local().map_err(serde::de::Error::custom)?;
        Ok(node)
    }
}

impl Node {
    /// Constructs a node before declaring its ports.
    pub fn new(id: NodeId, kind: NodeKind) -> Result<Self, ModelError> {
        let node = Self {
            id,
            kind,
            control_inputs: BTreeSet::new(),
            control_outputs: BTreeSet::new(),
            data_inputs: BTreeMap::new(),
            data_outputs: BTreeMap::new(),
        };
        node.validate_kind()?;
        Ok(node)
    }

    /// Adds a declared control input, returning a new node value.
    pub fn with_control_input(mut self, port: PortId) -> Result<Self, ModelError> {
        if !self.control_inputs.insert(port) {
            return Err(ModelError::new("node.control_inputs", "duplicate port"));
        }
        self.validate_port_count()?;
        Ok(self)
    }

    /// Adds a declared control output, returning a new node value.
    pub fn with_control_output(mut self, port: PortId) -> Result<Self, ModelError> {
        if !self.control_outputs.insert(port) {
            return Err(ModelError::new("node.control_outputs", "duplicate port"));
        }
        self.validate_port_count()?;
        Ok(self)
    }

    /// Adds a declared data input.
    pub fn with_data_input(mut self, port: PortId, value: DataPort) -> Result<Self, ModelError> {
        value.ensure_direction(PortDirection::Input)?;
        if self.data_inputs.insert(port, value).is_some() {
            return Err(ModelError::new("node.data_inputs", "duplicate port"));
        }
        self.validate_port_count()?;
        Ok(self)
    }

    /// Adds a declared data output.
    pub fn with_data_output(mut self, port: PortId, value: DataPort) -> Result<Self, ModelError> {
        value.ensure_direction(PortDirection::Output)?;
        if self.data_outputs.insert(port, value).is_some() {
            return Err(ModelError::new("node.data_outputs", "duplicate port"));
        }
        self.validate_port_count()?;
        Ok(self)
    }

    /// Node identity.
    #[must_use]
    pub const fn id(&self) -> &NodeId {
        &self.id
    }

    /// Immutable node configuration.
    #[must_use]
    pub const fn kind(&self) -> &NodeKind {
        &self.kind
    }

    /// Declared data inputs.
    #[must_use]
    pub const fn data_inputs(&self) -> &BTreeMap<PortId, DataPort> {
        &self.data_inputs
    }

    /// Declared data outputs.
    #[must_use]
    pub const fn data_outputs(&self) -> &BTreeMap<PortId, DataPort> {
        &self.data_outputs
    }

    /// Declared control inputs.
    #[must_use]
    pub const fn control_inputs(&self) -> &BTreeSet<PortId> {
        &self.control_inputs
    }

    /// Declared control outputs.
    #[must_use]
    pub const fn control_outputs(&self) -> &BTreeSet<PortId> {
        &self.control_outputs
    }

    pub(crate) fn validate_local(&self) -> Result<(), ModelError> {
        self.validate_port_count()?;
        for port in self.data_inputs.values() {
            port.ensure_direction(PortDirection::Input)?;
            if let Some(binding) = port.binding() {
                binding.validate()?;
            }
        }
        for port in self.data_outputs.values() {
            port.ensure_direction(PortDirection::Output)?;
        }
        self.validate_kind()
    }

    pub(crate) fn replace_kind(&mut self, kind: NodeKind) -> Result<(), ModelError> {
        let previous = std::mem::replace(&mut self.kind, kind);
        if let Err(error) = self.validate_kind() {
            self.kind = previous;
            return Err(error);
        }
        Ok(())
    }

    fn validate_port_count(&self) -> Result<(), ModelError> {
        let total = self.control_inputs.len()
            + self.control_outputs.len()
            + self.data_inputs.len()
            + self.data_outputs.len();
        if total > MAX_PORTS_PER_NODE {
            return Err(ModelError::new(
                "node.ports",
                format!("at most {MAX_PORTS_PER_NODE} ports are allowed"),
            ));
        }
        Ok(())
    }

    fn validate_kind(&self) -> Result<(), ModelError> {
        match &self.kind {
            NodeKind::Task { requirement } => {
                requirement
                    .validate()
                    .map_err(|error| ModelError::new("node.task.requirement", error.to_string()))?;
                Ok(())
            }
            NodeKind::Wait { duration_ms: 0 } => Err(ModelError::new(
                "node.wait.duration_ms",
                "wait duration must be nonzero",
            )),
            NodeKind::Branch { config } => config.arms.values().try_for_each(|condition| {
                condition
                    .validate()
                    .map_err(|error| ModelError::new("node.branch", error.to_string()))
            }),
            NodeKind::Repeat { config } => config
                .condition
                .validate()
                .map_err(|error| ModelError::new("node.repeat", error.to_string())),
            _ => Ok(()),
        }
    }
}

/// Relationship between declared source and target ports.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EdgeKind {
    /// Scheduling/control dependency.
    Control,
    /// Typed value dependency.
    Data,
}

/// Explicit graph edge.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Edge {
    id: crate::EdgeId,
    kind: EdgeKind,
    source_node: NodeId,
    source_port: PortId,
    target_node: NodeId,
    target_port: PortId,
}

impl Edge {
    /// Constructs an explicit port-to-port edge.
    #[must_use]
    pub const fn new(
        id: crate::EdgeId,
        kind: EdgeKind,
        source_node: NodeId,
        source_port: PortId,
        target_node: NodeId,
        target_port: PortId,
    ) -> Self {
        Self {
            id,
            kind,
            source_node,
            source_port,
            target_node,
            target_port,
        }
    }

    /// Edge identity.
    #[must_use]
    pub const fn id(&self) -> &crate::EdgeId {
        &self.id
    }

    /// Whether this is a control or typed data dependency.
    #[must_use]
    pub const fn kind(&self) -> EdgeKind {
        self.kind
    }

    /// Source node identity.
    #[must_use]
    pub const fn source_node(&self) -> &NodeId {
        &self.source_node
    }

    /// Declared source port.
    #[must_use]
    pub const fn source_port(&self) -> &PortId {
        &self.source_port
    }

    /// Target node identity.
    #[must_use]
    pub const fn target_node(&self) -> &NodeId {
        &self.target_node
    }

    /// Declared target port.
    #[must_use]
    pub const fn target_port(&self) -> &PortId {
        &self.target_port
    }
}

/// Validated semantic content from which revision identity is derived.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct SemanticBlueprint {
    workflow: WorkflowId,
    blueprint: BlueprintId,
    metadata: BlueprintMetadata,
    interface: WorkflowInterface,
    nodes: BTreeMap<NodeId, Node>,
    edges: BTreeMap<crate::EdgeId, Edge>,
}

impl SemanticBlueprint {
    pub(crate) fn empty(workflow: WorkflowId) -> Result<Self, ModelError> {
        let blueprint = BlueprintId::new(workflow.as_str())
            .map_err(|error| ModelError::new("blueprint.identity", error.to_string()))?;
        let metadata = BlueprintMetadata::default_for(&workflow)?;
        Ok(Self {
            workflow,
            blueprint,
            metadata,
            interface: WorkflowInterface::new([], [])?,
            nodes: BTreeMap::new(),
            edges: BTreeMap::new(),
        })
    }

    pub(crate) fn from_parts(
        workflow: WorkflowId,
        blueprint: BlueprintId,
        metadata: BlueprintMetadata,
        interface: WorkflowInterface,
        nodes: BTreeMap<NodeId, Node>,
        edges: BTreeMap<crate::EdgeId, Edge>,
    ) -> Self {
        Self {
            workflow,
            blueprint,
            metadata,
            interface,
            nodes,
            edges,
        }
    }

    /// Workflow identity owning the revision lineage.
    #[must_use]
    pub const fn workflow(&self) -> &WorkflowId {
        &self.workflow
    }

    /// Stable reusable package identity.
    #[must_use]
    pub const fn blueprint(&self) -> &BlueprintId {
        &self.blueprint
    }

    /// Bounded package metadata.
    #[must_use]
    pub const fn metadata(&self) -> &BlueprintMetadata {
        &self.metadata
    }

    /// Declared workflow interface.
    #[must_use]
    pub const fn interface(&self) -> &WorkflowInterface {
        &self.interface
    }

    /// Nodes in deterministic identity order.
    #[must_use]
    pub const fn nodes(&self) -> &BTreeMap<NodeId, Node> {
        &self.nodes
    }

    /// Edges in deterministic identity order.
    #[must_use]
    pub const fn edges(&self) -> &BTreeMap<crate::EdgeId, Edge> {
        &self.edges
    }

    pub(crate) fn nodes_mut(&mut self) -> &mut BTreeMap<NodeId, Node> {
        &mut self.nodes
    }

    pub(crate) fn edges_mut(&mut self) -> &mut BTreeMap<crate::EdgeId, Edge> {
        &mut self.edges
    }

    pub(crate) fn set_interface(&mut self, interface: WorkflowInterface) {
        self.interface = interface;
    }

    pub(crate) fn set_metadata(&mut self, metadata: BlueprintMetadata) {
        self.metadata = metadata;
    }

    pub(crate) fn replace_node(&mut self, node: Node) -> Option<Node> {
        self.nodes.insert(node.id.clone(), node)
    }
}
