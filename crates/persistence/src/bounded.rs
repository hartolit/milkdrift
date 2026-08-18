use std::fmt;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::{EvidenceId, PersistenceError};

/// Maximum bytes in a durable control-transition reason.
pub const MAX_REASON_BYTES: usize = 2_048;
/// Maximum bytes in a redacted durable detail string.
pub const MAX_DETAIL_BYTES: usize = 4_096;
/// Maximum evidence references carried by one fact.
pub const MAX_EVIDENCE_REFERENCES: usize = 32;
/// Maximum event envelopes committed for one accepted command.
pub const MAX_EVENTS_PER_COMMIT: usize = 512;
/// Maximum records returned by one page query.
pub const MAX_PAGE_SIZE: u32 = 1_000;
/// Maximum bytes in one artifact stream chunk.
pub const MAX_ARTIFACT_CHUNK_BYTES: usize = 1_048_576;

fn validate_text(
    value: &str,
    location: &'static str,
    maximum: usize,
    allow_empty: bool,
) -> Result<(), PersistenceError> {
    if (!allow_empty && value.is_empty()) || value.len() > maximum {
        let minimum = usize::from(!allow_empty);
        return Err(PersistenceError::Bounds {
            location,
            reason: format!("must contain {minimum}..={maximum} UTF-8 bytes"),
        });
    }
    if value.chars().any(char::is_control) {
        return Err(PersistenceError::Bounds {
            location,
            reason: "must not contain control characters".to_owned(),
        });
    }
    Ok(())
}

/// Bounded, non-empty reason recorded for a control transition.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct Reason(String);

impl Reason {
    /// Validates a reason.
    pub fn new(value: impl Into<String>) -> Result<Self, PersistenceError> {
        let value = value.into();
        validate_text(&value, "reason", MAX_REASON_BYTES, false)?;
        Ok(Self(value))
    }

    /// Returns the reason text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for Reason {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl Serialize for Reason {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for Reason {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Self::new(String::deserialize(deserializer)?).map_err(serde::de::Error::custom)
    }
}

/// Bounded, redacted detail suitable for durable diagnostics and progress.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct BoundedDetail(String);

impl BoundedDetail {
    /// Validates diagnostic text. Empty detail is allowed.
    pub fn new(value: impl Into<String>) -> Result<Self, PersistenceError> {
        let value = value.into();
        validate_text(&value, "detail", MAX_DETAIL_BYTES, true)?;
        Ok(Self(value))
    }

    /// Returns the redacted detail.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Serialize for BoundedDetail {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for BoundedDetail {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Self::new(String::deserialize(deserializer)?).map_err(serde::de::Error::custom)
    }
}

/// Three-letter uppercase currency code paired with cost observations.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct CurrencyCode(String);

impl CurrencyCode {
    /// Validates an uppercase three-letter currency code.
    pub fn new(value: impl Into<String>) -> Result<Self, PersistenceError> {
        let value = value.into();
        if value.len() != 3 || !value.bytes().all(|byte| byte.is_ascii_uppercase()) {
            return Err(PersistenceError::Bounds {
                location: "currency",
                reason: "must contain exactly three uppercase ASCII letters".to_owned(),
            });
        }
        Ok(Self(value))
    }

    /// Returns the validated currency code.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Serialize for CurrencyCode {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for CurrencyCode {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Self::new(String::deserialize(deserializer)?).map_err(serde::de::Error::custom)
    }
}

/// Relationship between an evidence reference and the fact carrying it.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceKind {
    /// Human or controller approval evidence.
    AuthorityDecision,
    /// Executor/worker observation.
    WorkerObservation,
    /// External-system receipt or status reference.
    ExternalReceipt,
    /// Artifact containing larger evidence.
    Artifact,
    /// Recovery or integrity observation.
    RecoveryObservation,
}

/// Bounded reference to evidence; evidence bytes are not embedded in events.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct EvidenceReference {
    /// Stable evidence identity.
    pub id: EvidenceId,
    /// Semantic relationship to the carrying fact.
    pub kind: EvidenceKind,
}

/// Validated page size with an inclusive global maximum.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(transparent)]
pub struct PageSize(u32);

impl PageSize {
    /// Constructs a non-zero bounded page size.
    pub fn new(value: u32) -> Result<Self, PersistenceError> {
        if value == 0 || value > MAX_PAGE_SIZE {
            return Err(PersistenceError::Bounds {
                location: "page_size",
                reason: format!("must be between 1 and {MAX_PAGE_SIZE}"),
            });
        }
        Ok(Self(value))
    }

    /// Returns the numeric page size.
    #[must_use]
    pub const fn get(self) -> u32 {
        self.0
    }
}

impl<'de> Deserialize<'de> for PageSize {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Self::new(u32::deserialize(deserializer)?).map_err(serde::de::Error::custom)
    }
}
