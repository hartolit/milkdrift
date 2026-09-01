use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use milkdrift_capability::OperationId;

use crate::{Condition, NodeId, PortId, RevisionId, WorkflowId};

use super::{ModelError, SchemaRef, WorkflowInterface};

const MAX_REPEAT_ITERATIONS: u32 = 10_000;

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
#[serde(
    rename_all = "snake_case",
    tag = "type",
    content = "quorum",
    deny_unknown_fields
)]
pub enum JoinPolicy {
    /// Wait for every owned branch.
    All,
    /// Continue after the first branch reaches any terminal outcome.
    Any,
    /// Continue after the first successful branch and cancel unfinished losers.
    FirstSuccess,
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
#[serde(
    rename_all = "snake_case",
    tag = "type",
    content = "operation",
    deny_unknown_fields
)]
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
