use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::Value;
use thiserror::Error;

use milkdrift_contracts::{CanonicalJsonError, JsonBoundKind, JsonLimits, canonical_json_bytes};

use crate::{ContextManifest, ModelResponse, ModelTaskRequest};

/// Current provider-neutral model contract schema.
pub const MODEL_CONTRACT_SCHEMA_VERSION_V1: u32 = 1;
const MAX_DOCUMENT_BYTES: usize = 2_097_152;
const LIMITS: JsonLimits = JsonLimits {
    maximum_depth: 48,
    maximum_string_bytes: 1_048_576,
    maximum_key_bytes: 192,
    maximum_container_items: 4_096,
};

/// Bounded portable model-contract failure.
#[derive(Debug, Error)]
pub enum ModelContractError {
    /// JSON syntax or shape was invalid.
    #[error("invalid model contract JSON: {0}")]
    Json(#[from] serde_json::Error),
    /// Hostile-input or semantic bound was exceeded.
    #[error("model contract bound exceeded at {location}: {reason}")]
    Bounds {
        /// JSON-like failure location.
        location: String,
        /// Stable human-readable bound.
        reason: String,
    },
    /// The document version is unsupported.
    #[error("unsupported {document} schema version {found}; supported version is {supported}")]
    UnsupportedVersion {
        /// Document family.
        document: &'static str,
        /// Supplied version.
        found: u32,
        /// Implemented version.
        supported: u32,
    },
    /// Typed semantic invariants were contradicted.
    #[error("invalid model contract: {0}")]
    Invalid(String),
}

pub(crate) fn encode<T: Serialize>(value: &T) -> Result<Vec<u8>, ModelContractError> {
    let bytes = canonical_json_bytes(value, LIMITS).map_err(map_canonical)?;
    if bytes.len() > MAX_DOCUMENT_BYTES {
        return Err(ModelContractError::Bounds {
            location: "$".to_owned(),
            reason: format!("document exceeds {MAX_DOCUMENT_BYTES} bytes"),
        });
    }
    Ok(bytes)
}

fn read<T: DeserializeOwned>(
    bytes: &[u8],
    document: &'static str,
) -> Result<T, ModelContractError> {
    if bytes.len() > MAX_DOCUMENT_BYTES {
        return Err(ModelContractError::Bounds {
            location: "$".to_owned(),
            reason: format!("document exceeds {MAX_DOCUMENT_BYTES} bytes"),
        });
    }
    milkdrift_contracts::preflight_json_structure(bytes, LIMITS).map_err(map_bound)?;
    let value = milkdrift_contracts::parse_json_without_duplicates(bytes)?;
    milkdrift_contracts::validate_json_value(&value, LIMITS).map_err(map_bound)?;
    let version = value
        .get("schema_version")
        .and_then(Value::as_u64)
        .and_then(|value| u32::try_from(value).ok())
        .ok_or_else(|| ModelContractError::Invalid("missing numeric schema_version".to_owned()))?;
    if version != MODEL_CONTRACT_SCHEMA_VERSION_V1 {
        return Err(ModelContractError::UnsupportedVersion {
            document,
            found: version,
            supported: MODEL_CONTRACT_SCHEMA_VERSION_V1,
        });
    }
    Ok(serde_json::from_value(value)?)
}

fn map_canonical(error: CanonicalJsonError) -> ModelContractError {
    match error {
        CanonicalJsonError::Json(error) => ModelContractError::Json(error),
        CanonicalJsonError::Bounds(bound) => map_bound(bound),
    }
}

fn map_bound(bound: milkdrift_contracts::JsonBoundViolation) -> ModelContractError {
    let noun = match bound.kind() {
        JsonBoundKind::Depth => "depth",
        JsonBoundKind::String => "string bytes",
        JsonBoundKind::Key => "key bytes",
        JsonBoundKind::Array => "array items",
        JsonBoundKind::Object => "object entries",
    };
    ModelContractError::Bounds {
        location: bound.path().to_owned(),
        reason: format!("{noun} exceed {}", bound.maximum()),
    }
}

macro_rules! document {
    ($name:ident, $body:ty, $field:ident, $label:literal) => {
        #[doc = concat!("Versioned portable ", $label, ".")]
        #[derive(Clone, Debug, PartialEq, Serialize)]
        #[serde(deny_unknown_fields)]
        pub struct $name {
            schema_version: u32,
            $field: $body,
        }

        impl $name {
            /// Wraps one validated body.
            #[must_use]
            pub const fn new($field: $body) -> Self {
                Self {
                    schema_version: MODEL_CONTRACT_SCHEMA_VERSION_V1,
                    $field,
                }
            }

            /// Current schema version.
            #[must_use]
            pub const fn schema_version(&self) -> u32 {
                self.schema_version
            }

            /// Typed document body.
            #[must_use]
            pub const fn body(&self) -> &$body {
                &self.$field
            }

            /// Deterministic canonical JSON.
            pub fn to_canonical_json(&self) -> Result<Vec<u8>, ModelContractError> {
                encode(self)
            }

            /// Bounds-checks, parses, and validates one document.
            pub fn from_json(bytes: &[u8]) -> Result<Self, ModelContractError> {
                read(bytes, $label)
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
            where
                D: serde::Deserializer<'de>,
            {
                #[derive(Deserialize)]
                #[serde(deny_unknown_fields)]
                struct Wire {
                    schema_version: u32,
                    $field: $body,
                }
                let wire = Wire::deserialize(deserializer)?;
                if wire.schema_version != MODEL_CONTRACT_SCHEMA_VERSION_V1 {
                    return Err(serde::de::Error::custom(
                        "unsupported model contract version",
                    ));
                }
                Ok(Self::new(wire.$field))
            }
        }
    };
}

document!(
    ModelTaskRequestDocument,
    ModelTaskRequest,
    request,
    "model task request"
);
document!(
    ModelResponseDocument,
    ModelResponse,
    response,
    "model response"
);
document!(
    ContextManifestDocument,
    ContextManifest,
    manifest,
    "context manifest"
);
