use std::{collections::BTreeSet, fmt};

use serde::{
    Deserialize, Serialize,
    de::{self, DeserializeOwned, MapAccess, SeqAccess, Visitor},
};
use serde_json::Value;

use crate::{
    CancellationAcknowledgement, CancellationRequest, CapabilityDescriptor, ContractError,
    InvocationEvent, InvocationRequest, MAX_DOCUMENT_BYTES, ResolvedCapabilitySnapshot,
    bounded::validate_document_value,
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
    reject_duplicate_json_keys(bytes)?;
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

fn reject_duplicate_json_keys(bytes: &[u8]) -> Result<(), serde_json::Error> {
    let mut deserializer = serde_json::Deserializer::from_slice(bytes);
    NoDuplicateJson::deserialize(&mut deserializer)?;
    deserializer.end()
}

struct NoDuplicateJson;

impl<'de> Deserialize<'de> for NoDuplicateJson {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        deserializer.deserialize_any(NoDuplicateJsonVisitor)
    }
}

struct NoDuplicateJsonVisitor;

impl<'de> Visitor<'de> for NoDuplicateJsonVisitor {
    type Value = NoDuplicateJson;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("JSON without duplicate object keys")
    }

    fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
    where
        A: MapAccess<'de>,
    {
        let mut keys = BTreeSet::new();
        while let Some(key) = map.next_key::<String>()? {
            if !keys.insert(key.clone()) {
                return Err(de::Error::custom(format!(
                    "duplicate JSON object key '{key}'"
                )));
            }
            map.next_value::<NoDuplicateJson>()?;
        }
        Ok(NoDuplicateJson)
    }

    fn visit_seq<A>(self, mut sequence: A) -> Result<Self::Value, A::Error>
    where
        A: SeqAccess<'de>,
    {
        while sequence.next_element::<NoDuplicateJson>()?.is_some() {}
        Ok(NoDuplicateJson)
    }

    fn visit_bool<E>(self, _: bool) -> Result<Self::Value, E> {
        Ok(NoDuplicateJson)
    }

    fn visit_i64<E>(self, _: i64) -> Result<Self::Value, E> {
        Ok(NoDuplicateJson)
    }

    fn visit_u64<E>(self, _: u64) -> Result<Self::Value, E> {
        Ok(NoDuplicateJson)
    }

    fn visit_f64<E>(self, _: f64) -> Result<Self::Value, E> {
        Ok(NoDuplicateJson)
    }

    fn visit_str<E>(self, _: &str) -> Result<Self::Value, E> {
        Ok(NoDuplicateJson)
    }

    fn visit_string<E>(self, _: String) -> Result<Self::Value, E> {
        Ok(NoDuplicateJson)
    }

    fn visit_none<E>(self) -> Result<Self::Value, E> {
        Ok(NoDuplicateJson)
    }

    fn visit_some<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        NoDuplicateJson::deserialize(deserializer)
    }

    fn visit_unit<E>(self) -> Result<Self::Value, E> {
        Ok(NoDuplicateJson)
    }
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
