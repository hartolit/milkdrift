//! Operator-owned verifier trust and applicability at consequential entry.
use crate::AuthorityError;
use milkdrift_workspace::{CandidateEvaluation, CausalId};
use serde::{Deserialize, Serialize};

/// Finite immutable policy. The managed owner supplies authenticated journal evidence; this
/// evaluator deliberately never accepts an arbitrary uploaded report as trusted evidence.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProtectedEffectPolicy {
    /// Exact policy format.
    pub schema_version: u32,
    /// Every named check must pass; no discretionary score or quorum exists.
    pub required_checks: Vec<String>,
    /// Pinned verifier implementation and effective configuration digest.
    pub verifier: String,
    /// Separate configured verifier producer.
    pub producer: CausalId,
    /// Inclusive candidate byte ceiling, applied before materialization.
    pub maximum_candidate_bytes: u64,
    /// Finite lifetime from evaluation acceptance; new entry after expiry refuses.
    pub validity_ms: u64,
}
impl ProtectedEffectPolicy {
    /// Read one bounded strict policy document before authoring its immutable agreement reference.
    pub fn from_json(bytes: &[u8]) -> Result<Self, AuthorityError> {
        if bytes.len() > 65_536 {
            return Err(invalid("effect policy exceeds 64 KiB"));
        }
        let value = milkdrift_contracts::parse_json_without_duplicates(bytes)
            .map_err(|e| invalid(&e.to_string()))?;
        let policy: Self = serde_json::from_value(value).map_err(|e| invalid(&e.to_string()))?;
        policy.validate()?;
        Ok(policy)
    }

    /// Validate operator input before making it available to delegated callers.
    pub fn validate(&self) -> Result<(), AuthorityError> {
        if self.schema_version != 1
            || self.required_checks.is_empty()
            || self.required_checks.len() > 32
            || self.required_checks.iter().any(|c| {
                c.is_empty()
                    || c.len() > 64
                    || !c
                        .bytes()
                        .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
            })
            || self
                .required_checks
                .iter()
                .collect::<std::collections::BTreeSet<_>>()
                .len()
                != self.required_checks.len()
            || !milkdrift_contracts::is_canonical_blake3_digest(&self.verifier)
            || self.maximum_candidate_bytes == 0
            || self.maximum_candidate_bytes > 16_777_216
            || self.validity_ms == 0
            || self.validity_ms > 86_400_000
        {
            return Err(invalid("invalid protected effect policy"));
        }
        Ok(())
    }
    /// Immutable policy identity committed by the blueprint agreement.
    pub fn digest(&self) -> Result<String, AuthorityError> {
        self.validate()?;
        let bytes = milkdrift_contracts::canonical_json_bytes(
            self,
            milkdrift_contracts::JsonLimits {
                maximum_depth: 8,
                maximum_string_bytes: 1024,
                maximum_key_bytes: 128,
                maximum_container_items: 64,
            },
        )
        .map_err(|_| invalid("invalid canonical effect policy"))?;
        let mut hash = blake3::Hasher::new();
        hash.update(b"milkdrift.protected-effect-policy.v1\0");
        hash.update(&bytes);
        Ok(format!("b3_{}", hash.finalize()))
    }
    /// Evaluate already-authenticated evidence against the active policy and current boundary time.
    pub fn require_pass(
        &self,
        evidence: &CandidateEvaluation,
        now: u64,
    ) -> Result<(), AuthorityError> {
        self.validate()?;
        evidence
            .validate()
            .map_err(|_| invalid("invalid candidate evidence"))?;
        if !evidence.complete
            || evidence.subject.policy != self.digest()?
            || evidence.subject.verifier != self.verifier
            || evidence.subject.producer != self.producer
            || evidence.subject.artifact.size_bytes() > self.maximum_candidate_bytes
            || now < evidence.started_at
            || now >= evidence.expires_at
            || evidence.expires_at.saturating_sub(evidence.started_at) != self.validity_ms
            || evidence
                .checks
                .iter()
                .map(|c| &c.name)
                .ne(self.required_checks.iter())
            || evidence.checks.iter().any(|c| c.passed != Some(true))
        {
            return Err(invalid(
                "verification is failed, unknown, expired or belongs to a different policy",
            ));
        }
        Ok(())
    }
}
fn invalid(reason: &str) -> AuthorityError {
    AuthorityError::InvalidContract(reason.to_owned())
}
