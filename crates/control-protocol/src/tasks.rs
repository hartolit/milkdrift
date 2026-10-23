//! Wire requests contain client aliases; acceptance returns daemon identities and exact receipts.
//!
//! These DTOs describe transport. Blueprint and control own validation and admission; generated
//! browser types are projections of these Rust declarations, never another semantic validator.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Text and immutable uploaded content accepted as named inputs.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[cfg_attr(feature = "web-codegen", derive(ts_rs::TS))]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum InputValue {
    /// Supplied UTF-8 text.
    Text {
        /// Exact UTF-8 content to freeze when the task starts.
        text: String,
    },
    /// An actor-owned upload captured by the daemon.
    Uploaded {
        /// Identity returned by this actor's successful upload.
        input_id: String,
    },
    /// An explicitly absent optional value.
    Absent,
}

/// A declared source; task references name batch aliases when creating new tasks.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[cfg_attr(feature = "web-codegen", derive(ts_rs::TS))]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum InputBinding {
    /// Inline input.
    Inline {
        /// Value captured with this definition.
        value: InputValue,
    },
    /// Named workflow input; missing required values keep the task queued.
    Supplied {
        /// Key in the workflow's supplied inputs.
        name: String,
        /// Whether absence is an accepted value.
        optional: bool,
    },
    /// Named immutable producer output.
    TaskOutput {
        /// Producer task identity or an alias in this batch.
        task: String,
        /// Named output declared by the producer.
        output: String,
        /// Whether settled absence is an accepted value.
        optional: bool,
    },
    /// Settled producer state and available output, including failure.
    SettledOutcome {
        /// Producer whose retained outcome becomes the input.
        task: String,
    },
}

/// An exact configured capability choice, obtained from this daemon's catalog.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[cfg_attr(feature = "web-codegen", derive(ts_rs::TS))]
#[serde(deny_unknown_fields)]
pub struct TaskCapability {
    /// Stable capability identity.
    pub id: String,
    /// Supported operation.
    pub operation: String,
}

/// One real input or publication slot from an adapter's configured contract.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[cfg_attr(feature = "web-codegen", derive(ts_rs::TS))]
#[serde(deny_unknown_fields)]
pub struct TaskInputSlotDto {
    /// Mechanical name assigned by the adapter, never user-invented.
    pub name: String,
    /// Readable label for connection choices.
    pub label: String,
    /// Whether this input must be connected.
    pub required: bool,
    /// False for a configured inline scalar substitution that cannot consume result artifacts.
    pub accepts_artifacts: bool,
    /// Exact media constraint, when one exists.
    pub media_type: Option<String>,
    /// Adapter-enforced maximum bytes.
    #[cfg_attr(feature = "web-codegen", ts(type = "number"))]
    pub max_bytes: u64,
}

/// A real adapter publication, which may be absent on successful settlement.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[cfg_attr(feature = "web-codegen", derive(ts_rs::TS))]
#[serde(deny_unknown_fields)]
pub struct TaskOutputPortDto {
    /// Exact name emitted by the adapter.
    pub name: String,
    /// Readable connection label.
    pub label: String,
    /// False means publication is conditional, not guaranteed on every success.
    pub required: bool,
    /// Media type emitted by the adapter.
    pub media_type: Option<String>,
    /// Maximum bytes permitted for the selected task output.
    #[cfg_attr(feature = "web-codegen", ts(type = "number"))]
    pub max_bytes: u64,
}

/// Safe authoring choices for one current, authorized registered generation.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[cfg_attr(feature = "web-codegen", derive(ts_rs::TS))]
#[serde(deny_unknown_fields)]
pub struct TaskCapabilityAuthoringDto {
    /// Exact executor selected by the task.
    pub capability: TaskCapability,
    /// Registered immutable configuration generation.
    #[cfg_attr(feature = "web-codegen", ts(type = "number"))]
    pub generation: u64,
    /// Actual configured model or process name.
    pub label: String,
    /// Explicit adapter category rather than a guess from its identity.
    pub category: String,
    /// Exact configured input choices.
    pub inputs: Vec<TaskInputSlotDto>,
    /// Whether generated additional context keys are supported.
    pub allows_additional_inputs: bool,
    /// Real publication choices.
    pub outputs: Vec<TaskOutputPortDto>,
    /// Preferred real publication, if the adapter defines one.
    pub default_output: Option<String>,
}

/// Filesystem scope already granted to this actor; selecting it does not grant permission.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[cfg_attr(feature = "web-codegen", derive(ts_rs::TS))]
#[serde(deny_unknown_fields)]
pub struct TaskProjectRootDto {
    /// Canonical root to combine with an optional relative subfolder.
    pub path: String,
    /// Last path component for display; path disambiguates equal labels.
    pub label: String,
    /// Current authority permits reads beneath this root.
    pub read: bool,
    /// Current authority permits writes beneath this root.
    pub write: bool,
}

/// A declared filesystem access coordinated by the host before entry.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[cfg_attr(feature = "web-codegen", derive(ts_rs::TS))]
#[serde(deny_unknown_fields)]
pub struct PathAccess {
    /// Absolute path within granted roots.
    pub path: String,
    /// Whether the task may write under this root.
    pub write: bool,
}

/// A named immutable output promised by a task.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[cfg_attr(feature = "web-codegen", derive(ts_rs::TS))]
#[serde(deny_unknown_fields)]
pub struct TaskOutputDeclarationDto {
    /// Whether successful settlement requires this output.
    #[serde(default = "required_by_default")]
    pub required: bool,
    /// Optional exact media type checked at settlement.
    pub media_type: Option<String>,
    /// Maximum retained bytes for this output.
    #[cfg_attr(feature = "web-codegen", ts(type = "number"))]
    pub max_bytes: u64,
}

/// One prospective consumer binding redirected by an adopted correction.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[cfg_attr(feature = "web-codegen", derive(ts_rs::TS))]
#[serde(deny_unknown_fields)]
pub struct TaskBindingRedirectDto {
    /// Unstarted consumer whose existing binding will change.
    pub consumer: String,
    /// Exact version observed by the caller.
    #[cfg_attr(feature = "web-codegen", ts(type = "number"))]
    pub expected_version: u64,
    /// Input currently bound to the original producer output.
    pub input: String,
}

/// Editable content until the owning runtime freezes the task's start.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[cfg_attr(feature = "web-codegen", derive(ts_rs::TS))]
#[serde(deny_unknown_fields)]
pub struct TaskDefinitionDto {
    /// Reviewed execution intent or unchanged accepted project provenance.
    #[serde(
        default,
        skip_serializing_if = "crate::task_execution::TaskExecutionDto::is_legacy"
    )]
    pub execution: crate::task_execution::TaskExecutionDto,
    /// Optional admission guard for the authoring contract observed while composing this task.
    /// It is not retained as a second capability selector in the accepted definition.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "web-codegen", ts(optional, type = "number"))]
    pub authoring_generation: Option<u64>,
    /// Opt in to structured action proposals; plain model text remains result data.
    #[serde(default)]
    pub actions: Option<TaskActionPolicyDto>,
    /// Exact tools permitted during this task's bounded model loop.
    #[serde(default)]
    pub tools: Vec<TaskToolDto>,
    /// Human-readable task title.
    pub title: String,
    /// Short explanation shown in list and timeline.
    pub description: String,
    /// Instructions delivered to the capability.
    pub instructions: String,
    /// Configured external work implementation.
    pub capability: TaskCapability,
    /// Named inputs with declared provenance.
    pub inputs: BTreeMap<String, InputBinding>,
    /// Named immutable outputs available to future consumers.
    #[serde(default)]
    pub outputs: BTreeMap<String, TaskOutputDeclarationDto>,
    /// Explicit producer IDs or batch aliases.
    pub dependencies: Vec<String>,
    /// Whether completion requires success or adopted correction.
    #[serde(default = "required_by_default")]
    pub required: bool,
    /// Prior task this future work addresses.
    pub corrective_of: Option<String>,
    /// Declared native filesystem roots.
    pub paths: Vec<PathAccess>,
}

/// Explicit grant to construct ordinary automatic feedback tasks after invalid model actions.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[cfg_attr(feature = "web-codegen", derive(ts_rs::TS))]
#[serde(deny_unknown_fields)]
pub struct TaskActionPolicyDto {
    /// Invalid/conflicted responses may request feedback within the same authority and allowance.
    pub automatic_repair: bool,
}

/// Native file tools use declared task paths; process tools select a configured capability.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[cfg_attr(feature = "web-codegen", derive(ts_rs::TS))]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum TaskToolDto {
    /// Read bounded UTF-8 content from a declared path.
    ReadFile,
    /// Write bounded UTF-8 content under a declared write root.
    WriteFile,
    /// Run the selected configured process tool.
    Process {
        /// Exact configured selection, subject to current authority.
        capability: TaskCapability,
    },
}

/// A client-local name paired with proposed task content.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[cfg_attr(feature = "web-codegen", derive(ts_rs::TS))]
#[serde(deny_unknown_fields)]
pub struct NewTaskDto {
    /// Alias valid only within this atomic request.
    pub alias: String,
    /// Proposed task.
    pub definition: TaskDefinitionDto,
}

fn required_by_default() -> bool {
    true
}

/// Effective cumulative workflow limits, displayed before submission.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[cfg_attr(feature = "web-codegen", derive(ts_rs::TS))]
#[serde(deny_unknown_fields)]
pub struct WorkflowLimitsDto {
    /// Total admitted tasks.
    pub tasks: u32,
    /// Simultaneously active tasks.
    pub concurrent_tasks: u32,
    /// Admitted model batches.
    pub model_batches: u32,
    /// Consecutive rejected responses.
    pub rejected_responses: u32,
    /// Failures on one corrective lineage.
    pub corrective_failures: u32,
    /// Cumulative active execution milliseconds.
    #[cfg_attr(feature = "web-codegen", ts(type = "number"))]
    pub active_execution_ms: u64,
    /// Tool invocations per task.
    pub tool_calls_per_task: u32,
}

/// Creates one finite or open-ended workflow through the daemon's ordinary command queue.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[cfg_attr(feature = "web-codegen", derive(ts_rs::TS))]
#[serde(deny_unknown_fields)]
pub struct CreateWorkflowRequest {
    /// Required project for ordinary new workflows; absent only in historical exact requests.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "web-codegen", ts(optional))]
    pub project_id: Option<String>,
    /// Exact request identity; changed reuse conflicts.
    pub request_id: String,
    /// Discoverable workflow title.
    pub title: String,
    /// Workflow brief.
    pub description: String,
    /// Initial tasks with client-local aliases.
    pub tasks: Vec<NewTaskDto>,
    /// Named values supplied at creation.
    pub inputs: BTreeMap<String, InputValue>,
    /// Explicit completion request for finite work.
    pub finish_when_settled: bool,
    /// Optional tighter policy; omitted uses daemon defaults.
    pub limits: Option<WorkflowLimitsDto>,
}

/// Prospective task editing and settlement controls.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[cfg_attr(feature = "web-codegen", derive(ts_rs::TS))]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum TaskChangeDto {
    /// Close a rejected response decision, optionally alongside reviewed future edits.
    DismissAdmission {
        /// Source task whose response was reviewed.
        task: String,
    },
    /// Prospectively change ceilings without clearing any spending or uncertainty.
    SetLimits {
        /// Complete next finite limits.
        limits: WorkflowLimitsDto,
    },
    /// Add future work.
    Add {
        /// Name used by other new tasks in this batch.
        alias: String,
        /// Proposed future task content.
        definition: TaskDefinitionDto,
    },
    /// Replace an unstarted definition.
    Revise {
        /// Unstarted task to change.
        task: String,
        /// Last observed definition version.
        #[cfg_attr(feature = "web-codegen", ts(type = "number"))]
        expected_version: u64,
        /// Replacement future definition.
        definition: TaskDefinitionDto,
    },
    /// Remove an unstarted task.
    Remove {
        /// Unstarted task to withdraw.
        task: String,
        /// Last observed definition version.
        #[cfg_attr(feature = "web-codegen", ts(type = "number"))]
        expected_version: u64,
    },
    /// Supply or replace future input.
    BindInput {
        /// Workflow input name.
        name: String,
        /// Value used only by future starts.
        value: InputValue,
    },
    /// Request cancellation of the exact task attempt.
    Stop {
        /// Task whose exact attempt should receive cancellation.
        task: String,
    },
    /// Adopt a successful correction while retaining original history.
    AdoptCorrection {
        /// Original failed or stopped obligation.
        task: String,
        /// Successful corrective task accepted for that obligation.
        correction: String,
        /// Explicit, version-guarded future bindings to redirect.
        #[serde(default)]
        redirects: Vec<TaskBindingRedirectDto>,
    },
    /// Continue future execution.
    Continue,
    /// Wait for a person.
    Wait {
        /// Explanation requiring a person's decision.
        question: String,
    },
    /// Request completion when obligations settle.
    Finish,
}

/// Atomic generation-guarded mutation.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[cfg_attr(feature = "web-codegen", derive(ts_rs::TS))]
#[serde(deny_unknown_fields)]
pub struct ChangeWorkflowRequest {
    /// Exact request identity or reserved action identity returned by the task.
    pub request_id: String,
    /// Last observed workflow edit generation.
    #[cfg_attr(feature = "web-codegen", ts(type = "number"))]
    pub expected_generation: u64,
    /// The whole batch succeeds or none of it does.
    pub changes: Vec<TaskChangeDto>,
}

/// Durable acceptance returned unchanged after exact replay.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[cfg_attr(feature = "web-codegen", derive(ts_rs::TS))]
#[serde(deny_unknown_fields)]
pub struct WorkflowReceipt {
    /// Client identity binding exact canonical request content.
    pub request_id: String,
    /// Daemon-assigned workflow identity.
    pub workflow_id: String,
    /// Accepted edit generation.
    #[cfg_attr(feature = "web-codegen", ts(type = "number"))]
    pub generation: u64,
    /// New task aliases mapped to durable identities.
    pub aliases: BTreeMap<String, String>,
}

/// Public lifecycle labels distinguish requests from evidence.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[cfg_attr(feature = "web-codegen", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum TaskStateDto {
    /// Waiting for inputs, dependencies or safe retry before external entry.
    Queued,
    /// Durable start accepted; external work may be running.
    Running,
    /// Completed with retained results.
    Completed,
    /// A failure was durably reported.
    Failed,
    /// Proven stopped before or after entry.
    Stopped,
    /// Entry may have happened and no terminal evidence exists.
    Uncertain,
}

/// Immutable task result and selected input identities.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[cfg_attr(feature = "web-codegen", derive(ts_rs::TS))]
#[serde(deny_unknown_fields)]
pub struct TaskResultDto {
    /// Actual named publications, independent of the producer's declaration map.
    #[serde(default)]
    pub outputs: BTreeMap<String, TaskPublishedOutputDto>,
    /// Declared text artifacts contain non-UTF-8 bytes; download individual artifacts.
    #[serde(default)]
    pub text_unavailable: bool,
    /// Displayable result content retained by the daemon.
    pub text: String,
    /// Whether text is a bounded preview of the complete retained result.
    pub text_truncated: bool,
    /// Content identity of the complete retained text, including bytes outside a preview.
    pub digest: String,
    /// Named immutable output artifacts.
    pub artifacts: Vec<String>,
    /// Selected named input byte identities.
    pub input_digests: BTreeMap<String, String>,
}

/// An actual retained named publication, not a display preview or invented default port.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[cfg_attr(feature = "web-codegen", derive(ts_rs::TS))]
#[serde(deny_unknown_fields)]
pub struct TaskPublishedOutputDto {
    /// Immutable artifact identity when current authority permits disclosure.
    pub artifact_id: Option<String>,
    /// Retained media type, when supplied by its owner.
    pub media_type: Option<String>,
    /// Exact retained byte count.
    #[cfg_attr(feature = "web-codegen", ts(type = "number"))]
    pub bytes: u64,
    /// Whether the selected actor can currently retrieve this artifact.
    pub available: bool,
}

/// Current supplied value and provenance, bounded separately from frozen start inputs.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[cfg_attr(feature = "web-codegen", derive(ts_rs::TS))]
#[serde(deny_unknown_fields)]
pub struct TaskSuppliedInputDto {
    /// Available, restricted by authority, or expired; an absent map key means missing.
    pub availability: String,
    /// Bounded captured value when readable.
    pub value: Option<FrozenInputDto>,
}

/// One task as observed through the selected daemon and actor.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[cfg_attr(feature = "web-codegen", derive(ts_rs::TS))]
#[serde(deny_unknown_fields)]
pub struct TaskReadDto {
    /// Captured inline values and upload provenance from this task's owning definition.
    #[serde(default)]
    pub inline_inputs: BTreeMap<String, TaskSuppliedInputDto>,
    /// Current shared values referenced by this task, separate from its frozen start.
    #[serde(default)]
    pub supplied_inputs: BTreeMap<String, TaskSuppliedInputDto>,
    /// Source response that admitted this task, available independently of workflow task paging.
    pub origin: Option<ModelAdmissionDto>,
    /// Stable workflow-local task identity.
    pub id: String,
    /// Last accepted definition generation.
    #[cfg_attr(feature = "web-codegen", ts(type = "number"))]
    pub version: u64,
    /// Actual accepted content.
    pub definition: TaskDefinitionDto,
    /// Durable runtime state.
    pub state: TaskStateDto,
    /// Missing input, failure, or uncertainty explanation.
    pub reason: Option<String>,
    /// Frozen start acceptance; retained while a pre-entry start waits again.
    #[cfg_attr(feature = "web-codegen", ts(type = "number | null"))]
    pub started_at: Option<u64>,
    /// Recorded settlement time; uncertainty has none.
    #[cfg_attr(feature = "web-codegen", ts(type = "number | null"))]
    pub ended_at: Option<u64>,
    /// A stop request does not prove the task stopped.
    pub stop_requested: bool,
    /// Server-issued bounded action identity for stopping this task.
    pub stop_request_id: String,
    /// Exact selected input bytes, capability generation and authority accepted at start.
    pub frozen: Option<FrozenTaskDto>,
    /// Available immutable result.
    pub result: Option<TaskResultDto>,
    /// Result exists but current artifact authority does not permit its projection.
    pub result_restricted: bool,
}

/// Actual selected bytes or immutable artifact identity, independent of a later file edit.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[cfg_attr(feature = "web-codegen", derive(ts_rs::TS))]
#[serde(deny_unknown_fields)]
pub struct FrozenInputDto {
    /// Current artifact authority prevents releasing this captured input.
    pub restricted: bool,
    /// Text, captured file, absent value or immutable artifact.
    pub kind: String,
    /// Captured filename, when present.
    pub name: Option<String>,
    /// Selected UTF-8 bytes, when text is available.
    pub text: Option<String>,
    /// Whether captured text was omitted or shortened by the inspector budget.
    pub text_truncated: bool,
    /// Exact selected byte digest.
    pub digest: Option<String>,
    /// Immutable artifact identity, when supplied as a reference.
    pub artifact_id: Option<String>,
}

/// Start facts remain unchanged by subsequent edits or capability refreshes.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[cfg_attr(feature = "web-codegen", derive(ts_rs::TS))]
#[serde(deny_unknown_fields)]
pub struct FrozenTaskDto {
    /// Selected inputs with causal names.
    pub inputs: BTreeMap<String, FrozenInputDto>,
    /// Exact selected capability generation.
    #[cfg_attr(feature = "web-codegen", ts(type = "number"))]
    pub capability_generation: u64,
    /// Authenticated actor whose authority governed start.
    pub actor_id: String,
    /// Frozen grant identity.
    pub grant_id: String,
    /// Frozen grant revision.
    #[cfg_attr(feature = "web-codegen", ts(type = "number"))]
    pub grant_revision: u64,
    /// Durable external-entry intent; false means entry has not yet been recorded.
    pub entered: bool,
}

/// Compact lifecycle projection; selected task content is fetched separately.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[cfg_attr(feature = "web-codegen", derive(ts_rs::TS))]
#[serde(deny_unknown_fields)]
pub struct TaskSummaryDto {
    /// Stable task identity.
    pub id: String,
    /// Accepted definition version.
    #[cfg_attr(feature = "web-codegen", ts(type = "number"))]
    pub version: u64,
    /// Human-readable accepted title.
    pub title: String,
    /// Human-readable accepted description.
    pub description: String,
    /// Accepted producer task identities used by the timeline.
    pub dependencies: Vec<String>,
    /// Durable lifecycle state.
    pub state: TaskStateDto,
    /// Durable external-entry intent, independent of frozen start acceptance.
    pub entered: bool,
    /// Missing input or settlement explanation.
    pub reason: Option<String>,
    /// Frozen start acceptance; a deferred queued task may retain this timestamp.
    #[cfg_attr(feature = "web-codegen", ts(type = "number | null"))]
    pub started_at: Option<u64>,
    /// Proven settlement time.
    #[cfg_attr(feature = "web-codegen", ts(type = "number | null"))]
    pub ended_at: Option<u64>,
    /// Whether cancellation has been requested.
    pub stop_requested: bool,
    /// Reserved exact stop action identity.
    pub stop_request_id: String,
    /// Whether a retained result can be inspected.
    pub has_result: bool,
}

/// Discovery metadata without task definitions, captured inputs or result bodies.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[cfg_attr(feature = "web-codegen", derive(ts_rs::TS))]
#[serde(deny_unknown_fields)]
pub struct WorkflowSummaryDto {
    /// Current project organization; null identifies an earlier unassociated workflow.
    #[serde(default)]
    pub project_id: Option<String>,
    /// Stable workflow identity.
    pub id: String,
    /// Discoverable accepted title.
    pub title: String,
    /// Workflow brief.
    pub description: String,
    /// Accepted edit generation.
    #[cfg_attr(feature = "web-codegen", ts(type = "number"))]
    pub generation: u64,
    /// Current lifecycle state.
    pub state: String,
    /// Creation timestamp.
    #[cfg_attr(feature = "web-codegen", ts(type = "number"))]
    pub created_at: u64,
    /// Most recent recorded change.
    #[cfg_attr(feature = "web-codegen", ts(type = "number"))]
    pub updated_at: u64,
    /// Number of retained tasks in this workflow.
    pub task_count: u32,
}

/// Authorized workflow projection used by both discovery and the inspector.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[cfg_attr(feature = "web-codegen", derive(ts_rs::TS))]
#[serde(deny_unknown_fields)]
pub struct WorkflowReadDto {
    /// Current organizational project, independent of accepted historical task bindings.
    #[serde(default)]
    pub project_id: Option<String>,
    /// Shared daemon-owned sources with bounded previews; absence means no supplied value.
    #[serde(default)]
    pub supplied_inputs: BTreeMap<String, TaskSuppliedInputDto>,
    /// Current actionable question preventing new entry.
    pub wait_question: Option<String>,
    /// Cumulative task admissions, including removed and failed tasks.
    pub admitted_tasks: u32,
    /// Cumulative accepted model batches, including feedback tasks.
    pub admitted_model_batches: u32,
    /// Consecutive rejected or nonprogressing responses.
    pub rejected_responses: u32,
    /// Cumulative elapsed execution across all simultaneous attempts.
    #[cfg_attr(feature = "web-codegen", ts(type = "number"))]
    pub active_execution_ms: u64,
    /// Response decisions for source tasks on this page; complete history follows task paging.
    pub admissions: Vec<ModelAdmissionDto>,
    /// Stable daemon-assigned workflow identity.
    pub id: String,
    /// Discoverable title.
    pub title: String,
    /// Brief and description.
    pub description: String,
    /// Optimistic edit generation.
    #[cfg_attr(feature = "web-codegen", ts(type = "number"))]
    pub generation: u64,
    /// Workflow lifecycle with waiting distinct from completion.
    pub state: String,
    /// Recorded creation time.
    #[cfg_attr(feature = "web-codegen", ts(type = "number"))]
    pub created_at: u64,
    /// Latest recorded update time.
    #[cfg_attr(feature = "web-codegen", ts(type = "number"))]
    pub updated_at: u64,
    /// Effective cumulative ceilings.
    pub limits: WorkflowLimitsDto,
    /// Tasks ordered for stable presentation; order does not schedule work.
    pub tasks: Vec<TaskSummaryDto>,
    /// Number of retained tasks across all pages.
    pub task_count: u32,
    /// Continue the live task list without loading full task content.
    pub next_task_cursor: Option<String>,
}

/// Inspectable link between immutable model output and its accepted or refused future work.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[cfg_attr(feature = "web-codegen", derive(ts_rs::TS))]
#[serde(deny_unknown_fields)]
pub struct ModelAdmissionDto {
    /// Source task on this workflow page.
    pub task_id: String,
    /// Pending, accepted, rejected, repaired or dismissed.
    pub status: String,
    /// Canonical immutable response digest, if the model published it.
    pub response_digest: Option<String>,
    /// Bounded rejection or decision reason.
    pub reason: Option<String>,
    /// Accepted request receipt identity.
    pub receipt_key: Option<String>,
    /// Stable identities allocated by the accepted batch.
    pub aliases: BTreeMap<String, String>,
}

/// One bounded search/list page; cursors bind daemon, actor and exact query.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[cfg_attr(feature = "web-codegen", derive(ts_rs::TS))]
#[serde(deny_unknown_fields)]
pub struct WorkflowPageDto {
    /// Accessible workflows in this page.
    pub items: Vec<WorkflowSummaryDto>,
    /// Opaque continuation, if more accessible results remain.
    pub next_cursor: Option<String>,
    /// True when the bounded search window is exhausted.
    pub truncated: bool,
}

/// Connection handshake after authenticated authorization.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[cfg_attr(feature = "web-codegen", derive(ts_rs::TS))]
#[serde(deny_unknown_fields)]
pub struct TaskSessionDto {
    /// Safe real connector choices for the permitted current executor generations.
    #[serde(default)]
    pub authoring: Vec<TaskCapabilityAuthoringDto>,
    /// Permitted configured filesystem roots for explicitly selected native file tools.
    #[serde(default)]
    pub project_roots: Vec<TaskProjectRootDto>,
    /// Maximum UTF-8 bytes in a supplied text value or captured file.
    pub input_limit_bytes: u32,
    /// Aggregate declared output allowance per task. Zero means an older server omitted it.
    #[serde(default)]
    #[cfg_attr(feature = "web-codegen", ts(type = "number"))]
    pub task_output_limit_bytes: u64,
    /// Installation identity required on subsequent task operations.
    pub daemon_id: String,
    /// Shared current daemon name; never a connection-local label.
    pub daemon_name: String,
    /// Generation of the observed name, independent of workflow changes.
    #[cfg_attr(feature = "web-codegen", ts(type = "number"))]
    pub daemon_name_generation: u64,
    /// Whether the authenticated actor may change the shared daemon name.
    pub can_rename_daemon: bool,
    /// Server-owned authenticated actor identity.
    pub actor_id: String,
    /// Effective default policy offered before start.
    pub limits: WorkflowLimitsDto,
    /// Available and permitted configured choices.
    pub capabilities: Vec<TaskCapability>,
}

/// Bounded UTF-8 upload. Binary files are refused until a supported mapping exists.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[cfg_attr(feature = "web-codegen", derive(ts_rs::TS))]
#[serde(deny_unknown_fields)]
pub struct TaskUploadRequest {
    /// Exact upload request identity.
    pub request_id: String,
    /// Basename shown as provenance.
    pub name: String,
    /// Text media type.
    pub media_type: String,
    /// Actual UTF-8 bytes captured by the daemon.
    pub text: String,
}

/// Exact uploaded content reference.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[cfg_attr(feature = "web-codegen", derive(ts_rs::TS))]
#[serde(deny_unknown_fields)]
pub struct TaskUploadReceipt {
    /// Actor-owned immutable input identity.
    pub input_id: String,
    /// Uploaded basename.
    pub name: String,
    /// Checked captured byte identity.
    pub digest: String,
    /// UTF-8 byte count.
    #[cfg_attr(feature = "web-codegen", ts(type = "number"))]
    pub bytes: u64,
}
