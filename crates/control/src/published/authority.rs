//! Input reads and future service actions use current policy, while replay retains its original
//! canonical commands. A caller's invoke grant never becomes the internal execution grant.
use super::{PublishedWorkflowService, failure, rejected};
use milkdrift_authority::{
    AuthorityBudget, AuthorityDecisionSnapshot, AuthorityOperation, AuthorityRequest,
    BoundaryTimeMillis, RequestedResourceFacts,
};
use milkdrift_capability::{InputReference, InvocationValueReference};
use milkdrift_persistence::published::PublishedInvocationPlan;
use milkdrift_runtime::{CommandAuthorityClaim, ExecutorError, RunCommandDocument};
use milkdrift_workspace::{ArtifactId, WorkspaceValue, WorkspaceValueReference};

impl PublishedWorkflowService {
    pub(super) fn current_decision(
        &self,
        mut request: AuthorityRequest,
    ) -> Result<AuthorityDecisionSnapshot, ExecutorError> {
        request.evaluated_at = BoundaryTimeMillis::new(self.clock.now().map_err(failure)?.get());
        let decision = self.authority.evaluate(&request).map_err(failure)?;
        if !decision.is_allowed() {
            return Err(rejected(
                "current authority refused the published service action",
            ));
        }
        Ok(decision)
    }

    pub(super) fn authorize_local_input(
        &self,
        caller: &AuthorityDecisionSnapshot,
        input: &InputReference,
    ) -> Result<(), ExecutorError> {
        let artifact = match input.value() {
            InvocationValueReference::Inline { .. } => None,
            InvocationValueReference::Artifact { reference } => {
                Some(ArtifactId::new(reference.identity()).map_err(failure)?)
            }
            InvocationValueReference::WorkspaceValue { identity, version } => {
                let reference: WorkspaceValueReference =
                    serde_json::from_str(identity).map_err(failure)?;
                if &reference.version().get().to_string() != version {
                    return Err(rejected(
                        "input workspace version differs from its reference",
                    ));
                }
                let mut read = caller.request().clone();
                read.operation = AuthorityOperation::ReadWorkspaceValue;
                read.resources.workspace_scope = Some(reference.scope().scope().clone());
                self.current_decision(read)?;
                let entry = self
                    .store
                    .value(&reference)
                    .map_err(failure)?
                    .ok_or_else(|| rejected("selected input is absent"))?;
                match entry.value() {
                    WorkspaceValue::Json(_) => None,
                    WorkspaceValue::Artifact(reference) => Some(reference.artifact().clone()),
                }
            }
        };
        if let Some(artifact) = artifact {
            self.authorize_artifact(caller.request().clone(), &artifact)?;
        }
        Ok(())
    }

    pub(super) fn authorize_artifact(
        &self,
        mut request: AuthorityRequest,
        artifact: &ArtifactId,
    ) -> Result<(), ExecutorError> {
        let metadata = self
            .store
            .metadata(artifact)
            .map_err(failure)?
            .ok_or_else(|| rejected("selected artifact metadata is unavailable"))?;
        request.resources = RequestedResourceFacts::empty();
        request.resources.artifact = Some(artifact.clone());
        request.resources.artifact_sensitivity = Some(metadata.sensitivity());
        request.budget = AuthorityBudget {
            artifact_bytes: Some(metadata.reference().size_bytes()),
            ..AuthorityBudget::default()
        };
        for operation in [
            AuthorityOperation::ReadArtifactMetadata,
            AuthorityOperation::ReadArtifactContent,
        ] {
            request.operation = operation;
            self.current_decision(request.clone())?;
        }
        Ok(())
    }

    pub(super) fn service_command_request(
        &self,
        plan: &PublishedInvocationPlan,
        command: &RunCommandDocument,
    ) -> Result<AuthorityRequest, ExecutorError> {
        let claim = CommandAuthorityClaim::new(
            plan.service.grant.clone(),
            plan.service.grant_revision,
            plan.service.grant_digest.clone(),
            plan.service.revocation_generation,
        )
        .map_err(failure)?;
        let mut request = command.authority_request(&claim).map_err(failure)?;
        let create =
            RunCommandDocument::from_json(plan.create_command.as_bytes()).map_err(failure)?;
        let milkdrift_runtime::RunCommand::CreateRun { workflow, .. } = create.command() else {
            return Err(rejected("published create association is invalid"));
        };
        request.resources.workflow = Some(workflow.clone());
        Ok(request)
    }

    pub(super) fn authorize_future_start(
        &self,
        plan: &PublishedInvocationPlan,
    ) -> Result<(), ExecutorError> {
        for text in [&plan.create_command, &plan.start_command] {
            let command = RunCommandDocument::from_json(text.as_bytes()).map_err(failure)?;
            if self
                .store
                .command_result(&plan.child_run, command.command_id())
                .map_err(failure)?
                .is_none()
            {
                self.current_decision(self.service_command_request(plan, &command)?)?;
            }
        }
        Ok(())
    }
}
