//! Safe, finite input and output choices supplied by an adapter's immutable configuration.

use std::collections::BTreeSet;

use crate::ContractError;

/// Media families the configured operation can interpret as task context.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TaskInputMedia {
    /// The configured operation consumes raw bytes without a media restriction.
    Any,
    /// Provider context supports text, JSON and images; arbitrary binary data has no mapping.
    TextJsonOrImage,
}

impl TaskInputMedia {
    /// Checks a known media type. Actual content validation remains with materialization.
    #[must_use]
    pub fn accepts(self, media: &str) -> bool {
        match self {
            Self::Any => true,
            Self::TextJsonOrImage => {
                media.starts_with("text/")
                    || media == "application/json"
                    || media.ends_with("+json")
                    || media.starts_with("image/")
            }
        }
    }
}

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
    /// Shared interpretation rule for input media, including additional context slots.
    pub input_media: TaskInputMedia,
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
    /// Validates bounded adapter metadata before registration.
    /// # Errors
    /// Refuses malformed ports, duplicate names and unsupported default output identities.
    pub fn validate(&self) -> Result<(), ContractError> {
        let label_valid = |value: &str| {
            !value.is_empty() && value.len() <= 512 && !value.chars().any(char::is_control)
        };
        if !label_valid(&self.label) || self.inputs.len() > 512 || self.outputs.len() > 256 {
            return Err(invalid("task authoring metadata exceeds its bounds"));
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
                    return Err(invalid(
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
            return Err(invalid("task authoring default must name a real output"));
        }
        Ok(())
    }
}

fn invalid(reason: &str) -> ContractError {
    ContractError::Bounds {
        location: "task_authoring".into(),
        reason: reason.into(),
    }
}
