//! Safe, finite input and output choices supplied by an adapter's immutable configuration.

use std::collections::{BTreeMap, BTreeSet};

use milkdrift_capability::OperationId;

use crate::AdapterError;

/// One configured input or published output. This contains no execution configuration.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TaskAuthoringPort {
    /// Exact input or publication key owned by the adapter.
    pub name: String,
    /// Human-readable description of that key.
    pub label: String,
    /// Whether the adapter requires this input or always publishes this output on success.
    pub required: bool,
    /// Whether this input can consume immutable artifact bytes rather than only inline text.
    /// Output ports always set this to true.
    pub accepts_artifacts: bool,
    /// Exact media type when the adapter defines one.
    pub media_type: Option<String>,
    /// Maximum bytes the adapter accepts or publishes for this port.
    pub max_bytes: u64,
}

/// The choices a task editor can offer for one registered operation generation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TaskAuthoringContract {
    /// Configured model or process name, excluding endpoint credentials and execution details.
    pub label: String,
    /// Configured input slots; additional context is separate from these exact slots.
    pub inputs: Vec<TaskAuthoringPort>,
    /// Whether independently named extra context inputs are accepted.
    pub allows_additional_inputs: bool,
    /// Actual output publication keys, including explicitly conditional outputs.
    pub outputs: Vec<TaskAuthoringPort>,
    /// A preferred real output, only when the adapter defines one.
    pub default_output: Option<String>,
}

impl TaskAuthoringContract {
    pub(crate) fn validate(&self) -> Result<(), AdapterError> {
        let label_valid = |value: &str| {
            !value.is_empty() && value.len() <= 512 && !value.chars().any(char::is_control)
        };
        if !label_valid(&self.label) || self.inputs.len() > 512 || self.outputs.len() > 256 {
            return Err(AdapterError::rejected(
                "task authoring metadata exceeds its bounds",
            ));
        }
        for ports in [&self.inputs, &self.outputs] {
            let mut names = BTreeSet::new();
            for port in ports {
                if port.name.is_empty()
                    || port.name.len() > 128
                    || !port
                        .name
                        .bytes()
                        .all(|byte| byte.is_ascii_alphanumeric() || b"_.-".contains(&byte))
                    || !names.insert(&port.name)
                    || !label_valid(&port.label)
                    || port.max_bytes == 0
                    || port
                        .media_type
                        .as_ref()
                        .is_some_and(|media| !label_valid(media))
                {
                    return Err(AdapterError::rejected(
                        "task authoring port is invalid or exceeds its bounds",
                    ));
                }
            }
        }
        if self
            .default_output
            .as_ref()
            .is_some_and(|name| !self.outputs.iter().any(|port| &port.name == name))
        {
            return Err(AdapterError::rejected(
                "task authoring default must name a real output",
            ));
        }
        Ok(())
    }
}

pub(crate) fn validate_contracts(
    contracts: &BTreeMap<OperationId, TaskAuthoringContract>,
    descriptor: &milkdrift_capability::CapabilityDescriptor,
) -> Result<(), AdapterError> {
    if contracts.len() > 256
        || contracts
            .keys()
            .any(|operation| descriptor.operation(operation).is_none())
    {
        return Err(AdapterError::rejected(
            "task authoring metadata names an unavailable operation",
        ));
    }
    for contract in contracts.values() {
        contract.validate()?;
    }
    Ok(())
}
