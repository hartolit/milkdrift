//! Selecting historical outputs uses stored publication metadata and the current editor's scope.

use super::TaskControlService;
use crate::{ActorAuthorityContext, ControlError};
use milkdrift_authority::{
    AuthorityBudget, AuthorityExecutionProvenance, AuthorityOperation, AuthorityRequest,
    BoundaryTimeMillis, DecisionId, RequestedResourceFacts,
};
use milkdrift_blueprint::{TaskBatch, TaskChange, TaskId, TaskInputBinding};
use milkdrift_capability::ArtifactReference;
use milkdrift_runtime::WorkflowRecord;
use milkdrift_workspace::{ArtifactId, RunId};
use std::collections::BTreeMap;

impl TaskControlService {
    pub(super) fn authorize_sources(
        &self,
        context: &ActorAuthorityContext,
        record: &WorkflowRecord,
        batch: &TaskBatch,
    ) -> Result<(), ControlError> {
        let mut sources = BTreeMap::new();
        for change in &batch.changes {
            match change {
                TaskChange::Add { definition, .. } | TaskChange::Revise { definition, .. } => {
                    for binding in definition.inputs.values() {
                        if let TaskInputBinding::TaskOutput { task, output, .. } = binding {
                            sources.entry((task, output.as_str())).or_insert(false);
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
        for ((task, output), required) in sources {
            let Some(producer) = record.tasks.get(task) else {
                // New aliases and absent producers are decided atomically by runtime validation.
                continue;
            };
            if !required && producer.definition.outputs.contains_key(output) {
                continue;
            }
            // Denied scope is resolved before looking for publication metadata or revealing it.
            self.authorize(
                context,
                &record.id,
                AuthorityOperation::InspectRun,
                "connect-output",
            )?;
            let reference = producer
                .result
                .as_ref()
                .and_then(|result| result.outputs.get(output))
                .ok_or_else(|| invalid("selected output is unavailable"))?;
            self.authorize_published_output(context, record, task, output, reference)?;
        }
        Ok(())
    }

    fn authorize_published_output(
        &self,
        context: &ActorAuthorityContext,
        record: &WorkflowRecord,
        task: &TaskId,
        output: &str,
        reference: &ArtifactReference,
    ) -> Result<(), ControlError> {
        let artifact = ArtifactId::new(reference.identity())
            .map_err(|_| invalid("selected output is unavailable"))?;
        let metadata = self.artifacts.metadata(&artifact)?.ok_or_else(|| {
            ControlError::AuthorizationDenied {
                reasons: vec![milkdrift_authority::DecisionReasonCode::ArtifactScopeMismatch],
                decision_digest: None,
            }
        })?;
        let mut resources = RequestedResourceFacts::empty();
        resources.workflow = Some(record.id.clone());
        resources.run =
            Some(RunId::new(record.id.as_str()).map_err(|error| invalid(error.to_string()))?);
        resources.artifact = Some(artifact);
        resources.artifact_sensitivity = Some(metadata.sensitivity());
        let claim = context.authority();
        for operation in [
            AuthorityOperation::ReadArtifactMetadata,
            AuthorityOperation::ReadArtifactContent,
        ] {
            let identity =
                serde_json::to_vec(&(context.actor(), &record.id, task, output, operation))?;
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
}

fn invalid(message: impl Into<String>) -> ControlError {
    ControlError::InvalidContract(message.into())
}
