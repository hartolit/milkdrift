use serde::{Serialize, de::DeserializeOwned};

use milkdrift_persistence::PersistenceError;

const MAX_INTERNAL_DOCUMENT_BYTES: usize = 8_388_608;

pub(crate) fn encode<T: Serialize>(
    value: &T,
    family: &'static str,
) -> Result<Vec<u8>, PersistenceError> {
    let bytes = serde_json::to_vec(value)
        .map_err(|cause| PersistenceError::InvalidDocument(format!("{family}: {cause}")))?;
    if bytes.len() > MAX_INTERNAL_DOCUMENT_BYTES {
        return Err(PersistenceError::Bounds {
            location: "storage_document",
            reason: format!("{family} exceeds {MAX_INTERNAL_DOCUMENT_BYTES} bytes"),
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
    let decoded: T = serde_json::from_slice(bytes).map_err(|cause| {
        PersistenceError::Corruption(format!("stored {family} failed decoding: {cause}"))
    })?;
    let canonical = serde_json::to_vec(&decoded).map_err(|cause| {
        PersistenceError::Corruption(format!(
            "stored {family} could not be canonically re-encoded: {cause}"
        ))
    })?;
    if canonical != bytes {
        return Err(PersistenceError::Corruption(format!(
            "stored {family} is not the exact canonical internal encoding"
        )));
    }
    Ok(decoded)
}

#[cfg(test)]
mod tests {
    use serde::{Deserialize, Serialize};

    use super::{decode, encode};

    #[derive(Debug, Eq, PartialEq, Deserialize, Serialize)]
    struct Example {
        first: u32,
        second: u32,
    }

    #[test]
    fn internal_documents_require_exact_canonical_bytes() -> Result<(), Box<dyn std::error::Error>>
    {
        let value = Example {
            first: 1,
            second: 2,
        };
        let bytes = encode(&value, "example")?;
        assert_eq!(decode::<Example>(&bytes, "example")?, value);
        assert!(decode::<Example>(br#"{"first":1,"first":9,"second":2}"#, "example").is_err());
        assert!(decode::<Example>(br#"{"second":2,"first":1}"#, "example").is_err());
        Ok(())
    }
}
