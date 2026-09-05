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
mod stream;

impl Cli {
    pub(crate) fn is_follow(&self) -> bool {
        matches!(
            &self.command,
            TopCommand::Run {
                command: crate::RunCommand::Timeline { follow: true, .. }
            } | TopCommand::Daemon {
                command: crate::DaemonCommand::Health(crate::StreamArgs { follow: true, .. })
            } | TopCommand::Capability {
                command: crate::CapabilityCommand::List(crate::StreamArgs { follow: true, .. })
            }
        )
    }

    pub(crate) fn is_wait(&self) -> bool {
        matches!(
            self.command,
            TopCommand::Run {
                command: crate::RunCommand::Wait { .. }
            }
        )
    }

    pub(crate) fn operation(&self) -> &'static str {
        use crate::{
            ArtifactCommand, AttemptCommand, BlueprintCommand, CapabilityCommand,
            ControllerCommand, DaemonCommand, LayoutCommand, PeerCommand, ProposalCommand,
            RunCommand, SequenceCommand,
        };
        match &self.command {
            TopCommand::Daemon { command } => match command {
                DaemonCommand::Health(_) => "daemon.health",
                DaemonCommand::Readiness => "daemon.readiness",
                DaemonCommand::Authority => "daemon.authority",
            },
            TopCommand::Blueprint { command } => match command {
                BlueprintCommand::Validate { .. } => "blueprint.validate",
                BlueprintCommand::Import { .. } => "blueprint.import",
                BlueprintCommand::Show { .. } => "blueprint.show",
                BlueprintCommand::Export { .. } => "blueprint.export",
                BlueprintCommand::List(_) => "blueprint.list",
                BlueprintCommand::Diff { .. } => "blueprint.diff",
            },
            TopCommand::Sequence { command } => match command {
                SequenceCommand::Validate { .. } => "sequence.validate",
                SequenceCommand::Compile { .. } => "sequence.compile",
                SequenceCommand::Import { .. } => "sequence.import",
                SequenceCommand::Show { .. } => "sequence.show",
                SequenceCommand::Status { .. } => "sequence.status",
                SequenceCommand::Stage { .. } => "sequence.stage",
                SequenceCommand::Remediate { .. } => "sequence.remediate",
            },
            TopCommand::Run { command } => match command {
                RunCommand::Start { .. } => "run.start",
                RunCommand::List(_) => "run.list",
                RunCommand::Show { .. } => "run.show",
                RunCommand::Wait { .. } => "run.wait",
                RunCommand::Pause { .. } => "run.pause",
                RunCommand::Resume { .. } => "run.resume",
                RunCommand::Cancel { .. } => "run.cancel",
                RunCommand::Signal { .. } => "run.signal",
                RunCommand::Timeline { .. } => "run.timeline",
            },
            TopCommand::Controller { command } => match command {
                ControllerCommand::Status { .. } => "controller.status",
                ControllerCommand::Continue { .. } => "controller.continue",
            },
            TopCommand::Node(_) => "node.inspect",
            TopCommand::Attempt { command } => match command {
                AttemptCommand::Inspect(_) => "attempt.inspect",
                AttemptCommand::Resolve(_) => "attempt.resolve",
            },
            TopCommand::Proposal { command } => match command {
                ProposalCommand::Submit { .. } => "proposal.submit",
                ProposalCommand::List { .. } => "proposal.list",
                ProposalCommand::Show { .. } => "proposal.show",
                ProposalCommand::Approve(_) | ProposalCommand::Reject(_) => "proposal.decide",
                ProposalCommand::Apply(_) => "proposal.apply",
            },
            TopCommand::Capability { command } => match command {
                CapabilityCommand::List(_) => "capability.list",
                CapabilityCommand::Show { .. } => "capability.show",
            },
            TopCommand::Provider { command } => match command {
                crate::ProviderCommand::List => "provider.list",
                crate::ProviderCommand::Show { .. } => "provider.show",
            },
            TopCommand::Peer { command } => match command {
                PeerCommand::List => "peer.list",
                PeerCommand::Show { .. } => "peer.show",
                PeerCommand::Connect { .. } => "peer.connect",
                PeerCommand::Reload { .. } => "peer.reload",
                PeerCommand::Disconnect { .. } => "peer.disconnect",
                PeerCommand::Drain { .. } => "peer.drain",
                PeerCommand::Revoke { .. } => "peer.revoke",
            },
            TopCommand::Artifact { command } => match command {
                ArtifactCommand::Metadata { .. } => "artifact.metadata",
                ArtifactCommand::Get { .. } => "artifact.get",
            },
            TopCommand::Layout { command } => match command {
                LayoutCommand::Get { .. } => "layout.get",
                LayoutCommand::Put { .. } => "layout.put",
            },
        }
    }
}

pub(crate) async fn execute(cli: Cli) -> Result<(), CliError> {
    if cli.json
        && matches!(
            &cli.command,
            TopCommand::Blueprint {
                command: crate::BlueprintCommand::Show { document: true, .. }
            }
        )
    {
        return Err(CliError::Invalid(
            "JSON document export requires an explicit output file".to_owned(),
        ));
    }
    if let TopCommand::Sequence {
        command:
            crate::SequenceCommand::Compile {
                file,
                author,
                output,
            },
    } = &cli.command
    {
        use std::io::Write as _;
        let bytes = crate::input::read_bounded(
            file,
            milkdrift_prompt_sequence::MAX_PROMPT_SEQUENCE_DOCUMENT_BYTES,
            "prompt sequence",
        )
        .await?;
        let document = milkdrift_prompt_sequence::PromptSequenceDocument::from_bytes(&bytes)
            .map_err(|error| CliError::Invalid(error.to_string()))?;
        let author = serde_json::from_value(serde_json::Value::String(author.clone()))
            .map_err(|error| CliError::Invalid(error.to_string()))?;
        let compiled = milkdrift_prompt_sequence::compile(&document, author)
            .map_err(|error| CliError::Invalid(error.to_string()))?;
        let bytes = compiled
            .to_canonical_json()
            .map_err(|error| CliError::Invalid(error.to_string()))?;
        let mut destination = crate::output::PendingFile::create(output)?;
        destination
            .write_all(&bytes)
            .and_then(|()| destination.commit())
            .map_err(|error| CliError::Internal(error.to_string()))?;
        return crate::output::success(
            &cli,
            "sequence.compile",
            &serde_json::json!({"revision_id": compiled.revision().id().as_str(), "output": output, "size": bytes.len()}),
        );
    }
    let session = CliSession::connect(cli).await?;
    match &session.cli().command {
        TopCommand::Daemon { command } => daemon::execute(&session, command).await,
        TopCommand::Blueprint { command } => blueprint::execute(&session, command).await,
        TopCommand::Sequence { command } => sequence::execute(&session, command).await,
        TopCommand::Run { command } => run::execute(&session, command).await,
        TopCommand::Controller { command } => controller::execute(&session, command).await,
        TopCommand::Node(arguments) => inspection::node(&session, arguments).await,
        TopCommand::Attempt { command } => inspection::execute(&session, command).await,
        TopCommand::Proposal { command } => proposal::execute(&session, command).await,
        TopCommand::Capability { command } => capability::execute(&session, command).await,
        TopCommand::Provider { command } => capability::provider(&session, command).await,
        TopCommand::Peer { command } => peer::execute(&session, command).await,
        TopCommand::Artifact { command } => artifact::execute(&session, command).await,
        TopCommand::Layout { command } => layout::execute(&session, command).await,
    }
}
