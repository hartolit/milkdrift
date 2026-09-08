//! Routing, parallel ownership, and bounded child calls within an ordinary blueprint.
//!
//! Branch chooses a route; fork/join owns parallel lifetimes; reducer chooses how values
//! combine. Subworkflow and repeat pin the child definition so later edits cannot change
//! an already declared call. The graph validator checks how these configurations connect.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use milkdrift_capability::OperationId;

use crate::{Condition, NodeId, PortId, RevisionId, WorkflowId};

use super::{ModelError, SchemaRef, WorkflowInterface};

const MAX_REPEAT_ITERATIONS: u32 = 10_000;

/// Chooses one outgoing control port from conditions on declared input values.
///
/// Runtime evaluates arms in port-key order and takes the first true condition. If none
/// matches, it takes `fallback`; without one, the branch fails. Every referenced
/// non-literal condition source must also be declared as an exact node input binding.
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

milkdrift_contracts::deserialize_via!(BranchConfig, BranchConfigWire, |wire| Self::new(
    wire.arms,
    wire.fallback
));

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

/// Starts one isolated child scope per named branch control port.
///
/// Declare the same ports on the fork node. To synchronize their routes, use a join
/// that names this fork; branches may also end at terminals. Branches keep their local
/// workspace values separate, and consumers receive only explicitly exposed results.
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

milkdrift_contracts::deserialize_via!(ForkConfig, ForkConfigWire, |wire| Self::new(wire.branches));

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

/// Decides when work owned by one fork can continue beyond its join.
///
/// [`JoinPolicy`] determines which branch outcomes satisfy the wait. Use a separate
/// [`ReducerConfig`] to combine values; synchronization alone is not a data reduction.
/// Graph validation checks fork ownership, branch convergence, and quorum feasibility.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct JoinConfig {
    fork: NodeId,
    policy: JoinPolicy,
}

impl JoinConfig {
    /// Creates a join owned by one fork.
    ///
    /// The fork must exist in the completed graph. A quorum must be nonzero and no
    /// greater than that fork's branch count; those checks require revision validation.
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

/// Combines multiple explicitly connected values at a named data input.
///
/// `Collect` keeps the ordered values, `First` picks the first in deterministic branch
/// order, and `Capability` delegates composition to an external operation. Unlike an
/// ordinary input, the reducer input can have multiple incoming data edges. Declare
/// their common item schema and a minimum count that the graph can supply.
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

milkdrift_contracts::deserialize_via!(ReducerConfig, ReducerConfigWire, |wire| Self::new(
    wire.input_port,
    wire.item_schema,
    wire.minimum_items,
    wire.strategy,
));

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

/// Calls a reusable workflow at an exact revision with an expected interface.
///
/// Copy the interface from the target revision and declare matching ports on the calling
/// node. The reference does not load the child. Runtime resolves the stored target,
/// creates the child execution, and imports its declared results. Upgrading the
/// target is an explicit mutation rather than following a moving workflow name.
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

milkdrift_contracts::deserialize_via!(CostCurrencyCode, String, |value| Self::new(value));

/// Optional time and observed-cost limits in addition to the repeat's iteration ceiling.
///
/// `None` leaves that dimension unset. Nonzero cost and its currency must be supplied
/// together. These are repeat-level limits checked by runtime; they do not reserve a
/// provider budget or predict the cost of the next invocation.
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

milkdrift_contracts::deserialize_via!(RepeatBudget, RepeatBudgetWire, |wire| {
    let budget = Self {
        max_duration_ms: wire.max_duration_ms,
        max_cost_micros: wire.max_cost_micros,
        max_cost_currency: wire.max_cost_currency,
    };
    budget.validate().map(|()| budget)
});

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

/// Repeats an exact child workflow while its post-iteration condition remains true.
///
/// This keeps the graph acyclic: repetition creates runtime occurrences of a pinned
/// body. Choose a hard iteration ceiling and decide what reaching a limit means through
/// [`RepeatTermination`]. Optional [`RepeatBudget`] limits can stop repetition earlier.
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

milkdrift_contracts::deserialize_via!(RepeatConfig, RepeatConfigWire, |wire| Self::new(
    wire.body,
    wire.condition,
    wire.maximum_iterations,
    wire.budget,
    wire.termination,
));

impl RepeatConfig {
    /// Constructs a repeat with a hard iteration limit and optional tighter budgets.
    ///
    /// `maximum_iterations` must be in 1..=10,000. Zero durations/costs, an unpaired
    /// cost/currency, and an oversized condition are refused. The calling node's ports
    /// must match the pinned body's interface when the revision is validated.
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
