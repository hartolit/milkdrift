use std::collections::BTreeMap;

use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;
use thiserror::Error;

use crate::ExtensionKey;

/// Largest accepted serialized public contract document.
pub const MAX_DOCUMENT_BYTES: usize = 1_048_576;
/// Largest accepted JSON container nesting depth.
pub const MAX_JSON_DEPTH: usize = 48;
pub(crate) const MAX_EXTENSION_ENTRIES: usize = 64;
pub(crate) const MAX_EXTENSION_BYTES: usize = 65_536;
const MAX_STRING_BYTES: usize = 32_768;
const MAX_CONTAINER_ITEMS: usize = 4_096;

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
        validate_json_value(&value, "$", 0)?;
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
    validate_json_value(value, "$", 0)
}

fn validate_json_value(value: &Value, location: &str, depth: usize) -> Result<(), ContractError> {
    if depth > MAX_JSON_DEPTH {
        return Err(ContractError::Bounds {
            location: location.to_owned(),
            reason: format!("nesting exceeds depth {MAX_JSON_DEPTH}"),
        });
    }
    match value {
        Value::String(text) if text.len() > MAX_STRING_BYTES => Err(ContractError::Bounds {
            location: location.to_owned(),
            reason: format!("string exceeds {MAX_STRING_BYTES} bytes"),
        }),
        Value::Array(values) => {
            if values.len() > MAX_CONTAINER_ITEMS {
                return Err(ContractError::Bounds {
                    location: location.to_owned(),
                    reason: format!("array exceeds {MAX_CONTAINER_ITEMS} items"),
                });
            }
            for (index, child) in values.iter().enumerate() {
                validate_json_value(child, &format!("{location}[{index}]"), depth + 1)?;
            }
            Ok(())
        }
        Value::Object(map) => {
            if map.len() > MAX_CONTAINER_ITEMS {
                return Err(ContractError::Bounds {
                    location: location.to_owned(),
                    reason: format!("object exceeds {MAX_CONTAINER_ITEMS} entries"),
                });
            }
            for (key, child) in map {
                if key.len() > 192 {
                    return Err(ContractError::Bounds {
                        location: location.to_owned(),
                        reason: "object key exceeds 192 bytes".to_owned(),
                    });
                }
                validate_json_value(child, &format!("{location}.{key}"), depth + 1)?;
            }
            Ok(())
        }
        _ => Ok(()),
    }
}
