use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::Value;

use milkdrift_contracts::{CanonicalJsonError, canonical_json_bytes as encode_canonical_json};

use crate::{
    CancellationAcknowledgement, CancellationRequest, CapabilityDescriptor, ContractError,
    InvocationEvent, InvocationRequest, MAX_DOCUMENT_BYTES, ResolvedCapabilitySnapshot,
    bounded::{DOCUMENT_JSON_LIMITS, contract_bound, validate_document_value},
};

/// Schema version implemented by the first capability contract format.
pub const SCHEMA_VERSION_V1: u32 = 1;
/// Invocation request schema adding an explicit frozen context-manifest reference.
pub const INVOCATION_REQUEST_SCHEMA_VERSION_V2: u32 = 2;

pub(crate) fn canonical_json_bytes<T: Serialize>(value: &T) -> Result<Vec<u8>, ContractError> {
    let bytes =
        encode_canonical_json(value, DOCUMENT_JSON_LIMITS).map_err(|error| match error {
            CanonicalJsonError::Json(error) => ContractError::InvalidJson(error),
            CanonicalJsonError::Bounds(violation) => contract_bound(violation),
        })?;
    if bytes.len() > MAX_DOCUMENT_BYTES {
        return Err(ContractError::Bounds {
            location: "$".to_owned(),
            reason: format!("document exceeds {MAX_DOCUMENT_BYTES} bytes"),
        });
    }
    Ok(bytes)
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
    let value = milkdrift_contracts::parse_json_without_duplicates(bytes)?;
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
        #[derive(Clone, Debug, PartialEq, Serialize)]
        #[serde(deny_unknown_fields)]
        pub struct $name {
            schema_version: u32,
            $field: $body,
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
                if wire.schema_version != SCHEMA_VERSION_V1 {
                    return Err(serde::de::Error::custom(format!(
                        "unsupported {} schema version {}; supported version is {}",
                        $label, wire.schema_version, SCHEMA_VERSION_V1
                    )));
                }
                Ok(Self {
                    schema_version: wire.schema_version,
                    $field: wire.$field,
                })
            }
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
/// Versioned portable invocation request with explicit context binding in v2.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct InvocationRequestDocument {
    schema_version: u32,
    request: InvocationRequest,
}

impl InvocationRequestDocument {
    /// Wraps a request in the current v2 envelope.
    #[must_use]
    pub const fn new(request: InvocationRequest) -> Self {
        Self {
            schema_version: INVOCATION_REQUEST_SCHEMA_VERSION_V2,
            request,
        }
    }

    /// Current envelope version.
    #[must_use]
    pub const fn schema_version(&self) -> u32 {
        self.schema_version
    }

    /// Immutable request body.
    #[must_use]
    pub const fn body(&self) -> &InvocationRequest {
        &self.request
    }

    /// Deterministic canonical JSON.
    pub fn to_canonical_json(&self) -> Result<Vec<u8>, ContractError> {
        canonical_json_bytes(self)
    }

    /// Reads v2 and deliberately migrates unambiguous context-free v1 requests.
    pub fn from_json(bytes: &[u8]) -> Result<Self, ContractError> {
        if bytes.len() > MAX_DOCUMENT_BYTES {
            return Err(ContractError::Bounds {
                location: "$".to_owned(),
                reason: format!("document exceeds {MAX_DOCUMENT_BYTES} bytes"),
            });
        }
        let value = milkdrift_contracts::parse_json_without_duplicates(bytes)?;
        validate_document_value(&value)?;
        let version = value
            .get("schema_version")
            .and_then(Value::as_u64)
            .and_then(|version| u32::try_from(version).ok())
            .ok_or_else(|| {
                ContractError::InvalidContract("missing numeric schema_version".to_owned())
            })?;
        if !matches!(
            version,
            SCHEMA_VERSION_V1 | INVOCATION_REQUEST_SCHEMA_VERSION_V2
        ) {
            return Err(ContractError::UnsupportedVersion {
                document: "invocation request",
                found: version,
                supported: INVOCATION_REQUEST_SCHEMA_VERSION_V2,
            });
        }
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Wire {
            schema_version: u32,
            request: InvocationRequest,
        }
        let wire: Wire = serde_json::from_value(value)?;
        debug_assert_eq!(wire.schema_version, version);
        Ok(Self::new(wire.request))
    }
}

impl<'de> Deserialize<'de> for InvocationRequestDocument {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Wire {
            schema_version: u32,
            request: InvocationRequest,
        }
        let wire = Wire::deserialize(deserializer)?;
        if !matches!(
            wire.schema_version,
            SCHEMA_VERSION_V1 | INVOCATION_REQUEST_SCHEMA_VERSION_V2
        ) {
            return Err(serde::de::Error::custom(
                "unsupported invocation request schema version",
            ));
        }
        Ok(Self::new(wire.request))
    }
}
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
document!(
    /// Versioned portable exact capability resolution snapshot.
    ResolvedCapabilitySnapshotDocument,
    ResolvedCapabilitySnapshot,
    snapshot,
    "resolved capability snapshot"
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
