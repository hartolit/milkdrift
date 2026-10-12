use milkdrift_authority::ActorRef;
use milkdrift_runtime::CommandAuthorityClaim;
use serde::{Deserialize, Serialize};

/// Immutable authenticated actor/grant context supplied by a trusted caller boundary.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ActorAuthorityContext {
    actor: ActorRef,
    authority: CommandAuthorityClaim,
}

impl ActorAuthorityContext {
    /// Constructs exact actor and grant-revision context.
    #[must_use]
    pub const fn new(actor: ActorRef, authority: CommandAuthorityClaim) -> Self {
        Self { actor, authority }
    }

    /// Authenticated actor reference.
    #[must_use]
    pub const fn actor(&self) -> &ActorRef {
        &self.actor
    }

    /// Exact immutable authority claim.
    #[must_use]
    pub const fn authority(&self) -> &CommandAuthorityClaim {
        &self.authority
    }
}
