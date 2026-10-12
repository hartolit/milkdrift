use std::fmt;

use serde::Serialize;
use thiserror::Error;

/// Error returned by a blueprint identity constructor.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
#[error("invalid {kind}: {reason}")]
pub struct IdentityError {
    kind: &'static str,
    reason: String,
}

fn validate(value: &str, kind: &'static str, max: usize) -> Result<(), IdentityError> {
    if value.is_empty() || value.len() > max {
        return Err(IdentityError {
            kind,
            reason: format!("length must be between 1 and {max} bytes"),
        });
    }
    if !value.is_ascii()
        || !value
            .as_bytes()
            .first()
            .is_some_and(u8::is_ascii_alphanumeric)
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':'))
    {
        return Err(IdentityError {
            kind,
            reason: "must start with an alphanumeric and contain safe ASCII identity characters"
                .to_owned(),
        });
    }
    Ok(())
}

macro_rules! identity_type {
    ($(#[$meta:meta])* $name:ident, $max:expr) => {
        milkdrift_contracts::validated_string_type! {
            $(#[$meta])*
            pub struct $name;
            error = IdentityError;
            validate = |value, kind| validate(value, kind, $max);
        }
    };
}

identity_type!(
    /// Daemon-issued workflow identity.
    WorkflowId, 128
);
identity_type!(
    /// Mechanical task provenance coordinate retained by independent capability-host contracts.
    NodeId, 128
);
identity_type!(
    /// Daemon-assigned task identity, or an alias before atomic admission.
    TaskId, 128
);
/// Content-derived identity retained by capability provenance and exact authority snapshots.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct RevisionId(String);

impl RevisionId {
    pub(crate) fn parse(value: String) -> Result<Self, IdentityError> {
        validate_digest_identity(&value, "RevisionId", "rev_")?;
        Ok(Self(value))
    }

    /// Returns the revision identity text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

milkdrift_contracts::deserialize_via!(RevisionId, String, |value| Self::parse(value));

impl fmt::Display for RevisionId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}
fn validate_digest_identity(
    value: &str,
    kind: &'static str,
    prefix: &str,
) -> Result<(), IdentityError> {
    let digest = value.strip_prefix(prefix).ok_or_else(|| IdentityError {
        kind,
        reason: format!("must begin with '{prefix}'"),
    })?;
    if digest.len() != 64
        || !digest
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(IdentityError {
            kind,
            reason: "must contain a lowercase 32-byte BLAKE3 digest".to_owned(),
        });
    }
    Ok(())
}
