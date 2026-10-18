//! A storage credential cannot widen the accepted task actor's artifact read scope.

use super::{
    AdapterExecutionContext, ArtifactReference, InvocationDataError, StoreInvocationDataAccess,
};
use milkdrift_authority::{
    AuthorityBudget, AuthorityEvaluator, AuthorityOperation, BoundaryTimeMillis, DecisionId,
    RequestedResourceFacts,
};
use milkdrift_persistence::{ArtifactReadAuthority, EvidenceId};
use milkdrift_runtime::BoundaryClock;
use std::sync::Arc;

pub(super) struct TaskArtifactAuthority {
    pub(super) evaluator: Arc<dyn AuthorityEvaluator>,
    pub(super) clock: Arc<dyn BoundaryClock>,
}

impl StoreInvocationDataAccess {
    pub(super) fn authorize_task_artifact(
        &self,
        context: &AdapterExecutionContext,
        reference: &ArtifactReference,
    ) -> Result<ArtifactReadAuthority, InvocationDataError> {
        let Some(policy) = self
            .task_authority
            .as_ref()
            .filter(|_| context.authority().is_some())
        else {
            return Ok(self.read_authority.clone());
        };
        let metadata = self
            .store
            .metadata(reference.artifact())
            .map_err(|_| denied())?
            .ok_or_else(denied)?;
        let mut request = context
            .entry_authorization()
            .ok_or_else(denied)?
            .request()
            .clone();
        request.resources = RequestedResourceFacts::empty();
        request.resources.artifact = Some(metadata.reference().artifact().clone());
        request.resources.artifact_sensitivity = Some(metadata.sensitivity());
        request.budget = AuthorityBudget {
            artifact_bytes: Some(metadata.reference().size_bytes()),
            ..AuthorityBudget::default()
        };
        request.evaluated_at =
            BoundaryTimeMillis::new(policy.clock.now().map_err(|_| denied())?.get());
        let mut authorized = None;
        for operation in [
            AuthorityOperation::ReadArtifactMetadata,
            AuthorityOperation::ReadArtifactContent,
        ] {
            request.operation = operation;
            let identity =
                serde_json::to_vec(&(context.entry_authorization(), reference, operation))
                    .map_err(|_| denied())?;
            request.decision =
                DecisionId::new(format!("task_artifact_{}", blake3::hash(&identity)))
                    .map_err(|_| denied())?;
            let decision = policy.evaluator.evaluate(&request).map_err(|_| denied())?;
            if !decision.is_allowed() {
                return Err(denied());
            }
            authorized = Some(ArtifactReadAuthority::Authorized {
                actor: request.actor.clone(),
                evidence: EvidenceId::new(format!("task-artifact:{}", decision.digest()))
                    .map_err(|_| denied())?,
            });
        }
        if metadata.reference() != reference {
            return Err(InvocationDataError::Integrity(
                "task artifact differs from its retained publication".into(),
            ));
        }
        authorized.ok_or_else(denied)
    }
}

fn denied() -> InvocationDataError {
    InvocationDataError::Rejected(
        "task artifact read is not authorized by its accepted actor and current grant".into(),
    )
}
