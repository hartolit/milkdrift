use serde::{Serialize, Serializer};

use crate::PersistenceError;

/// Maximum bytes in a redacted durable detail string.
pub const MAX_DETAIL_BYTES: usize = 4_096;
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

/// Bounded, redacted detail suitable for durable diagnostics and progress.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct BoundedDetail(String);

impl BoundedDetail {
    /// Validates diagnostic text. Empty detail is allowed.
    ///
    /// # Errors
    /// Refuses more than [`MAX_DETAIL_BYTES`] bytes or control characters.
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

milkdrift_contracts::deserialize_via!(BoundedDetail, String, |value| Self::new(value));

/// Validated page size with an inclusive global maximum.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(transparent)]
pub struct PageSize(u32);

impl PageSize {
    /// Constructs a non-zero bounded page size.
    ///
    /// # Errors
    /// Refuses zero or values above [`MAX_PAGE_SIZE`].
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

milkdrift_contracts::deserialize_via!(PageSize, u32, |value| Self::new(value));
