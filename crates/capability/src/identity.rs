use crate::ContractError;

#[derive(Clone, Copy)]
enum IdRule {
    Simple,
    Namespaced,
    Extension,
}

fn validate_id(
    value: &str,
    type_name: &'static str,
    max: usize,
    rule: IdRule,
) -> Result<(), ContractError> {
    if value.is_empty() || value.len() > max {
        return Err(ContractError::InvalidIdentity {
            type_name,
            reason: format!("length must be between 1 and {max} bytes"),
        });
    }
    if !value.is_ascii() {
        return Err(ContractError::InvalidIdentity {
            type_name,
            reason: "must contain ASCII characters only".to_owned(),
        });
    }
    let valid = value.bytes().all(|byte| {
        byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':' | b'/')
    });
    if !valid || !value.as_bytes()[0].is_ascii_alphanumeric() {
        return Err(ContractError::InvalidIdentity {
            type_name,
            reason: "must start with an alphanumeric character and use only alphanumerics, '-', '_', '.', ':', or '/'".to_owned(),
        });
    }
    let namespace_ok = match rule {
        IdRule::Simple => true,
        IdRule::Namespaced => value.split_once('.').is_some_and(|(namespace, name)| {
            !namespace.is_empty() && !name.is_empty() && !name.starts_with('.')
        }),
        IdRule::Extension => value.split_once('/').is_some_and(|(namespace, name)| {
            namespace.contains('.')
                && !namespace.starts_with('.')
                && !namespace.ends_with('.')
                && !name.is_empty()
        }),
    };
    if !namespace_ok {
        return Err(ContractError::InvalidIdentity {
            type_name,
            reason: match rule {
                IdRule::Simple => "invalid identity".to_owned(),
                IdRule::Namespaced => "must be namespaced as 'namespace.name'".to_owned(),
                IdRule::Extension => {
                    "must use a DNS-like namespace followed by '/', for example 'org.example/key'"
                        .to_owned()
                }
            },
        });
    }
    Ok(())
}

macro_rules! identity_type {
    ($(#[$meta:meta])* $name:ident, $max:expr, $rule:expr) => {
        milkdrift_contracts::validated_string_type! {
            $(#[$meta])*
            pub struct $name;
            error = ContractError;
            validate = |value, kind| validate_id(value, kind, $max, $rule);
        }

        impl TryFrom<String> for $name {
            type Error = ContractError;

            fn try_from(value: String) -> Result<Self, Self::Error> {
                Self::new(value)
            }
        }
    };
}

identity_type!(
    /// Stable identity of one advertised capability.
    CapabilityId,
    128,
    IdRule::Simple
);
identity_type!(
    /// Namespaced identity of an operation, such as `model.generate`.
    OperationId,
    128,
    IdRule::Namespaced
);
identity_type!(
    /// Namespaced identity of an explicitly advertised feature.
    FeatureId,
    128,
    IdRule::Namespaced
);
identity_type!(
    /// Opaque provider configuration profile reference; never a credential value.
    ProviderProfileRef,
    128,
    IdRule::Simple
);
identity_type!(
    /// Stable identity of one invocation.
    InvocationId,
    128,
    IdRule::Simple
);
identity_type!(
    /// Caller-selected identity used for idempotency handling.
    IdempotencyKey,
    192,
    IdRule::Simple
);
identity_type!(
    /// Namespaced identity of a structured value schema.
    SchemaId,
    128,
    IdRule::Namespaced
);
identity_type!(
    /// DNS-namespaced extension key, such as `org.example/hint`.
    ExtensionKey,
    192,
    IdRule::Extension
);
identity_type!(
    /// Bounded trust-zone label used for policy matching.
    TrustZone,
    96,
    IdRule::Simple
);
