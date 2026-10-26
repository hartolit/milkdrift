//! Conditional saved sketches and deliberate, exactly recoverable admission are separate operations.

use crate::tasks::{InputValue, TaskCapability, TaskChangeDto, TaskDefinitionDto};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Advertised finite operational envelope for unfinished work.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-codegen", derive(ts_rs::TS))]
#[serde(deny_unknown_fields)]
pub struct AuthoringBoundsDto {
    /// Encoded latest document bytes.
    pub document_bytes: u32,
    /// Pending cards, including edits of accepted tasks.
    pub pending_tasks: u32,
    /// Source cards.
    pub sources: u32,
    /// Intended data and order wires.
    pub connections: u32,
    /// Independently saved card positions.
    pub positions: u32,
    /// Maximum actions admitted by one Run or Apply.
    pub run_actions: u32,
}

/// A deliberately chosen default for new cards, with its reviewed generation.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-codegen", derive(ts_rs::TS))]
#[serde(deny_unknown_fields)]
pub struct AuthoringModelDto {
    /// Exact configured executor.
    pub capability: TaskCapability,
    /// Reviewed current contract.
    #[cfg_attr(feature = "web-codegen", ts(type = "number"))]
    pub generation: u64,
}

/// Empty fields are permitted here; saving this card never admits an executable task.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-codegen", derive(ts_rs::TS))]
#[serde(deny_unknown_fields)]
pub struct AuthoringTaskDto {
    /// Stable card identity, or existing task identity for a candidate edit.
    pub id: String,
    /// Accepted version being edited; null for a new task.
    #[cfg_attr(feature = "web-codegen", ts(type = "number | null"))]
    pub version: Option<u64>,
    /// Candidate fields; input/output construction belongs to shared admission.
    pub definition: TaskDefinitionDto,
    /// True when the person overrode the suggested title.
    pub title_edited: bool,
    /// Reviewed executor contract; null while unassigned.
    #[cfg_attr(feature = "web-codegen", ts(type = "number | null"))]
    pub contract_generation: Option<u64>,
}

/// Text, captured file or deliberately missing input arranged on the canvas.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-codegen", derive(ts_rs::TS))]
#[serde(deny_unknown_fields)]
pub struct AuthoringSourceDto {
    /// Stable card identity.
    pub id: String,
    /// Readable source name.
    pub label: String,
    /// Captured value, or null for unfinished input.
    pub value: Option<InputValue>,
    /// Workflow input name, including a new name deliberately awaiting a later supplied value;
    /// null identifies an independent source.
    pub shared_name: Option<String>,
}

/// A visible wire's intended meaning.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-codegen", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum AuthoringConnectionKindDto {
    /// Content feeds a real input.
    Data,
    /// Independent order requires success.
    AfterSuccess,
    /// Independent order requires proven settlement.
    AfterCompletion,
}

/// Unresolved endpoints preserve intent and block included work from running.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-codegen", derive(ts_rs::TS))]
#[serde(deny_unknown_fields)]
pub struct AuthoringConnectionDto {
    /// Stable wire identity used by local diagnostics.
    pub id: String,
    /// Source card or accepted task identity.
    pub source: String,
    /// Receiving card or accepted task identity.
    pub target: String,
    /// Real adapter input, or null before a meaningful choice is resolved.
    pub input: Option<String>,
    /// Real adapter output, or null before a meaningful choice is resolved.
    pub output: Option<String>,
    /// Data or independent ordering.
    pub kind: AuthoringConnectionKindDto,
    /// Explicitly accepts absent settled content.
    pub optional: bool,
    /// Explicitly accepts the settled outcome, including failure.
    pub outcome: bool,
}

/// The latest non-executable sketch; the daemon removes admitted definitions after Run.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-codegen", derive(ts_rs::TS))]
#[serde(deny_unknown_fields)]
pub struct AuthoringDocumentDto {
    /// Incomplete new tasks or unapplied candidate edits.
    pub tasks: Vec<AuthoringTaskDto>,
    /// Arranged captured inputs.
    pub sources: Vec<AuthoringSourceDto>,
    /// Resolved or unfinished wires.
    pub connections: Vec<AuthoringConnectionDto>,
    /// Other prospective changes; add/revise and direct controls are refused here.
    pub changes: Vec<TaskChangeDto>,
    /// Deliberate usual executor choice.
    pub usual_model: Option<AuthoringModelDto>,
    /// Daemon-maintained card-to-task references; a save cannot invent mappings.
    pub accepted: BTreeMap<String, String>,
}

/// Current saved content. A matching digest/content proves what is saved now, not historical replay.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-codegen", derive(ts_rs::TS))]
#[serde(deny_unknown_fields)]
pub struct AuthoringSnapshotDto {
    /// Conditional-write generation, independent of execution progress and positions.
    #[cfg_attr(feature = "web-codegen", ts(type = "number"))]
    pub generation: u64,
    /// Canonical stored content identity.
    pub digest: String,
    /// Current bounded sketch.
    pub document: AuthoringDocumentDto,
}

/// Compare-and-replace of current authoring; no idempotency identity or historical receipt.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-codegen", derive(ts_rs::TS))]
#[serde(deny_unknown_fields)]
pub struct CheckpointAuthoringRequest {
    /// Last observed authoring generation.
    #[cfg_attr(feature = "web-codegen", ts(type = "number"))]
    pub expected_generation: u64,
    /// Whole replacement sketch.
    pub document: AuthoringDocumentDto,
}

/// Presentation coordinate, never scheduling or elapsed time.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-codegen", derive(ts_rs::TS))]
#[serde(deny_unknown_fields)]
pub struct CanvasPositionDto {
    /// Horizontal canvas coordinate.
    pub x: f64,
    /// Vertical canvas coordinate.
    pub y: f64,
}

/// Independent saved layout; camera and selection remain client preferences.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-codegen", derive(ts_rs::TS))]
#[serde(deny_unknown_fields)]
pub struct PositionsSnapshotDto {
    /// Layout-only conditional-write generation.
    #[cfg_attr(feature = "web-codegen", ts(type = "number"))]
    pub generation: u64,
    /// Canonical saved layout identity.
    pub digest: String,
    /// Bounded positions keyed by card identity.
    pub positions: BTreeMap<String, CanvasPositionDto>,
}

/// A layout gesture cannot invalidate an authoring submission.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-codegen", derive(ts_rs::TS))]
#[serde(deny_unknown_fields)]
pub struct PutPositionsRequest {
    /// Last observed layout generation.
    #[cfg_attr(feature = "web-codegen", ts(type = "number"))]
    pub expected_generation: u64,
    /// Complete replacement layout.
    pub positions: BTreeMap<String, CanvasPositionDto>,
}

/// Named scope requested for review; inspecting a card does not alter it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-codegen", derive(ts_rs::TS))]
#[serde(deny_unknown_fields)]
pub struct ReviewAuthoringRequest {
    /// Saved sketch reviewed by this caller.
    #[cfg_attr(feature = "web-codegen", ts(type = "number"))]
    pub authoring_generation: u64,
    /// Pending card identities deliberately included.
    pub tasks: Vec<String>,
    /// Positions in the saved changes list deliberately included.
    pub changes: Vec<u32>,
    /// Accepted predecessors whose current definitions are deliberately used despite candidate edits.
    pub use_current: Vec<String>,
}

/// A bounded repair attached to an authorized visible object.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-codegen", derive(ts_rs::TS))]
#[serde(deny_unknown_fields)]
pub struct AuthoringIssueDto {
    /// Stable repair category; prose is not a policy protocol.
    pub code: String,
    /// Useful explanation for the person making the change.
    pub message: String,
    /// Affected card when known.
    pub task_id: Option<String>,
    /// Affected wire when known.
    pub connection_id: Option<String>,
    /// Affected source when known.
    pub source_id: Option<String>,
    /// Affected saved change when known.
    pub change_index: Option<u32>,
}

/// Reviewed full scope, including required new predecessors. It executes nothing.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-codegen", derive(ts_rs::TS))]
#[serde(deny_unknown_fields)]
pub struct AuthoringReviewDto {
    /// Saved content generation used by this review.
    #[cfg_attr(feature = "web-codegen", ts(type = "number"))]
    pub authoring_generation: u64,
    /// Current workflow decision observation for Run.
    #[cfg_attr(feature = "web-codegen", ts(type = "number"))]
    pub expected_generation: u64,
    /// Whole named set to review before submission.
    pub tasks: Vec<String>,
    /// Explicit selected saved changes.
    pub changes: Vec<u32>,
    /// Existing candidate predecessors deliberately used unchanged.
    pub use_current: Vec<String>,
    /// Included work cannot run until these repairs are made.
    pub issues: Vec<AuthoringIssueDto>,
}

/// Exactly recoverable admission of the saved, reviewed scope.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-codegen", derive(ts_rs::TS))]
#[serde(deny_unknown_fields)]
pub struct RunAuthoringRequest {
    /// Exact actor-scoped request identity.
    pub request_id: String,
    /// Reviewed workflow semantic guard.
    #[cfg_attr(feature = "web-codegen", ts(type = "number"))]
    pub expected_generation: u64,
    /// Exact reviewed saved content generation.
    #[cfg_attr(feature = "web-codegen", ts(type = "number"))]
    pub authoring_generation: u64,
    /// Whole named task scope; no implicit subset or predecessor insertion at submission.
    pub tasks: Vec<String>,
    /// Deliberately reviewed saved change indices.
    pub changes: Vec<u32>,
    /// Explicit choices to keep current accepted predecessor definitions.
    pub use_current: Vec<String>,
}
