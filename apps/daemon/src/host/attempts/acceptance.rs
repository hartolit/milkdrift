use super::{LocatedAttempt, Owner};
use crate::host::{ActorSession, PublicFailure, read_model::internal, read_model::snake_debug};
use milkdrift_control::{
    RESULT_ACCEPTANCE_OUTPUT, ResultAcceptance, WORKFLOW_ACCEPT_RESULT_OPERATION,
};
use milkdrift_control_protocol::{ErrorCode, ResultAcceptanceRead};

impl Owner {
    pub(super) fn attach_acceptance(
        &mut self,
        session: &ActorSession,
        located: &mut LocatedAttempt,
    ) -> Result<(), PublicFailure> {
        if located.value.capability_id.as_deref() != Some("milkdrift-workflow-control")
            || located
                .value
                .operation_contract
                .as_ref()
                .is_none_or(|contract| contract.operation != WORKFLOW_ACCEPT_RESULT_OPERATION)
        {
            return Ok(());
        }
        let Some(output) = located
            .value
            .outputs
            .iter()
            .find(|output| output.name == RESULT_ACCEPTANCE_OUTPUT)
        else {
            return Ok(());
        };
        // Reports contain at most bounded requirements, fixed reasons, and short diagnostics.
        // Never parse arbitrary output content merely because its name resembles a decision.
        if output.artifact.size > 65_536 {
            return Err(internal());
        }
        let chunk = match self.artifact_range(
            session,
            &output.artifact.artifact_id,
            0,
            65_536,
            "acceptance-inspection",
        ) {
            Ok(chunk) => chunk,
            Err(error) if matches!(error.code, ErrorCode::Unauthorized | ErrorCode::NotFound) => {
                return Ok(());
            }
            Err(error) => return Err(error),
        };
        if !chunk.end || chunk.metadata.digest != output.artifact.digest {
            return Err(internal());
        }
        let result: ResultAcceptance =
            serde_json::from_slice(&chunk.bytes).map_err(|_| internal())?;
        if result.schema_version != milkdrift_control::RESULT_ACCEPTANCE_SCHEMA_VERSION {
            return Err(internal());
        }
        located.value.result_acceptance = Some(ResultAcceptanceRead {
            accepted: result.accepted,
            reason: snake_debug(&result.reason),
            requirement: result
                .requirement
                .map(serde_json::to_value)
                .transpose()
                .map_err(|_| internal())?,
            finish_reason: result.model.map(|model| snake_debug(&model.finish_reason)),
            checkpoint: result.checkpoint,
        });
        Ok(())
    }
}
