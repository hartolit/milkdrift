use std::collections::BTreeMap;

use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;
use thiserror::Error;

use milkdrift_contracts::{JsonBoundKind, JsonBoundViolation, JsonLimits};

use crate::ExtensionKey;

/// Largest accepted serialized public contract document.
pub const MAX_DOCUMENT_BYTES: usize = 1_048_576;
/// Largest accepted JSON container nesting depth.
pub const MAX_JSON_DEPTH: usize = 48;
pub(crate) const MAX_EXTENSION_ENTRIES: usize = 64;
pub(crate) const MAX_EXTENSION_BYTES: usize = 65_536;
const MAX_STRING_BYTES: usize = 32_768;
const MAX_CONTAINER_ITEMS: usize = 4_096;
pub(crate) const DOCUMENT_JSON_LIMITS: JsonLimits = JsonLimits {
    maximum_depth: MAX_JSON_DEPTH,
    maximum_string_bytes: MAX_STRING_BYTES,
    maximum_key_bytes: 192,
    maximum_container_items: MAX_CONTAINER_ITEMS,
};

/// Error returned when a capability contract violates a stable invariant.
#[derive(Debug, Error)]
pub enum ContractError {
    /// A typed identity did not meet its length, character, or namespace rule.
    #[error("invalid {type_name}: {reason}")]
    InvalidIdentity {
        /// Identity type being validated.
        type_name: &'static str,
        /// Concise stable diagnostic detail.
        reason: String,
    },
    /// A bounded field exceeded its contract.
    #[error("contract bound exceeded at {location}: {reason}")]
    Bounds {
        /// JSON-like location of the field.
        location: String,
        /// Description of the violated bound.
        reason: String,
    },
    /// A document version is newer or otherwise unsupported.
    #[error("unsupported {document} schema version {found}; supported version is {supported}")]
    UnsupportedVersion {
        /// Kind of document being read.
        document: &'static str,
        /// Version found on input.
        found: u32,
        /// Version implemented by this crate.
        supported: u32,
    },
    /// JSON syntax or shape was invalid.
    #[error("invalid JSON contract: {0}")]
    InvalidJson(#[from] serde_json::Error),
    /// A semantic contract invariant was violated.
    #[error("invalid capability contract: {0}")]
    InvalidContract(String),
}

/// JSON value checked against document depth, string, and collection bounds.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(transparent)]
pub struct BoundedJson(Value);

impl BoundedJson {
    /// Validates and stores a JSON value.
    pub fn new(value: Value) -> Result<Self, ContractError> {
        validate_json_value(&value)?;
        if serde_json::to_vec(&value)?.len() > MAX_EXTENSION_BYTES {
            return Err(ContractError::Bounds {
                location: "$".to_owned(),
                reason: format!("value exceeds {MAX_EXTENSION_BYTES} serialized bytes"),
            });
        }
        Ok(Self(value))
    }

    /// Returns the checked JSON value.
    #[must_use]
    pub fn value(&self) -> &Value {
        &self.0
    }
}

impl<'de> Deserialize<'de> for BoundedJson {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = Value::deserialize(deserializer)?;
        Self::new(value).map_err(serde::de::Error::custom)
    }
}

pub(crate) fn validate_extensions(
    extensions: &BTreeMap<ExtensionKey, BoundedJson>,
) -> Result<(), ContractError> {
    if extensions.len() > MAX_EXTENSION_ENTRIES {
        return Err(ContractError::Bounds {
            location: "extensions".to_owned(),
            reason: format!("at most {MAX_EXTENSION_ENTRIES} entries are allowed"),
        });
    }
    let bytes = serde_json::to_vec(extensions)?;
    if bytes.len() > MAX_EXTENSION_BYTES {
        return Err(ContractError::Bounds {
            location: "extensions".to_owned(),
            reason: format!("extensions exceed {MAX_EXTENSION_BYTES} serialized bytes"),
        });
    }
    Ok(())
}

pub(crate) fn validate_document_value(value: &Value) -> Result<(), ContractError> {
    validate_json_value(value)
}

fn validate_json_value(value: &Value) -> Result<(), ContractError> {
    milkdrift_contracts::validate_json_value(value, DOCUMENT_JSON_LIMITS).map_err(contract_bound)
}

pub(crate) fn contract_bound(violation: JsonBoundViolation) -> ContractError {
    let reason = match violation.kind() {
        JsonBoundKind::Depth => format!("nesting exceeds depth {}", violation.maximum()),
        JsonBoundKind::String => format!("string exceeds {} bytes", violation.maximum()),
        JsonBoundKind::Key => format!("object key exceeds {} bytes", violation.maximum()),
        JsonBoundKind::Array => format!("array exceeds {} items", violation.maximum()),
        JsonBoundKind::Object => format!("object exceeds {} entries", violation.maximum()),
    };
    ContractError::Bounds {
        location: violation.path().to_owned(),
        reason,
    }
}
