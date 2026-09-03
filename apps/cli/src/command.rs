//! Command-family composition over one connected control-client session.

use crate::{Cli, TopCommand, error::CliError, session::CliSession};

mod artifact;
mod blueprint;
mod capability;
mod controller;
mod daemon;
mod inspection;
mod layout;
mod peer;
mod proposal;
mod run;
mod sequence;

pub(crate) async fn execute(cli: Cli) -> Result<(), CliError> {
    let session = CliSession::connect(cli).await?;
    match &session.cli().command {
        TopCommand::Daemon { command } => daemon::execute(&session, command).await,
        TopCommand::Blueprint { command } => blueprint::execute(&session, command).await,
        TopCommand::Sequence { command } => sequence::execute(&session, command).await,
        TopCommand::Run { command } => run::execute(&session, command).await,
        TopCommand::Controller { command } => controller::execute(&session, command).await,
        TopCommand::Node(arguments) => inspection::node(&session, arguments).await,
        TopCommand::Attempt(arguments) => inspection::attempt(&session, arguments).await,
        TopCommand::Proposal { command } => proposal::execute(&session, command).await,
        TopCommand::Capability { command } => capability::execute(&session, command).await,
        TopCommand::Peer { command } => peer::execute(&session, command).await,
        TopCommand::Artifact { command } => artifact::execute(&session, command).await,
        TopCommand::Layout { command } => layout::execute(&session, command).await,
    }
}
