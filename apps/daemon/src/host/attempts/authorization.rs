//! Run-scope authority filtering shared by run, node, attempt, and stream reads.

use super::Owner;
use crate::host::{
    ActorSession, AuthorityDecisionSnapshot, AuthorityOperation, PublicFailure,
    RequestedResourceFacts, RunId, RunQueryStore, WorkflowRunScope, invalid, not_found,
    public_persistence, unauthorized,
};

impl Owner {
    pub(in crate::host) fn authorize_run_read(
        &self,
        session: &ActorSession,
        run: &str,
        operation: AuthorityOperation,
        boundary: &str,
    ) -> Result<AuthorityDecisionSnapshot, PublicFailure> {
        let run = RunId::new(run.to_owned()).map_err(|error| invalid(&error.to_string()))?;
        let mut resources = RequestedResourceFacts::empty();
        resources.run = Some(run.clone());
        match &session.grant.resources().workflow_run {
            WorkflowRunScope::Any => {}
            WorkflowRunScope::Workflow { workflow } => {
                resources.workflow = Some(workflow.clone());
            }
            WorkflowRunScope::Run {
                run: allowed,
                workflow,
            } => {
                if allowed != &run {
                    return Err(unauthorized());
                }
                resources.workflow = workflow.clone();
            }
        }
        let decision = self.authorize(session, operation, resources.clone(), boundary)?;
        let summary = self
            .store
            .run_summary(&run)
            .map_err(public_persistence)?
            .ok_or_else(not_found)?;
        if resources
            .workflow
            .as_ref()
            .is_some_and(|workflow| workflow != &summary.workflow)
        {
            return Err(not_found());
        }
        Ok(decision)
    }
}
