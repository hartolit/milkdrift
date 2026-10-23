//! Ordinary task definitions. Input availability is decided at execution, not authoring.

use std::collections::{BTreeMap, BTreeSet};

use milkdrift_capability::CapabilityRequirement;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::TaskId;

/// The default maximum number of ordinary tasks admitted to one workflow.
pub const DEFAULT_WORKFLOW_TASKS: u32 = 256;
/// A single atomic human or agent edit is deliberately smaller than a workflow.
pub const MAX_TASK_BATCH_ACTIONS: usize = 32;
/// Combined instruction and inline-input ceiling for a task definition.
pub const MAX_TASK_DEFINITION_BYTES: usize = 1_048_576;
/// Maximum UTF-8 text bytes in one instruction, supplied text or captured file.
pub const MAX_TASK_TEXT_BYTES: usize = 16_384;
/// Aggregate output allowance of one task, also bounding an undeclared retained output binding.
pub const MAX_TASK_OUTPUT_BYTES: u64 = 2_097_152;
/// Escaped instructions and known input content must fit the shared inline execution contract.
const MAX_TASK_RENDERED_BYTES: usize = 24_576;

/// A captured value carries its bytes, rather than a mutable filesystem path.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum TaskInputValue {
    /// Human text or an explicitly supplied text input.
    Text {
        /// Exact supplied UTF-8 content.
        text: String,
    },
    /// Uploaded or host-captured UTF-8 file bytes and their checked content identity.
    TextFile {
        /// Display basename, never an executable filesystem path.
        name: String,
        /// Captured UTF-8 bytes used by execution.
        text: String,
        /// Canonical BLAKE3 identity checked against the captured bytes.
        digest: String,
        /// Original uploaded artifact identity; exact request replay distinguishes equal bytes from different uploads.
        #[serde(default)]
        source_artifact: Option<String>,
    },
    /// A provider-neutral structured capability input.
    Json {
        /// Explicit structured value; the capability validates its own request schema.
        value: serde_json::Value,
    },
    /// Exact immutable result bytes already retained by the owning daemon.
    Artifact {
        /// Immutable content identity selected from an accepted task result.
        reference: milkdrift_capability::ArtifactReference,
    },
    /// An explicitly optional supplied value was omitted.
    Absent,
}

impl TaskInputValue {
    /// Checks bounded content and captured-file integrity.
    ///
    /// # Errors
    /// Refuses oversized values, invalid basenames and source identities, or mismatched digests.
    pub fn validate(&self) -> Result<(), TaskDefinitionError> {
        let bytes = serde_json::to_vec(self).map_err(|e| TaskDefinitionError(e.to_string()))?;
        if bytes.len() > MAX_TASK_DEFINITION_BYTES {
            return Err(TaskDefinitionError("input exceeds 1 MiB".into()));
        }
        let content_bytes = match self {
            Self::Text { text } | Self::TextFile { text, .. } => text.len(),
            Self::Json { value } => serde_json::to_vec(value)
                .map_err(|error| TaskDefinitionError(error.to_string()))?
                .len(),
            Self::Artifact { .. } | Self::Absent => 0,
        };
        if content_bytes > MAX_TASK_TEXT_BYTES {
            return Err(TaskDefinitionError("task text or JSON input exceeds 16 KiB; shorten or split the input before submission".into()));
        }
        if let Self::TextFile {
            name,
            text,
            digest,
            source_artifact,
        } = self
        {
            if name.is_empty() || name.len() > 255 || name.contains(['/', '\\', '\0']) {
                return Err(TaskDefinitionError(
                    "captured file name must be a bounded basename".into(),
                ));
            }
            if digest != &format!("b3_{}", blake3::hash(text.as_bytes())) {
                return Err(TaskDefinitionError(
                    "captured text-file digest differs from its bytes".into(),
                ));
            }
            if source_artifact
                .as_ref()
                .is_some_and(|identity| identity.is_empty() || identity.len() > 1024)
            {
                return Err(TaskDefinitionError(
                    "captured file source identity is invalid".into(),
                ));
            }
        }
        Ok(())
    }

    /// Makes a byte-identified UTF-8 file input from accepted upload bytes.
    ///
    /// # Errors
    /// Refuses an oversized input or a name that is not a bounded basename.
    pub fn text_file(name: String, text: String) -> Result<Self, TaskDefinitionError> {
        let value = Self::TextFile {
            name,
            digest: format!("b3_{}", blake3::hash(text.as_bytes())),
            text,
            source_artifact: None,
        };
        value.validate()?;
        Ok(value)
    }
}

/// A named task input. Output and outcome bindings imply an explicit dependency.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum TaskInputBinding {
    /// Content supplied directly with the accepted definition.
    Inline {
        /// The value frozen if this definition starts.
        value: TaskInputValue,
    },
    /// A named workflow input that may arrive after the task is authored.
    Supplied {
        /// Workflow input name.
        name: String,
        /// Missing or explicitly absent optional values become typed absence.
        /// Required values keep work queued in either case; empty text and JSON null remain values.
        #[serde(default)]
        optional: bool,
    },
    /// A declared future output or an actual named retained result from a producer dependency.
    TaskOutput {
        /// Producer identity or an alias in the same atomic request.
        task: TaskId,
        /// Output name recorded by the producer's capability.
        output: String,
        /// A settled producer without this output may supply typed absence.
        #[serde(default)]
        optional: bool,
    },
    /// Completion, failure, or proven stop of a declared dependency, including available results.
    SettledOutcome {
        /// Producer identity; uncertain work is never treated as settled.
        task: TaskId,
    },
}

/// Declared native root access, acquired by capability-host before file capture or entry.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaskPathAccess {
    /// Absolute native path resolved and rechecked by the execution host.
    pub path: String,
    /// A write claim excludes overlapping readers and writers until durable settlement.
    pub write: bool,
}

/// A named artifact obligation checked against immutable capability output evidence.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaskOutputDeclaration {
    /// A successful task must publish this output when true.
    #[serde(default = "required_by_default")]
    pub required: bool,
    /// Exact expected media type, when the caller requires one.
    #[serde(default)]
    pub media_type: Option<String>,
    /// Maximum artifact bytes; unknown artifact sizes cannot satisfy this declaration.
    pub max_bytes: u64,
}

impl TaskOutputDeclaration {
    /// Checks retained output evidence against this producer's media and byte contract.
    ///
    /// # Errors
    /// Refuses unknown or excessive size and a media type differing from an explicit declaration.
    pub fn validate_reference(
        &self,
        reference: &milkdrift_capability::ArtifactReference,
    ) -> Result<(), TaskDefinitionError> {
        if reference
            .size_bytes()
            .is_none_or(|bytes| bytes > self.max_bytes)
        {
            return Err(TaskDefinitionError(
                "output size is unknown or exceeds its allowance".into(),
            ));
        }
        if self
            .media_type
            .as_deref()
            .is_some_and(|expected| reference.media_type() != Some(expected))
        {
            return Err(TaskDefinitionError(
                "output media type differs from its contract".into(),
            ));
        }
        Ok(())
    }
}

/// One explicitly named future input redirected by correction adoption.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaskBindingRedirect {
    /// Consumer whose unstarted definition is being changed.
    pub consumer: TaskId,
    /// Exact consumer version observed by the editor.
    pub expected_version: u64,
    /// Named input currently bound to an output of the corrected task.
    pub input: String,
}

/// Tools a model task may request; listing a tool never expands its actor's authority.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum TaskTool {
    /// Reads bounded UTF-8 files beneath declared task paths.
    ReadFile,
    /// Writes bounded UTF-8 files beneath declared writable task paths.
    WriteFile,
    /// Invokes one configured process capability through its existing adapter.
    Process {
        /// Exact selection constraints checked under the task's grant.
        capability: CapabilityRequirement,
    },
}

/// Opt-in policy for structured future-work proposals from one model task.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaskActionPolicy {
    /// Permit an ordinary follow-up task to repair invalid or conflicted model actions.
    /// The follow-up retains the same capability, tools and cumulative workflow allowance.
    pub automatic_repair: bool,
}

/// Future task content. Runtime freezes the accepted version together with resolved inputs.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaskDefinition {
    /// Reviewed project choice, resolved to immutable effective facts by shared admission.
    #[serde(default, skip_serializing_if = "crate::TaskExecution::is_legacy")]
    pub execution: crate::TaskExecution,
    /// Opt in to structured future-work proposals. Ordinary result text is never executable.
    #[serde(default)]
    pub actions: Option<TaskActionPolicy>,
    /// Human-readable purpose displayed in task discovery and the timeline.
    pub title: String,
    /// Optional bounded explanation shown before a task starts.
    #[serde(default)]
    pub description: String,
    /// Instructions frozen at accepted task start.
    pub instructions: String,
    /// Required external operation and explicit selection constraints.
    pub capability: CapabilityRequirement,
    /// Named values selected independently of display order or instruction text.
    #[serde(default)]
    pub inputs: BTreeMap<String, TaskInputBinding>,
    /// Named output obligations checked before successful workflow completion.
    #[serde(default)]
    pub outputs: BTreeMap<String, TaskOutputDeclaration>,
    /// Producers that must satisfy their declared outcome condition before entry.
    #[serde(default)]
    pub dependencies: BTreeSet<TaskId>,
    /// Whether successful completion or explicit successful correction is a workflow obligation.
    #[serde(default = "required_by_default")]
    pub required: bool,
    /// Earlier task addressed by this new work; does not relabel that task's history.
    #[serde(default)]
    pub corrective_of: Option<TaskId>,
    /// Explicit native access coordinated through durable host claims.
    #[serde(default)]
    pub paths: Vec<TaskPathAccess>,
    /// Explicit tool catalog available only inside this task's bounded model loop.
    #[serde(default)]
    pub tools: Vec<TaskTool>,
}

const fn required_by_default() -> bool {
    true
}

impl TaskDefinition {
    /// Validates definition bounds and explicit dependency consistency without requiring values.
    ///
    /// # Errors
    /// Refuses invalid names, paths, bounds, or bindings missing their dependency declaration.
    pub fn validate(&self) -> Result<(), TaskDefinitionError> {
        if let Some(binding) = self.execution.binding()
            && (binding.project_id.is_empty()
                || binding.project_id.len() > 256
                || binding.generation == 0
                || binding.folder.identity.is_empty()
                || binding.folder.identity.len() > 256
                || binding.folder.physical.len() > 4096
                || !std::path::Path::new(&binding.folder.physical).is_absolute()
                || self.paths != binding.paths
                || binding.paths.len() != binding.scopes.len()
                || binding
                    .paths
                    .iter()
                    .zip(&binding.scopes)
                    .any(|(path, scope)| path.path != scope.requested)
                || (binding.access == crate::TaskProjectAccess::None && !binding.paths.is_empty())
                || (binding.access != crate::TaskProjectAccess::Write
                    && binding.paths.iter().any(|path| path.write))
                || self.tools != binding.tools)
        {
            return Err(TaskDefinitionError(
                "project binding differs from its effective native access or tools".into(),
            ));
        }
        if self.title.trim().is_empty() || self.title.len() > 256 || self.description.len() > 4096 {
            return Err(TaskDefinitionError(
                "task title or description is empty or oversized".into(),
            ));
        }
        if self.instructions.trim().is_empty()
            || self.instructions.len() > MAX_TASK_TEXT_BYTES
            || self.inputs.len() > 64
            || self.outputs.len() > 64
            || self.dependencies.len() > 256
            || self.paths.len() > 32
            || self.tools.len() > 32
        {
            return Err(TaskDefinitionError(
                "task instructions or collection bounds are invalid".into(),
            ));
        }
        let mut output_bytes = 0_u64;
        let mut tool_keys = BTreeSet::new();
        for tool in &self.tools {
            let key = serde_json::to_string(tool)
                .map_err(|error| TaskDefinitionError(error.to_string()))?;
            if !tool_keys.insert(key) {
                return Err(TaskDefinitionError("duplicate model tool selection".into()));
            }
            if matches!(tool, TaskTool::ReadFile) && self.paths.is_empty()
                || matches!(tool, TaskTool::WriteFile) && !self.paths.iter().any(|path| path.write)
            {
                return Err(TaskDefinitionError(
                    "file tools require matching task path claims".into(),
                ));
            }
        }
        for (name, declaration) in &self.outputs {
            validate_name(name)?;
            if declaration.max_bytes == 0
                || declaration.media_type.as_ref().is_some_and(|value| {
                    value.is_empty() || value.len() > 255 || value.contains(['\r', '\n'])
                })
            {
                return Err(TaskDefinitionError(
                    "output declaration needs a positive byte bound and valid media type".into(),
                ));
            }
            output_bytes = output_bytes
                .checked_add(declaration.max_bytes)
                .ok_or_else(|| TaskDefinitionError("output byte bound overflow".into()))?;
        }
        if output_bytes > MAX_TASK_OUTPUT_BYTES {
            return Err(TaskDefinitionError(
                "task declared output allowance exceeds 2 MiB".into(),
            ));
        }
        for (name, binding) in &self.inputs {
            validate_name(name)?;
            if matches!(name.as_str(), "instructions" | "milkdrift.model_task") {
                return Err(TaskDefinitionError(
                    "task input name is reserved for the execution request".into(),
                ));
            }
            match binding {
                TaskInputBinding::Inline { value } => value.validate()?,
                TaskInputBinding::Supplied { name, .. } => validate_name(name)?,
                TaskInputBinding::TaskOutput { task, output, .. } => {
                    validate_name(output)?;
                    if !self.dependencies.contains(task) {
                        return Err(TaskDefinitionError(format!(
                            "output binding requires dependency {task}"
                        )));
                    }
                }
                TaskInputBinding::SettledOutcome { task } => {
                    if !self.dependencies.contains(task) {
                        return Err(TaskDefinitionError(format!(
                            "outcome binding requires dependency {task}"
                        )));
                    }
                }
            }
        }
        for access in &self.paths {
            if !std::path::Path::new(&access.path).is_absolute()
                || access.path.len() > 4096
                || access.path.contains('\0')
            {
                return Err(TaskDefinitionError(
                    "native task paths must be bounded absolute paths".into(),
                ));
            }
        }
        if serde_json::to_vec(self)
            .map_err(|e| TaskDefinitionError(e.to_string()))?
            .len()
            > MAX_TASK_DEFINITION_BYTES
        {
            return Err(TaskDefinitionError("task definition exceeds 1 MiB".into()));
        }
        Ok(())
    }

    /// Checks known future input content against the shared model/process inline request budget.
    /// Missing supplied values remain prospective and are checked when a later edit supplies them.
    ///
    /// # Errors
    /// Refuses escaped instructions and known input content above 24 KiB, before admission.
    pub fn validate_supplied_inputs(
        &self,
        supplied: &BTreeMap<String, TaskInputValue>,
    ) -> Result<(), TaskDefinitionError> {
        let encoded = |text: &str| {
            serde_json::to_vec(text)
                .map(|bytes| bytes.len())
                .map_err(|error| TaskDefinitionError(error.to_string()))
        };
        let mut total = encoded(&self.instructions)?;
        for (name, binding) in &self.inputs {
            total = total
                .checked_add(encoded(&format!("\n\nInput {name}:\n"))?)
                .ok_or_else(|| TaskDefinitionError("task input budget overflow".into()))?;
            let value = match binding {
                TaskInputBinding::Inline { value } => Some(value),
                TaskInputBinding::Supplied { name, .. } => supplied.get(name),
                TaskInputBinding::TaskOutput { .. } | TaskInputBinding::SettledOutcome { .. } => {
                    None
                }
            };
            if let Some(value) = value {
                value.validate()?;
                let rendered = match value {
                    TaskInputValue::Text { text } | TaskInputValue::TextFile { text, .. } => {
                        text.clone()
                    }
                    TaskInputValue::Json { value } => serde_json::to_string(value)
                        .map_err(|error| TaskDefinitionError(error.to_string()))?,
                    TaskInputValue::Artifact { .. } => {
                        "[immutable named artifact supplied with this request]".into()
                    }
                    TaskInputValue::Absent => "[explicitly absent optional input]".into(),
                };
                total = total
                    .checked_add(encoded(&rendered)?)
                    .ok_or_else(|| TaskDefinitionError("task input budget overflow".into()))?;
            }
        }
        if total > MAX_TASK_RENDERED_BYTES {
            return Err(TaskDefinitionError("task instructions and supplied inputs exceed the combined 24 KiB escaped-content budget; shorten or split the task".into()));
        }
        Ok(())
    }
}

fn validate_name(name: &str) -> Result<(), TaskDefinitionError> {
    if name.is_empty()
        || name.len() > 128
        || !name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.'))
    {
        return Err(TaskDefinitionError(
            "input/output name must contain 1..=128 safe ASCII bytes".into(),
        ));
    }
    Ok(())
}

/// Cumulative workflow limits. Restart and future edits do not reset spent allowances.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkflowLimits {
    /// Cumulative admitted tasks, including later removed or failed tasks.
    pub tasks: u32,
    /// Maximum simultaneous tasks; unresolved entered work still consumes a slot.
    pub concurrent_tasks: u32,
    /// Cumulative model-produced change batches permitted by the workflow policy.
    pub model_batches: u32,
    /// Consecutive rejected or nonprogressing responses permitted before waiting for a person.
    pub rejected_responses: u32,
    /// Failed attempts permitted on one corrective lineage.
    pub corrective_failures: u32,
    /// Inclusive cumulative active-execution allowance, shared by simultaneous attempts.
    /// Entry reserves each adapter's enforced duration; unresolved work retains its reservation.
    pub active_execution_ms: u64,
    /// Tool invocations permitted within one task's model/tool loop.
    pub tool_calls_per_task: u32,
}

impl Default for WorkflowLimits {
    fn default() -> Self {
        Self {
            tasks: DEFAULT_WORKFLOW_TASKS,
            concurrent_tasks: 4,
            model_batches: 32,
            rejected_responses: 3,
            corrective_failures: 3,
            active_execution_ms: 3_600_000,
            tool_calls_per_task: 16,
        }
    }
}

impl WorkflowLimits {
    /// Checks finite policy bounds before any workflow work is admitted.
    ///
    /// # Errors
    /// Refuses zero, unsupported, or inconsistent collection and duration limits.
    pub fn validate(&self) -> Result<(), TaskDefinitionError> {
        if self.tasks == 0
            || self.tasks > 4096
            || self.concurrent_tasks == 0
            || self.concurrent_tasks > self.tasks
            || self.concurrent_tasks > 64
            || self.model_batches == 0
            || self.model_batches > 4096
            || self.rejected_responses == 0
            || self.corrective_failures == 0
            || self.active_execution_ms == 0
            || self.active_execution_ms > 604_800_000
            || self.tool_calls_per_task == 0
            || self.tool_calls_per_task > 1024
        {
            return Err(TaskDefinitionError(
                "workflow limits are outside bounded supported values".into(),
            ));
        }
        Ok(())
    }
}

/// One atomic future-work change. New identities are allocated only by the owning daemon.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum TaskChange {
    /// Close a rejected model proposal after inspecting it; edits can accompany this action.
    DismissAdmission {
        /// Exact source task whose immutable response remains retained.
        task: TaskId,
    },
    /// Add future work and return its daemon identity in the receipt's alias map.
    Add {
        /// Request-local identity available to other changes in this batch.
        alias: TaskId,
        /// Complete new task content.
        definition: TaskDefinition,
    },
    /// Replace only an unstarted task's next definition.
    Revise {
        /// Exact existing task.
        task: TaskId,
        /// Version observed by the caller; stale versions refuse the whole batch.
        expected_version: u64,
        /// Complete next definition, retained as a new immutable version.
        definition: TaskDefinition,
    },
    /// Remove an unstarted obligation while retaining its history.
    Remove {
        /// Exact existing task.
        task: TaskId,
        /// Version observed by the caller.
        expected_version: u64,
    },
    /// Supply or replace a future workflow input without changing any frozen start.
    BindInput {
        /// Workflow input binding name.
        name: String,
        /// New immutable value for future starts.
        value: TaskInputValue,
    },
    /// Request cancellation without claiming a process or remote operation has stopped.
    Stop {
        /// Exact task whose current attempt receives the request.
        task: TaskId,
    },
    /// Satisfy a settled failed obligation through explicitly linked successful work.
    AdoptCorrection {
        /// Original failed or stopped task.
        task: TaskId,
        /// Successful task that explicitly names the original correction target.
        correction: TaskId,
        /// Only these named pending bindings follow the correction; started consumers refuse.
        #[serde(default)]
        redirects: Vec<TaskBindingRedirect>,
    },
    /// Replace future policy ceilings without refunding prior work or changing a grant.
    SetLimits {
        /// Complete finite limits; the admitted and elapsed account remains unchanged.
        limits: WorkflowLimits,
    },
    /// Release an explicit wait so ordinary readiness can be reconsidered.
    Continue,
    /// Record a bounded question and pause new entry until an authorized continuation.
    Wait {
        /// Actionable human-facing question.
        question: String,
    },
    /// Request completion once required obligations and every entered operation settle.
    Finish,
}

/// Generation-guarded changes shared by humans and model action admission.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaskBatch {
    /// Workflow generation the author inspected. Authoring requires an exact match; a singleton
    /// wait, continue or stop may cross progress-only changes since the last workflow decision.
    pub expected_generation: u64,
    /// Changes admitted together or refused together.
    pub changes: Vec<TaskChange>,
}

/// A task alias is local to this request and never selects a preexisting daemon task.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NewTask {
    /// Request-local name replaced by a daemon-assigned identity on admission.
    pub alias: TaskId,
    /// Complete initial future definition.
    pub definition: TaskDefinition,
}

/// Complete initial future work admitted through the same batch rules as later edits.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkflowCreate {
    /// Project selected for new work; absence preserves the earlier typed contract.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
    /// Searchable workflow title.
    pub title: String,
    /// Bounded description displayed independently of task instructions.
    #[serde(default)]
    pub description: String,
    /// Initial tasks sharing one alias namespace.
    pub tasks: Vec<NewTask>,
    /// Explicit supplied values; missing required values are valid while authoring.
    #[serde(default)]
    pub inputs: BTreeMap<String, TaskInputValue>,
    /// Finite cumulative policy, with the ordinary ADR defaults when omitted.
    #[serde(default)]
    pub limits: WorkflowLimits,
    /// Initial explicit finish request for finite human-authored work.
    #[serde(default = "required_by_default")]
    pub finish_when_settled: bool,
}

/// Invalid bounded task definition or dependency structure.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
#[error("invalid task definition: {0}")]
pub struct TaskDefinitionError(pub String);

/// Checks the whole prospective task set before any part of a batch can be accepted.
/// The owning runtime supplies named outputs from retained producer results; a caller's requested
/// artifact or a declaration added to a different task is not published evidence.
///
/// # Errors
/// Refuses invalid definitions, excess tasks, missing references, self-dependencies and cycles.
pub fn validate_task_set<'a>(
    tasks: &BTreeMap<TaskId, TaskDefinition>,
    limits: &WorkflowLimits,
    published_output: impl Fn(&TaskId, &str) -> Option<&'a milkdrift_capability::ArtifactReference>,
) -> Result<(), TaskDefinitionError> {
    limits.validate()?;
    if tasks.len()
        > usize::try_from(limits.tasks).map_err(|e| TaskDefinitionError(e.to_string()))?
    {
        return Err(TaskDefinitionError(
            "workflow task capacity exhausted".into(),
        ));
    }
    let mut remaining = BTreeMap::new();
    for (id, definition) in tasks {
        definition.validate()?;
        if definition
            .dependencies
            .iter()
            .any(|dependency| dependency == id || !tasks.contains_key(dependency))
        {
            return Err(TaskDefinitionError(format!(
                "task {id} has a missing or self dependency"
            )));
        }
        if let Some(corrected) = &definition.corrective_of {
            if corrected == id || !tasks.contains_key(corrected) {
                return Err(TaskDefinitionError(format!(
                    "task {id} has an invalid correction origin"
                )));
            }
            let mut seen = BTreeSet::from([id.clone()]);
            let mut origin = Some(corrected);
            while let Some(current) = origin {
                if !seen.insert(current.clone()) {
                    return Err(TaskDefinitionError(
                        "corrective origins contain a cycle".into(),
                    ));
                }
                origin = tasks
                    .get(current)
                    .and_then(|task| task.corrective_of.as_ref());
            }
        }
        for binding in definition.inputs.values() {
            if let TaskInputBinding::TaskOutput {
                task: producer,
                output,
                ..
            } = binding
            {
                let declaration = tasks
                    .get(producer)
                    .and_then(|task| task.outputs.get(output));
                let retained = published_output(producer, output);
                if declaration.is_none() && retained.is_none() {
                    return Err(TaskDefinitionError(format!(
                        "task {id} references output {producer}/{output} without a declaration or retained publication"
                    )));
                }
                if declaration.is_none()
                    && let Some(reference) = retained
                {
                    if reference.media_type().is_none() {
                        return Err(TaskDefinitionError(format!(
                            "task {id} references output {producer}/{output} without a retained media type"
                        )));
                    }
                    TaskOutputDeclaration {
                        required: true,
                        media_type: None,
                        max_bytes: MAX_TASK_OUTPUT_BYTES,
                    }
                    .validate_reference(reference)?;
                }
            }
        }
        let mut waits_for = definition.dependencies.clone();
        if let Some(corrected) = &definition.corrective_of {
            waits_for.insert(corrected.clone());
        }
        remaining.insert(id.clone(), waits_for);
    }
    while !remaining.is_empty() {
        let ready: BTreeSet<_> = remaining
            .iter()
            .filter(|(_, dependencies)| dependencies.is_empty())
            .map(|(id, _)| id.clone())
            .collect();
        if ready.is_empty() {
            return Err(TaskDefinitionError(
                "task dependencies contain a cycle".into(),
            ));
        }
        remaining.retain(|id, _| !ready.contains(id));
        for dependencies in remaining.values_mut() {
            dependencies.retain(|id| !ready.contains(id));
        }
    }
    Ok(())
}
