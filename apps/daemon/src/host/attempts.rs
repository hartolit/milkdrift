//! Find exact attempt evidence after it has left the compact run frontier.
//!
//! Run and node views use current projections. Attempt reads first try that projection, then
//! reconstruct historical evidence and attach separately authorized context. Historical reads
//! bound memory but can scan substantial journal history.

use super::{
    Owner, PublicFailure, read_model::internal, read_model::invalid, read_model::not_found,
    read_model::public_run,
};
use crate::auth::ActorSession;
use milkdrift_authority::AuthorityOperation;
use milkdrift_control::{ControlCommand, ControlResult};
use milkdrift_control_protocol::{AttemptRead, ErrorCode, NodeRead, RunRead};
use milkdrift_workspace::RunId;

mod acceptance;
mod authorization;
mod context;
mod history;
mod model;
mod projection;

struct LocatedAttempt {
    node_id: String,
    revision_id: String,
    value: AttemptRead,
}

impl Owner {
    pub(super) fn run_read(
        &self,
        session: &ActorSession,
        run: &str,
    ) -> Result<RunRead, PublicFailure> {
        let run_id = RunId::new(run.to_owned()).map_err(|error| invalid(&error.to_string()))?;
        self.authorize_run_read(session, run, AuthorityOperation::InspectRun, "read:run")?;
        let result = match self.inspect_control(
            session,
            ControlCommand::InspectRun { run: run_id },
            None,
            "run",
        ) {
            Err(error) if error.code == ErrorCode::Unauthorized => return Err(not_found()),
            result => result?,
        };
        let ControlResult::RunInspection { value } = result else {
            return Err(internal());
        };
        Ok(public_run(value))
    }

    pub(super) fn node_read(
        &self,
        session: &ActorSession,
        run: &str,
        execution: &str,
    ) -> Result<NodeRead, PublicFailure> {
        self.authorize_run_read(
            session,
            run,
            AuthorityOperation::InspectNodeExecution,
            "read:node-execution",
        )?;
        self.run_read(session, run)?
            .nodes
            .into_iter()
            .find(|node| node.execution_id == execution)
            .ok_or_else(not_found)
    }

    pub(super) fn attempt_read(
        &mut self,
        session: &ActorSession,
        run: &str,
        attempt: &str,
    ) -> Result<AttemptRead, PublicFailure> {
        self.authorize_run_read(
            session,
            run,
            AuthorityOperation::InspectAttempt,
            "read:attempt",
        )?;
        let mut located = match self.current_attempt_read(session, run, attempt)? {
            Some(mut current) => {
                if let Ok(historical) = self.historical_attempt_read(run, attempt) {
                    current.value.peer_id = historical.value.peer_id;
                }
                current
            }
            None => self.historical_attempt_read(run, attempt)?,
        };
        self.attach_context(session, attempt, &mut located)?;
        self.attach_acceptance(session, &mut located)?;
        self.attach_model_generation(session, &mut located)?;
        Ok(located.value)
    }
}
