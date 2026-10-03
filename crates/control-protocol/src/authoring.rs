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
#[allow(missing_docs)]
pub enum BlueprintEdit {
    Rename {
        name: String,
    },
    AddModel {
        step: String,
        capability: String,
        prompt: String,
        maximum_output_units: u64,
    },
    Prompt {
        step: String,
        prompt: String,
    },
    Model {
        step: String,
        capability: String,
    },
    Input {
        name: String,
    },
    Connect {
        step: String,
        input: String,
        source: ModelInputSource,
    },
    Disconnect {
        step: String,
        input: String,
    },
    Output {
        step: String,
        name: String,
    },
    Remove {
        step: String,
    },
    Move {
        step: String,
        before: Option<String>,
    },
}
