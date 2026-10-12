//! Shared scoped admission for human, CLI, browser and model-proposed ordinary task changes.

use crate::{ActorAuthorityContext, ControlError};
use milkdrift_authority::{
    AuthorityBudget, AuthorityDecisionSnapshot, AuthorityEvaluator, AuthorityExecutionProvenance,
    AuthorityOperation, AuthorityRequest, BoundaryTimeMillis, DecisionId, RequestedResourceFacts,
};
use milkdrift_blueprint::{TaskBatch, TaskChange, WorkflowCreate, WorkflowId};
use milkdrift_persistence::WorkflowCommandReceipt;
use milkdrift_runtime::{BoundaryClock, TaskRuntime, WorkflowRecord};
use milkdrift_workspace::RunId;
use std::sync::Arc;

/// All ordinary edits enter through the same evaluator and runtime transaction owner.
pub struct TaskControlService {
    runtime: Arc<TaskRuntime>,
    authority: Arc<dyn AuthorityEvaluator>,
    clock: Arc<dyn BoundaryClock>,
}

impl TaskControlService {
    /// Connects scoped command admission to the same runtime used by execution workers.
    #[must_use]
    pub fn new(
        runtime: Arc<TaskRuntime>,
        authority: Arc<dyn AuthorityEvaluator>,
        clock: Arc<dyn BoundaryClock>,
    ) -> Self {
        Self {
            runtime,
            authority,
            clock,
        }
    }

    /// Authorizes initial scope before runtime allocates task identities or accepts a receipt.
    ///
    /// # Errors
    /// Returns grant denials, invalid task definitions, conflicting request reuse or store failure.
    pub fn create(
        &self,
        context: &ActorAuthorityContext,
        key: &str,
        request: WorkflowCreate,
    ) -> Result<WorkflowCommandReceipt, ControlError> {
        self.create_request(context, key, request, None)
    }

    /// Admits mapped task content while binding replay to the original canonical public request.
    ///
    /// # Errors
    /// Returns authority, canonical request, task validation, replay or storage failures.
    pub fn create_exact(
        &self,
        context: &ActorAuthorityContext,
        key: &str,
        request: WorkflowCreate,
        canonical_transport: &[u8],
    ) -> Result<WorkflowCommandReceipt, ControlError> {
        self.create_request(context, key, request, Some(canonical_transport))
    }

    fn create_request(
        &self,
        context: &ActorAuthorityContext,
        key: &str,
        request: WorkflowCreate,
        transport: Option<&[u8]>,
    ) -> Result<WorkflowCommandReceipt, ControlError> {
        let workflow = TaskRuntime::workflow_identity(context.actor().as_str(), key)?;
        self.authorize(context, &workflow, AuthorityOperation::CreateRun, key)?;
        let decision = self.authorize(context, &workflow, AuthorityOperation::StartRun, key)?;
        Ok(match transport {
            Some(bytes) => self.runtime.create_exact(
                context.actor().as_str(),
                key,
                request,
                decision,
                bytes,
            )?,
            None => self
                .runtime
                .create(context.actor().as_str(), key, request, decision)?,
        })
    }

    /// Stop requires cancellation scope; changing future definitions requires scoped edit scope.
    ///
    /// # Errors
    /// Refuses denied scope, stale versions, invalid edits, conflicting keys or store failures.
    pub fn edit(
        &self,
        context: &ActorAuthorityContext,
        key: &str,
        workflow: &WorkflowId,
        batch: TaskBatch,
    ) -> Result<WorkflowCommandReceipt, ControlError> {
        self.edit_request(context, key, workflow, batch, None)
    }

    /// Applies an authorized edit with exact public-envelope replay identity.
    ///
    /// # Errors
    /// Returns authority, canonical request, task edit, replay or storage failures.
    pub fn edit_exact(
        &self,
        context: &ActorAuthorityContext,
        key: &str,
        workflow: &WorkflowId,
        batch: TaskBatch,
        canonical_transport: &[u8],
    ) -> Result<WorkflowCommandReceipt, ControlError> {
        self.edit_request(context, key, workflow, batch, Some(canonical_transport))
    }

    fn edit_request(
        &self,
        context: &ActorAuthorityContext,
        key: &str,
        workflow: &WorkflowId,
        batch: TaskBatch,
        transport: Option<&[u8]>,
    ) -> Result<WorkflowCommandReceipt, ControlError> {
        if batch
            .changes
            .iter()
            .any(|change| matches!(change, TaskChange::Stop { .. }))
        {
            self.authorize(context, workflow, AuthorityOperation::Cancel, key)?;
        }
        if batch
            .changes
            .iter()
            .any(|change| !matches!(change, TaskChange::Stop { .. }))
        {
            self.authorize(context, workflow, AuthorityOperation::Apply, key)?;
        }
        Ok(match transport {
            Some(bytes) => {
                self.runtime
                    .edit_exact(context.actor().as_str(), key, workflow, batch, bytes)?
            }
            None => self
                .runtime
                .edit(context.actor().as_str(), key, workflow, batch)?,
        })
    }

    /// Rechecks current grants for every read, including reconnect and cached stream refresh.
    ///
    /// # Errors
    /// Returns authorization denials and runtime storage or integrity failures.
    pub fn workflow(
        &self,
        context: &ActorAuthorityContext,
        workflow: &WorkflowId,
    ) -> Result<Option<WorkflowRecord>, ControlError> {
        self.authorize(
            context,
            workflow,
            AuthorityOperation::InspectRun,
            "read-workflow",
        )?;
        Ok(self.runtime.workflow(workflow)?)
    }

    /// Shared read/discovery admission. A denial reveals no hidden workflow facts.
    ///
    /// # Errors
    /// Refuses invalid authority coordinates, clock failures, or denied current grants.
    pub fn authorize(
        &self,
        context: &ActorAuthorityContext,
        workflow: &WorkflowId,
        operation: AuthorityOperation,
        request_key: &str,
    ) -> Result<AuthorityDecisionSnapshot, ControlError> {
        let mut resources = RequestedResourceFacts::empty();
        resources.workflow = Some(workflow.clone());
        resources.run = Some(
            RunId::new(workflow.as_str())
                .map_err(|e| ControlError::InvalidContract(e.to_string()))?,
        );
        let claim = context.authority();
        let identity = serde_json::to_vec(&(context.actor(), workflow, operation, request_key))?;
        let request = AuthorityRequest {
            decision: DecisionId::new(format!("task_decision_{}", blake3::hash(&identity)))?,
            actor: context.actor().clone(),
            grant: claim.grant().clone(),
            grant_revision: claim.grant_revision(),
            grant_digest: claim.grant_digest().clone(),
            revocation_generation: claim.revocation_generation(),
            operation,
            resources,
            budget: AuthorityBudget::default(),
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
        Ok(decision)
    }
}
