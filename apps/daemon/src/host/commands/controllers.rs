//! Controller inspection and checkpoint continuation mapping.

use super::super::{
    ActorSession, CommandAccepted, CommandRequest, ControlCommand, ControlResult, NodeExecutionId,
    Owner, PublicFailure, RepeatDecisionId, RunId, RunQueryStore, RunSequence, internal, invalid,
    public_persistence,
};

pub(super) fn inspect(
    owner: &Owner,
    session: &ActorSession,
    request: &CommandRequest,
    run_id: &str,
    controller_execution: &str,
) -> Result<CommandAccepted, PublicFailure> {
    let run = RunId::new(run_id.to_owned()).map_err(|error| invalid(&error.to_string()))?;
    let result = owner.execute_control_result(
        session,
        request,
        request.expected_sequence,
        None,
        ControlCommand::InspectController {
            run,
            controller_execution: NodeExecutionId::new(controller_execution.to_owned())
                .map_err(|error| invalid(&error.to_string()))?,
        },
        "inspect-controller",
    )?;
    let ControlResult::ControllerStatus { value } = result else {
        return Err(internal());
    };
    Ok(CommandAccepted {
        command_id: request.command_id.clone(),
        replayed: false,
        resulting_sequence: value.last_assessment_sequence.map(RunSequence::get),
        result_type: "controller_status".to_owned(),
        value: serde_json::to_value(value).map_err(|_| internal())?,
    })
}

pub(super) fn continue_checkpoint(
    owner: &Owner,
    session: &ActorSession,
    request: &CommandRequest,
    run_id: &str,
    controller_execution: &str,
    decision_id: &str,
) -> Result<CommandAccepted, PublicFailure> {
    let run = RunId::new(run_id.to_owned()).map_err(|error| invalid(&error.to_string()))?;
    let result = owner.execute_control_result(
        session,
        request,
        request.expected_sequence,
        None,
        ControlCommand::ContinueController {
            run,
            controller_execution: NodeExecutionId::new(controller_execution.to_owned())
                .map_err(|error| invalid(&error.to_string()))?,
            decision: RepeatDecisionId::new(decision_id.to_owned())
                .map_err(|error| invalid(&error.to_string()))?,
        },
        "continue-controller",
    )?;
    let ControlResult::ControllerStatus { value } = result else {
        return Err(internal());
    };
    let resulting_sequence = owner
        .store
        .run_summary(&value.run)
        .map_err(public_persistence)?
        .map(|summary| summary.through_sequence.get());
    Ok(CommandAccepted {
        command_id: request.command_id.clone(),
        replayed: false,
        resulting_sequence,
        result_type: "controller_continued".to_owned(),
        value: serde_json::to_value(value).map_err(|_| internal())?,
    })
}
