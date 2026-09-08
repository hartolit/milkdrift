//! Keep shared document checks consistent across Milkdrift's schema owners.
//!
//! A saved request must have one meaning and fit the reader's limits. These helpers
//! reject ambiguous JSON, bound its structure, and give writers consistent bytes.
//! Application callers use their domain's document reader; document implementers compose
//! the helpers below with their own versions, fields, constructors, and total-byte limits.
//!
//! [`preflight_json_structure`] limits structure before tree allocation, while
//! [`parse_json_without_duplicates`] and [`validate_json_value`] check parsed input.
//! [`canonical_json_bytes`] supplies compact, recursively key-sorted output, preserving
//! array order. The document owner decides what the resulting value means.
//!
//! This example uses illustrative limits to read and re-encode a small value. Its error
//! mapping is deliberately local; a production owner maps failures to its contract errors.
//!
//! ```
//! use milkdrift_contracts::{
//!     JsonLimits, canonical_json_bytes, parse_json_without_duplicates,
//!     preflight_json_structure, validate_json_value,
//! };
//!
//! let limits = JsonLimits {
//!     maximum_depth: 4,
//!     maximum_string_bytes: 64,
//!     maximum_key_bytes: 32,
//!     maximum_container_items: 8,
//! };
//! let input = br#"{"z":[2,1],"a":"ok"}"#;
//! let maximum_document_bytes = 256;
//! if input.len() > maximum_document_bytes {
//!     return Err("document too large".to_owned());
//! }
//! preflight_json_structure(input, limits).map_err(|error| format!("{error:?}"))?;
//! let value = parse_json_without_duplicates(input).map_err(|error| error.to_string())?;
//! validate_json_value(&value, limits).map_err(|error| format!("{error:?}"))?;
//! let bytes = canonical_json_bytes(&value, limits).map_err(|error| format!("{error:?}"))?;
//! assert!(bytes.len() <= maximum_document_bytes);
//! assert_eq!(bytes, br#"{"a":"ok","z":[2,1]}"#);
//! assert!(parse_json_without_duplicates(br#"{"a":1,"a":2}"#).is_err());
//! # Ok::<(), String>(())
//! ```
//!
//! [`validated_string_type!`] and [`deserialize_via!`] reuse the owner's constructors
//! during deserialization. [`is_canonical_blake3_digest`] and [`truncate_utf8`] provide
//! the shared lexical checks and byte-bounded text handling.

use std::{collections::BTreeSet, fmt};

use serde::{
    Deserialize, Deserializer, Serialize,
    de::{MapAccess, SeqAccess, Visitor},
};
use serde_json::{Map, Value};

mod text;

pub use text::{is_canonical_blake3_digest, truncate_utf8};

/// Define a string wrapper whose constructor and Serde reader use the same caller-owned check.
///
/// Use it for domain identities whose accepted text must be preserved exactly. The
/// supplied validator receives the text and generated type name as `(&str, &'static str)`
/// and returns `Result<(), E>`. `E` must implement [`fmt::Display`] for Serde errors.
///
/// The wrapper owns a private `String`, exposes `new` and `as_str`, and compares, hashes,
/// displays, and serializes the stored text. Display and Debug are unredacted, so do not
/// use it for secret values. Add domain-specific `FromStr` or `TryFrom` implementations
/// separately when callers need them.
///
/// Invoke at module scope in the domain owner, with a dependency named `serde`.
///
/// ```
/// use milkdrift_contracts::validated_string_type;
///
/// validated_string_type! {
///     /// Name accepted by this example's caller.
///     pub struct ShortName;
///     error = &'static str;
///     validate = |value: &str, _kind: &'static str| {
///         if value.is_empty() || value.len() > 8 {
///             Err("name must contain 1..=8 UTF-8 bytes")
///         } else {
///             Ok(())
///         }
///     };
/// }
///
/// let name = ShortName::new("review")?;
/// assert_eq!(name.as_str(), "review");
/// assert_eq!(serde_json::to_string(&name)?, r#""review""#);
/// assert_eq!(serde_json::from_str::<ShortName>(r#""review""#)?, name);
/// assert!(ShortName::new("").is_err());
/// assert!(serde_json::from_str::<ShortName>(r#""""#).is_err());
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
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
            /// Owns the supplied text after applying this type's validator.
            ///
            /// Returns the validator's error if the text is refused; accepted text is
            /// stored exactly as supplied, without trimming or normalization.
            pub fn new(value: impl Into<String>) -> Result<Self, $error> {
                let value = value.into();
                ($validator)(&value, stringify!($name))?;
                Ok(Self(value))
            }

            /// Borrows the stored, validated text without allocating.
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

/// Implement Serde deserialization by reading a wire type and converting it to a validated type.
///
/// Use this when deriving `Deserialize` on the public type would bypass its constructor.
/// The wire type describes accepted input fields, often with `#[serde(deny_unknown_fields)]`.
/// After it deserializes, the conversion expression receives that owned value and must
/// return `Result<Target, E>`, where `E` implements [`fmt::Display`]. Wire decoding errors
/// stop before conversion; conversion errors become custom Serde errors.
///
/// Invoke at module scope with a dependency named `serde`. The macro implements only
/// `Deserialize`; use the document reader's JSON checks for byte bounds and duplicate keys.
///
/// ```
/// use milkdrift_contracts::deserialize_via;
/// use serde::Deserialize;
///
/// #[derive(Deserialize)]
/// #[serde(deny_unknown_fields)]
/// struct VersionWire { version: u32 }
///
/// #[derive(Debug, PartialEq)]
/// struct Version(u32);
/// impl Version {
///     fn new(value: u32) -> Result<Self, &'static str> {
///         if value == 0 { Err("version must be nonzero") } else { Ok(Self(value)) }
///     }
/// }
/// deserialize_via!(Version, VersionWire, |wire| Self::new(wire.version));
///
/// assert_eq!(serde_json::from_str::<Version>(r#"{"version":1}"#)?, Version(1));
/// assert!(serde_json::from_str::<Version>(r#"{"version":0}"#).is_err());
/// assert!(serde_json::from_str::<Version>(r#"{"version":1,"extra":true}"#).is_err());
/// # Ok::<(), serde_json::Error>(())
/// ```
#[macro_export]
macro_rules! deserialize_via {
    ($target:ty, $wire:ty, |$value:ident| $conversion:expr $(,)?) => {
        impl<'de> ::serde::Deserialize<'de> for $target {
            fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
            where
                D: ::serde::Deserializer<'de>,
            {
                let $value = <$wire as ::serde::Deserialize>::deserialize(deserializer)?;
                ($conversion).map_err(::serde::de::Error::custom)
            }
        }
    };
}

/// Choose the depth, text size, and per-container limits for one document family.
///
/// Construct this with all four fields; there is no shared default. Maxima are inclusive,
/// and zero is a real limit, never an unlimited sentinel. There is no separate budget for
/// total input/output bytes or tree nodes, and no numeric-token length limit. A document
/// owner checks total bytes separately, before parsing and after encoding.
///
/// [`validate_json_value`] checks decoded values. [`preflight_json_structure`] uses the
/// same fields on the byte representation, with the differences described below. Both
/// checks are needed when a reader uses preflight; success there is not full validation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct JsonLimits {
    /// Maximum value depth: the root is zero and every array element or object value
    /// adds one, including scalars. At zero, `[]` is allowed but `[0]` fails decoded
    /// validation. Preflight checks only container openings, so `[0]` passes that scan.
    pub maximum_depth: usize,
    /// Maximum UTF-8 bytes in each decoded string value, excluding quotes.
    /// Preflight counts the encoded bytes inside quotes, including escape syntax:
    /// `"\u0061"` counts as six bytes there and one byte after decoding.
    pub maximum_string_bytes: usize,
    /// Maximum UTF-8 bytes in each decoded object key, excluding quotes.
    /// Preflight counts escape syntax as it does for string values.
    pub maximum_key_bytes: usize,
    /// Maximum entries in each object or elements in each array, checked separately
    /// for every container. Zero permits only empty containers.
    pub maximum_container_items: usize,
}

/// Which [`JsonLimits`] maximum caused a refusal, for mapping into the owner's error type.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum JsonBoundKind {
    /// Value depth, or container-opening depth during preflight.
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

/// A structural check's first reported refusal, with its category and configured maximum.
///
/// Returned by [`validate_json_value`] and [`preflight_json_structure`], or carried by
/// [`CanonicalJsonError::Bounds`]. Inspect it through the accessors and map it into the
/// owning document's error type. It does not retain the rejected value or actual size.
/// Preflight has no decoded path and reports `$`; decoded validation locates the value
/// or container as described by [`Self::path`].
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct JsonBoundViolation {
    path: String,
    kind: JsonBoundKind,
    maximum: usize,
}

impl JsonBoundViolation {
    /// Diagnostic location, rooted at `$`, with `.key` and `[index]` steps for decoded
    /// values. A key-length violation points to its containing object; preflight always
    /// returns `$`. Keys are inserted without escaping, so this is not a JSON Pointer
    /// or an unambiguous address for keys containing dots or brackets.
    #[must_use]
    pub fn path(&self) -> &str {
        &self.path
    }

    /// Structural category that violated the limit.
    #[must_use]
    pub const fn kind(&self) -> JsonBoundKind {
        self.kind
    }

    /// The configured maximum, in depth, bytes, or entries according to [`Self::kind`]
    /// and the check used. This is the allowed limit, not the measured offending size.
    #[must_use]
    pub const fn maximum(&self) -> usize {
        self.maximum
    }
}

/// Encode a serializable value as compact JSON with recursively sorted object keys.
///
/// First converts `value` to a [`serde_json::Value`], checks it with
/// [`validate_json_value`], sorts keys by Rust string ordering, then serializes the tree.
/// Array order is preserved, including arrays containing objects. The caller's value is
/// not mutated. JSON string/number encoding follows Serde JSON; this function does not
/// add Unicode normalization, a schema envelope, or a digest.
///
/// The temporary tree and returned bytes are allocated before any caller-owned total-byte
/// check. Use the [`crate` example](crate) for the read/write sequence, and check the
/// returned length against the document owner's limit. This function cannot recover
/// duplicate keys already lost while constructing a `Value`; parse input bytes with
/// [`parse_json_without_duplicates`] first when duplicate rejection is required.
///
/// # Errors
///
/// Returns [`CanonicalJsonError::Json`] if conversion or serialization fails, or
/// [`CanonicalJsonError::Bounds`] if the converted value exceeds a supplied limit.
/// Successful serialization does not itself validate the value's domain meaning.
pub fn canonical_json_bytes<T: Serialize>(
    value: &T,
    limits: JsonLimits,
) -> Result<Vec<u8>, CanonicalJsonError> {
    let mut value = serde_json::to_value(value).map_err(CanonicalJsonError::Json)?;
    validate_json_value(&value, limits).map_err(CanonicalJsonError::Bounds)?;
    sort_value(&mut value);
    serde_json::to_vec(&value).map_err(CanonicalJsonError::Json)
}

/// Parse exactly one JSON value, refusing duplicate object keys at every depth.
///
/// Object keys are compared after JSON escape decoding, so `"a"` and `"\u0061"` count
/// as duplicates in the same object even when their values agree. Equal keys in separate
/// objects are allowed. Leading and trailing whitespace is accepted; a second value is not.
///
/// This allocates a value tree without caller-supplied structural or total-byte limits.
/// Check the input length and use [`preflight_json_structure`] before this call when
/// reading bounded documents, then apply [`validate_json_value`] and the owner's schema
/// and semantic checks. Parsing alone accepts unknown fields and unsupported versions.
///
/// # Errors
///
/// Returns a Serde JSON error for invalid JSON (including its parser recursion limit),
/// a duplicate decoded key, or trailing non-whitespace input.
///
/// ```
/// use milkdrift_contracts::parse_json_without_duplicates;
///
/// assert!(parse_json_without_duplicates(br#"{"a":1,"\u0061":1}"#).is_err());
/// assert!(parse_json_without_duplicates(b"true false").is_err());
/// assert_eq!(parse_json_without_duplicates(b" true \n")?, serde_json::json!(true));
/// # Ok::<(), serde_json::Error>(())
/// ```
pub fn parse_json_without_duplicates(bytes: &[u8]) -> Result<Value, serde_json::Error> {
    let mut deserializer = serde_json::Deserializer::from_slice(bytes);
    let value = DuplicateCheckedValue::deserialize(&mut deserializer)?.0;
    deserializer.end()?;
    Ok(value)
}

/// Check an existing JSON tree against the caller's [`JsonLimits`], without changing it.
///
/// Depth includes scalar children; string and key sizes count decoded UTF-8 bytes.
/// The first violation stops traversal and supplies a diagnostic path.
///
/// Use this after duplicate-checked parsing or before encoding. It cannot detect duplicate
/// keys already discarded by another parser, and does not check document size, schema
/// versions, allowed fields, or domain meaning. Those checks remain with the caller.
///
/// # Errors
///
/// Returns a [`JsonBoundViolation`] when a value exceeds one of the supplied maxima.
///
/// ```
/// use milkdrift_contracts::{JsonBoundKind, JsonLimits, validate_json_value};
/// use serde_json::json;
///
/// let limits = JsonLimits {
///     maximum_depth: 2, maximum_string_bytes: 2,
///     maximum_key_bytes: 8, maximum_container_items: 4,
/// };
/// assert!(validate_json_value(&json!({"names": ["é"]}), limits).is_ok());
/// let error = validate_json_value(&json!({"names": ["€"]}), limits)
///     .expect_err("three UTF-8 bytes exceed the two-byte string limit");
/// assert_eq!(error.kind(), JsonBoundKind::String);
/// assert_eq!(error.path(), "$.names[0]");
/// assert_eq!(error.maximum(), 2);
/// ```
pub fn validate_json_value(value: &Value, limits: JsonLimits) -> Result<(), JsonBoundViolation> {
    validate_value(value, "$", 0, limits)
}

/// Scan JSON bytes for structural excess before allocating a decoded value tree.
///
/// Call this after the owner's input-byte check and before [`parse_json_without_duplicates`].
/// The scan keeps a stack of open containers, counts their entries, and counts bytes inside
/// quoted keys and strings. It does not decode escape sequences: `"\u0061"` uses six bytes
/// against the string limit even though parsing produces one byte. Such input can be
/// refused here while an equivalent unescaped spelling passes.
///
/// Depth is checked on container openings only; scalar children need the later
/// [`validate_json_value`] check. This is not a syntax checker: malformed, incomplete, or
/// duplicate-bearing input may pass. Always follow success with duplicate-checked parsing,
/// decoded-value validation, and the owner's schema and semantic checks.
///
/// # Errors
///
/// Returns the first detected [`JsonBoundViolation`], with path `$`. No total-byte limit
/// or numeric-token length limit is supplied by this scan.
///
/// ```
/// use milkdrift_contracts::{
///     JsonLimits, parse_json_without_duplicates, preflight_json_structure, validate_json_value,
/// };
///
/// let limits = JsonLimits {
///     maximum_depth: 0, maximum_string_bytes: 1,
///     maximum_key_bytes: 8, maximum_container_items: 2,
/// };
/// let escaped = br#""\u0061""#;
/// assert!(preflight_json_structure(escaped, limits).is_err());
/// assert!(validate_json_value(&parse_json_without_duplicates(escaped)?, limits).is_ok());
/// assert!(preflight_json_structure(b"[0]", limits).is_ok());
/// assert!(validate_json_value(&parse_json_without_duplicates(b"[0]")?, limits).is_err());
/// assert!(preflight_json_structure(b"[", limits).is_ok());
/// assert!(parse_json_without_duplicates(b"[").is_err());
/// # Ok::<(), serde_json::Error>(())
/// ```
pub fn preflight_json_structure(
    bytes: &[u8],
    limits: JsonLimits,
) -> Result<(), JsonBoundViolation> {
    #[derive(Clone, Copy)]
    struct Frame {
        commas: usize,
        has_content: bool,
    }

    let mut frames: Vec<Frame> = Vec::new();
    let mut in_string = false;
    let mut escaped = false;
    let mut string_bytes = 0usize;
    let mut index = 0usize;
    while index < bytes.len() {
        let byte = bytes[index];
        if in_string {
            // Count wire bytes, including escape spelling, without allocating decoded text.
            if escaped {
                escaped = false;
                string_bytes = string_bytes.saturating_add(1);
            } else if byte == b'\\' {
                escaped = true;
                string_bytes = string_bytes.saturating_add(1);
            } else if byte == b'"' {
                in_string = false;
                let mut next = index.saturating_add(1);
                while next < bytes.len() && bytes[next].is_ascii_whitespace() {
                    next = next.saturating_add(1);
                }
                let (kind, maximum) = if bytes.get(next) == Some(&b':') {
                    (JsonBoundKind::Key, limits.maximum_key_bytes)
                } else {
                    (JsonBoundKind::String, limits.maximum_string_bytes)
                };
                if string_bytes > maximum {
                    return Err(violation("$", kind, maximum));
                }
            } else {
                string_bytes = string_bytes.saturating_add(1);
            }
            index = index.saturating_add(1);
            continue;
        }
        match byte {
            b'"' => {
                if let Some(frame) = frames.last_mut() {
                    frame.has_content = true;
                }
                in_string = true;
                escaped = false;
                string_bytes = 0;
            }
            b'{' | b'[' => {
                if let Some(frame) = frames.last_mut() {
                    frame.has_content = true;
                }
                let depth = frames.len();
                if depth > limits.maximum_depth {
                    return Err(violation("$", JsonBoundKind::Depth, limits.maximum_depth));
                }
                frames.push(Frame {
                    commas: 0,
                    has_content: false,
                });
            }
            b'}' | b']' => {
                if let Some(frame) = frames.pop() {
                    let items = if frame.has_content {
                        frame.commas.saturating_add(1)
                    } else {
                        0
                    };
                    if items > limits.maximum_container_items {
                        let kind = if byte == b'}' {
                            JsonBoundKind::Object
                        } else {
                            JsonBoundKind::Array
                        };
                        return Err(violation("$", kind, limits.maximum_container_items));
                    }
                }
            }
            b',' => {
                if let Some(frame) = frames.last_mut() {
                    frame.commas = frame.commas.saturating_add(1);
                }
            }
            byte if byte.is_ascii_whitespace() => {}
            _ => {
                if let Some(frame) = frames.last_mut() {
                    frame.has_content = true;
                }
            }
        }
        index = index.saturating_add(1);
    }
    Ok(())
}

/// Distinguish serialization failures from structural-limit refusals during encoding.
///
/// [`canonical_json_bytes`] returns this for the document owner to map into its own error type.
#[derive(Debug)]
pub enum CanonicalJsonError {
    /// Conversion to a JSON value or encoding of that value failed; retains Serde's error.
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
            // Check the decoded key before insertion could replace an earlier value.
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
        preflight_json_structure, validate_json_value,
    };

    const LIMITS: JsonLimits = JsonLimits {
        maximum_depth: 4,
        maximum_string_bytes: 8,
        maximum_key_bytes: 8,
        maximum_container_items: 2,
    };

    #[derive(serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    struct PositiveWire {
        value: u32,
    }

    #[derive(Debug, Eq, PartialEq)]
    struct Positive(u32);

    crate::deserialize_via!(Positive, PositiveWire, |wire| {
        (wire.value > 0)
            .then_some(Positive(wire.value))
            .ok_or("value must be positive")
    });

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
        // The scalar at depth five must fail even though its parent is at the depth limit.
        let result = validate_json_value(&json!({"a": {"b": {"c": {"d": {"e": 1}}}}}), LIMITS);
        assert!(result.is_err());
        if let Err(error) = result {
            assert_eq!(error.kind(), JsonBoundKind::Depth);
            assert_eq!(error.path(), "$.a.b.c.d.e");
            assert_eq!(error.maximum(), 4);
        }
    }

    #[test]
    fn lexical_preflight_rejects_large_containers_before_value_allocation() {
        let array = preflight_json_structure(br#"[1,2,3]"#, LIMITS);
        assert!(
            array.is_err(),
            "three array entries passed the preflight bound"
        );
        if let Err(array) = array {
            assert_eq!(array.kind(), JsonBoundKind::Array);
        }
        let string = preflight_json_structure(br#"{"key":"123456789"}"#, LIMITS);
        assert!(
            string.is_err(),
            "oversized string passed the preflight bound"
        );
        if let Err(string) = string {
            assert_eq!(string.kind(), JsonBoundKind::String);
        }
        assert!(preflight_json_structure(br#"{"a":[1,2]}"#, LIMITS).is_ok());
    }

    #[test]
    fn validated_deserialize_uses_wire_shape_and_owner_conversion() {
        assert_eq!(
            serde_json::from_str::<Positive>(r#"{"value":1}"#).ok(),
            Some(Positive(1))
        );
        assert!(serde_json::from_str::<Positive>(r#"{"value":0}"#).is_err());
        assert!(serde_json::from_str::<Positive>(r#"{"value":1,"extra":true}"#).is_err());
    }
}
