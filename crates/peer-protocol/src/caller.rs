//! Authenticated caller realms and origin are separate from their HTTP transport.

use crate::{DelegatedAuthorization, PeerExecutionProvenance};
use milkdrift_authority::{ActorRef, GrantDigest, GrantId};
use milkdrift_capability::PeerId;
use serde::{Deserialize, Serialize};

/// Identity supplied by authentication, never selected by an invocation's input document.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd, Deserialize, Serialize)]
#[serde(rename_all = "snake_case", tag = "realm", deny_unknown_fields)]
pub enum ServingPrincipal {
    /// An independently authenticated client actor.
    Client {
        /// Server-configured actor identity.
        actor: ActorRef,
    },
    /// An authenticated peer whose targeted delegation must also be checked.
    Peer {
        /// Server-configured peer identity.
        peer: PeerId,
    },
}

/// Exact replay namespace, including the target host and authenticated caller realm.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ServingCaller {
    /// Installation accepting the work.
    pub host: PeerId,
    /// Authenticated principal within this installation.
    pub principal: ServingPrincipal,
}

impl ServingCaller {
    /// Constructs a namespace after peer authentication.
    #[must_use]
    pub fn peer(host: &PeerId, peer: &PeerId) -> Self {
        Self {
            host: host.clone(),
            principal: ServingPrincipal::Peer { peer: peer.clone() },
        }
    }

    /// Canonical collision-free key for bounded durable indexes.
    #[must_use]
    pub fn storage_key(&self) -> String {
        let (realm, principal) = match &self.principal {
            ServingPrincipal::Client { actor } => ("client", actor.as_str()),
            ServingPrincipal::Peer { peer } => ("peer", peer.as_str()),
        };
        format!(
            "{}:{}:{realm}:{principal}",
            self.host.as_str().len(),
            self.host
        )
    }

    /// The authenticated peer, only for the peer realm.
    #[must_use]
    pub const fn peer_identity(&self) -> Option<&PeerId> {
        match &self.principal {
            ServingPrincipal::Peer { peer } => Some(peer),
            ServingPrincipal::Client { .. } => None,
        }
    }
}

impl std::fmt::Display for ServingCaller {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.storage_key())
    }
}

/// Selection provenance independent of the caller's transport.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case", tag = "type", deny_unknown_fields)]
pub enum InvocationOrigin {
    /// Only explicitly supplied inputs; no workflow history may be discovered.
    Direct,
    /// Real originating workflow coordinates.
    Workflow {
        /// Exact immutable attempt provenance.
        provenance: PeerExecutionProvenance,
    },
}

impl InvocationOrigin {
    /// Real workflow coordinates when the origin is a workflow delegation.
    #[must_use]
    pub const fn workflow(&self) -> Option<&PeerExecutionProvenance> {
        match self {
            Self::Workflow { provenance } => Some(provenance),
            Self::Direct => None,
        }
    }
}

/// Server-created client authority binding retained with the canonical acceptance.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ClientInvocationAuthorization {
    /// Host accepting the request.
    pub host: PeerId,
    /// Independently authenticated actor.
    pub actor: ActorRef,
    /// Exact accepted grant identity.
    pub grant: GrantId,
    /// Exact accepted grant revision.
    pub grant_revision: u64,
    /// Canonical accepted grant digest.
    pub grant_digest: GrantDigest,
    /// Revocation generation at acceptance.
    pub revocation_generation: u64,
}

/// Accepted authorization basis. Public client payloads never supply this value.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(
    rename_all = "snake_case",
    tag = "type",
    content = "basis",
    deny_unknown_fields
)]
pub enum ServingAuthorization {
    /// Independent client authentication and its server-owned grant.
    Client(ClientInvocationAuthorization),
    /// Targeted delegation supplied through an authenticated peer relationship.
    Peer(Box<DelegatedAuthorization>),
}

impl ServingAuthorization {
    pub(crate) fn validate(&self) -> Result<(), crate::PeerProtocolError> {
        match self {
            Self::Peer(delegation) => delegation.validate(),
            Self::Client(client) if client.grant_revision > 0 => Ok(()),
            Self::Client(_) => Err(crate::PeerProtocolError::InvalidContract(
                "client grant revision must be nonzero".to_owned(),
            )),
        }
    }
    /// Namespace implied by these authenticated facts.
    #[must_use]
    pub fn caller(&self) -> ServingCaller {
        match self {
            Self::Client(client) => ServingCaller {
                host: client.host.clone(),
                principal: ServingPrincipal::Client {
                    actor: client.actor.clone(),
                },
            },
            Self::Peer(peer) => ServingCaller::peer(&peer.target_peer, &peer.issuer_peer),
        }
    }
    /// Actor whose exact grant authorized acceptance.
    #[must_use]
    pub const fn actor(&self) -> &ActorRef {
        match self {
            Self::Client(client) => &client.actor,
            Self::Peer(peer) => &peer.actor,
        }
    }
    /// Serving host named by the accepted authorization.
    #[must_use]
    pub const fn host(&self) -> &PeerId {
        match self {
            Self::Client(client) => &client.host,
            Self::Peer(peer) => &peer.target_peer,
        }
    }
    /// Direct or workflow origin; a client submission always has direct meaning.
    #[must_use]
    pub fn origin(&self) -> InvocationOrigin {
        match self {
            Self::Client(_) => InvocationOrigin::Direct,
            Self::Peer(peer) => peer.origin.clone(),
        }
    }
    /// Peer-only targeted delegation, absent for client authentication.
    #[must_use]
    pub const fn delegation(&self) -> Option<&DelegatedAuthorization> {
        match self {
            Self::Peer(peer) => Some(peer),
            Self::Client(_) => None,
        }
    }
}

impl From<DelegatedAuthorization> for ServingAuthorization {
    fn from(value: DelegatedAuthorization) -> Self {
        Self::Peer(Box::new(value))
    }
}

impl From<ClientInvocationAuthorization> for ServingAuthorization {
    fn from(value: ClientInvocationAuthorization) -> Self {
        Self::Client(value)
    }
}
