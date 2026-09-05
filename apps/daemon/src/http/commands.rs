//! Commands transport adaptation over typed daemon calls.
use super::{ApiError, AppState, authenticate, owner_error, protocol_error, success};
use axum::{body::Bytes, extract::State, http::HeaderMap, http::StatusCode, response::Response};
use milkdrift_control_protocol::{Command, CommandRequest, ErrorCode, decode_json};
use std::time::Instant;
use tracing::{info, warn};

struct CommandTrace {
    operation: &'static str,
    run: Option<String>,
    revision: Option<String>,
    attempt: Option<String>,
    proposal: Option<String>,
}

pub(super) async fn command(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, ApiError> {
    let (request_id, session) = authenticate(&state, &headers)?;
    if !state.host.accepting_mutations() {
        return Err(ApiError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            ErrorCode::Unavailable,
            "daemon is draining and refuses new mutations",
            true,
            Some(request_id),
        ));
    }
    let request: CommandRequest =
        decode_json(&body).map_err(|error| protocol_error(error, request_id.clone()))?;
    request
        .validate()
        .map_err(|error| protocol_error(error, request_id.clone()))?;
    let actor = session.actor.as_str().to_owned();
    let command_id = request.command_id.clone();
    let trace = command_trace(&request.command);
    let started = Instant::now();
    let result = state.host.command(session, request).await;
    match result {
        Ok(value) => {
            info!(
                request_id,
                command_id,
                actor,
                operation = trace.operation,
                run = ?trace.run,
                revision = ?trace.revision,
                attempt = ?trace.attempt,
                proposal = ?trace.proposal,
                latency_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
                outcome = "accepted",
                result_type = %value.result_type,
                replayed = value.replayed,
                "control command completed"
            );
            success(request_id, value)
        }
        Err(error) => {
            warn!(
                request_id,
                command_id,
                actor,
                operation = trace.operation,
                run = ?trace.run,
                revision = ?trace.revision,
                attempt = ?trace.attempt,
                proposal = ?trace.proposal,
                latency_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
                outcome = "rejected",
                code = ?error.code,
                overload = error.code == ErrorCode::Overload,
                "control command failed"
            );
            Err(owner_error(error, request_id))
        }
    }
}

fn command_trace(command: &Command) -> CommandTrace {
    let (operation, run, revision, attempt, proposal) = match command {
        Command::ImportBlueprint { .. } => ("import_blueprint", None, None, None, None),
        Command::ValidateBlueprint { .. } => ("validate_blueprint", None, None, None, None),
        Command::ImportPromptSequence { .. } => ("import_prompt_sequence", None, None, None, None),
        Command::ValidatePromptSequence { .. } => {
            ("validate_prompt_sequence", None, None, None, None)
        }
        Command::StartRun {
            run_id,
            revision_id,
            ..
        } => (
            "start_run",
            Some(run_id.as_str()),
            Some(revision_id.as_str()),
            None,
            None,
        ),
        Command::PauseRun { run_id } => ("pause_run", Some(run_id.as_str()), None, None, None),
        Command::ResumeRun { run_id } => ("resume_run", Some(run_id.as_str()), None, None, None),
        Command::CancelRun { run_id } => ("cancel_run", Some(run_id.as_str()), None, None, None),
        Command::SignalRun { run_id, .. } => {
            ("signal_run", Some(run_id.as_str()), None, None, None)
        }
        Command::ResolveWork {
            run_id, attempt_id, ..
        } => (
            "resolve_work",
            Some(run_id.as_str()),
            None,
            Some(attempt_id.as_str()),
            None,
        ),
        Command::InspectController { run_id, .. } => (
            "inspect_controller",
            Some(run_id.as_str()),
            None,
            None,
            None,
        ),
        Command::ContinueController { run_id, .. } => (
            "continue_controller",
            Some(run_id.as_str()),
            None,
            None,
            None,
        ),
        Command::SubmitProposal { .. } => ("submit_proposal", None, None, None, None),
        Command::DecideProposal {
            run_id,
            proposal_id,
            proposed_revision,
            ..
        } => (
            "decide_proposal",
            Some(run_id.as_str()),
            Some(proposed_revision.as_str()),
            None,
            Some(proposal_id.as_str()),
        ),
        Command::ApplyProposal {
            run_id,
            proposal_id,
            proposed_revision,
            ..
        } => (
            "apply_proposal",
            Some(run_id.as_str()),
            Some(proposed_revision.as_str()),
            None,
            Some(proposal_id.as_str()),
        ),
        Command::PutLayout { layout } => (
            "put_layout",
            None,
            Some(layout.revision_id.as_str()),
            None,
            None,
        ),
    };
    CommandTrace {
        operation,
        run: run.map(str::to_owned),
        revision: revision.map(str::to_owned),
        attempt: attempt.map(str::to_owned),
        proposal: proposal.map(str::to_owned),
    }
}
