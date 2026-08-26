use std::{fmt, str::FromStr};

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use thiserror::Error;

const MAX_ID_BYTES: usize = 192;

/// Failure to construct or validate an authority contract.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum AuthorityError {
    /// A bounded identity or opaque reference is malformed.
    #[error("invalid {kind}: {reason}")]
    InvalidIdentity {
        /// Identity type.
        kind: &'static str,
        /// Stable validation explanation.
        reason: String,
    },
    /// A closed authority contract contains inconsistent facts.
    #[error("invalid authority contract: {0}")]
    InvalidContract(String),
    /// A serialized contract exceeds a defensive bound.
    #[error("authority bounds exceeded at {location}: {reason}")]
    Bounds {
        /// Stable field location.
        location: &'static str,
        /// Bounded explanation.
        reason: String,
    },
    /// JSON could not be encoded or decoded.
    #[error("invalid authority JSON: {0}")]
    Json(String),
    /// A future schema is not understood.
    #[error("unsupported {document} schema {found}; supported schema is {supported}")]
    UnsupportedVersion {
        /// Document family.
        document: &'static str,
        /// Supplied schema.
        found: u32,
        /// Latest supported schema.
        supported: u32,
    },
}

fn validate_identity(value: &str, kind: &'static str) -> Result<(), AuthorityError> {
    if value.is_empty() || value.len() > MAX_ID_BYTES {
        return Err(AuthorityError::InvalidIdentity {
            kind,
            reason: format!("length must be between 1 and {MAX_ID_BYTES} bytes"),
        });
    }
    if !value.is_ascii()
        || !value.as_bytes()[0].is_ascii_alphanumeric()
        || !value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':' | b'/')
        })
    {
        return Err(AuthorityError::InvalidIdentity {
            kind,
            reason: "must start with an ASCII alphanumeric and contain only alphanumerics, '-', '_', '.', ':', or '/'".to_owned(),
        });
    }
    Ok(())
}

macro_rules! identity_type {
    ($(#[$meta:meta])* $name:ident) => {
        milkdrift_contracts::validated_string_type! {
            $(#[$meta])*
            pub struct $name;
            error = AuthorityError;
            validate = validate_identity;
        }

        impl AsRef<str> for $name {
            fn as_ref(&self) -> &str { self.as_str() }
        }

        impl FromStr for $name {
            type Err = AuthorityError;
            fn from_str(value: &str) -> Result<Self, Self::Err> { Self::new(value) }
        }
    };
}

identity_type!(/// Canonical identity of a human, AI, controller, peer, or system actor.
    ActorRef);
identity_type!(/// Stable identity of an immutable authority grant lineage.
    GrantId);
identity_type!(/// Stable identity of an evaluator policy lineage.
    PolicyId);
identity_type!(/// Stable identity of one authorization decision.
    DecisionId);
identity_type!(/// Non-secret network configuration profile reference.
    NetworkProfileRef);

/// Opaque reference to secret material owned by a later resolver boundary.
///
/// The reference is serialized as its validated opaque text. Human-facing formatting is
/// deliberately redacted so logging the reference cannot accidentally become a pattern for
/// logging resolved values.
#[derive(Clone, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct SecretRef(String);

impl SecretRef {
    /// Constructs a validated opaque secret reference.
    pub fn new(value: impl Into<String>) -> Result<Self, AuthorityError> {
        let value = value.into();
        validate_identity(&value, "SecretRef")?;
        Ok(Self(value))
    }

    /// Returns the opaque lookup key to a trusted resolver implementation.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for SecretRef {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("SecretRef([redacted])")
    }
}

impl fmt::Display for SecretRef {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("[secret-ref]")
    }
}

impl Serialize for SecretRef {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for SecretRef {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Self::new(String::deserialize(deserializer)?).map_err(serde::de::Error::custom)
    }
}
