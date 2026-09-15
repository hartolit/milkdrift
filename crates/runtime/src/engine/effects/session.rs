//! Compare model session intent at the last runtime boundary that owns both the
//! governing revision and the frozen request, including recovered and retry requests.

use milkdrift_authority::{
    AuthorityBudget, AuthorityExecutionProvenance, AuthorityOperation, BoundaryTimeMillis,
    DecisionId, ExecutionAuthorityBasis, RequestedResourceFacts,
};
use milkdrift_blueprint::{ContextSessionPolicy, NodeId, NodeKind, RevisionId};
use milkdrift_capability::{InvocationRequest, InvocationValueReference};
use milkdrift_model::{MODEL_TASK_INPUT_NAME, ModelTaskRequestDocument, SessionSelection};
use milkdrift_persistence::{ArtifactReadAuthority, EvidenceId, TimestampMillis};
use milkdrift_workspace::{ArtifactId, ArtifactSensitivity};

use super::RuntimeService;
use crate::RuntimeError;

impl RuntimeService {
    pub(super) fn validate_task_session(
        &self,
        revision: &RevisionId,
        node: &NodeId,
        request: &InvocationRequest,
        category: Option<&milkdrift_capability::CapabilityCategory>,
        basis: &ExecutionAuthorityBasis,
        now: TimestampMillis,
    ) -> Result<(), RuntimeError> {
        let revision = self.load_validated_revision(revision, None)?;
        let Some(NodeKind::Task { config }) = revision
            .semantic()
            .nodes()
            .get(node)
            .map(|node| node.kind())
        else {
            return Ok(());
        };
        if request.operation().as_str() != milkdrift_model::MODEL_GENERATE_OPERATION
            || category.is_some_and(|category| {
                category != &milkdrift_capability::CapabilityCategory::Model
            })
        {
            return if config.context_policy().session() == ContextSessionPolicy::Fresh {
                Ok(())
            } else {
                Err(invalid(
                    "non-model capabilities support only Fresh execution with explicitly selected durable evidence",
                ))
            };
        }
        let Some(input) = request
            .inputs()
            .iter()
            .find(|input| input.name() == MODEL_TASK_INPUT_NAME)
        else {
            // Only the provider-neutral request declares SessionSelection. The chosen
            // adapter still owns its required input/schema checks before external entry.
            return Ok(());
        };
        let bytes = match input.value() {
            InvocationValueReference::Inline { value } => serde_json::to_vec(value.value())?,
            InvocationValueReference::Artifact { reference } => {
                let artifact = ArtifactId::new(reference.identity())
                    .map_err(|_| invalid("model task artifact identity is invalid"))?;
                let metadata = self
                    .store
                    .metadata(&artifact)?
                    .ok_or_else(|| invalid("model task artifact is unavailable"))?;
                if reference.digest() != metadata.reference().digest().to_hex()
                    || reference.size_bytes() != Some(metadata.reference().size_bytes())
                    || reference.media_type() != Some(metadata.reference().media_type().as_str())
                {
                    return Err(invalid(
                        "model task artifact contradicts its immutable reference",
                    ));
                }
                // Direct inputs travel even when omitted from context selection. Loading a
                // request for validation must therefore authorize its artifact independently.
                if metadata.sensitivity() != ArtifactSensitivity::Public {
                    let mut resources = RequestedResourceFacts::empty();
                    resources.revision = Some(revision.id().clone());
                    resources.artifact = Some(artifact);
                    resources.artifact_sensitivity = Some(metadata.sensitivity());
                    let digest =
                        blake3::hash(format!("{}:model-session", request.invocation()).as_bytes());
                    let decision = self.authority.evaluate(&basis.request(
                        DecisionId::new(format!("decision:{digest}"))?,
                        AuthorityOperation::ReadArtifactContent,
                        resources,
                        AuthorityBudget {
                            artifact_bytes: reference.size_bytes(),
                            ..AuthorityBudget::default()
                        },
                        BoundaryTimeMillis::new(now.get()),
                        AuthorityExecutionProvenance {
                            revision: Some(revision.id().clone()),
                            node: Some(node.clone()),
                            ..AuthorityExecutionProvenance::default()
                        },
                    ))?;
                    if !decision.is_allowed() {
                        return Err(invalid("model task artifact read was denied"));
                    }
                }
                crate::context::read_model_document_bytes(
                    self.store.as_ref(),
                    reference,
                    ArtifactReadAuthority::Authorized {
                        actor: basis.actor().clone(),
                        evidence: EvidenceId::new(format!(
                            "model-session:{}",
                            request.invocation()
                        ))?,
                    },
                )
                .map_err(|_| {
                    invalid("model task artifact cannot be read within its exact document bound")
                })?
            }
            InvocationValueReference::WorkspaceValue { .. } => {
                return Err(invalid(
                    "model task must be inline or an immutable artifact",
                ));
            }
        };
        let task = ModelTaskRequestDocument::from_json(&bytes)
            .map_err(|_| invalid("model task contract is malformed"))?;
        let agrees = matches!(
            (config.context_policy().session(), task.body().session()),
            (ContextSessionPolicy::Fresh, SessionSelection::Fresh)
                | (
                    ContextSessionPolicy::ExplicitContinuation,
                    SessionSelection::ExplicitContinuation { .. }
                )
                | (
                    ContextSessionPolicy::ProviderManaged,
                    SessionSelection::ProviderManaged { .. }
                )
        );
        if !agrees {
            return Err(invalid(
                "model request session contradicts the governing task context policy",
            ));
        }
        if matches!(
            task.body().session(),
            SessionSelection::ExplicitContinuation { .. }
        ) {
            let reference = request
                .context_manifest()
                .ok_or_else(|| invalid("continuation manifest is absent"))?;
            let manifest = crate::read_context_manifest(
                self.store.as_ref(),
                reference,
                ArtifactReadAuthority::Authorized {
                    actor: basis.actor().clone(),
                    evidence: EvidenceId::new(format!("continuation:{}", request.invocation()))?,
                },
            )
            .map_err(|error| invalid(&error.to_string()))?;
            let projection = self.projection(manifest.run())?;
            let execution = projection
                .node_executions()
                .get(manifest.execution())
                .ok_or_else(|| invalid("continuation execution is absent"))?;
            crate::DurableContextCandidateSource::new(self.store.as_ref(), self.authority.as_ref())
                .check_continuation(
                    &crate::ContextSourceRequest {
                        identity: crate::ContextBuildIdentity {
                            run: manifest.run().clone(),
                            revision: revision.id().clone(),
                            node: node.clone(),
                            execution: manifest.execution().clone(),
                            attempt: manifest.attempt().clone(),
                        },
                        revision: &revision,
                        policy: config.context_policy(),
                        scope: execution.scope(),
                        direct_inputs: request.inputs(),
                        required_direct_inputs: &std::collections::BTreeSet::new(),
                        through_sequence: projection.sequence(),
                        projection: &projection,
                        authority: basis,
                        evaluated_at_ms: now.get(),
                    },
                    &manifest,
                )
                .map_err(|error| invalid(&error.to_string()))?;
        }
        Ok(())
    }
}

fn invalid(reason: &str) -> RuntimeError {
    RuntimeError::Scheduling(reason.to_owned())
}
