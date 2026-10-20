//! Project selection and retained execution provenance for task authoring.
use crate::{projects::ProjectSettingsDto, tasks::TaskToolDto};
use serde::{Deserialize, Serialize};

/// File access selected for one task independently of model inference.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-codegen", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum TaskProjectAccessDto {
    /// No project access.
    None,
    /// Read-only project access.
    Read,
    /// Writable project access.
    Write,
}

/// Pinned native folder provenance, retained unchanged by task text edits.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-codegen", derive(ts_rs::TS))]
#[serde(deny_unknown_fields)]
pub struct TaskProjectFolderDto {
    /// Selected native absolute path.
    pub requested: String,
    /// Canonical native absolute path.
    pub physical: String,
    /// Native identity, never a client assertion granting authority.
    pub identity: String,
}

/// Immutable effective project selection exposed by accepted task reads.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-codegen", derive(ts_rs::TS))]
#[serde(deny_unknown_fields)]
pub struct TaskProjectBindingDto {
    /// Effective bounded project directories claimed by this task.
    pub paths: Vec<crate::tasks::PathAccess>,
    /// Pinned native identities corresponding to those directories.
    pub scopes: Vec<TaskProjectFolderDto>,
    /// Owning project registration.
    pub project_id: String,
    /// Reviewed project generation at acceptance.
    #[cfg_attr(feature = "web-codegen", ts(type = "number"))]
    pub generation: u64,
    /// Pinned folder.
    pub folder: TaskProjectFolderDto,
    /// Effective environment and changes consent.
    pub settings: ProjectSettingsDto,
    /// Effective project access.
    pub access: TaskProjectAccessDto,
    /// Exact effective tool selection.
    pub tools: Vec<TaskToolDto>,
}

/// Omission retains earlier request meaning; ordinary new project work chooses explicitly.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-codegen", derive(ts_rs::TS))]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum TaskExecutionDto {
    /// Earlier accepted definitions retain their original paths and tools.
    #[default]
    Legacy,
    /// Read-only projection when current authority cannot disclose project execution.
    /// This marker cannot be submitted as an authoring choice.
    Restricted,
    /// Resolve the reviewed project defaults at acceptance.
    Project {
        /// Exact project generation shown by the daemon.
        #[cfg_attr(feature = "web-codegen", ts(type = "number"))]
        expected_generation: u64,
    },
    /// Complete deliberate settings, including empty tools and access.
    Explicit {
        /// Exact project generation shown by the daemon.
        #[cfg_attr(feature = "web-codegen", ts(type = "number"))]
        expected_generation: u64,
        /// Complete environment selection.
        settings: ProjectSettingsDto,
        /// Complete access choice.
        access: TaskProjectAccessDto,
        /// Complete tool selection.
        tools: Vec<TaskToolDto>,
    },
    /// Retain this accepted binding when editing text; new tasks cannot manufacture one.
    Bound {
        /// Accepted immutable facts returned by the daemon.
        binding: TaskProjectBindingDto,
    },
}

impl TaskExecutionDto {
    /// Keeps historical canonical request bytes unchanged when the field was absent.
    #[must_use]
    pub(crate) const fn is_legacy(&self) -> bool {
        matches!(self, Self::Legacy)
    }
}
