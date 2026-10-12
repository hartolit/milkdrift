//! Browser origins are operator choices, validated before opening the store.

use std::collections::BTreeSet;

use super::ConfigError;

pub(super) fn validate_origins(origins: &[String]) -> Result<(), ConfigError> {
    if origins.len() > 32 {
        return Err(ConfigError::Invalid(
            "at most 32 browser origins may be configured".into(),
        ));
    }
    let mut seen = BTreeSet::new();
    for origin in origins {
        let parsed = url::Url::parse(origin).map_err(|_| {
            ConfigError::Invalid("browser origin must be an exact HTTP(S) origin".into())
        })?;
        let loopback = match parsed.host() {
            Some(url::Host::Ipv4(address)) => address.is_loopback(),
            Some(url::Host::Ipv6(address)) => address.is_loopback(),
            Some(url::Host::Domain(name)) => name == "localhost",
            None => false,
        };
        if origin.len() > 512
            || !seen.insert(origin)
            || parsed.origin().ascii_serialization() != *origin
            || !parsed.username().is_empty()
            || parsed.password().is_some()
            || parsed.query().is_some()
            || parsed.fragment().is_some()
            || !(parsed.scheme() == "https" || parsed.scheme() == "http" && loopback)
        {
            return Err(ConfigError::Invalid(
                "browser origins must be unique exact HTTPS origins (loopback HTTP is allowed), without a path, credentials, query or wildcard".into(),
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::validate_origins;

    #[test]
    fn origins_are_exact_and_plaintext_is_local() {
        assert!(
            validate_origins(&[
                "http://127.0.0.1:5173".into(),
                "https://work.example".into()
            ])
            .is_ok()
        );
        for value in [
            "*",
            "null",
            "http://work.example",
            "https://work.example/",
            "https://work.example/path",
            "https://user@work.example",
            "https://work.example?q=a",
        ] {
            assert!(
                validate_origins(&[value.into()]).is_err(),
                "accepted {value}"
            );
        }
        assert!(
            validate_origins(&["https://work.example".into(), "https://work.example".into()])
                .is_err()
        );
    }
}
