//! Exact candidate identities and finite observations retained by a trusted evaluation owner.
//!
//! A valid document is not proof of a trusted producer. The host reads these records only from
//! its private evaluation journal; uploaded artifacts cannot supply an acceptance decision.
use crate::{ArtifactReference, CausalId, WorkspaceError};
use milkdrift_capability::managed::ManagedName;
use serde::{Deserialize, Serialize};

/// One independently observed declared check. Absence of a result remains unknown.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CandidateCheck {
    /// Name from the fixed operator policy.
    pub name: String,
    /// `None` records an incomplete observation, never a pass.
    pub passed: Option<bool>,
    /// Bounded redacted observation, without application logs or credentials.
    pub diagnostic: String,
}

/// Facts frozen before a verifier starts. The target generation is the intended publication.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CandidateSubject {
    /// Immutable build bytes, including dirty source changes when the build is a source artifact.
    pub artifact: ArtifactReference,
    /// Exact target installation.
    pub target: ManagedName,
    /// Expected installed generation before publication.
    pub generation: u64,
    /// Immutable agreement accepted for this target.
    pub agreement: String,
    /// Exact effective non-secret configuration.
    pub configuration: String,
    /// Effect policy digest, independently bound by the agreement.
    pub policy: String,
    /// Exact trusted implementation/configuration generation.
    pub verifier: String,
    /// Configured trusted producer identity; not an agent-supplied artifact provenance claim.
    pub producer: CausalId,
}

/// A retained evaluation, with incomplete or failed checks as durable outcomes.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CandidateEvaluation {
    /// Exact supported evidence format.
    pub schema_version: u32,
    /// Host-derived identity of the accepted evaluation request.
    pub identity: String,
    /// Frozen applicability facts.
    pub subject: CandidateSubject,
    /// Boundary time at acceptance of evaluation.
    pub started_at: u64,
    /// Last permitted time for a new publication entry.
    pub expires_at: u64,
    /// `false` remains unknown after interruption; an exact replay does not rerun it.
    pub complete: bool,
    /// One observation for every required check, in declared order.
    pub checks: Vec<CandidateCheck>,
}
impl CandidateEvaluation {
    /// Check finite document shape. Authenticity must be established by the retaining owner.
    pub fn validate(&self) -> Result<(), WorkspaceError> {
        let s = &self.subject;
        if self.schema_version != 1
            || s.generation == 0
            || s.artifact.size_bytes() == 0
            || self.expires_at <= self.started_at
            || self.checks.is_empty()
            || self.checks.len() > 32
            || [
                &self.identity,
                &s.agreement,
                &s.configuration,
                &s.policy,
                &s.verifier,
            ]
            .into_iter()
            .any(|d| !milkdrift_contracts::is_canonical_blake3_digest(d))
            || self.checks.iter().any(|c| {
                c.name.is_empty()
                    || c.name.len() > 64
                    || !c
                        .name
                        .bytes()
                        .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
                    || c.diagnostic.len() > 256
                    || (!self.complete && c.passed.is_some())
            })
            || self
                .checks
                .iter()
                .map(|c| &c.name)
                .collect::<std::collections::BTreeSet<_>>()
                .len()
                != self.checks.len()
        {
            return Err(WorkspaceError::InvalidArtifact(
                "invalid candidate evaluation".to_owned(),
            ));
        }
        Ok(())
    }
}
