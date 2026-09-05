use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use milkdrift_capability::{CapabilityRequirement, OperationId};

use crate::{ContextSemanticRole, NodeId, PortId, TaskContextPolicy};

use super::{
    BranchConfig, DataPort, ForkConfig, JoinConfig, ModelError, PinnedSubworkflow, PortDirection,
    ReducerConfig, RepeatConfig,
};

const MAX_PORTS_PER_NODE: usize = 256;

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

/// Private-invariant configuration for an externally executed task.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TaskConfig {
    requirement: CapabilityRequirement,
    context_policy: TaskContextPolicy,
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    output_context_roles: BTreeSet<ContextSemanticRole>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TaskConfigWire {
    requirement: CapabilityRequirement,
    context_policy: TaskContextPolicy,
    #[serde(default)]
    output_context_roles: BTreeSet<ContextSemanticRole>,
}

milkdrift_contracts::deserialize_via!(TaskConfig, TaskConfigWire, |wire| Self::new(
    wire.requirement,
    wire.context_policy
)
.and_then(|config| config.with_output_context_roles(wire.output_context_roles)));

impl TaskConfig {
    /// Constructs a task from one capability requirement and immutable context policy.
    pub fn new(
        requirement: CapabilityRequirement,
        context_policy: TaskContextPolicy,
    ) -> Result<Self, ModelError> {
        requirement
            .validate()
            .map_err(|error| ModelError::new("task.requirement", error.to_string()))?;
        let _digest = context_policy.digest()?;
        Ok(Self {
            requirement,
            context_policy,
            output_context_roles: BTreeSet::new(),
        })
    }

    /// Declares the canonical semantic roles of this task's published outputs.
    pub fn with_output_context_roles(
        mut self,
        roles: BTreeSet<ContextSemanticRole>,
    ) -> Result<Self, ModelError> {
        if roles.len() > 256 {
            return Err(ModelError::new(
                "task.output_context_roles",
                "at most 256 output context roles are supported",
            ));
        }
        self.output_context_roles = roles;
        Ok(self)
    }

    /// Constructs a task using the deliberate v2 direct-input-only default policy.
    pub fn direct_inputs(requirement: CapabilityRequirement) -> Result<Self, ModelError> {
        Self::new(requirement, TaskContextPolicy::default())
    }

    /// Capability requirement resolved by the live host.
    #[must_use]
    pub const fn requirement(&self) -> &CapabilityRequirement {
        &self.requirement
    }

    /// Immutable causal context selection policy.
    #[must_use]
    pub const fn context_policy(&self) -> &TaskContextPolicy {
        &self.context_policy
    }

    /// Canonical semantic roles attached to every output occurrence of this task.
    #[must_use]
    pub const fn output_context_roles(&self) -> &BTreeSet<ContextSemanticRole> {
        &self.output_context_roles
    }
}

/// Complete semantic behavior of one definition-time node.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case", tag = "type", deny_unknown_fields)]
pub enum NodeKind {
    /// Invoke an operation through capability selection.
    Task {
        /// Requirement and causal context policy.
        config: TaskConfig,
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

impl NodeKind {
    /// Constructs a task with an explicit immutable context policy.
    pub fn task(
        requirement: CapabilityRequirement,
        context_policy: TaskContextPolicy,
    ) -> Result<Self, ModelError> {
        Ok(Self::Task {
            config: TaskConfig::new(requirement, context_policy)?,
        })
    }

    /// Constructs a task with the deliberate direct-declared-inputs-only policy.
    pub fn task_direct_inputs(requirement: CapabilityRequirement) -> Result<Self, ModelError> {
        Ok(Self::Task {
            config: TaskConfig::direct_inputs(requirement)?,
        })
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

milkdrift_contracts::deserialize_via!(Node, NodeWire, |wire| {
    let node = Self {
        id: wire.id,
        kind: wire.kind,
        control_inputs: wire.control_inputs,
        control_outputs: wire.control_outputs,
        data_inputs: wire.data_inputs,
        data_outputs: wire.data_outputs,
    };
    node.validate_local().map(|()| node)
});

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
            NodeKind::Task { config } => {
                config
                    .requirement()
                    .validate()
                    .map_err(|error| ModelError::new("node.task.requirement", error.to_string()))?;
                let _digest = config.context_policy().digest()?;
                Ok(())
            }
            NodeKind::Wait { duration_ms: 0 } => Err(ModelError::new(
                "node.wait.duration_ms",
                "wait duration must be nonzero",
            )),
            NodeKind::Branch { config } => config.arms().values().try_for_each(|condition| {
                condition
                    .validate()
                    .map_err(|error| ModelError::new("node.branch", error.to_string()))
            }),
            NodeKind::Repeat { config } => config
                .condition()
                .validate()
                .map_err(|error| ModelError::new("node.repeat", error.to_string())),
            _ => Ok(()),
        }
    }
}
