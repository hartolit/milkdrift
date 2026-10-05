//! Ordinary editor gestures over existing blueprint mutations. Drafts are client-owned files;
//! only a successful save creates an executable immutable revision in the daemon.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Unsubmitted edits against genesis or one exact stored revision. Mutations use the blueprint
/// owner's wire contract; clients retain returned operations without deriving semantic hashes.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct BlueprintDraft {
    /// Workflow identity, independent of its display name.
    pub workflow_id: String,
    /// Exact immutable parent, absent for a new workflow.
    pub base_revision: Option<String>,
    /// Ordered existing blueprint mutations. No credentials or per-run values belong here.
    pub mutations: Vec<Value>,
}

/// A fresh model step after the final failed completeness check of an editor workflow.
/// The daemon retains the original check and selects only the failed response and run inputs.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ModelRepair {
    /// Existing final step held for review.
    pub failed_step: String,
    /// New step identity; existing nodes cannot be replaced by this convenience.
    pub repair_step: String,
    /// Exact permitted model capability.
    pub capability: String,
    /// Fresh instruction, with evidence supplied through normal input bindings.
    pub prompt: String,
    /// Explicit output allowance for the new invocation.
    pub maximum_output_units: u64,
}

/// Explicit source for one named model input; no conversation history is implicitly included.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum ModelInputSource {
    /// A required input supplied separately for each run.
    RunInput {
        /// Declared interface field.
        name: String,
    },
    /// Only the selected earlier step's final text.
    Step {
        /// Earlier model step.
        step: String,
    },
}

/// Scriptable conveniences for the supported model workflow editor. Richer definitions must
/// round-trip exactly through that editor or the daemon refuses the edit before saving.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum BlueprintEdit {
    /// Change the workflow's display name.
    Rename {
        /// New display name; the workflow identity stays the same.
        name: String,
    },
    /// Append a model step with an explicit capability and output allowance.
    AddModel {
        /// New distinct step identity.
        step: String,
        /// Exact permitted model capability.
        capability: String,
        /// Fresh instruction for this step.
        prompt: String,
        /// Finite output-unit allowance for each invocation.
        maximum_output_units: u64,
    },
    /// Replace one step's instruction while retaining its output allowance.
    Prompt {
        /// Existing model step identity.
        step: String,
        /// Replacement instruction.
        prompt: String,
    },
    /// Select an exact permitted model capability for one existing step.
    Model {
        /// Existing model step identity.
        step: String,
        /// Replacement model capability identity.
        capability: String,
    },
    /// Declare a required per-run input before connecting it to a step.
    Input {
        /// New workflow interface field name.
        name: String,
    },
    /// Remove an unused declaration. Connected inputs refuse without disconnecting consumers.
    RemoveInput {
        /// Existing workflow input name.
        name: String,
    },
    /// Rename a declaration and only its workflow-input references atomically.
    /// An existing source renamed to itself is unchanged; an absent source still refuses.
    RenameInput {
        /// Existing workflow input name.
        name: String,
        /// Valid distinct destination, or the same name for an unchanged draft.
        new_name: String,
    },
    /// Bind a named step input to one explicit source.
    Connect {
        /// Receiving model step identity.
        step: String,
        /// Nonreserved input port name on the receiving step.
        input: String,
        /// Declared run input or earlier step whose content is selected.
        source: ModelInputSource,
    },
    /// Remove one existing named input connection.
    Disconnect {
        /// Receiving model step identity.
        step: String,
        /// Existing input port whose source binding is removed.
        input: String,
    },
    /// Select one step's final text as the workflow output.
    Output {
        /// Existing model step whose final text is returned.
        step: String,
        /// Workflow output interface field name.
        name: String,
    },
    /// Remove an unused step after disconnecting its consumers and output selection.
    Remove {
        /// Existing step identity to remove.
        step: String,
    },
    /// Reorder a step while retaining its explicit data connections.
    Move {
        /// Existing step identity to move.
        step: String,
        /// Existing step to precede, or `None` to move to the end.
        before: Option<String>,
    },
}
