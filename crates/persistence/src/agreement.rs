//! Accepted scope identity carried by a run and its children without replacing authority.

use milkdrift_blueprint::{BlueprintRevision, RevisionId};
use milkdrift_workspace::RunId;
use serde::{Deserialize, Serialize};

use crate::PersistenceError;

/// Reference to the outermost agreement accepted by a scope of work.
///
/// Children inherit this exact reference. Their immutable creation revisions still define any
/// inner obligations, but cannot grant permission to edit an enclosing protected child boundary.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct AcceptedAgreement {
    origin_run: RunId,
    origin_revision: RevisionId,
    agreement_digest: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Wire {
    origin_run: RunId,
    origin_revision: RevisionId,
    agreement_digest: String,
}

milkdrift_contracts::deserialize_via!(AcceptedAgreement, Wire, |wire| {
    if !milkdrift_contracts::is_canonical_blake3_digest(&wire.agreement_digest) {
        Err(PersistenceError::InvalidDocument(
            "invalid accepted agreement digest".to_owned(),
        ))
    } else {
        Ok(Self {
            origin_run: wire.origin_run,
            origin_revision: wire.origin_revision,
            agreement_digest: wire.agreement_digest,
        })
    }
});

impl AcceptedAgreement {
    /// Bind a validated governed revision when the runtime accepts its run scope.
    pub fn new(origin_run: RunId, revision: &BlueprintRevision) -> Result<Self, PersistenceError> {
        let agreement = revision.semantic().agreement().ok_or_else(|| {
            PersistenceError::InvalidDocument(
                "accepted agreement requires a governed revision".to_owned(),
            )
        })?;
        Ok(Self {
            origin_run,
            origin_revision: revision.id().clone(),
            agreement_digest: agreement.digest().to_owned(),
        })
    }
    /// Run whose accepted scope owns the editable method region.
    #[must_use]
    pub const fn origin_run(&self) -> &RunId {
        &self.origin_run
    }
    /// Initial revision retaining the exact immutable agreement definition.
    #[must_use]
    pub const fn origin_revision(&self) -> &RevisionId {
        &self.origin_revision
    }
    /// Independently addressed agreement definition.
    #[must_use]
    pub fn agreement_digest(&self) -> &str {
        &self.agreement_digest
    }
}
