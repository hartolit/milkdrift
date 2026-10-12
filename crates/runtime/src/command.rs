use crate::RuntimeError;
use milkdrift_authority::{GrantDigest, GrantId};
use serde::{Deserialize, Serialize};

/// Exact grant revision and revocation generation presented with an external command.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CommandAuthorityClaim {
    grant: GrantId,
    grant_revision: u64,
    grant_digest: GrantDigest,
    revocation_generation: u64,
}

impl CommandAuthorityClaim {
    /// Constructs a production claim bound to the authenticated immutable grant digest.
    ///
    /// # Errors
    /// Rejects a zero grant revision. Constructing a claim does not establish its authority.
    pub fn new(
        grant: GrantId,
        grant_revision: u64,
        grant_digest: GrantDigest,
        revocation_generation: u64,
    ) -> Result<Self, RuntimeError> {
        if grant_revision == 0 {
            return Err(RuntimeError::InvalidCommand(
                "authority grant revision must be nonzero".to_owned(),
            ));
        }
        Ok(Self {
            grant,
            grant_revision,
            grant_digest,
            revocation_generation,
        })
    }

    /// Exact grant lineage.
    #[must_use]
    pub const fn grant(&self) -> &GrantId {
        &self.grant
    }

    /// Exact immutable grant revision.
    #[must_use]
    pub const fn grant_revision(&self) -> u64 {
        self.grant_revision
    }

    /// Exact immutable digest authenticated with the grant revision.
    #[must_use]
    pub const fn grant_digest(&self) -> &GrantDigest {
        &self.grant_digest
    }

    /// Exact revocation generation observed by the caller boundary.
    #[must_use]
    pub const fn revocation_generation(&self) -> u64 {
        self.revocation_generation
    }
}
