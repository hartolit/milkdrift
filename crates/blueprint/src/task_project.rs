//! A submitted project choice becomes one immutable execution binding at admission.

use serde::{Deserialize, Serialize};

use crate::{ProjectSettings, TaskPathAccess, TaskTool};

/// Project file permission selected independently of model inference.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskProjectAccess {
    /// No project files or project process tools are available.
    None,
    /// Native readers or an enforced read-only worker may read the project.
    Read,
    /// Project tools may change files; their durable claim excludes competing work.
    Write,
}

/// Native folder provenance captured by registration and checked again before entry.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaskProjectFolder {
    /// User-selected absolute spelling, including any accepted alias.
    pub requested: String,
    /// Canonical absolute path used by every project tool.
    pub physical: String,
    /// Native identity; a replacement at the same path is a different folder.
    pub identity: String,
}

/// Effective accepted selection, independent of subsequent project settings changes.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaskProjectBinding {
    /// Effective native scope; file-only work may select prepared subdirectories.
    pub paths: Vec<TaskPathAccess>,
    /// Native identities of each effective scope, pinned before acceptance.
    pub scopes: Vec<TaskProjectFolder>,
    /// Daemon-owned project registration.
    pub project_id: String,
    /// Settings generation reviewed when this definition was accepted.
    pub generation: u64,
    /// Exact registered native folder identity.
    pub folder: TaskProjectFolder,
    /// Environment choice and project policy used during admission.
    pub settings: ProjectSettings,
    /// Effective task access; explicit empty access remains empty.
    pub access: TaskProjectAccess,
    /// Exact admitted tool catalog, including a deliberately empty catalog.
    pub tools: Vec<TaskTool>,
}

/// An authoring choice is distinct from the binding retained after acceptance.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum TaskExecution {
    /// Pre-project task semantics, retained only for earlier accepted work.
    #[default]
    Legacy,
    /// Resolve the project defaults after checking the reviewed generation.
    Project {
        /// Project generation the submitter inspected.
        expected_generation: u64,
    },
    /// Complete override; omission never turns empty access or tools into inheritance.
    Explicit {
        /// Project generation the submitter inspected.
        expected_generation: u64,
        /// Complete environment selection.
        settings: ProjectSettings,
        /// Complete access choice.
        access: TaskProjectAccess,
        /// Complete tool selection.
        tools: Vec<TaskTool>,
    },
    /// Immutable accepted provenance; edits may retain the exact previous binding.
    Bound {
        /// Selection resolved by the daemon's shared admission path.
        binding: TaskProjectBinding,
    },
}

impl TaskExecution {
    /// Earlier records omit the field so their canonical content remains unchanged.
    #[must_use]
    pub const fn is_legacy(&self) -> bool {
        matches!(self, Self::Legacy)
    }

    /// Returns accepted project facts, never unresolved authoring intent.
    #[must_use]
    pub const fn binding(&self) -> Option<&TaskProjectBinding> {
        match self {
            Self::Bound { binding } => Some(binding),
            Self::Legacy | Self::Project { .. } | Self::Explicit { .. } => None,
        }
    }
}
