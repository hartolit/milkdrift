//! Saved unfinished work is a mutable proposal, separate from accepted task definitions.
//!
//! Connections retain the person's intent even before a model or port is chosen. Shared control
//! resolves that intent into ordinary definitions only for a deliberately reviewed Run scope.

use std::collections::{BTreeMap, BTreeSet};

use milkdrift_capability::{CapabilityId, CapabilityRequirement, OperationId};
use serde::{Deserialize, Serialize};

use crate::{
    TaskActionPolicy, TaskChange, TaskDefinition, TaskDefinitionError, TaskExecution, TaskId,
    TaskInputBinding, TaskInputValue, TaskOutputDeclaration, TaskPathAccess, TaskTool,
};

/// Initial operational envelope for one latest saved sketch.
pub const MAX_AUTHORING_DOCUMENT_BYTES: usize = 2 * 1024 * 1024;
/// Unfinished task cards, including proposed edits of accepted tasks.
pub const MAX_AUTHORING_TASKS: usize = 200;
/// Independently arranged text and captured-file sources.
pub const MAX_AUTHORING_SOURCES: usize = 400;
/// Data and ordering intentions, including unresolved endpoints.
pub const MAX_AUTHORING_CONNECTIONS: usize = 1024;

/// A chosen executor can remain visible when unavailable; saving does not select a replacement.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuthoringCapability {
    /// Configured capability identity; empty while the card is undefined.
    pub id: String,
    /// Configured operation; empty while the card is undefined.
    pub operation: String,
}

impl AuthoringCapability {
    /// Constructs the selected executable requirement when Run needs a complete choice.
    /// # Errors
    /// Refuses missing or malformed choices; never substitutes an available capability.
    pub fn requirement(&self) -> Result<CapabilityRequirement, TaskDefinitionError> {
        Ok(
            CapabilityRequirement::new(OperationId::new(&self.operation).map_err(invalid)?)
                .exact(CapabilityId::new(&self.id).map_err(invalid)?),
        )
    }
}

/// A deliberately saved usual choice for newly placed cards.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuthoringModel {
    /// Exact configured choice.
    pub capability: AuthoringCapability,
    /// Contract generation reviewed when this preference was chosen.
    pub generation: u64,
}

/// Editable fields may be incomplete. Their presence is never evidence of task admission.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuthoringDefinition {
    /// Human-editable title, possibly empty before instructions are entered.
    pub title: String,
    /// Optional explanation.
    pub description: String,
    /// Instructions, possibly empty.
    pub instructions: String,
    /// Explicit choice or an empty, unassigned choice.
    pub capability: AuthoringCapability,
    /// Transport-compatible field; candidates must project bindings into connections before Run.
    pub inputs: BTreeMap<String, TaskInputBinding>,
    /// Accepted output promises retained for prospective edits.
    pub outputs: BTreeMap<String, TaskOutputDeclaration>,
    /// Transport-compatible field; Run requires explicit ordering connections instead.
    pub dependencies: crate::TaskDependencies,
    /// Whether success is a workflow obligation.
    pub required: bool,
    /// Earlier task addressed by future work.
    pub corrective_of: Option<TaskId>,
    /// Explicit native access choice.
    pub paths: Vec<TaskPathAccess>,
    /// Explicit bounded tools.
    pub tools: Vec<TaskTool>,
    /// Permission to propose automatic future work.
    pub actions: Option<TaskActionPolicy>,
    /// Reviewed project setup or explicit restrictions.
    pub execution: TaskExecution,
}

impl AuthoringDefinition {
    /// Converts complete fields without supplying missing instructions, models or authority.
    /// Shared control subsequently constructs wires and runtime validates the complete task set.
    /// # Errors
    /// Refuses a missing or malformed capability selection.
    pub fn task_definition(&self) -> Result<TaskDefinition, TaskDefinitionError> {
        Ok(TaskDefinition {
            capability_generation: None,
            title: self.title.clone(),
            description: self.description.clone(),
            instructions: self.instructions.clone(),
            capability: self.capability.requirement()?,
            inputs: self.inputs.clone(),
            outputs: self.outputs.clone(),
            dependencies: self.dependencies.clone(),
            required: self.required,
            corrective_of: self.corrective_of.clone(),
            paths: self.paths.clone(),
            tools: self.tools.clone(),
            actions: self.actions.clone(),
            execution: self.execution.clone(),
        })
    }
}

/// One incomplete new task or an unapplied replacement of an accepted future task.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuthoringTask {
    /// Stable card identity; an accepted task ID when version is present.
    pub id: String,
    /// Accepted version being edited, or none for a new card.
    pub version: Option<u64>,
    /// Candidate fields; never a second copy of an admitted task after Run.
    pub definition: AuthoringDefinition,
    /// Whether the person replaced the suggested title.
    pub title_edited: bool,
    /// Adapter contract reviewed while choosing this executor.
    pub contract_generation: Option<u64>,
}

/// Captured content or an intentionally missing supplied value.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuthoringSource {
    /// Stable source-card identity.
    pub id: String,
    /// Readable source label.
    pub label: String,
    /// Captured bytes; none retains an unfinished input intention.
    pub value: Option<TaskInputValue>,
    /// Workflow input name, including a new name deliberately awaiting a later supplied value.
    pub shared_name: Option<String>,
}

/// Data readiness and independent ordering are different meanings.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthoringConnectionKind {
    /// A successful named result or supplied content feeds a real input.
    Data,
    /// Independent ordering requires successful completion.
    AfterSuccess,
    /// Independent ordering requires proven settlement, including failure.
    AfterCompletion,
}

/// A physical wire may retain unfinished endpoints without pretending it can execute.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuthoringConnection {
    /// Stable wire identity for local diagnostics.
    pub id: String,
    /// Source card or accepted task identity.
    pub source: String,
    /// Receiving card or accepted task identity.
    pub target: String,
    /// Real input key, or an unresolved input choice.
    pub input: Option<String>,
    /// Real named result, or an unresolved output choice.
    pub output: Option<String>,
    /// Data or deliberately independent ordering.
    pub kind: AuthoringConnectionKind,
    /// Deliberately accepts settled absence.
    pub optional: bool,
    /// Deliberately consumes a settled outcome including failure.
    pub outcome: bool,
}

/// One latest bounded sketch. Accepted definitions remain exclusively in workflow history.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuthoringDocument {
    /// Pending new cards and unapplied edits.
    pub tasks: Vec<AuthoringTask>,
    /// Pending or reusable captured sources.
    pub sources: Vec<AuthoringSource>,
    /// Intended wiring and placement references to accepted topology, including unresolved choices.
    pub connections: Vec<AuthoringConnection>,
    /// Other explicit prospective operations, excluding add/revise and direct controls.
    pub changes: Vec<TaskChange>,
    /// Deliberately chosen preference; changes never retroactively replace selected models.
    pub usual_model: Option<AuthoringModel>,
    /// Daemon-maintained references from admitted card identities to accepted task identities.
    pub accepted: BTreeMap<String, String>,
}

impl AuthoringDocument {
    /// Validates finite storage and structural intent without requiring executable content.
    /// # Errors
    /// Refuses oversized, duplicate, malformed or unsupported authoring content.
    pub fn validate(&self) -> Result<(), TaskDefinitionError> {
        if self.tasks.len() > MAX_AUTHORING_TASKS
            || self.sources.len() > MAX_AUTHORING_SOURCES
            || self.connections.len() > MAX_AUTHORING_CONNECTIONS
            || self.changes.len() > 200
            || self.accepted.len() > 600
            || serde_json::to_vec(self).map_err(invalid)?.len() > MAX_AUTHORING_DOCUMENT_BYTES
        {
            return Err(invalid(
                "Saved work exceeds its finite envelope. Remove unused cards or shorten content.",
            ));
        }
        let mut ids = BTreeSet::new();
        for task in &self.tasks {
            check_id(&task.id)?;
            if !ids.insert(&task.id) {
                return Err(invalid("Card identities must be unique"));
            }
            let definition = &task.definition;
            if definition.title.len() > 256
                || definition.description.len() > 4096
                || definition.instructions.len() > crate::MAX_TASK_TEXT_BYTES
                || definition.capability.id.len() > 256
                || definition.capability.operation.len() > 256
                || definition.inputs.len() > 64
                || definition.outputs.len() > 64
                || definition.paths.len() > 32
                || definition.tools.len() > 32
                || task.version == Some(0)
                || task.contract_generation == Some(0)
            {
                return Err(invalid("A task card exceeds its content bounds"));
            }
        }
        for source in &self.sources {
            check_id(&source.id)?;
            if !ids.insert(&source.id)
                || source.label.len() > 256
                || source
                    .shared_name
                    .as_ref()
                    .is_some_and(|name| name.is_empty() || name.len() > 256)
            {
                return Err(invalid("Source identities or labels are invalid"));
            }
            if let Some(value) = &source.value {
                value.validate()?;
            }
        }
        let mut wires = BTreeSet::new();
        for wire in &self.connections {
            for id in [&wire.id, &wire.source, &wire.target] {
                check_id(id)?;
            }
            if !wires.insert(&wire.id)
                || wire
                    .input
                    .as_ref()
                    .is_some_and(|name| name.is_empty() || name.len() > 256)
                || wire
                    .output
                    .as_ref()
                    .is_some_and(|name| name.is_empty() || name.len() > 256)
                || (wire.kind != AuthoringConnectionKind::Data && (wire.optional || wire.outcome))
            {
                return Err(invalid("Connection identities or choices are invalid"));
            }
        }
        for (alias, task) in &self.accepted {
            check_id(alias)?;
            check_id(task)?;
        }
        if self.changes.iter().any(|change| {
            matches!(
                change,
                TaskChange::Add { .. }
                    | TaskChange::Revise { .. }
                    | TaskChange::Stop { .. }
                    | TaskChange::Wait { .. }
                    | TaskChange::Continue
            )
        }) {
            return Err(invalid(
                "Task cards own candidate definitions; pause, resume and stop are direct commands",
            ));
        }
        if let Some(model) = &self.usual_model {
            model.capability.requirement()?;
            if model.generation == 0 {
                return Err(invalid("Choose a current model generation"));
            }
        }
        Ok(())
    }
}

fn check_id(id: &str) -> Result<(), TaskDefinitionError> {
    TaskId::new(id).map(|_| ()).map_err(invalid)
}

fn invalid(error: impl std::fmt::Display) -> TaskDefinitionError {
    TaskDefinitionError(error.to_string())
}
