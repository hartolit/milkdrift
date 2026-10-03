//! Run lifecycle, signal, and retained-work command planning.

use serde_json::Value;

use super::super::{
    ActorSession, AttemptId, CommandAccepted, CommandRequest, ControlCommand, CorrelationKey,
    Owner, PublicFailure, ReconciliationDecisionId, RunId, ScopeId, SignalDeliveryMode, SignalId,
    SignalTypeId, WorkflowId, WorkspaceScope, accepted_sequence, default_workspace_budget, invalid,
    map_resolve, parse_revision_id,
};

pub(super) fn start(
    owner: &mut Owner,
    session: &ActorSession,
    request: &CommandRequest,
    run_id: &str,
    workflow_id: &str,
    revision_id: &str,
    inputs: &[milkdrift_control_protocol::RunInput],
) -> Result<CommandAccepted, PublicFailure> {
    let (run, create_sequence) = create(
        owner,
        session,
        request,
        run_id,
        workflow_id,
        revision_id,
        inputs,
    )?;
    let sequence = owner.execute_control(
        session,
        request,
        Some(create_sequence),
        ControlCommand::StartRun { run },
        "start",
    )?;
    accepted_sequence(request, sequence, "run_started")
}

fn create(
    owner: &mut Owner,
    session: &ActorSession,
    request: &CommandRequest,
    run_id: &str,
    workflow_id: &str,
    revision_id: &str,
    inputs: &[milkdrift_control_protocol::RunInput],
) -> Result<(RunId, u64), PublicFailure> {
    if request
        .expected_sequence
        .is_some_and(|sequence| sequence != 0)
        || request
            .expected_revision
            .as_deref()
            .is_some_and(|guard| guard != revision_id)
    {
        return Err(invalid(
            "start requires sequence zero and the requested revision",
        ));
    }
    let run = RunId::new(run_id.to_owned()).map_err(|error| invalid(&error.to_string()))?;
    let workflow =
        WorkflowId::new(workflow_id.to_owned()).map_err(|error| invalid(&error.to_string()))?;
    let revision = parse_revision_id(revision_id)?;
    let root_scope = WorkspaceScope::run_root(
        run.clone(),
        ScopeId::new("root").map_err(|error| invalid(&error.to_string()))?,
    );
    let defaults = default_workspace_budget().map_err(|error| invalid(&error.to_string()))?;
    let artifact_bytes = session
        .grant
        .budget()
        .artifact_bytes
        .unwrap_or(0)
        .min(defaults.max_total_artifact_bytes());
    let workspace_budget = milkdrift_workspace::WorkspaceBudget::new(
        defaults.max_value_versions(),
        defaults.max_inline_bytes_per_value(),
        defaults.max_total_inline_bytes(),
        defaults.max_artifacts(),
        defaults.max_bytes_per_artifact().min(artifact_bytes),
        artifact_bytes,
    )
    .map_err(|error| invalid(&error.to_string()))?;
    let inputs = super::inputs::resolve(
        owner,
        session,
        revision_id,
        &root_scope,
        &workspace_budget,
        inputs,
    )?;
    let create_sequence = owner.execute_control(
        session,
        request,
        Some(0),
        ControlCommand::CreateRun {
            run: run.clone(),
            workflow,
            revision,
            root_scope,
            workspace_budget,
            inputs,
        },
        "create",
    )?;
    Ok((run, create_sequence))
}

pub(super) fn pause(
    owner: &Owner,
    session: &ActorSession,
    request: &CommandRequest,
    run_id: &str,
) -> Result<CommandAccepted, PublicFailure> {
    simple(owner, session, request, run_id, "pause", |run| {
        ControlCommand::PauseRun { run }
    })
}

pub(super) fn resume(
    owner: &Owner,
    session: &ActorSession,
    request: &CommandRequest,
    run_id: &str,
) -> Result<CommandAccepted, PublicFailure> {
    simple(owner, session, request, run_id, "resume", |run| {
        ControlCommand::ResumeRun { run }
    })
}

pub(super) fn cancel(
    owner: &Owner,
    session: &ActorSession,
    request: &CommandRequest,
    run_id: &str,
) -> Result<CommandAccepted, PublicFailure> {
    simple(owner, session, request, run_id, "cancel", |run| {
        ControlCommand::RequestCancellation { run }
    })
}

pub(super) struct SignalArguments<'a> {
    pub(super) run_id: &'a str,
    pub(super) signal_id: &'a str,
    pub(super) signal_type: &'a str,
    pub(super) correlation: Option<&'a str>,
    pub(super) broadcast: bool,
    pub(super) payload: &'a Value,
}

pub(super) fn signal(
    owner: &Owner,
    session: &ActorSession,
    request: &CommandRequest,
    arguments: SignalArguments<'_>,
) -> Result<CommandAccepted, PublicFailure> {
    let run =
        RunId::new(arguments.run_id.to_owned()).map_err(|error| invalid(&error.to_string()))?;
    let command = ControlCommand::Signal {
        run,
        signal: SignalId::new(arguments.signal_id.to_owned())
            .map_err(|error| invalid(&error.to_string()))?,
        signal_type: SignalTypeId::new(arguments.signal_type.to_owned())
            .map_err(|error| invalid(&error.to_string()))?,
        correlation: arguments
            .correlation
            .map(|value| CorrelationKey::new(value.to_owned()))
            .transpose()
            .map_err(|error| invalid(&error.to_string()))?,
        mode: if arguments.broadcast {
            SignalDeliveryMode::Broadcast
        } else {
            SignalDeliveryMode::OneShot
        },
        payload: milkdrift_capability::BoundedJson::new(arguments.payload.clone())
            .map_err(|error| invalid(&error.to_string()))?,
    };
    let sequence = owner.execute_control(
        session,
        request,
        request.expected_sequence,
        command,
        "signal",
    )?;
    accepted_sequence(request, sequence, "signal_delivered")
}

pub(super) struct ResolveArguments<'a> {
    pub(super) run_id: &'a str,
    pub(super) attempt_id: &'a str,
    pub(super) decision_id: &'a str,
    pub(super) action: milkdrift_control_protocol::ResolveAction,
    pub(super) remediation_node: Option<&'a str>,
}

pub(super) fn resolve(
    owner: &Owner,
    session: &ActorSession,
    request: &CommandRequest,
    arguments: ResolveArguments<'_>,
) -> Result<CommandAccepted, PublicFailure> {
    let run =
        RunId::new(arguments.run_id.to_owned()).map_err(|error| invalid(&error.to_string()))?;
    let command = ControlCommand::ResolveExternalWork {
        run,
        attempt: AttemptId::new(arguments.attempt_id.to_owned())
            .map_err(|error| invalid(&error.to_string()))?,
        decision: ReconciliationDecisionId::new(arguments.decision_id.to_owned())
            .map_err(|error| invalid(&error.to_string()))?,
        action: map_resolve(arguments.action),
        remediation_node: arguments
            .remediation_node
            .map(|value| milkdrift_blueprint::NodeId::new(value.to_owned()))
            .transpose()
            .map_err(|error| invalid(&error.to_string()))?,
    };
    let sequence = owner.execute_control(
        session,
        request,
        request.expected_sequence,
        command,
        "resolve",
    )?;
    accepted_sequence(request, sequence, "external_work_resolved")
}

fn simple<F>(
    owner: &Owner,
    session: &ActorSession,
    request: &CommandRequest,
    run_id: &str,
    suffix: &str,
    build: F,
) -> Result<CommandAccepted, PublicFailure>
where
    F: FnOnce(RunId) -> ControlCommand,
{
    let run = RunId::new(run_id.to_owned()).map_err(|error| invalid(&error.to_string()))?;
    let sequence = owner.execute_control(
        session,
        request,
        request.expected_sequence,
        build(run),
        suffix,
    )?;
    accepted_sequence(request, sequence, &format!("run_{suffix}d"))
}

#[cfg(test)]
mod tests;
