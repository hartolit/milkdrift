use milkdrift_control_protocol::{Command, ResolveAction};

use crate::{
    AttemptCommand, AttemptInspect, AttemptResolve, NodeInspect, ResolveChoice, error::CliError,
    session::CliSession,
};

pub(super) async fn node(session: &CliSession, arguments: &NodeInspect) -> Result<(), CliError> {
    session.output(
        "node.inspect",
        &session
            .client()
            .node(&arguments.run, &arguments.execution)
            .await?,
    )
}

async fn attempt(session: &CliSession, arguments: &AttemptInspect) -> Result<(), CliError> {
    session.output(
        "attempt.inspect",
        &session
            .client()
            .attempt(&arguments.run, &arguments.attempt)
            .await?,
    )
}

pub(super) async fn execute(
    session: &CliSession,
    command: &AttemptCommand,
) -> Result<(), CliError> {
    match command {
        AttemptCommand::Inspect(arguments) => attempt(session, arguments).await,
        AttemptCommand::Resolve(arguments) => resolve(session, arguments).await,
    }
}

async fn resolve(session: &CliSession, arguments: &AttemptResolve) -> Result<(), CliError> {
    let action = match arguments.action {
        ResolveChoice::Query => ResolveAction::Query,
        ResolveChoice::Retry => ResolveAction::Retry,
        ResolveChoice::Compensate => ResolveAction::Compensate,
        ResolveChoice::Retain => ResolveAction::Retain,
        ResolveChoice::ResolveSucceeded => ResolveAction::ResolveSucceeded,
        ResolveChoice::ResolveFailed => ResolveAction::ResolveFailed,
    };
    if matches!(
        action,
        ResolveAction::Retry
            | ResolveAction::Compensate
            | ResolveAction::ResolveSucceeded
            | ResolveAction::ResolveFailed
    ) {
        session.confirm(match action {
            ResolveAction::Retry => {
                "request retry under the daemon's durable idempotency and side-effect policy"
            }
            ResolveAction::Compensate => "create explicit compensation work",
            ResolveAction::ResolveSucceeded => {
                "resolve this uncertain attempt as succeeded from durable evidence"
            }
            ResolveAction::ResolveFailed => {
                "resolve this uncertain attempt as failed from durable evidence"
            }
            ResolveAction::Query | ResolveAction::Retain => "resolve retained external work",
        })?;
    }
    let request = session.command_request(Command::ResolveWork {
        run_id: arguments.run.clone(),
        attempt_id: arguments.attempt.clone(),
        decision_id: arguments.decision.clone(),
        action,
        remediation_node: arguments.remediation_node.clone(),
    })?;
    session.output("attempt.resolve", &session.client().submit(&request).await?)
}
