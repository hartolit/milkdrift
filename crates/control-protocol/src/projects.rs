//! Daemon-owned project registration, bounded folder choices and exact creation recovery.
use serde::{Deserialize, Serialize};

/// Reviewed configured execution location, separate from model selection.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-codegen", derive(ts_rs::TS))]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ProjectEnvironmentDto {
    /// Bounded native file tools only.
    Host,
    /// Configured project-aware trusted host process.
    Process {
        /// Capability identity.
        capability: String,
        /// Supported bounded operation.
        operation: String,
        /// Reviewed generation.
        #[cfg_attr(feature = "web-codegen", ts(type = "number"))]
        generation: u64,
    },
    /// Configured managed worker with verified project attachment.
    Podman {
        /// Capability identity.
        capability: String,
        /// Supported bounded operation.
        operation: String,
        /// Reviewed generation.
        #[cfg_attr(feature = "web-codegen", ts(type = "number"))]
        generation: u64,
    },
}
/// Settings for future submissions; existing accepted tasks keep their selected content.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-codegen", derive(ts_rs::TS))]
#[serde(deny_unknown_fields)]
pub struct ProjectSettingsDto {
    /// Tool execution location.
    pub environment: ProjectEnvironmentDto,
    /// Explicit permission request for project changes, still constrained by the actor's grant.
    pub allow_changes: bool,
}
/// One authorized project; no inaccessible names, paths or counts are projected.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-codegen", derive(ts_rs::TS))]
#[serde(deny_unknown_fields)]
pub struct ProjectDto {
    /// Daemon-issued identity.
    pub id: String,
    /// Shared readable name.
    pub name: String,
    /// Guard for prospective changes.
    #[cfg_attr(feature = "web-codegen", ts(type = "number"))]
    pub generation: u64,
    /// Ordinary folder on the daemon's machine.
    pub folder: String,
    /// Current defaults.
    pub settings: ProjectSettingsDto,
    /// Available or a truthful explanation of missing/replaced roots.
    pub availability: String,
    /// Whether the actor may change this registration.
    pub can_manage: bool,
}
/// Bounded authorized registrations.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-codegen", derive(ts_rs::TS))]
#[serde(deny_unknown_fields)]
pub struct ProjectPageDto {
    /// Authorized records in this page.
    pub items: Vec<ProjectDto>,
    /// Opaque authority-bound continuation.
    pub next_cursor: Option<String>,
}
/// One permitted existing directory on the daemon.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-codegen", derive(ts_rs::TS))]
#[serde(deny_unknown_fields)]
pub struct ProjectDirectoryDto {
    /// Absolute path accepted only after daemon authority and native identity checks.
    pub path: String,
    /// Readable final component.
    pub name: String,
    /// Current authorization to create one child here.
    pub can_create: bool,
}
/// Bounded non-recursive folder choices.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-codegen", derive(ts_rs::TS))]
#[serde(deny_unknown_fields)]
pub struct ProjectDirectoryPageDto {
    /// Inspected parent, or null for the actor's permitted roots.
    pub parent: Option<String>,
    /// Authorized existing child directories.
    pub items: Vec<ProjectDirectoryDto>,
    /// Opaque continuation, if the bounded directory window continues.
    pub next_cursor: Option<String>,
}
/// An existing capability's project compatibility, without arguments or secrets.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-codegen", derive(ts_rs::TS))]
#[serde(deny_unknown_fields)]
pub struct ProjectEnvironmentChoiceDto {
    /// Exact configured choice.
    pub environment: ProjectEnvironmentDto,
    /// Readable configured label.
    pub label: String,
    /// Whether this configured generation currently accepts new work; physical project
    /// attachment prerequisites are checked again at execution.
    pub available: bool,
    /// Setup, qualification or refusal explanation, including prerequisites of available choices.
    pub reason: Option<String>,
}
/// Current compatible configured choices; this does not install capabilities.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-codegen", derive(ts_rs::TS))]
#[serde(deny_unknown_fields)]
pub struct ProjectEnvironmentPageDto {
    /// Authorized configured choices.
    pub items: Vec<ProjectEnvironmentChoiceDto>,
}
/// Explicit prospective project operation.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-codegen", derive(ts_rs::TS))]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ProjectCommandDto {
    /// Register an existing permitted folder without changing its bytes.
    Attach {
        /// Readable name.
        name: String,
        /// Existing daemon folder.
        folder: String,
        /// Reviewed execution defaults.
        settings: ProjectSettingsDto,
    },
    /// Exclusively create one child beneath a pinned existing permitted parent.
    Create {
        /// Readable name.
        name: String,
        /// Existing daemon parent folder.
        parent: String,
        /// Exactly one new component.
        folder_name: String,
        /// Reviewed execution defaults.
        settings: ProjectSettingsDto,
    },
    /// Rename the registration without renaming its directory.
    Rename {
        /// Selected registration.
        project_id: String,
        /// Reviewed project generation.
        #[cfg_attr(feature = "web-codegen", ts(type = "number"))]
        expected_generation: u64,
        /// New readable label.
        name: String,
    },
    /// Change defaults for later submissions only.
    Settings {
        /// Selected registration.
        project_id: String,
        /// Reviewed project generation.
        #[cfg_attr(feature = "web-codegen", ts(type = "number"))]
        expected_generation: u64,
        /// Complete prospective settings.
        settings: ProjectSettingsDto,
    },
    /// Unregister only an unused project, preserving all files.
    Remove {
        /// Selected registration.
        project_id: String,
        /// Reviewed project generation.
        #[cfg_attr(feature = "web-codegen", ts(type = "number"))]
        expected_generation: u64,
    },
    /// Organize an earlier workflow without changing its accepted history.
    Associate {
        /// Selected project.
        project_id: String,
        /// Reviewed project generation.
        #[cfg_attr(feature = "web-codegen", ts(type = "number"))]
        expected_generation: u64,
        /// Existing workflow.
        workflow_id: String,
    },
    /// Explicitly inspect and attach the exact intended directory after uncertain creation.
    Reconcile {
        /// Original actor-local creation request identity.
        operation_id: String,
    },
    /// Abandon unresolved registration while preserving any directory.
    Abandon {
        /// Original actor-local creation request identity.
        operation_id: String,
    },
}
/// Exact actor-scoped project command.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-codegen", derive(ts_rs::TS))]
#[serde(deny_unknown_fields)]
pub struct ProjectCommandRequest {
    /// Changed reuse conflicts; replay preserves the original response.
    pub request_id: String,
    /// Deliberate operation.
    pub command: ProjectCommandDto,
}
/// Exact accepted result or durable unresolved creation, independently of later changes.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-codegen", derive(ts_rs::TS))]
#[serde(deny_unknown_fields)]
pub struct ProjectReceiptDto {
    /// Original intended folder for an unresolved operation, disclosed only under current scope.
    pub operation_folder: Option<String>,
    /// Proposed project label for an unresolved operation.
    pub operation_name: Option<String>,
    /// Original actor-local command key.
    pub request_id: String,
    /// Original registration result, if one was accepted.
    pub project: Option<ProjectDto>,
    /// Original key used to reconcile an unresolved creation.
    pub operation_id: Option<String>,
    /// Accepted, unresolved or abandoned.
    pub status: String,
    /// Accepted organizational association, if any.
    pub workflow_id: Option<String>,
}
