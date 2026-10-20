//! A project's execution defaults are independent of its tasks' model selection.
use serde::{Deserialize, Serialize};

/// A configured tool location. Capability choices bind an exact advertised generation.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ProjectEnvironment {
    /// Native bounded file tools without an additional process tool.
    Host,
    /// A registered project-aware host process; this is trusted host execution.
    Process {
        /// Registered capability identity.
        capability: String,
        /// Registered bounded process operation.
        operation: String,
        /// Reviewed descriptor generation.
        generation: u64,
    },
    /// A registered managed worker with an explicit project attachment.
    Podman {
        /// Registered worker capability identity.
        capability: String,
        /// Registered bounded worker operation.
        operation: String,
        /// Reviewed descriptor generation.
        generation: u64,
    },
}

/// Defaults resolved into immutable task content when a new submission is accepted.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectSettings {
    /// Where tools run, separate from the selected model or agent.
    pub environment: ProjectEnvironment,
    /// Explicit consent to mutation-capable tools, subject to current authority.
    pub allow_changes: bool,
}
