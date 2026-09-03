use std::{fmt, str::FromStr};

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::ControlError;

const MAX_ID_BYTES: usize = 128;

fn validate_identity(value: &str, kind: &'static str) -> Result<(), ControlError> {
    if value.is_empty() || value.len() > MAX_ID_BYTES {
        return Err(ControlError::InvalidIdentity {
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
        return Err(ControlError::InvalidIdentity {
            kind,
            reason:
                "must start with an ASCII alphanumeric and contain only safe identity characters"
                    .to_owned(),
        });
    }
    Ok(())
}

macro_rules! identity_type {
    ($(#[$meta:meta])* $name:ident) => {
        $(#[$meta])*
        #[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
        #[serde(transparent)]
        pub struct $name(String);

        impl $name {
            /// Constructs a bounded safe identity.
            pub fn new(value: impl Into<String>) -> Result<Self, ControlError> {
                let value = value.into();
                validate_identity(&value, stringify!($name))?;
                Ok(Self(value))
            }

            /// Returns the identity text.
            #[must_use]
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
            where
                D: Deserializer<'de>,
            {
                Self::new(String::deserialize(deserializer)?).map_err(serde::de::Error::custom)
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str(&self.0)
            }
        }

        impl FromStr for $name {
            type Err = ControlError;

            fn from_str(value: &str) -> Result<Self, Self::Err> {
                Self::new(value)
            }
        }
    };
}

identity_type!(/// Stable identity and idempotency scope of one control request.
    ControlId);
identity_type!(/// Stable identity of one untrusted workflow proposal.
    ProposalId);
identity_type!(/// Stable identity of one immutable controller policy lineage.
    ControllerId);

/// Domain-separated deterministic digest of a canonical proposal body.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ProposalDigest(String);

impl ProposalDigest {
    pub(crate) fn for_bytes(bytes: &[u8]) -> Self {
        let mut hasher = blake3::Hasher::new();
        hasher.update(b"milkdrift.workflow-proposal.v1\0");
        hasher.update(bytes);
        Self(format!("b3_{}", hasher.finalize()))
    }

    fn parse(value: String) -> Result<Self, ControlError> {
        if !milkdrift_contracts::is_canonical_blake3_digest(&value) {
            return Err(ControlError::InvalidIdentity {
                kind: "ProposalDigest",
                reason: "expected b3_ plus 64 lowercase hexadecimal characters".to_owned(),
            });
        }
        Ok(Self(value))
    }

    /// Returns the canonical digest text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ProposalDigest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl Serialize for ProposalDigest {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for ProposalDigest {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Self::parse(String::deserialize(deserializer)?).map_err(serde::de::Error::custom)
    }
}

/// Domain-separated deterministic digest of every executable controller-policy field.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ControllerPolicyDigest(String);

impl ControllerPolicyDigest {
    pub(crate) fn for_bytes(bytes: &[u8]) -> Self {
        let mut hasher = blake3::Hasher::new();
        hasher.update(b"milkdrift.controller-policy.v1\0");
        hasher.update(bytes);
        Self(format!("cp1_{}", hasher.finalize()))
    }

    fn parse(value: String) -> Result<Self, ControlError> {
        let Some(hex) = value.strip_prefix("cp1_") else {
            return Err(ControlError::InvalidIdentity {
                kind: "ControllerPolicyDigest",
                reason: "missing cp1_ prefix".to_owned(),
            });
        };
        if hex.len() != 64
            || !hex
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            return Err(ControlError::InvalidIdentity {
                kind: "ControllerPolicyDigest",
                reason: "expected 64 lowercase hexadecimal characters".to_owned(),
            });
        }
        Ok(Self(value))
    }

    /// Returns the canonical digest text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ControllerPolicyDigest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl Serialize for ControllerPolicyDigest {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for ControllerPolicyDigest {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Self::parse(String::deserialize(deserializer)?).map_err(serde::de::Error::custom)
    }
}
