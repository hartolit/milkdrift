use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::Value;

use crate::{
    CancellationAcknowledgement, CancellationRequest, CapabilityDescriptor, ContractError,
    InvocationEvent, InvocationRequest, MAX_DOCUMENT_BYTES, bounded::validate_document_value,
};

/// Schema version implemented by the first capability contract format.
pub const SCHEMA_VERSION_V1: u32 = 1;

/// Returns a deterministic compact JSON representation with recursively sorted object keys.
pub fn canonical_json_bytes<T: Serialize>(value: &T) -> Result<Vec<u8>, ContractError> {
    let mut value = serde_json::to_value(value)?;
    validate_document_value(&value)?;
    sort_value(&mut value);
    let bytes = serde_json::to_vec(&value)?;
    if bytes.len() > MAX_DOCUMENT_BYTES {
        return Err(ContractError::Bounds {
            location: "$".to_owned(),
            reason: format!("document exceeds {MAX_DOCUMENT_BYTES} bytes"),
        });
    }
    Ok(bytes)
}

fn sort_value(value: &mut Value) {
    match value {
        Value::Object(map) => {
            for child in map.values_mut() {
                sort_value(child);
            }
            let old = std::mem::take(map);
            let mut entries: Vec<_> = old.into_iter().collect();
            entries.sort_by(|left, right| left.0.cmp(&right.0));
            map.extend(entries);
        }
        Value::Array(values) => {
            for child in values {
                sort_value(child);
            }
        }
        _ => {}
    }
}

fn read_document<T: DeserializeOwned>(
    bytes: &[u8],
    document: &'static str,
) -> Result<T, ContractError> {
    if bytes.len() > MAX_DOCUMENT_BYTES {
        return Err(ContractError::Bounds {
            location: "$".to_owned(),
            reason: format!("document exceeds {MAX_DOCUMENT_BYTES} bytes"),
        });
    }
    let value: Value = serde_json::from_slice(bytes)?;
    validate_document_value(&value)?;
    let version = value
        .get("schema_version")
        .and_then(Value::as_u64)
        .and_then(|version| u32::try_from(version).ok())
        .ok_or_else(|| {
            ContractError::InvalidContract("missing numeric schema_version".to_owned())
        })?;
    if version != SCHEMA_VERSION_V1 {
        return Err(ContractError::UnsupportedVersion {
            document,
            found: version,
            supported: SCHEMA_VERSION_V1,
        });
    }
    Ok(serde_json::from_value(value)?)
}

macro_rules! document {
    ($(#[$meta:meta])* $name:ident, $body:ty, $field:ident, $label:literal) => {
        $(#[$meta])*
        #[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
        #[serde(deny_unknown_fields)]
        pub struct $name {
            schema_version: u32,
            $field: $body,
        }

        impl $name {
            /// Wraps a validated contract in the current schema envelope.
            #[must_use]
            pub const fn new($field: $body) -> Self {
                Self {
                    schema_version: SCHEMA_VERSION_V1,
                    $field,
                }
            }

            /// Returns the current schema version.
            #[must_use]
            pub const fn schema_version(&self) -> u32 {
                self.schema_version
            }

            /// Returns the envelope body.
            #[must_use]
            pub const fn body(&self) -> &$body {
                &self.$field
            }

            /// Serializes the envelope as deterministic compact JSON.
            pub fn to_canonical_json(&self) -> Result<Vec<u8>, ContractError> {
                canonical_json_bytes(self)
            }

            /// Parses, bounds-checks, version-checks, and validates an envelope.
            pub fn from_json(bytes: &[u8]) -> Result<Self, ContractError> {
                read_document(bytes, $label)
            }
        }
    };
}

document!(
    /// Versioned portable capability descriptor.
    CapabilityDescriptorDocument,
    CapabilityDescriptor,
    descriptor,
    "capability descriptor"
);
document!(
    /// Versioned portable invocation request.
    InvocationRequestDocument,
    InvocationRequest,
    request,
    "invocation request"
);
document!(
    /// Versioned portable invocation event.
    InvocationEventDocument,
    InvocationEvent,
    event,
    "invocation event"
);
document!(
    /// Versioned portable cancellation request.
    CancellationRequestDocument,
    CancellationRequest,
    request,
    "cancellation request"
);
document!(
    /// Versioned portable cancellation acknowledgement.
    CancellationAcknowledgementDocument,
    CancellationAcknowledgement,
    acknowledgement,
    "cancellation acknowledgement"
);

impl CancellationRequestDocument {
    /// Performs semantic validation in addition to envelope validation.
    pub fn validate(&self) -> Result<(), ContractError> {
        self.body().validate()
    }
}

impl CancellationAcknowledgementDocument {
    /// Performs semantic validation in addition to envelope validation.
    pub fn validate(&self) -> Result<(), ContractError> {
        self.body().validate()
    }
}
