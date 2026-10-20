//! Read immutable responses and route proposals through the same checked future-work command.
use super::TaskControlService;
use crate::{ActorAuthorityContext, ControlError};
use milkdrift_authority::{
    AuthorityBudget, AuthorityExecutionProvenance, AuthorityOperation, BoundaryTimeMillis,
    DecisionId, RequestedResourceFacts,
};
use milkdrift_blueprint::{TaskBatch, TaskChange, TaskId, TaskInputBinding, TaskInputValue};
use milkdrift_persistence::{
    ArtifactReadAuthority, ArtifactReadRequest, ArtifactStore, EvidenceId, PersistenceError,
    StorageFailureClass, WorkflowStore,
};
use milkdrift_runtime::{CommandAuthorityClaim, ModelAdmissionStatus, WorkflowRecord};

impl TaskControlService {
    /// Processes a bounded page before daemon scheduling. A saved response survives failure at
    /// every boundary; admission and its source linkage share the runtime's receipt transaction.
    ///
    /// # Errors
    /// Returns unavailable/corrupt storage failures. Invalid proposals become retained decisions.
    pub fn process_model_admissions(
        &self,
        workflows: &dyn WorkflowStore,
    ) -> Result<(), ControlError> {
        let mut cursor = self
            .admission_cursor
            .lock()
            .map_err(|_| invalid("model admission cursor unavailable"))?;
        let page = workflows.workflows(cursor.as_deref(), 16)?;
        for snapshot in &page.items {
            let record = self
                .runtime
                .workflow(&snapshot.workflow)?
                .ok_or_else(|| invalid("admission workflow disappeared"))?;
            let sources: Vec<_> = record
                .admissions
                .iter()
                .filter(|(source, admission)| {
                    record
                        .tasks
                        .get(*source)
                        .is_some_and(|task| task.state.is_settled())
                        && (admission.status == ModelAdmissionStatus::Pending
                            || admission.status == ModelAdmissionStatus::Rejected
                                && admission.repair_allowed
                                && !has_distinct_wait(&record, source))
                })
                .take(32)
                .map(|(id, _)| id.clone())
                .collect();
            for source in sources {
                let record = self
                    .runtime
                    .workflow(&snapshot.workflow)?
                    .ok_or_else(|| invalid("admission workflow disappeared"))?;
                self.process_response(&record, &source, self.artifacts.as_ref())?;
            }
        }
        *cursor = page.next;
        Ok(())
    }

    fn process_response(
        &self,
        record: &WorkflowRecord,
        source: &TaskId,
        artifacts: &dyn ArtifactStore,
    ) -> Result<(), ControlError> {
        let admission = record
            .admissions
            .get(source)
            .ok_or_else(|| invalid("model source disappeared"))?;
        let task = record
            .tasks
            .get(source)
            .ok_or_else(|| invalid("model source task disappeared"))?;
        if !task.state.is_settled() {
            return Ok(());
        }
        if task.state != milkdrift_runtime::TaskState::Completed {
            self.runtime.reject_model_batch(
                &record.id,
                source,
                "source response is retained but its task did not complete successfully",
                false,
            )?;
            return Ok(());
        }
        let basis = &record.execution_authority;
        let context = ActorAuthorityContext::new(
            basis.actor().clone(),
            CommandAuthorityClaim::new(
                basis.grant().clone(),
                basis.grant_revision(),
                basis.grant_digest().clone(),
                basis.revocation_generation(),
            )?,
        );
        if admission.status == ModelAdmissionStatus::Rejected {
            if has_distinct_wait(record, source) {
                return Ok(());
            }
            let result = Self::feedback_batch(record, source).and_then(|mut batch| {
                self.authorize_batch(&context, record, &batch)?;
                let original = serde_json::to_vec(&serde_json::to_value(&batch)?)?;
                self.resolve_project_batch(&context, record, &mut batch, true)?;
                Ok(self.runtime.admit_model_batch_exact(
                    context.actor().as_str(),
                    &record.id,
                    source,
                    batch,
                    true,
                    &original,
                )?)
            });
            if let Err(error) = result {
                // Feedback uses the same exact receipt boundary as the original proposal.
                // An unavailable store is not proof that a repair was refused or uncommitted.
                if persistence_error(&error).is_some() && !is_definite_storage_refusal(&error) {
                    return Err(error);
                }
                // An explicit wait may win after the read above but before checked admission.
                // Keep repair eligible until continuation instead of turning that race into refusal.
                let current = self
                    .runtime
                    .workflow(&record.id)?
                    .ok_or_else(|| invalid("feedback workflow disappeared"))?;
                if has_distinct_wait(&current, source) {
                    return Ok(());
                }
                self.runtime
                    .refuse_model_repair(&record.id, source, &error.to_string())?;
            }
            return Ok(());
        }
        let result = self
            .read_batch(record, source, artifacts)
            .and_then(|mut batch| {
                self.authorize_batch(&context, record, &batch)?;
                let original = serde_json::to_vec(&serde_json::to_value(&batch)?)?;
                self.resolve_project_batch(&context, record, &mut batch, false)?;
                Ok(self.runtime.admit_model_batch_exact(
                    context.actor().as_str(),
                    &record.id,
                    source,
                    batch,
                    false,
                    &original,
                )?)
            });
        if let Err(error) = result {
            // Storage outages are retried from Pending. A durable admission may have committed
            // despite a lost response, so never convert an indeterminate store result to rejection.
            if persistence_error(&error).is_some() && !is_definite_storage_refusal(&error) {
                return Err(error);
            }
            let repair = !is_definite_storage_refusal(&error)
                && !matches!(
                    error,
                    ControlError::AuthorizationDenied { .. }
                        | ControlError::Authority(_)
                        | ControlError::Runtime(milkdrift_runtime::RuntimeError::Executor(
                            milkdrift_runtime::ExecutorError::AuthorityDenied { .. }
                        ))
                )
                && record
                    .tasks
                    .get(source)
                    .and_then(|task| task.definition.actions.as_ref())
                    .is_some_and(|policy| policy.automatic_repair);
            self.runtime
                .reject_model_batch(&record.id, source, &error.to_string(), repair)?;
        }
        Ok(())
    }

    fn authorize_batch(
        &self,
        context: &ActorAuthorityContext,
        record: &WorkflowRecord,
        batch: &TaskBatch,
    ) -> Result<(), ControlError> {
        self.authorize(
            context,
            &record.id,
            AuthorityOperation::Apply,
            "model-actions",
        )?;
        if batch
            .changes
            .iter()
            .any(|change| matches!(change, TaskChange::Stop { .. }))
        {
            self.authorize(
                context,
                &record.id,
                AuthorityOperation::Cancel,
                "model-actions",
            )?;
        }
        self.authorize_sources(context, record, batch)?;
        Ok(())
    }

    fn read_batch(
        &self,
        record: &WorkflowRecord,
        source: &TaskId,
        artifacts: &dyn ArtifactStore,
    ) -> Result<TaskBatch, ControlError> {
        let reference = record
            .admissions
            .get(source)
            .and_then(|admission| admission.response.as_ref())
            .ok_or_else(|| {
                invalid("action-enabled task did not publish a canonical model_response")
            })?;
        let artifact = milkdrift_workspace::ArtifactId::new(reference.identity())
            .map_err(|error| invalid(error.to_string()))?;
        let metadata = artifacts
            .metadata(&artifact)?
            .ok_or_else(|| invalid("saved model response is unavailable"))?;
        let mut resources = RequestedResourceFacts::empty();
        resources.artifact = Some(artifact);
        resources.artifact_sensitivity = Some(metadata.sensitivity());
        let request = record.execution_authority.request(
            DecisionId::new(format!("model_read_{source}"))?,
            AuthorityOperation::ReadArtifactContent,
            resources,
            AuthorityBudget::default(),
            BoundaryTimeMillis::new(self.clock.now()?.get()),
            AuthorityExecutionProvenance::default(),
        );
        let decision = self.authority.evaluate(&request)?;
        if !decision.is_allowed() {
            return Err(ControlError::AuthorizationDenied {
                reasons: decision.reason_codes().to_vec(),
                decision_digest: Some(decision.digest().into()),
            });
        }
        if reference.size_bytes().is_none_or(|bytes| bytes > 2_097_152) {
            return Err(invalid("model response exceeds the bounded reader"));
        }
        let authority = ArtifactReadAuthority::Authorized {
            actor: record.execution_authority.actor().clone(),
            evidence: EvidenceId::new(format!("model-read-{}", decision.digest()))?,
        };
        let mut bytes = Vec::new();
        loop {
            let offset = u64::try_from(bytes.len()).map_err(|error| invalid(error.to_string()))?;
            let chunk = artifacts.read_chunk(&ArtifactReadRequest::new(
                metadata.reference().clone(),
                offset,
                262_144,
                authority.clone(),
            )?)?;
            if bytes
                .len()
                .checked_add(chunk.bytes.len())
                .is_none_or(|size| size > 2_097_152)
                || chunk.bytes.is_empty() && !chunk.end_of_artifact
            {
                return Err(invalid(
                    "model response chunk exceeded its bound or made no progress",
                ));
            }
            bytes.extend(chunk.bytes);
            if chunk.end_of_artifact {
                break;
            }
        }
        if u64::try_from(bytes.len()).ok() != reference.size_bytes()
            || blake3::hash(&bytes).to_hex().as_str() != reference.digest()
        {
            return Err(invalid(
                "retained model response differs from its immutable reference",
            ));
        }
        let response = milkdrift_model::ModelResponseDocument::from_json(&bytes)
            .map_err(|error| invalid(error.to_string()))?;
        let structured = response.body().structured().ok_or_else(|| invalid("model response has no structured TaskBatch; result text is never interpreted as actions"))?;
        Ok(serde_json::from_value(structured.value().clone())?)
    }

    fn feedback_batch(record: &WorkflowRecord, source: &TaskId) -> Result<TaskBatch, ControlError> {
        let admission = record
            .admissions
            .get(source)
            .ok_or_else(|| invalid("feedback source missing"))?;
        let original = record
            .tasks
            .get(source)
            .ok_or_else(|| invalid("feedback task missing"))?;
        let mut definition = original.definition.clone();
        definition.title = format!(
            "Repair actions: {}",
            milkdrift_contracts::truncate_utf8(&original.definition.title, 220)
        );
        definition.description = format!("Feedback for retained model response from {source}");
        definition.corrective_of = None;
        definition.required = false;
        definition.inputs = original
            .frozen
            .as_ref()
            .map(|start| {
                start
                    .inputs
                    .iter()
                    .map(|(name, value)| {
                        (
                            name.clone(),
                            TaskInputBinding::Inline {
                                value: value.clone(),
                            },
                        )
                    })
                    .collect()
            })
            .unwrap_or_default();
        definition.dependencies.clear();
        definition.instructions = format!(
            "Repair the rejected structured action batch. Preserve the original intent within the same authority. Use the refreshed snapshot supplied with this task. Do not repeat started work. Rejection: {}\nOriginal instructions: {}",
            admission.reason.as_deref().unwrap_or("invalid actions"),
            original.definition.instructions
        );
        if let Some(response) = &admission.response {
            if definition.inputs.len() >= 64 {
                return Err(invalid(
                    "automatic feedback cannot add its response to 64 frozen inputs",
                ));
            }
            let name = (0..=64)
                .map(|index| {
                    if index == 0 {
                        "rejected_response".to_owned()
                    } else {
                        format!("rejected_response_{index}")
                    }
                })
                .find(|name| !definition.inputs.contains_key(name))
                .ok_or_else(|| invalid("automatic feedback response input namespace exhausted"))?;
            definition.instructions.push_str(&format!(
                "\nThe immutable rejected response is supplied as input {name}."
            ));
            definition.inputs.insert(
                name,
                TaskInputBinding::Inline {
                    value: TaskInputValue::Artifact {
                        reference: response.clone(),
                    },
                },
            );
        }
        Ok(TaskBatch {
            expected_generation: record.generation,
            changes: vec![TaskChange::Add {
                alias: TaskId::new("feedback").map_err(|error| invalid(error.to_string()))?,
                definition,
            }],
        })
    }
}

fn invalid(message: impl Into<String>) -> ControlError {
    ControlError::InvalidContract(message.into())
}

fn has_distinct_wait(record: &WorkflowRecord, source: &TaskId) -> bool {
    record.wait_question.is_some() && record.model_admission_wait.as_ref() != Some(source)
}

fn persistence_error(error: &ControlError) -> Option<&PersistenceError> {
    match error {
        ControlError::Persistence(error)
        | ControlError::Runtime(milkdrift_runtime::RuntimeError::Persistence(error)) => Some(error),
        _ => None,
    }
}

fn is_definite_storage_refusal(error: &ControlError) -> bool {
    // Capacity and stale project authority cannot improve by generating another task.
    // These are known transaction refusals; source history remains accepted and unchanged.
    matches!(
        persistence_error(error),
        Some(
            PersistenceError::Bounds { .. }
                | PersistenceError::ProjectGenerationConflict { .. }
                | PersistenceError::Storage {
                    class: StorageFailureClass::ResourceExhausted,
                    ..
                }
        )
    )
}
