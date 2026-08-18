use std::{collections::BTreeSet, fmt};

use milkdrift_workspace::RunId;
use serde::{
    Deserialize, Deserializer, Serialize,
    de::{MapAccess, SeqAccess, Visitor},
};
use serde_json::{Map, Value};

use crate::{
    EventId, IntegrityDigest, PersistenceError, RunEventKind, RunSequence, TimestampMillis,
};

/// Current run-event envelope schema.
pub const RUN_EVENT_SCHEMA_VERSION_V1: u32 = 1;
/// Maximum encoded event size. Larger content belongs in workspace/artifact storage.
pub const MAX_EVENT_DOCUMENT_BYTES: usize = 1_048_576;
const MAX_DOCUMENT_DEPTH: usize = 64;
const MAX_CONTAINER_ITEMS: usize = 4_096;
const MAX_STRING_BYTES: usize = 65_536;
const EVENT_CHECKSUM_DOMAIN: &str = "milkdrift.run-event-envelope.v1";

/// Explicit, checksummed schema-v1 append-only event envelope.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RunEventEnvelope {
    schema_version: u32,
    event_id: EventId,
    run_id: RunId,
    sequence: RunSequence,
    occurred_at: TimestampMillis,
    kind: RunEventKind,
    checksum: IntegrityDigest,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RunEventEnvelopeWire {
    schema_version: u32,
    event_id: EventId,
    run_id: RunId,
    sequence: RunSequence,
    occurred_at: TimestampMillis,
    kind: RunEventKind,
    checksum: IntegrityDigest,
}

#[derive(Serialize)]
struct EventChecksumInput<'a> {
    domain: &'static str,
    schema_version: u32,
    event_id: &'a EventId,
    run_id: &'a RunId,
    sequence: RunSequence,
    occurred_at: TimestampMillis,
    kind: &'a RunEventKind,
}

impl RunEventEnvelope {
    /// Constructs a validated envelope and calculates its canonical checksum.
    pub fn new(
        event_id: EventId,
        run_id: RunId,
        sequence: RunSequence,
        occurred_at: TimestampMillis,
        kind: RunEventKind,
    ) -> Result<Self, PersistenceError> {
        if sequence == RunSequence::ZERO {
            return Err(PersistenceError::InvalidDocument(
                "event sequence must be greater than zero".to_owned(),
            ));
        }
        kind.validate_for_run(&run_id)?;
        let checksum = calculate_event_checksum(
            RUN_EVENT_SCHEMA_VERSION_V1,
            &event_id,
            &run_id,
            sequence,
            occurred_at,
            &kind,
        )?;
        let envelope = Self {
            schema_version: RUN_EVENT_SCHEMA_VERSION_V1,
            event_id,
            run_id,
            sequence,
            occurred_at,
            kind,
            checksum,
        };
        // Enforce the encoded bound at the trusted construction boundary too.
        let _ = envelope.to_canonical_json()?;
        Ok(envelope)
    }

    /// Returns the explicit envelope schema.
    #[must_use]
    pub const fn schema_version(&self) -> u32 {
        self.schema_version
    }

    /// Returns the stable event identity.
    #[must_use]
    pub const fn event_id(&self) -> &EventId {
        &self.event_id
    }

    /// Returns the owning run aggregate.
    #[must_use]
    pub const fn run_id(&self) -> &RunId {
        &self.run_id
    }

    /// Returns the authoritative per-run event sequence.
    #[must_use]
    pub const fn sequence(&self) -> RunSequence {
        self.sequence
    }

    /// Returns the boundary-clock timestamp recorded with the fact.
    #[must_use]
    pub const fn occurred_at(&self) -> TimestampMillis {
        self.occurred_at
    }

    /// Returns the closed event fact.
    #[must_use]
    pub const fn kind(&self) -> &RunEventKind {
        &self.kind
    }

    /// Returns the canonical envelope checksum.
    #[must_use]
    pub const fn checksum(&self) -> &IntegrityDigest {
        &self.checksum
    }

    /// Encodes deterministic recursively key-sorted compact JSON.
    pub fn to_canonical_json(&self) -> Result<Vec<u8>, PersistenceError> {
        canonical_json_bytes(self, MAX_EVENT_DOCUMENT_BYTES)
    }

    /// Bounds-checks, duplicate-key-checks, version-checks, decodes, and verifies JSON.
    ///
    /// Future schemas, malformed fields, and checksum failures are returned explicitly;
    /// none are interpreted as an absent event.
    pub fn from_json(bytes: &[u8]) -> Result<Self, PersistenceError> {
        if bytes.len() > MAX_EVENT_DOCUMENT_BYTES {
            return Err(PersistenceError::Bounds {
                location: "event_document",
                reason: format!("exceeds {MAX_EVENT_DOCUMENT_BYTES} bytes"),
            });
        }
        let value = parse_json_without_duplicates(bytes)?;
        validate_value(&value, "$", 0)?;
        let version = value
            .get("schema_version")
            .and_then(Value::as_u64)
            .and_then(|value| u32::try_from(value).ok())
            .ok_or_else(|| {
                PersistenceError::InvalidDocument(
                    "run event requires a numeric u32 schema_version".to_owned(),
                )
            })?;
        if version != RUN_EVENT_SCHEMA_VERSION_V1 {
            return Err(PersistenceError::UnsupportedVersion {
                document: "run_event",
                found: version,
                supported: RUN_EVENT_SCHEMA_VERSION_V1,
            });
        }
        let wire: RunEventEnvelopeWire = serde_json::from_value(value)?;
        wire.kind.validate_for_run(&wire.run_id)?;
        if wire.sequence == RunSequence::ZERO {
            return Err(PersistenceError::InvalidDocument(
                "event sequence must be greater than zero".to_owned(),
            ));
        }
        let expected = calculate_event_checksum(
            wire.schema_version,
            &wire.event_id,
            &wire.run_id,
            wire.sequence,
            wire.occurred_at,
            &wire.kind,
        )?;
        if expected != wire.checksum {
            return Err(PersistenceError::Corruption(format!(
                "event {} checksum mismatch: expected {expected}, stored {}",
                wire.event_id, wire.checksum
            )));
        }
        Ok(Self {
            schema_version: wire.schema_version,
            event_id: wire.event_id,
            run_id: wire.run_id,
            sequence: wire.sequence,
            occurred_at: wire.occurred_at,
            kind: wire.kind,
            checksum: wire.checksum,
        })
    }
}

fn calculate_event_checksum(
    schema_version: u32,
    event_id: &EventId,
    run_id: &RunId,
    sequence: RunSequence,
    occurred_at: TimestampMillis,
    kind: &RunEventKind,
) -> Result<IntegrityDigest, PersistenceError> {
    let input = EventChecksumInput {
        domain: EVENT_CHECKSUM_DOMAIN,
        schema_version,
        event_id,
        run_id,
        sequence,
        occurred_at,
        kind,
    };
    let bytes = canonical_json_bytes(&input, MAX_EVENT_DOCUMENT_BYTES)?;
    Ok(IntegrityDigest::hash(&bytes))
}

pub(crate) fn canonical_json_bytes<T: Serialize>(
    value: &T,
    maximum_bytes: usize,
) -> Result<Vec<u8>, PersistenceError> {
    let mut value = serde_json::to_value(value)?;
    validate_value(&value, "$", 0)?;
    sort_value(&mut value);
    let bytes = serde_json::to_vec(&value)?;
    if bytes.len() > maximum_bytes {
        return Err(PersistenceError::Bounds {
            location: "document",
            reason: format!("canonical JSON exceeds {maximum_bytes} bytes"),
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
            let previous = std::mem::take(map);
            let mut entries: Vec<_> = previous.into_iter().collect();
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

fn validate_value(value: &Value, location: &str, depth: usize) -> Result<(), PersistenceError> {
    if depth > MAX_DOCUMENT_DEPTH {
        return Err(PersistenceError::Bounds {
            location: "document.depth",
            reason: format!("{location} exceeds depth {MAX_DOCUMENT_DEPTH}"),
        });
    }
    match value {
        Value::String(text) if text.len() > MAX_STRING_BYTES => Err(PersistenceError::Bounds {
            location: "document.string",
            reason: format!("{location} exceeds {MAX_STRING_BYTES} bytes"),
        }),
        Value::Array(values) => {
            if values.len() > MAX_CONTAINER_ITEMS {
                return Err(PersistenceError::Bounds {
                    location: "document.array",
                    reason: format!("{location} exceeds {MAX_CONTAINER_ITEMS} entries"),
                });
            }
            for (index, child) in values.iter().enumerate() {
                validate_value(child, &format!("{location}[{index}]"), depth + 1)?;
            }
            Ok(())
        }
        Value::Object(values) => {
            if values.len() > MAX_CONTAINER_ITEMS {
                return Err(PersistenceError::Bounds {
                    location: "document.object",
                    reason: format!("{location} exceeds {MAX_CONTAINER_ITEMS} entries"),
                });
            }
            for (key, child) in values {
                if key.len() > MAX_STRING_BYTES {
                    return Err(PersistenceError::Bounds {
                        location: "document.key",
                        reason: format!("key below {location} exceeds {MAX_STRING_BYTES} bytes"),
                    });
                }
                validate_value(child, &format!("{location}.{key}"), depth + 1)?;
            }
            Ok(())
        }
        _ => Ok(()),
    }
}

struct DuplicateCheckedValue(Value);

impl<'de> Deserialize<'de> for DuplicateCheckedValue {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct CheckedVisitor;

        impl<'de> Visitor<'de> for CheckedVisitor {
            type Value = DuplicateCheckedValue;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("a JSON value without duplicate object keys")
            }

            fn visit_bool<E>(self, value: bool) -> Result<Self::Value, E> {
                Ok(DuplicateCheckedValue(Value::Bool(value)))
            }

            fn visit_i64<E>(self, value: i64) -> Result<Self::Value, E> {
                Ok(DuplicateCheckedValue(Value::Number(value.into())))
            }

            fn visit_u64<E>(self, value: u64) -> Result<Self::Value, E> {
                Ok(DuplicateCheckedValue(Value::Number(value.into())))
            }

            fn visit_f64<E>(self, value: f64) -> Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                serde_json::Number::from_f64(value)
                    .map(Value::Number)
                    .map(DuplicateCheckedValue)
                    .ok_or_else(|| E::custom("non-finite JSON number"))
            }

            fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                Ok(DuplicateCheckedValue(Value::String(value.to_owned())))
            }

            fn visit_string<E>(self, value: String) -> Result<Self::Value, E> {
                Ok(DuplicateCheckedValue(Value::String(value)))
            }

            fn visit_none<E>(self) -> Result<Self::Value, E> {
                Ok(DuplicateCheckedValue(Value::Null))
            }

            fn visit_unit<E>(self) -> Result<Self::Value, E> {
                Ok(DuplicateCheckedValue(Value::Null))
            }

            fn visit_some<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
            where
                D: Deserializer<'de>,
            {
                DuplicateCheckedValue::deserialize(deserializer)
            }

            fn visit_seq<A>(self, mut sequence: A) -> Result<Self::Value, A::Error>
            where
                A: SeqAccess<'de>,
            {
                let mut values = Vec::new();
                while let Some(DuplicateCheckedValue(value)) = sequence.next_element()? {
                    values.push(value);
                }
                Ok(DuplicateCheckedValue(Value::Array(values)))
            }

            fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
            where
                A: MapAccess<'de>,
            {
                let mut keys = BTreeSet::new();
                let mut values = Map::new();
                while let Some(key) = map.next_key::<String>()? {
                    if !keys.insert(key.clone()) {
                        return Err(serde::de::Error::custom(format!(
                            "duplicate JSON object key '{key}'"
                        )));
                    }
                    let DuplicateCheckedValue(value) = map.next_value()?;
                    values.insert(key, value);
                }
                Ok(DuplicateCheckedValue(Value::Object(values)))
            }
        }

        deserializer.deserialize_any(CheckedVisitor)
    }
}

pub(crate) fn parse_json_without_duplicates(bytes: &[u8]) -> Result<Value, PersistenceError> {
    let mut deserializer = serde_json::Deserializer::from_slice(bytes);
    let value = DuplicateCheckedValue::deserialize(&mut deserializer)?.0;
    deserializer.end()?;
    Ok(value)
}
