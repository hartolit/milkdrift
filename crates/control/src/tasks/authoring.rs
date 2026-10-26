//! Conditional sketches preserve unfinished work; Run compiles only a deliberately reviewed scope.
mod construction;

use super::TaskControlService;
use crate::{ActorAuthorityContext, ControlError};
use milkdrift_authority::AuthorityOperation;
use milkdrift_blueprint::authoring::AuthoringDocument;
use milkdrift_blueprint::{TaskBatch, TaskInputBinding, TaskInputValue, WorkflowId};
use milkdrift_persistence::{
    WorkflowAuthoringCommit, WorkflowAuthoringSnapshot, WorkflowCommandReceipt,
    WorkflowPositionsCommit, WorkflowPositionsSnapshot, WorkflowStore,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

/// A named admission scope independent of the client's inspector selection.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuthoringScope {
    /// Candidate card identities.
    pub tasks: Vec<String>,
    /// Explicit saved change indices.
    pub changes: Vec<u32>,
    /// Candidate predecessors deliberately used with their currently accepted definition.
    pub use_current: Vec<String>,
}

/// A bounded repair tied to the authorized sketch, never a hidden resource identity.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuthoringIssue {
    /// Stable category for contextual presentation.
    pub code: String,
    /// Human explanation of the necessary repair.
    pub message: String,
    /// Affected candidate.
    pub task_id: Option<String>,
    /// Affected visible connection.
    pub connection_id: Option<String>,
    /// Affected visible source.
    pub source_id: Option<String>,
    /// Affected saved change.
    pub change_index: Option<u32>,
}

impl AuthoringIssue {
    fn task(code: &str, message: impl Into<String>, task: Option<&str>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            task_id: task.map(str::to_owned),
            connection_id: None,
            source_id: None,
            change_index: None,
        }
    }
}

/// Full review adds required new predecessors visibly but performs no admission.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuthoringReview {
    /// Exact sketch generation inspected.
    pub authoring_generation: u64,
    /// Semantic workflow observation for the resulting Run.
    pub expected_generation: u64,
    /// Whole scope requiring a deliberate submission.
    pub tasks: Vec<String>,
    /// Explicit selected changes.
    pub changes: Vec<u32>,
    /// Explicit choices to use accepted predecessors unchanged.
    pub use_current: Vec<String>,
    /// At most 64 local repairs; later repairs follow after these are addressed.
    pub issues: Vec<AuthoringIssue>,
}

impl TaskControlService {
    fn authoring_store(&self) -> Result<&dyn WorkflowStore, ControlError> {
        self.authoring_store
            .as_deref()
            .ok_or_else(|| invalid("Saved authoring is not configured"))
    }

    /// Reads the current sketch under current workflow, project and source authority.
    /// # Errors
    /// Refuses denied scope, unavailable sources, missing workflows or corrupt content.
    pub fn authoring(
        &self,
        context: &ActorAuthorityContext,
        workflow: &WorkflowId,
    ) -> Result<WorkflowAuthoringSnapshot, ControlError> {
        self.authorize(
            context,
            workflow,
            AuthorityOperation::InspectRun,
            "read-authoring",
        )?;
        let snapshot = self
            .authoring_store()?
            .workflow_authoring(workflow)?
            .ok_or_else(|| invalid("Workflow is unavailable"))?;
        self.authorize_authoring_artifacts(context, workflow, &snapshot.protected_sources)?;
        self.authorize_authoring_projects(context, &decode_document(&snapshot)?)?;
        Ok(snapshot)
    }

    /// Replaces only the latest sketch, without receipts, task admission or runtime transitions.
    /// # Errors
    /// Refuses stale content, invalid bounds, invented mappings, source denial and capacity.
    pub fn checkpoint_authoring(
        &self,
        context: &ActorAuthorityContext,
        workflow: &WorkflowId,
        expected_generation: u64,
        document: AuthoringDocument,
    ) -> Result<WorkflowAuthoringSnapshot, ControlError> {
        self.authorize(
            context,
            workflow,
            AuthorityOperation::Apply,
            "save-authoring",
        )?;
        self.authorize(
            context,
            workflow,
            AuthorityOperation::InspectRun,
            "save-authoring",
        )?;
        // Removing now-denied sources or project bindings must remain possible. Inspect only
        // server-owned mapping facts here; authorize the replacement before committing it.
        let current = self
            .authoring_store()?
            .workflow_authoring(workflow)?
            .ok_or_else(|| invalid("Workflow is unavailable"))?;
        let prior = decode_document(&current)?;
        document
            .validate()
            .map_err(|error| invalid(error.to_string()))?;
        if document
            .accepted
            .iter()
            .any(|(alias, task)| prior.accepted.get(alias) != Some(task))
        {
            return Err(invalid(
                "Only Run can create or retarget accepted task references",
            ));
        }
        let record = self
            .runtime
            .workflow(workflow)?
            .ok_or_else(|| invalid("Workflow is unavailable"))?;
        for task in &document.tasks {
            if task.version.is_some() && !record.tasks.keys().any(|id| id.as_str() == task.id) {
                return Err(invalid(
                    "A candidate edit must name an existing workflow task",
                ));
            }
            if task.version.is_none()
                && (prior.accepted.contains_key(&task.id)
                    || prior.accepted.values().any(|id| id == &task.id)
                    || record.tasks.keys().any(|id| id.as_str() == task.id))
            {
                return Err(invalid(
                    "A new card cannot reuse an accepted card or task identity",
                ));
            }
        }
        if document.sources.iter().any(|source| {
            prior.accepted.contains_key(&source.id)
                || prior.accepted.values().any(|id| id == &source.id)
                || record.tasks.keys().any(|id| id.as_str() == source.id)
        }) {
            return Err(invalid(
                "A source card cannot reuse an accepted card or task identity",
            ));
        }
        let protected_sources = protected_sources(&document, &record);
        self.authorize_authoring_artifacts(context, workflow, &protected_sources)?;
        self.authorize_authoring_projects(context, &document)?;
        Ok(self.authoring_store()?.checkpoint_workflow_authoring(
            workflow,
            WorkflowAuthoringCommit {
                expected_generation,
                document: serde_json::to_value(document)?,
                protected_sources,
            },
        )?)
    }

    /// Reads separately versioned presentation positions.
    /// # Errors
    /// Refuses current read authority or unavailable workflow/storage.
    pub fn authoring_positions(
        &self,
        context: &ActorAuthorityContext,
        workflow: &WorkflowId,
    ) -> Result<WorkflowPositionsSnapshot, ControlError> {
        self.authorize(
            context,
            workflow,
            AuthorityOperation::InspectRun,
            "read-positions",
        )?;
        self.authoring_store()?
            .workflow_positions(workflow)?
            .ok_or_else(|| invalid("Workflow is unavailable"))
    }

    /// Saves layout without changing authoring or execution generations.
    /// # Errors
    /// Refuses current edit authority, stale layout, invalid positions or capacity.
    pub fn checkpoint_positions(
        &self,
        context: &ActorAuthorityContext,
        workflow: &WorkflowId,
        commit: WorkflowPositionsCommit,
    ) -> Result<WorkflowPositionsSnapshot, ControlError> {
        self.authorize(
            context,
            workflow,
            AuthorityOperation::Apply,
            "save-positions",
        )?;
        Ok(self
            .authoring_store()?
            .checkpoint_workflow_positions(workflow, commit)?)
    }

    /// Calculates visible necessary predecessors and local repairs without scheduling anything.
    /// # Errors
    /// Refuses current authority, stale saved generation or unavailable/corrupt sources.
    pub fn review_authoring(
        &self,
        context: &ActorAuthorityContext,
        workflow: &WorkflowId,
        generation: u64,
        scope: &AuthoringScope,
    ) -> Result<AuthoringReview, ControlError> {
        let snapshot = self.authoring(context, workflow)?;
        check_generation(&snapshot, generation)?;
        let document = decode_document(&snapshot)?;
        let record = self
            .runtime
            .workflow(workflow)?
            .ok_or_else(|| invalid("Workflow is unavailable"))?;
        Ok(self
            .compile_authoring(context, &record, &document, generation, scope)?
            .0)
    }

    /// Admits exactly the reviewed scope and updates its mapping in the same durable receipt commit.
    /// # Errors
    /// Refuses changed authoring, unreviewed predecessors, incomplete work, semantic conflicts,
    /// authority loss or capacity. Exact replay precedes all current authoring resolution.
    #[expect(
        clippy::too_many_arguments,
        reason = "Run binds actor, workflow, exact request and reviewed scope to independent workflow and authoring generations"
    )]
    pub fn run_authoring(
        &self,
        context: &ActorAuthorityContext,
        workflow: &WorkflowId,
        key: &str,
        expected_generation: u64,
        generation: u64,
        scope: &AuthoringScope,
        canonical_transport: &[u8],
    ) -> Result<WorkflowCommandReceipt, ControlError> {
        if let Some(receipt) =
            self.replay_exact(context, key, Some(workflow), canonical_transport)?
        {
            return Ok(receipt);
        }
        self.authorize(context, workflow, AuthorityOperation::Apply, key)?;
        let snapshot = self.authoring(context, workflow)?;
        check_generation(&snapshot, generation)?;
        let mut document = decode_document(&snapshot)?;
        let record = self
            .runtime
            .workflow(workflow)?
            .ok_or_else(|| invalid("Workflow is unavailable"))?;
        let (mut review, changes) =
            self.compile_authoring(context, &record, &document, generation, scope)?;
        if review.issues.len() < 64
            && review.tasks.iter().collect::<BTreeSet<_>>()
                != scope.tasks.iter().collect::<BTreeSet<_>>()
        {
            review.issues.push(AuthoringIssue::task(
                "missing_predecessor",
                "Review the required new predecessors before running this scope",
                None,
            ));
        }
        if !review.issues.is_empty() {
            return Err(ControlError::AuthoringIssues(review.issues));
        }
        let mut batch = TaskBatch {
            expected_generation,
            changes,
        };
        self.authorize_sources(context, &record, &batch)?;
        self.resolve_project_batch(context, &record, &mut batch, false)?;
        preserve_connection_references(&mut document, &batch);
        document
            .tasks
            .retain(|task| !scope.tasks.contains(&task.id));
        document.changes = document
            .changes
            .into_iter()
            .enumerate()
            .filter_map(|(index, change)| {
                u32::try_from(index)
                    .ok()
                    .filter(|index| !scope.changes.contains(index))
                    .map(|_| change)
            })
            .collect();
        // Accepted-only wires preserve source-card placement. They are presentation references:
        // clients must match them against current topology, and compilation ignores their targets.
        let protected_sources = protected_sources(&document, &record);
        Ok(self.runtime.edit_authoring_exact(
            context.actor().as_str(),
            key,
            workflow,
            batch,
            canonical_transport,
            WorkflowAuthoringCommit {
                expected_generation: generation,
                document: serde_json::to_value(document)?,
                protected_sources,
            },
        )?)
    }

    fn authorize_authoring_projects(
        &self,
        context: &ActorAuthorityContext,
        document: &AuthoringDocument,
    ) -> Result<(), ControlError> {
        for task in &document.tasks {
            // Incomplete model or instruction fields do not weaken the authority needed to
            // disclose an already captured project binding retained in an editable candidate.
            if let Some(binding) = task.definition.execution.binding() {
                self.authorize_project_binding(context, binding)?;
            }
        }
        Ok(())
    }

    fn authorize_authoring_artifacts(
        &self,
        context: &ActorAuthorityContext,
        workflow: &WorkflowId,
        sources: &[String],
    ) -> Result<(), ControlError> {
        for source in sources {
            self.authorize_artifact_source(context, workflow, source)?;
        }
        Ok(())
    }
}

fn context_input_name(wire: &str) -> String {
    let name = format!("context_{wire}");
    if name.len() <= 128
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.'))
    {
        name
    } else {
        // Wire identities allow more characters and bytes than executable input names. Keep
        // ordinary names stable; hashed names use a separate prefix from preserved names.
        format!("context-hash_{}", blake3::hash(wire.as_bytes()))
    }
}

fn preserve_connection_references(document: &mut AuthoringDocument, batch: &TaskBatch) {
    for wire in &mut document.connections {
        if wire.kind != milkdrift_blueprint::authoring::AuthoringConnectionKind::Data {
            continue;
        }
        let definition = batch.changes.iter().find_map(|change| match change {
            milkdrift_blueprint::TaskChange::Add { alias, definition }
                if alias.as_str() == wire.target =>
            {
                Some(definition)
            }
            milkdrift_blueprint::TaskChange::Revise {
                task, definition, ..
            } if task.as_str() == wire.target => Some(definition),
            _ => None,
        });
        let Some(definition) = definition else {
            continue;
        };
        // Unresolved input selection is admitted only for one actual slot or generated context.
        if wire.input.is_none() {
            let context = context_input_name(&wire.id);
            wire.input = if definition.inputs.contains_key(&context) {
                Some(context)
            } else if definition.inputs.len() == 1 {
                definition.inputs.keys().next().cloned()
            } else {
                None
            };
        }
        if let Some(TaskInputBinding::TaskOutput { output, .. }) = wire
            .input
            .as_ref()
            .and_then(|input| definition.inputs.get(input))
        {
            wire.output = Some(output.clone());
        }
    }
}

fn check_generation(
    snapshot: &WorkflowAuthoringSnapshot,
    expected: u64,
) -> Result<(), ControlError> {
    if snapshot.generation != expected {
        return Err(
            milkdrift_persistence::PersistenceError::AuthoringGenerationConflict {
                workflow: snapshot.workflow.clone(),
                expected,
                actual: snapshot.generation,
            }
            .into(),
        );
    }
    Ok(())
}

fn decode_document(
    snapshot: &WorkflowAuthoringSnapshot,
) -> Result<AuthoringDocument, ControlError> {
    if snapshot.document.is_null() {
        Ok(AuthoringDocument::default())
    } else {
        Ok(serde_json::from_value(snapshot.document.clone())?)
    }
}

fn protected_sources(
    document: &AuthoringDocument,
    record: &milkdrift_runtime::WorkflowRecord,
) -> Vec<String> {
    let mut sources = BTreeSet::new();
    let mut capture = |value: &TaskInputValue| match value {
        TaskInputValue::TextFile {
            source_artifact: Some(id),
            ..
        } => {
            sources.insert(id.clone());
        }
        TaskInputValue::Artifact { reference } => {
            sources.insert(reference.identity().to_owned());
        }
        _ => {}
    };
    for source in &document.sources {
        if let Some(value) = &source.value {
            capture(value);
        }
        if let Some(value) = source
            .shared_name
            .as_ref()
            .and_then(|name| record.inputs.get(name))
        {
            capture(value);
        }
    }
    for task in &document.tasks {
        for binding in task.definition.inputs.values() {
            if let TaskInputBinding::Inline { value } = binding {
                capture(value);
            }
        }
    }
    for change in &document.changes {
        if let milkdrift_blueprint::TaskChange::BindInput { value, .. } = change {
            capture(value);
        }
    }
    for wire in &document.connections {
        let source = document.accepted.get(&wire.source).unwrap_or(&wire.source);
        if let Some(result) = record
            .tasks
            .iter()
            .find(|(id, _)| id.as_str() == source)
            .and_then(|(_, task)| task.result.as_ref())
        {
            if wire.outcome {
                sources.extend(
                    result
                        .outputs
                        .values()
                        .chain(result.terminal.outputs())
                        .map(|reference| reference.identity().to_owned()),
                );
            } else if let Some(reference) = wire
                .output
                .as_ref()
                .and_then(|name| result.outputs.get(name))
            {
                sources.insert(reference.identity().to_owned());
            }
        }
    }
    sources.into_iter().collect()
}

fn invalid(message: impl Into<String>) -> ControlError {
    ControlError::InvalidContract(message.into())
}
