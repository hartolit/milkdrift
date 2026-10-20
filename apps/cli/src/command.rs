//! Command-family composition over one connected control-client session.

use crate::{Cli, TopCommand, error::CliError, session::CliSession};

mod artifact;
mod capability;
mod daemon;
mod input;
mod invocation;
mod peer;
mod project;
mod resource;
mod stream;
mod workflow;

impl Cli {
    pub(crate) fn is_follow(&self) -> bool {
        matches!(
            &self.command,
            TopCommand::Daemon {
                command: crate::DaemonCommand::Health(crate::StreamArgs { follow: true, .. })
            } | TopCommand::Capability {
                command: crate::CapabilityCommand::List(crate::StreamArgs { follow: true, .. })
            }
        )
    }

    pub(crate) fn is_wait(&self) -> bool {
        matches!(
            self.command,
            TopCommand::Invocation {
                command: crate::InvocationCommand::Wait { .. }
            }
        )
    }

    pub(crate) fn operation(&self) -> &'static str {
        use crate::{ArtifactCommand, CapabilityCommand, DaemonCommand, PeerCommand};
        match &self.command {
            TopCommand::Project(_) => "project",
            TopCommand::Workflow(args) => match args.command {
                crate::workflow_args::WorkflowCommand::List { .. } => "workflow.list",
                crate::workflow_args::WorkflowCommand::Show { .. } => "workflow.show",
                crate::workflow_args::WorkflowCommand::Create { .. } => "workflow.create",
                crate::workflow_args::WorkflowCommand::Edit { .. } => "workflow.edit",
                crate::workflow_args::WorkflowCommand::Receipt { .. } => "workflow.receipt",
                crate::workflow_args::WorkflowCommand::Task { .. } => "workflow.task",
                crate::workflow_args::WorkflowCommand::Result { .. } => "workflow.result",
                crate::workflow_args::WorkflowCommand::Stop { .. } => "workflow.stop",
                crate::workflow_args::WorkflowCommand::Upload { .. } => "workflow.upload",
                crate::workflow_args::WorkflowCommand::Capabilities => "workflow.capabilities",
            },
            TopCommand::Resource(_) => "resource.manage",
            TopCommand::Invocation { command } => match command {
                crate::InvocationCommand::Catalog => "invocation.catalog",
                crate::InvocationCommand::Prepare { .. } => "invocation.prepare",
                crate::InvocationCommand::Submit { .. } => "invocation.submit",
                crate::InvocationCommand::Lookup { .. } => "invocation.lookup",
                crate::InvocationCommand::Show { .. } => "invocation.show",
                crate::InvocationCommand::Observations { .. } => "invocation.observations",
                crate::InvocationCommand::Wait { .. } => "invocation.wait",
                crate::InvocationCommand::Cancel { .. } => "invocation.cancel",
                crate::InvocationCommand::Output { .. } => "invocation.output",
            },
            TopCommand::Daemon { command } => match command {
                DaemonCommand::Name => "daemon.name",
                DaemonCommand::Rename { .. } => "daemon.rename",
                DaemonCommand::RenameReceipt { .. } => "daemon.rename_receipt",
                DaemonCommand::Health(_) => "daemon.health",
                DaemonCommand::Readiness => "daemon.readiness",
                DaemonCommand::Authority => "daemon.authority",
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
                ArtifactCommand::Upload { .. } => "artifact.upload",
                ArtifactCommand::Metadata { .. } => "artifact.metadata",
                ArtifactCommand::Digest { .. } => "artifact.digest",
                ArtifactCommand::Get { .. } => "artifact.get",
            },
        }
    }
}

pub(crate) async fn execute(cli: Cli) -> Result<(), CliError> {
    if let TopCommand::Artifact {
        command: crate::ArtifactCommand::Digest { file },
    } = &cli.command
    {
        let bytes = crate::input::read_bounded(file, 16_777_216, "candidate or verifier").await?;
        return crate::output::success(
            &cli,
            "artifact.digest",
            &serde_json::json!({"digest":blake3::hash(&bytes).to_hex().to_string(),"size":bytes.len()}),
        );
    }
    let session = CliSession::connect(cli).await?;
    match &session.cli().command {
        TopCommand::Project(args) => project::execute(&session, args).await,
        TopCommand::Workflow(args) => workflow::execute(&session, args).await,
        TopCommand::Resource(args) => resource::execute(&session, args).await,
        TopCommand::Invocation { command } => invocation::execute(&session, command).await,
        TopCommand::Daemon { command } => daemon::execute(&session, command).await,
        TopCommand::Capability { command } => capability::execute(&session, command).await,
        TopCommand::Provider { command } => capability::provider(&session, command).await,
        TopCommand::Peer { command } => peer::execute(&session, command).await,
        TopCommand::Artifact { command } => artifact::execute(&session, command).await,
    }
}
