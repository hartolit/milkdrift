use crate::{AttemptInspect, NodeInspect, error::CliError, session::CliSession};

pub(super) async fn node(session: &CliSession, arguments: &NodeInspect) -> Result<(), CliError> {
    session.output(
        "node.inspect",
        &session
            .client()
            .node(&arguments.run, &arguments.execution)
            .await?,
    )
}

pub(super) async fn attempt(
    session: &CliSession,
    arguments: &AttemptInspect,
) -> Result<(), CliError> {
    session.output(
        "attempt.inspect",
        &session
            .client()
            .attempt(&arguments.run, &arguments.attempt)
            .await?,
    )
}
