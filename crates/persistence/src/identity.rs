use std::{fmt, str::FromStr};

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::PersistenceError;

const MAX_ID_BYTES: usize = 192;

fn validate_identity(value: &str, kind: &'static str) -> Result<(), PersistenceError> {
    if value.is_empty() || value.len() > MAX_ID_BYTES {
        return Err(PersistenceError::InvalidIdentity {
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
        return Err(PersistenceError::InvalidIdentity {
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
            error = PersistenceError;
            validate = validate_identity;
        }

        impl AsRef<str> for $name {
            fn as_ref(&self) -> &str {
                self.as_str()
            }
        }

        impl FromStr for $name {
            type Err = PersistenceError;

            fn from_str(value: &str) -> Result<Self, Self::Err> {
                Self::new(value)
            }
        }
    };
}

identity_type!(/// Identity and idempotency key of a requested command.
    CommandId);
identity_type!(/// Stable identity of an append-only event.
    EventId);
identity_type!(/// Stable identity of one semantic node execution.
    NodeExecutionId);
identity_type!(/// Stable identity of one immutable execution attempt.
    AttemptId);
identity_type!(/// Stable identity of a worker lease.
    LeaseId);
identity_type!(/// Stable identity of an externally delivered signal.
    SignalId);
identity_type!(/// Namespaced semantic type of an external signal.
    SignalTypeId);
identity_type!(/// Optional bounded signal correlation identity.
    CorrelationKey);
identity_type!(/// Stable identity of a durable timer.
    TimerId);
identity_type!(/// Identity tying a revision-adoption request to its plans and decisions.
    ReconciliationId);
identity_type!(/// Stable identity of one immutable reconciliation plan.
    ReconciliationPlanId);
identity_type!(/// Decision idempotency identity, scoped by its owning plan or attempt.
    ReconciliationDecisionId);
identity_type!(/// Decision idempotency identity, scoped by its owning repeat execution.
    RepeatDecisionId);
identity_type!(/// Stable, non-secret identity of a worker/controller instance.
    WorkerId);
identity_type!(/// Stable identity of an artifact publication session.
    ArtifactPublicationId);
identity_type!(/// Stable identity of an optional projection snapshot.
    SnapshotId);
identity_type!(/// Stable identity of an artifact/blob publication session.
    PublicationId);
identity_type!(/// Stable reference to supporting evidence.
    EvidenceId);

/// The sole per-run aggregate sequence authority. Zero means an empty journal.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct RunSequence(u64);

impl RunSequence {
    /// Empty-journal sequence.
    pub const ZERO: Self = Self(0);
    /// First event sequence.
    pub const FIRST: Self = Self(1);

    /// Constructs a sequence, including zero for an empty journal.
    #[must_use]
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    /// Returns the numeric sequence.
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }

    /// Returns the next sequence, refusing overflow.
    pub fn next(self) -> Result<Self, PersistenceError> {
        self.0
            .checked_add(1)
            .map(Self)
            .ok_or(PersistenceError::SequenceOverflow)
    }
}

impl<'de> Deserialize<'de> for RunSequence {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Ok(Self(u64::deserialize(deserializer)?))
    }
}

impl fmt::Display for RunSequence {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

/// Portable timestamp fact as Unix epoch milliseconds.
///
/// A timestamp is supplied by a boundary clock and merely recorded by persistence;
/// no persistence operation reads the wall clock.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct TimestampMillis(u64);

impl TimestampMillis {
    /// Constructs an epoch-millisecond timestamp.
    #[must_use]
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    /// Returns the epoch-millisecond value.
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }
}

impl<'de> Deserialize<'de> for TimestampMillis {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Ok(Self(u64::deserialize(deserializer)?))
    }
}

impl fmt::Display for TimestampMillis {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

/// Lowercase BLAKE3 digest with an explicit `b3_` algorithm prefix.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct IntegrityDigest(String);

impl IntegrityDigest {
    /// Parses and validates a canonical digest.
    pub fn new(value: impl Into<String>) -> Result<Self, PersistenceError> {
        let value = value.into();
        if !milkdrift_contracts::is_canonical_blake3_digest(&value) {
            return Err(PersistenceError::InvalidDigest(
                "expected b3_ plus 64 lowercase hexadecimal characters".to_owned(),
            ));
        }
        Ok(Self(value))
    }

    /// Hashes bytes with BLAKE3 and formats the canonical identity.
    #[must_use]
    pub fn hash(bytes: &[u8]) -> Self {
        Self(format!("b3_{}", blake3::hash(bytes)))
    }

    /// Returns canonical digest text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for IntegrityDigest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl Serialize for IntegrityDigest {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for IntegrityDigest {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Self::new(String::deserialize(deserializer)?).map_err(serde::de::Error::custom)
    }
}
