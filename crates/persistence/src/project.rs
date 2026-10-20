//! Bounded project registrations and exact recovery of filesystem creation intent.
use crate::{DaemonDisplayName, NativePathRoot, PersistenceError};
use milkdrift_authority::ActorRef;
use milkdrift_blueprint::{ProjectSettings, WorkflowId};
use serde::{Deserialize, Serialize};

/// Registrations are bounded independently of retained workflow history.
pub const MAX_PROJECTS: u64 = 1_024;
/// Receipt capacity never evicts unresolved creation or earlier exact acceptances.
pub const MAX_PROJECT_OPERATIONS: u64 = 8_192;
/// Maximum bytes of one canonical project command and its result.
pub const MAX_PROJECT_OPERATION_BYTES: usize = 32_768;

/// A user-owned directory registered on one daemon. Removing this record never removes files.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectRecord {
    /// Daemon-issued identity, unrelated to the display name.
    pub id: String,
    /// Shared normalized label.
    pub name: DaemonDisplayName,
    /// Optimistic settings and name guard, starting at one.
    pub generation: u64,
    /// Captured native directory identity, rechecked before new execution.
    pub folder: NativePathRoot,
    /// Defaults for subsequent submissions only.
    pub settings: ProjectSettings,
}

/// Filesystem creation is outside the database transaction; this state never claims otherwise.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case", deny_unknown_fields)]
pub enum ProjectOperationState {
    /// Intent committed before exclusive creation. A restart must not retry the effect.
    Creating {
        /// Identity and permissions of the permitted existing parent.
        parent: NativePathRoot,
        /// Exactly one new child component.
        folder_name: String,
        /// Proposed registration awaiting captured child identity.
        project: ProjectRecord,
    },
    /// Accepted original result, retained unchanged after later project edits/removal.
    Accepted {
        /// Registration accepted by this command, when applicable.
        project: Option<ProjectRecord>,
        /// Organizational association accepted by this command, when applicable.
        workflow: Option<WorkflowId>,
    },
    /// Explicitly abandoned creation; any directory remains untouched.
    Abandoned,
}

/// One actor-local exact command, including unresolved creation intent.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectOperation {
    /// Stable daemon-issued recovery identity.
    pub id: String,
    /// Authenticated receipt owner.
    pub actor: ActorRef,
    /// Exact actor-local request key.
    pub key: String,
    /// Original canonical public request, before resolving mutable defaults.
    pub canonical_request: Vec<u8>,
    /// Durable state and original result.
    pub state: ProjectOperationState,
}

/// An authorized registration mutation, committed with its exact response.
#[derive(Clone, Debug)]
pub enum ProjectMutation {
    /// Register an already inspected directory.
    Attach(ProjectRecord),
    /// Replace the label or settings under the observed guard.
    Update {
        /// Complete next label/settings, preserving pinned folder.
        project: ProjectRecord,
        /// Previously observed generation.
        expected_generation: u64,
    },
    /// Remove only an unused registration, preserving its directory.
    Remove {
        /// Registration identity.
        id: String,
        /// Previously observed generation.
        expected_generation: u64,
    },
    /// Associate an existing workflow without rewriting accepted history or receipts.
    Associate {
        /// Registration identity.
        id: String,
        /// Previously observed generation.
        expected_generation: u64,
        /// Existing workflow to organize.
        workflow: WorkflowId,
    },
    /// Reserve exact intent before any directory effect.
    BeginCreate,
    /// Explicitly settle a prior uncertain creation and retain this reconciliation's own receipt.
    Reconcile {
        /// Original actor-local command identity.
        original_key: String,
        /// Inspected original folder, or abandonment without deleting anything.
        folder: Option<NativePathRoot>,
    },
}

/// Durable project ownership; callers supply authenticated authorization and native observations.
pub trait ProjectStore: Send + Sync {
    /// Read one registration. Authorization precedes public disclosure.
    /// # Errors
    /// Returns storage and integrity failures.
    fn project(&self, id: &str) -> Result<Option<ProjectRecord>, PersistenceError>;
    /// Read at most 200 identity-ordered registrations.
    /// # Errors
    /// Refuses invalid bounds and returns storage failures.
    fn projects(
        &self,
        after: Option<&str>,
        limit: u16,
    ) -> Result<Vec<ProjectRecord>, PersistenceError>;
    /// Read the current organizational association, independently of frozen task content.
    /// # Errors
    /// Returns storage and integrity failures.
    fn project_for_workflow(
        &self,
        workflow: &WorkflowId,
    ) -> Result<Option<ProjectRecord>, PersistenceError>;
    /// Recover only the exact authenticated actor's request.
    /// # Errors
    /// Refuses malformed keys and returns storage failures.
    fn project_operation(
        &self,
        actor: &ActorRef,
        key: &str,
    ) -> Result<Option<ProjectOperation>, PersistenceError>;
    /// Inspect an actor's bounded unfinished operation set.
    /// # Errors
    /// Returns storage and integrity failures.
    fn project_operations(
        &self,
        actor: &ActorRef,
    ) -> Result<Vec<ProjectOperation>, PersistenceError>;
    /// Accept a registration change or reserve creation intent with its exact command.
    /// # Errors
    /// Refuses changed replay, stale generations, references, duplicates and capacity exhaustion.
    fn mutate_project(
        &self,
        operation: ProjectOperation,
        mutation: ProjectMutation,
    ) -> Result<ProjectOperation, PersistenceError>;
    /// Settle one previously reserved intent using an explicitly verified directory or abandonment.
    /// # Errors
    /// Refuses changed operation ownership, completed intent and storage failures.
    fn settle_project_creation(
        &self,
        actor: &ActorRef,
        key: &str,
        folder: Option<NativePathRoot>,
    ) -> Result<ProjectOperation, PersistenceError>;
}
