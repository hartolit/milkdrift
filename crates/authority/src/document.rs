use serde::Serialize;

use crate::AuthorityError;

/// Version of the immutable authority grant contract.
pub const AUTHORITY_GRANT_SCHEMA_VERSION_V2: u32 = 2;
/// Maximum canonical bytes in a grant or decision document.
pub const MAX_AUTHORITY_DOCUMENT_BYTES: usize = 262_144;

const LIMITS: milkdrift_contracts::JsonLimits = milkdrift_contracts::JsonLimits {
    maximum_depth: 48,
    maximum_string_bytes: 8_192,
    maximum_key_bytes: 256,
    maximum_container_items: 2_048,
};

pub(crate) fn canonical_json<T: Serialize>(value: &T) -> Result<Vec<u8>, AuthorityError> {
    let bytes =
        milkdrift_contracts::canonical_json_bytes(value, LIMITS).map_err(|error| match error {
            milkdrift_contracts::CanonicalJsonError::Json(error) => {
                AuthorityError::Json(error.to_string())
            }
            milkdrift_contracts::CanonicalJsonError::Bounds(violation) => AuthorityError::Bounds {
                location: "authority.document",
                reason: format!(
                    "{} exceeds {:?} limit {}",
                    violation.path(),
                    violation.kind(),
                    violation.maximum()
                ),
            },
        })?;
    if bytes.len() > MAX_AUTHORITY_DOCUMENT_BYTES {
        return Err(AuthorityError::Bounds {
            location: "authority.document",
            reason: format!("canonical JSON exceeds {MAX_AUTHORITY_DOCUMENT_BYTES} bytes"),
        });
    }
    Ok(bytes)
}
