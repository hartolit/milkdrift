use crate::{PeerCommand, error::CliError, session::CliSession};

pub(super) async fn execute(session: &CliSession, command: &PeerCommand) -> Result<(), CliError> {
    match command {
        PeerCommand::List => session.output("peer.list", &session.client().peers().await?),
        PeerCommand::Show { peer } => {
            session.output("peer.show", &session.client().peer(peer).await?)
        }
        PeerCommand::Connect { peer } => session.output(
            "peer.connect",
            &session.client().peer_action(peer, "connect").await?,
        ),
        PeerCommand::Reload { peer } => session.output(
            "peer.reload",
            &session.client().peer_action(peer, "reload").await?,
        ),
        PeerCommand::Disconnect { peer } => {
            session.confirm("disconnect and drain this peer")?;
            session.output(
                "peer.disconnect",
                &session.client().peer_action(peer, "disconnect").await?,
            )
        }
        PeerCommand::Drain { peer } => {
            session.confirm("drain this peer")?;
            session.output(
                "peer.drain",
                &session.client().peer_action(peer, "drain").await?,
            )
        }
        PeerCommand::Revoke { peer } => {
            session.confirm("revoke this live peer relationship")?;
            session.output(
                "peer.revoke",
                &session.client().peer_action(peer, "revoke").await?,
            )
        }
    }
}
