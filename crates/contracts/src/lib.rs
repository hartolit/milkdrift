//! Low-level deterministic JSON mechanics shared by durable contract owners.
//!
//! This crate deliberately owns no workflow identities, schema versions, error policy,
//! or business limits. Domain crates supply those policies explicitly and map violations
//! into their own stable error classifications.

use std::{collections::BTreeSet, fmt};

use serde::{
    Deserialize, Deserializer, Serialize,
    de::{MapAccess, SeqAccess, Visitor},
};
use serde_json::{Map, Value};

/// Defines a private-storage validated string newtype while leaving validation and errors
/// in the invoking domain.
///
/// The validator expression receives `(&str, &'static str)` and returns the declared error.
/// Domain crates may add conversions that are meaningful at their own API boundary.
#[macro_export]
macro_rules! validated_string_type {
    (
        $(#[$meta:meta])*
        $visibility:vis struct $name:ident;
        error = $error:ty;
        validate = $validator:expr;
    ) => {
        $(#[$meta])*
        #[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
        $visibility struct $name(String);

        impl $name {
            /// Constructs and validates the identity.
            pub fn new(value: impl Into<String>) -> Result<Self, $error> {
                let value = value.into();
                ($validator)(&value, stringify!($name))?;
                Ok(Self(value))
            }

            /// Returns the validated identity text.
            #[must_use]
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl ::std::fmt::Display for $name {
            fn fmt(&self, formatter: &mut ::std::fmt::Formatter<'_>) -> ::std::fmt::Result {
                formatter.write_str(&self.0)
            }
        }

        impl ::serde::Serialize for $name {
            fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
            where
                S: ::serde::Serializer,
            {
                serializer.serialize_str(&self.0)
            }
        }

        impl<'de> ::serde::Deserialize<'de> for $name {
            fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
            where
                D: ::serde::Deserializer<'de>,
            {
                let value = <String as ::serde::Deserialize>::deserialize(deserializer)?;
                Self::new(value).map_err(::serde::de::Error::custom)
            }
        }
    };
}

/// Structural JSON bounds whose meanings are shared across contract domains.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct JsonLimits {
    /// Maximum recursive container depth, with the root at depth zero.
    pub maximum_depth: usize,
    /// Maximum UTF-8 byte length of a JSON string value.
    pub maximum_string_bytes: usize,
    /// Maximum UTF-8 byte length of an object key.
    pub maximum_key_bytes: usize,
    /// Maximum number of values in an array or entries in an object.
    pub maximum_container_items: usize,
}

/// The structural category that exceeded a configured JSON limit.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum JsonBoundKind {
    /// Recursive container nesting depth.
    Depth,
    /// String value byte length.
    String,
    /// Object key byte length.
    Key,
    /// Array item count.
    Array,
    /// Object entry count.
    Object,
}

/// One precisely located structural JSON bound violation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct JsonBoundViolation {
    path: String,
    kind: JsonBoundKind,
    maximum: usize,
}

impl JsonBoundViolation {
    /// JSON-like path of the value or container that violated the limit.
    #[must_use]
    pub fn path(&self) -> &str {
        &self.path
    }

    /// Structural category that violated the limit.
    #[must_use]
    pub const fn kind(&self) -> JsonBoundKind {
        self.kind
    }

    /// Configured maximum for the violated category.
    #[must_use]
    pub const fn maximum(&self) -> usize {
        self.maximum
    }
}

/// Serializes a value to recursively key-sorted compact JSON after structural validation.
pub fn canonical_json_bytes<T: Serialize>(
    value: &T,
    limits: JsonLimits,
) -> Result<Vec<u8>, CanonicalJsonError> {
    let mut value = serde_json::to_value(value).map_err(CanonicalJsonError::Json)?;
    validate_json_value(&value, limits).map_err(CanonicalJsonError::Bounds)?;
    sort_value(&mut value);
    serde_json::to_vec(&value).map_err(CanonicalJsonError::Json)
}

/// Parses exactly one JSON value while rejecting duplicate object keys at every depth.
pub fn parse_json_without_duplicates(bytes: &[u8]) -> Result<Value, serde_json::Error> {
    let mut deserializer = serde_json::Deserializer::from_slice(bytes);
    let value = DuplicateCheckedValue::deserialize(&mut deserializer)?.0;
    deserializer.end()?;
    Ok(value)
}

/// Validates a parsed JSON value against explicit structural limits.
pub fn validate_json_value(value: &Value, limits: JsonLimits) -> Result<(), JsonBoundViolation> {
    validate_value(value, "$", 0, limits)
}

/// Failure while producing canonical JSON.
#[derive(Debug)]
pub enum CanonicalJsonError {
    /// Serialization failed.
    Json(serde_json::Error),
    /// The serialized value exceeded a structural bound.
    Bounds(JsonBoundViolation),
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

fn validate_value(
    value: &Value,
    path: &str,
    depth: usize,
    limits: JsonLimits,
) -> Result<(), JsonBoundViolation> {
    if depth > limits.maximum_depth {
        return Err(violation(path, JsonBoundKind::Depth, limits.maximum_depth));
    }
    match value {
        Value::String(text) if text.len() > limits.maximum_string_bytes => Err(violation(
            path,
            JsonBoundKind::String,
            limits.maximum_string_bytes,
        )),
        Value::Array(values) => {
            if values.len() > limits.maximum_container_items {
                return Err(violation(
                    path,
                    JsonBoundKind::Array,
                    limits.maximum_container_items,
                ));
            }
            for (index, child) in values.iter().enumerate() {
                validate_value(child, &format!("{path}[{index}]"), depth + 1, limits)?;
            }
            Ok(())
        }
        Value::Object(values) => {
            if values.len() > limits.maximum_container_items {
                return Err(violation(
                    path,
                    JsonBoundKind::Object,
                    limits.maximum_container_items,
                ));
            }
            for (key, child) in values {
                if key.len() > limits.maximum_key_bytes {
                    return Err(violation(
                        path,
                        JsonBoundKind::Key,
                        limits.maximum_key_bytes,
                    ));
                }
                validate_value(child, &format!("{path}.{key}"), depth + 1, limits)?;
            }
            Ok(())
        }
        _ => Ok(()),
    }
}

fn violation(path: &str, kind: JsonBoundKind, maximum: usize) -> JsonBoundViolation {
    JsonBoundViolation {
        path: path.to_owned(),
        kind,
        maximum,
    }
}

struct DuplicateCheckedValue(Value);

impl<'de> Deserialize<'de> for DuplicateCheckedValue {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_any(DuplicateCheckedVisitor)
    }
}

struct DuplicateCheckedVisitor;

impl<'de> Visitor<'de> for DuplicateCheckedVisitor {
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

    fn visit_str<E>(self, value: &str) -> Result<Self::Value, E> {
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

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{
        JsonBoundKind, JsonLimits, canonical_json_bytes, parse_json_without_duplicates,
        validate_json_value,
    };

    const LIMITS: JsonLimits = JsonLimits {
        maximum_depth: 4,
        maximum_string_bytes: 8,
        maximum_key_bytes: 8,
        maximum_container_items: 2,
    };

    #[test]
    fn canonical_ordering_and_duplicate_rejection_are_recursive() {
        let value = json!({"z": [{"b": 2, "a": 1}], "a": true});
        assert_eq!(
            canonical_json_bytes(&value, LIMITS).ok().as_deref(),
            Some(br#"{"a":true,"z":[{"a":1,"b":2}]}"#.as_slice())
        );
        assert!(parse_json_without_duplicates(br#"{"a":{"b":1,"b":2}}"#).is_err());
    }

    #[test]
    fn structural_bounds_report_kind_path_and_limit() {
        let result = validate_json_value(&json!({"a": {"b": {"c": {"d": {"e": 1}}}}}), LIMITS);
        assert!(result.is_err());
        if let Err(error) = result {
            assert_eq!(error.kind(), JsonBoundKind::Depth);
            assert_eq!(error.path(), "$.a.b.c.d.e");
            assert_eq!(error.maximum(), 4);
        }
    }
}
