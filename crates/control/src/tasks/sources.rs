//! Selecting historical outputs uses stored publication metadata and the current editor's scope.

use super::TaskControlService;
use crate::{ActorAuthorityContext, ControlError};
use milkdrift_authority::{
    AuthorityBudget, AuthorityExecutionProvenance, AuthorityOperation, AuthorityRequest,
    BoundaryTimeMillis, DecisionId, RequestedResourceFacts,
};
use milkdrift_blueprint::{TaskBatch, TaskChange, TaskId, TaskInputBinding, WorkflowId};
use milkdrift_capability::ArtifactReference;
use milkdrift_runtime::WorkflowRecord;
use milkdrift_workspace::{ArtifactId, RunId};
use std::collections::{BTreeMap, BTreeSet};

impl TaskControlService {
    pub(super) fn authorize_sources(
        &self,
        context: &ActorAuthorityContext,
        record: &WorkflowRecord,
        batch: &TaskBatch,
    ) -> Result<(), ControlError> {
        let revisions = batch
            .changes
            .iter()
            .filter_map(|change| match change {
                TaskChange::Revise {
                    task, definition, ..
                } => Some((task, definition)),
                _ => None,
            })
            .collect::<BTreeMap<_, _>>();
        let mut sources = BTreeMap::new();
        let mut outcomes = BTreeSet::new();
        for change in &batch.changes {
            match change {
                TaskChange::Add { definition, .. } | TaskChange::Revise { definition, .. } => {
                    for binding in definition.inputs.values() {
                        match binding {
                            TaskInputBinding::TaskOutput { task, output, .. } => {
                                sources.entry((task, output.as_str())).or_insert(false);
                            }
                            TaskInputBinding::SettledOutcome { task } => {
                                outcomes.insert(task);
                            }
                            _ => {}
                        }
                    }
                }
                TaskChange::AdoptCorrection {
                    task,
                    correction,
                    redirects,
                } => {
                    for redirect in redirects {
                        if let Some(TaskInputBinding::TaskOutput { output, .. }) = record
                            .tasks
                            .get(&redirect.consumer)
                            .and_then(|consumer| consumer.definition.inputs.get(&redirect.input))
                            && record.tasks.get(task).is_some_and(|source| {
                                !source.definition.outputs.contains_key(output)
                            })
                        {
                            sources.insert((task, output.as_str()), true);
                            sources.insert((correction, output.as_str()), true);
                        }
                    }
                }
                _ => {}
            }
        }
        for task in outcomes {
            self.authorize_outcome_source(context, record, task)?;
        }
        for ((task, output), required) in sources {
            let Some(producer) = record.tasks.get(task) else {
                // New aliases and absent producers are decided atomically by runtime validation.
                continue;
            };
            let reference = producer
                .result
                .as_ref()
                .and_then(|result| result.outputs.get(output));
            let definition = revisions.get(task).copied().unwrap_or(&producer.definition);
            if reference.is_none() && !required && definition.outputs.contains_key(output) {
                // This batch explicitly introduces the promise on a still-editable producer.
                // Runtime checks its version/start guards and the entire prospective graph.
                continue;
            }
            // Denied scope is resolved before looking for publication metadata or revealing it.
            self.authorize(
                context,
                &record.id,
                AuthorityOperation::InspectRun,
                "connect-output",
            )?;
            let reference = reference.ok_or_else(|| invalid("selected output is unavailable"))?;
            self.authorize_published_output(context, record, reference)?;
        }
        Ok(())
    }

    pub(super) fn authorize_outcome_source(
        &self,
        context: &ActorAuthorityContext,
        record: &WorkflowRecord,
        task: &TaskId,
    ) -> Result<(), ControlError> {
        let Some(result) = record.tasks.get(task).and_then(|task| task.result.as_ref()) else {
            // Future outcomes remain valid; no retained content is being selected yet.
            return Ok(());
        };
        self.authorize(
            context,
            &record.id,
            AuthorityOperation::InspectRun,
            "connect-outcome",
        )?;
        for reference in result.outputs.values().chain(result.terminal.outputs()) {
            self.authorize_published_output(context, record, reference)?;
        }
        Ok(())
    }

    pub(super) fn authorize_published_output(
        &self,
        context: &ActorAuthorityContext,
        record: &WorkflowRecord,
        reference: &ArtifactReference,
    ) -> Result<(), ControlError> {
        let metadata =
            self.authorized_artifact_source(context, &record.id, reference.identity())?;
        let stored = metadata.reference();
        if stored.digest().to_hex() != reference.digest()
            || Some(stored.media_type().as_str()) != reference.media_type()
            || Some(stored.size_bytes()) != reference.size_bytes()
        {
            return Err(invalid(
                "selected output differs from its retained publication",
            ));
        }
        Ok(())
    }

    pub(super) fn authorize_artifact_source(
        &self,
        context: &ActorAuthorityContext,
        workflow: &WorkflowId,
        artifact_id: &str,
    ) -> Result<(), ControlError> {
        self.authorized_artifact_source(context, workflow, artifact_id)
            .map(|_| ())
    }

    fn authorized_artifact_source(
        &self,
        context: &ActorAuthorityContext,
        workflow: &WorkflowId,
        artifact_id: &str,
    ) -> Result<milkdrift_workspace::ArtifactMetadata, ControlError> {
        let artifact =
            ArtifactId::new(artifact_id).map_err(|_| invalid("selected output is unavailable"))?;
        let metadata = self.artifacts.metadata(&artifact)?.ok_or_else(|| {
            ControlError::AuthorizationDenied {
                reasons: vec![milkdrift_authority::DecisionReasonCode::ArtifactScopeMismatch],
                decision_digest: None,
            }
        })?;
        let mut resources = RequestedResourceFacts::empty();
        resources.workflow = Some(workflow.clone());
        resources.run =
            Some(RunId::new(workflow.as_str()).map_err(|error| invalid(error.to_string()))?);
        resources.artifact = Some(artifact);
        resources.artifact_sensitivity = Some(metadata.sensitivity());
        let claim = context.authority();
        for operation in [
            AuthorityOperation::ReadArtifactMetadata,
            AuthorityOperation::ReadArtifactContent,
        ] {
            let identity =
                serde_json::to_vec(&(context.actor(), workflow, artifact_id, operation))?;
            let request = AuthorityRequest {
                decision: DecisionId::new(format!("task_source_{}", blake3::hash(&identity)))?,
                actor: context.actor().clone(),
                grant: claim.grant().clone(),
                grant_revision: claim.grant_revision(),
                grant_digest: claim.grant_digest().clone(),
                revocation_generation: claim.revocation_generation(),
                operation,
                resources: resources.clone(),
                budget: AuthorityBudget {
                    artifact_bytes: Some(metadata.reference().size_bytes()),
                    ..AuthorityBudget::default()
                },
                evaluated_at: BoundaryTimeMillis::new(self.clock.now()?.get()),
                provenance: AuthorityExecutionProvenance::default(),
            };
            let decision = self.authority.evaluate(&request)?;
            if !decision.is_allowed() {
                return Err(ControlError::AuthorizationDenied {
                    reasons: decision.reason_codes().to_vec(),
                    decision_digest: Some(decision.digest().to_owned()),
                });
            }
        }
        Ok(metadata)
    }
}

fn invalid(message: impl Into<String>) -> ControlError {
    ControlError::InvalidContract(message.into())
}
