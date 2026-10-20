//! Define immutable ordinary task content and validate future dependency changes.
//!
//! Task inputs are explicit bindings. Missing supplied values keep accepted work queued;
//! invalid references and cycles refuse the entire edit before persistence. Runtime owns
//! start freezing, execution and results; clients share these contracts without scheduling.

mod identity;
mod project;
mod task;
mod task_project;

pub use identity::{IdentityError, NodeId, RevisionId, TaskId, WorkflowId};
pub use project::{ProjectEnvironment, ProjectSettings};
pub use task::{
    DEFAULT_WORKFLOW_TASKS, MAX_TASK_BATCH_ACTIONS, MAX_TASK_DEFINITION_BYTES,
    MAX_TASK_OUTPUT_BYTES, MAX_TASK_TEXT_BYTES, NewTask, TaskActionPolicy, TaskBatch,
    TaskBindingRedirect, TaskChange, TaskDefinition, TaskDefinitionError, TaskInputBinding,
    TaskInputValue, TaskOutputDeclaration, TaskPathAccess, TaskTool, WorkflowCreate,
    WorkflowLimits, validate_task_set,
};
pub use task_project::{TaskExecution, TaskProjectAccess, TaskProjectBinding, TaskProjectFolder};
