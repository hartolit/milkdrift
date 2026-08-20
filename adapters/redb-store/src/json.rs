use serde::{Deserialize, Serialize, de::DeserializeOwned};

use milkdrift_persistence::PersistenceError;

const MAX_INTERNAL_DOCUMENT_BYTES: usize = 8_388_608;
const INTERNAL_DOCUMENT_SCHEMA_VERSION: u32 = 1;
const INTERNAL_DOCUMENT_DIGEST_DOMAIN: &[u8] = b"milkdrift.redb.internal-document.v1\0";

#[derive(Serialize)]
#[serde(deny_unknown_fields)]
struct InternalDocumentEnvelopeRef<'a, T> {
    schema_version: u32,
    family: &'static str,
    checksum: String,
    payload: &'a T,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct InternalDocumentEnvelope<T> {
    schema_version: u32,
    family: String,
    checksum: String,
    payload: T,
}

pub(crate) fn encode<T: Serialize>(
    value: &T,
    family: &'static str,
) -> Result<Vec<u8>, PersistenceError> {
    let payload = canonical_payload(value, family, false)?;
    let envelope = InternalDocumentEnvelopeRef {
        schema_version: INTERNAL_DOCUMENT_SCHEMA_VERSION,
        family,
        checksum: checksum(family, &payload),
        payload: value,
    };
    let bytes = serde_json::to_vec(&envelope)
        .map_err(|cause| PersistenceError::InvalidDocument(format!("{family}: {cause}")))?;
    if bytes.len() > MAX_INTERNAL_DOCUMENT_BYTES {
        return Err(PersistenceError::Bounds {
            location: "storage_document",
            reason: format!("{family} envelope exceeds {MAX_INTERNAL_DOCUMENT_BYTES} bytes"),
        });
    }
    Ok(bytes)
}

pub(crate) fn decode<T: DeserializeOwned + Serialize>(
    bytes: &[u8],
    family: &'static str,
) -> Result<T, PersistenceError> {
    if bytes.len() > MAX_INTERNAL_DOCUMENT_BYTES {
        return Err(PersistenceError::Corruption(format!(
            "stored {family} exceeds {MAX_INTERNAL_DOCUMENT_BYTES} bytes"
        )));
    }
    let decoded: InternalDocumentEnvelope<T> = serde_json::from_slice(bytes).map_err(|cause| {
        PersistenceError::Corruption(format!("stored {family} envelope failed decoding: {cause}"))
    })?;
    let canonical = serde_json::to_vec(&decoded).map_err(|cause| {
        PersistenceError::Corruption(format!(
            "stored {family} envelope could not be canonically re-encoded: {cause}"
        ))
    })?;
    if canonical != bytes {
        return Err(PersistenceError::Corruption(format!(
            "stored {family} is not the exact canonical internal envelope"
        )));
    }
    if decoded.schema_version != INTERNAL_DOCUMENT_SCHEMA_VERSION {
        return Err(PersistenceError::UnsupportedVersion {
            document: family,
            found: decoded.schema_version,
            supported: INTERNAL_DOCUMENT_SCHEMA_VERSION,
        });
    }
    if decoded.family != family {
        return Err(PersistenceError::Corruption(format!(
            "stored {family} envelope names family {}",
            decoded.family
        )));
    }
    let payload = canonical_payload(&decoded.payload, family, true)?;
    if decoded.checksum != checksum(family, &payload) {
        return Err(PersistenceError::Corruption(format!(
            "stored {family} checksum does not match its canonical payload"
        )));
    }
    Ok(decoded.payload)
}

/// Converts one exact legacy canonical JSON value into the current envelope.
///
/// This is used only by the atomic document-format migration performed before a
/// store becomes available for ordinary reads or writes.
pub(crate) fn migrate_legacy(
    bytes: &[u8],
    family: &'static str,
) -> Result<Vec<u8>, PersistenceError> {
    if bytes.len() > MAX_INTERNAL_DOCUMENT_BYTES {
        return Err(PersistenceError::Corruption(format!(
            "legacy stored {family} exceeds {MAX_INTERNAL_DOCUMENT_BYTES} bytes"
        )));
    }
    let value: serde_json::Value = serde_json::from_slice(bytes).map_err(|cause| {
        PersistenceError::Corruption(format!(
            "legacy stored {family} failed decoding during migration: {cause}"
        ))
    })?;
    if value.as_object().is_some_and(|object| {
        object.contains_key("schema_version")
            && object.contains_key("family")
            && object.contains_key("checksum")
            && object.contains_key("payload")
    }) {
        return Err(PersistenceError::Corruption(format!(
            "stored {family} is already enveloped but the document-format marker is missing"
        )));
    }
    let checksum = checksum(family, bytes);
    let family = serde_json::to_string(family).map_err(|cause| {
        PersistenceError::Corruption(format!(
            "legacy stored {family} family could not be encoded: {cause}"
        ))
    })?;
    let mut migrated = format!(
        "{{\"schema_version\":{INTERNAL_DOCUMENT_SCHEMA_VERSION},\"family\":{family},\"checksum\":\"{checksum}\",\"payload\":"
    )
    .into_bytes();
    migrated.extend_from_slice(bytes);
    migrated.push(b'}');
    if migrated.len() > MAX_INTERNAL_DOCUMENT_BYTES {
        return Err(PersistenceError::Corruption(format!(
            "migrated stored document exceeds {MAX_INTERNAL_DOCUMENT_BYTES} bytes"
        )));
    }
    Ok(migrated)
}

fn canonical_payload<T: Serialize>(
    value: &T,
    family: &'static str,
    stored: bool,
) -> Result<Vec<u8>, PersistenceError> {
    serde_json::to_vec(value).map_err(|cause| {
        if stored {
            PersistenceError::Corruption(format!(
                "stored {family} payload could not be canonically re-encoded: {cause}"
            ))
        } else {
            PersistenceError::InvalidDocument(format!("{family}: {cause}"))
        }
    })
}

fn checksum(family: &str, payload: &[u8]) -> String {
    let mut hasher = blake3::Hasher::new();
    hasher.update(INTERNAL_DOCUMENT_DIGEST_DOMAIN);
    hasher.update(&(family.len() as u64).to_be_bytes());
    hasher.update(family.as_bytes());
    hasher.update(&(payload.len() as u64).to_be_bytes());
    hasher.update(payload);
    hasher.finalize().to_hex().to_string()
}

#[cfg(test)]
mod tests {
    use serde::{Deserialize, Serialize};

    use super::{decode, encode, migrate_legacy};

    #[derive(Debug, Eq, PartialEq, Deserialize, Serialize)]
    struct Example {
        first: u32,
        second: u32,
    }

    #[derive(Debug, Eq, PartialEq, Deserialize, Serialize)]
    struct Nonalphabetic {
        zebra: u32,
        alpha: u32,
    }

    #[test]
    fn internal_documents_require_exact_checked_canonical_envelopes()
    -> Result<(), Box<dyn std::error::Error>> {
        let value = Example {
            first: 1,
            second: 2,
        };
        let bytes = encode(&value, "example")?;
        assert_eq!(decode::<Example>(&bytes, "example")?, value);
        assert!(decode::<Example>(&bytes, "other example").is_err());

        let mut tampered: serde_json::Value = serde_json::from_slice(&bytes)?;
        tampered["payload"]["first"] = serde_json::json!(9);
        let tampered = serde_json::to_vec(&tampered)?;
        assert!(decode::<Example>(&tampered, "example").is_err());

        assert!(decode::<Example>(br#"{"first":1,"second":2}"#, "example").is_err());
        let migrated = migrate_legacy(br#"{"first":1,"second":2}"#, "example")?;
        assert_eq!(decode::<Example>(&migrated, "example")?, value);
        let nonalphabetic = migrate_legacy(br#"{"zebra":2,"alpha":1}"#, "nonalphabetic")?;
        assert_eq!(
            decode::<Nonalphabetic>(&nonalphabetic, "nonalphabetic")?,
            Nonalphabetic { zebra: 2, alpha: 1 }
        );
        Ok(())
    }
}
