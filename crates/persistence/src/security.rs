use crate::{IntegrityDigest, PageSize, PersistenceError, TimestampMillis};
use milkdrift_authority::{ActorRef, GrantDigest, GrantId};
use serde::{Deserialize, Serialize};
const MAX_APPLICATION_CURSOR_BYTES: usize = 1_024;
/// Opaque stable continuation for one application-state family.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ApplicationCursor(Vec<u8>);

impl ApplicationCursor {
    /// Validates bounded non-empty adapter-owned cursor bytes.
    ///
    /// # Errors
    /// Refuses empty cursors and cursors longer than 1,024 bytes.
    pub fn new(value: Vec<u8>) -> Result<Self, PersistenceError> {
        if value.is_empty() || value.len() > MAX_APPLICATION_CURSOR_BYTES {
            return Err(PersistenceError::InvalidCursor(format!(
                "application cursor must contain 1..={MAX_APPLICATION_CURSOR_BYTES} bytes"
            )));
        }
        Ok(Self(value))
    }
    /// Returns opaque adapter-owned bytes.
    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }
}

/// Bounded application-state page request.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ApplicationPageQuery {
    /// Exclusive continuation from a prior page.
    pub after: Option<ApplicationCursor>,
    /// Nonzero global page size.
    pub limit: PageSize,
}

/// Bounded application-state page.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ApplicationPage<T> {
    /// Verified page items.
    pub items: Vec<T>,
    /// Exclusive continuation, absent at the observed end.
    pub next: Option<ApplicationCursor>,
}

/// Bounded decision facts appended for protected local operations and reads.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SecurityAuditEntry {
    /// Evaluation timestamp.
    pub evaluated_at: TimestampMillis,
    /// Authenticated actor.
    pub actor: ActorRef,
    /// Exact grant identity.
    pub grant: GrantId,
    /// Exact grant revision.
    pub grant_revision: u64,
    /// Exact grant digest.
    pub grant_digest: GrantDigest,
    /// Closed operation label.
    pub operation: String,
    /// Redacted digest of resource facts.
    pub resource_digest: IntegrityDigest,
    /// Exact decision digest.
    pub decision_digest: String,
    /// Stable allowed/denied outcome.
    pub outcome: String,
    /// Bounded stable reason codes.
    pub reason_codes: Vec<String>,
}

impl SecurityAuditEntry {
    /// Verifies fields before append or after decoding.
    ///
    /// # Errors
    /// Refuses zero grant revision, more than 64 reasons, invalid operation/outcome/reason
    /// identities, or a noncanonical decision digest.
    pub fn validate(&self) -> Result<(), PersistenceError> {
        if self.grant_revision == 0 || self.reason_codes.len() > 64 {
            return Err(PersistenceError::InvalidDocument(
                "security audit grant revision/reason count is invalid".to_owned(),
            ));
        }
        validate_identity_text(&self.operation, "security_audit_operation")?;
        validate_identity_text(&self.outcome, "security_audit_outcome")?;
        validate_digest_text(&self.decision_digest, "security audit decision digest")?;
        for reason in &self.reason_codes {
            validate_identity_text(reason, "security_audit_reason")?;
        }
        Ok(())
    }
}

/// One sequenced retained security decision record.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SecurityAuditRecord {
    /// Monotonic append sequence; gaps can result from bounded prefix retention.
    pub sequence: u64,
    /// Exact bounded decision facts.
    pub entry: SecurityAuditEntry,
}

/// Narrow append-only bounded security-audit port.
pub trait SecurityAuditStore: Send + Sync {
    /// Appends one decision, atomically evicting only the oldest audit row at its independent
    /// retention bound. Command receipts are never evicted through this port.
    ///
    /// # Errors
    /// Refuses invalid entries and returns storage, sequence overflow or retained audit corruption.
    fn append_security_audit(
        &self,
        entry: &SecurityAuditEntry,
    ) -> Result<SecurityAuditRecord, PersistenceError>;
    /// Lists retained audit rows in stable sequence order.
    ///
    /// # Errors
    /// Refuses invalid continuations and returns storage failures or corrupt audit records.
    fn security_audit(
        &self,
        query: &ApplicationPageQuery,
    ) -> Result<ApplicationPage<SecurityAuditRecord>, PersistenceError>;
}

fn validate_identity_text(value: &str, location: &'static str) -> Result<(), PersistenceError> {
    if value.is_empty()
        || value.len() > 192
        || !value.is_ascii()
        || !value
            .as_bytes()
            .first()
            .is_some_and(u8::is_ascii_alphanumeric)
        || !value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':' | b'/')
        })
    {
        return Err(PersistenceError::Bounds {
            location,
            reason: "must be a 1..=192 byte portable identity".to_owned(),
        });
    }
    Ok(())
}

fn validate_digest_text(value: &str, label: &str) -> Result<(), PersistenceError> {
    IntegrityDigest::new(value.to_owned())
        .map(|_| ())
        .map_err(|_| {
            PersistenceError::InvalidDocument(format!("{label} is not a canonical BLAKE3 digest"))
        })
}
