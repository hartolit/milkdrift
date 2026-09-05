//! Exact account, reservation and transition identities.
use crate::{AttemptId, PersistenceError};

macro_rules! account_identity {
    ($(#[$meta:meta])* $name:ident) => {
        milkdrift_contracts::validated_string_type! {
            $(#[$meta])*
            pub struct $name;
            error = PersistenceError;
            validate = validate_account_identity;
        }
    };
}

fn validate_account_identity(value: &str, kind: &'static str) -> Result<(), PersistenceError> {
    if value.is_empty()
        || value.len() > 192
        || !value.is_ascii()
        || !value.as_bytes()[0].is_ascii_alphanumeric()
        || !value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':' | b'/')
        })
    {
        return Err(PersistenceError::InvalidIdentity {
            kind,
            reason: "must contain 1..=192 safe ASCII identity bytes".to_owned(),
        });
    }
    Ok(())
}

account_identity!(/// One immutable continuous-controller resource account.
    ControllerAccountId);
account_identity!(/// One exact final-entry reservation.
    ControllerReservationId);
account_identity!(/// One idempotent durable account transition.
    ControllerTransitionId);

impl ControllerReservationId {
    /// Derives the one stable reservation identity for an account-bound attempt.
    pub fn for_attempt(
        account: &ControllerAccountId,
        attempt: &AttemptId,
    ) -> Result<Self, PersistenceError> {
        Self::new(format!(
            "controller-reservation:{}",
            framed_digest(
                b"milkdrift.controller-reservation.v1\0",
                &[account.as_str(), attempt.as_str()],
            )
        ))
    }
}

pub(super) fn framed_digest(domain: &[u8], values: &[&str]) -> String {
    let mut hasher = blake3::Hasher::new();
    hasher.update(domain);
    for value in values {
        hasher.update(&(value.len() as u64).to_be_bytes());
        hasher.update(value.as_bytes());
    }
    hasher.finalize().to_hex().to_string()
}
