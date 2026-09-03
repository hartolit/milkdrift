use crate::PeerProtocolError;

fn validate_identity(
    value: &str,
    kind: &'static str,
    maximum: usize,
) -> Result<(), PeerProtocolError> {
    if value.is_empty() || value.len() > maximum {
        return Err(PeerProtocolError::InvalidIdentity {
            kind,
            reason: format!("length must be between 1 and {maximum} bytes"),
        });
    }
    if !value.is_ascii()
        || !value.as_bytes()[0].is_ascii_alphanumeric()
        || !value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':' | b'/')
        })
    {
        return Err(PeerProtocolError::InvalidIdentity {
            kind,
            reason: "must be safe ASCII and start with an alphanumeric character".to_owned(),
        });
    }
    Ok(())
}

macro_rules! identity {
    ($(#[$meta:meta])* $name:ident, $maximum:expr) => {
        milkdrift_contracts::validated_string_type! {
            $(#[$meta])*
            pub struct $name;
            error = PeerProtocolError;
            validate = |value, kind| validate_identity(value, kind, $maximum);
        }
    };
}

identity!(/// Immutable client-generated request identity and idempotency key.
    PeerRequestId, 192);
identity!(/// Stable identity of one durably accepted remote execution.
    PeerExecutionId, 192);
identity!(/// One daemon boot/session identity; it conveys no trust.
    SessionId, 192);
identity!(/// Opaque constrained server-side delegation reference.
    DelegationRef, 192);
identity!(/// Idempotent artifact-transfer session identity.
    TransferId, 192);
identity!(/// Canonical BLAKE3 catalog digest including its `b3_` prefix.
    CatalogDigest, 67);
