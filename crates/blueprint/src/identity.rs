use std::fmt;

use serde::{Deserialize, Deserializer, Serialize, Serializer};
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
        || !value.as_bytes()[0].is_ascii_alphanumeric()
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
        $(#[$meta])*
        #[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
        pub struct $name(String);

        impl $name {
            /// Constructs a validated typed identity.
            pub fn new(value: impl Into<String>) -> Result<Self, IdentityError> {
                let value = value.into();
                validate(&value, stringify!($name), $max)?;
                Ok(Self(value))
            }

            /// Returns the identity text.
            #[must_use]
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str(&self.0)
            }
        }

        impl Serialize for $name {
            fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
            where
                S: Serializer,
            {
                serializer.serialize_str(&self.0)
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
            where
                D: Deserializer<'de>,
            {
                let value = String::deserialize(deserializer)?;
                Self::new(value).map_err(serde::de::Error::custom)
            }
        }
    };
}

identity_type!(
    /// Reusable blueprint package identity.
    BlueprintId,
    128
);
identity_type!(
    /// Top-level workflow identity with a revision lineage.
    WorkflowId,
    128
);
identity_type!(
    /// Definition-time graph node identity.
    NodeId,
    128
);
identity_type!(
    /// Data or control port identity scoped to a node.
    PortId,
    96
);
identity_type!(
    /// Graph edge identity.
    EdgeId,
    128
);
identity_type!(
    /// Workflow interface field identity.
    FieldId,
    96
);
identity_type!(
    /// Bounded provenance reference; it grants no authority.
    AuthorRef,
    192
);
identity_type!(
    /// Identity used to correlate an atomic mutation batch.
    MutationBatchId,
    128
);

impl MutationBatchId {
    pub(crate) fn from_hash(hash: blake3::Hash) -> Self {
        Self(format!("batch_{hash}"))
    }
}

/// Exact immutable revision identity derived by the kernel.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct RevisionId(String);

impl RevisionId {
    pub(crate) fn from_hash(hash: blake3::Hash) -> Self {
        Self(format!("rev_{hash}"))
    }

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

impl<'de> Deserialize<'de> for RevisionId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Self::parse(String::deserialize(deserializer)?).map_err(serde::de::Error::custom)
    }
}

impl fmt::Display for RevisionId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

/// Digest of deterministic semantic blueprint content, independent of lineage and layout.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct ContentDigest(String);

impl ContentDigest {
    pub(crate) fn from_hash(hash: blake3::Hash) -> Self {
        Self(format!("b3_{hash}"))
    }

    pub(crate) fn parse(value: String) -> Result<Self, IdentityError> {
        validate_digest_identity(&value, "ContentDigest", "b3_")?;
        Ok(Self(value))
    }

    /// Returns the digest text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl<'de> Deserialize<'de> for ContentDigest {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Self::parse(String::deserialize(deserializer)?).map_err(serde::de::Error::custom)
    }
}

impl fmt::Display for ContentDigest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

/// Domain-separated digest of one node configuration or its incident dependencies.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct NodeFingerprint(String);

impl NodeFingerprint {
    pub(crate) fn from_hash(hash: blake3::Hash) -> Self {
        Self(format!("node_b3_{hash}"))
    }

    fn parse(value: String) -> Result<Self, IdentityError> {
        validate_digest_identity(&value, "NodeFingerprint", "node_b3_")?;
        Ok(Self(value))
    }

    /// Returns the stable lowercase digest identity.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl<'de> Deserialize<'de> for NodeFingerprint {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Self::parse(String::deserialize(deserializer)?).map_err(serde::de::Error::custom)
    }
}

impl fmt::Display for NodeFingerprint {
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
