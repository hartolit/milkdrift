//! Explicit environment-backed secret resolution.
//!
//! Configuration maps opaque references to exact environment variable names. The
//! resolver never enumerates the ambient environment and never retains resolved values.

use std::{collections::BTreeMap, env, fmt};

use milkdrift_authority::{SecretRef, SensitiveSecret};
use milkdrift_capability_host::{SecretResolver, SecretResolverError};
use thiserror::Error;

/// Invalid non-secret resolver configuration.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum SecretEnvConfigError {
    /// An environment variable name is empty, too large, or platform-unsafe.
    #[error("invalid environment variable name for secret reference")]
    InvalidEnvironmentName,
    /// Two opaque references were configured for the same environment name.
    #[error("an environment variable name may be mapped only once")]
    DuplicateEnvironmentName,
}

/// Minimal resolver from explicitly configured opaque references to environment names.
pub struct SecretEnvResolver {
    variables: BTreeMap<SecretRef, String>,
}

impl SecretEnvResolver {
    /// Validates and installs the exact reference-to-name mapping.
    pub fn new(variables: BTreeMap<SecretRef, String>) -> Result<Self, SecretEnvConfigError> {
        let mut names = std::collections::BTreeSet::new();
        for name in variables.values() {
            if !valid_environment_name(name) {
                return Err(SecretEnvConfigError::InvalidEnvironmentName);
            }
            if !names.insert(name) {
                return Err(SecretEnvConfigError::DuplicateEnvironmentName);
            }
        }
        Ok(Self { variables })
    }

    /// Returns the configured environment name without resolving its value.
    #[must_use]
    pub fn environment_name(&self, reference: &SecretRef) -> Option<&str> {
        self.variables.get(reference).map(String::as_str)
    }
}

impl SecretResolver for SecretEnvResolver {
    fn resolve(&self, reference: &SecretRef) -> Result<SensitiveSecret, SecretResolverError> {
        let name = self
            .variables
            .get(reference)
            .ok_or(SecretResolverError::Unavailable)?;
        let value = env::var_os(name).ok_or(SecretResolverError::Unavailable)?;
        Ok(SensitiveSecret::new(environment_bytes(value)?))
    }
}

impl fmt::Debug for SecretEnvResolver {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SecretEnvResolver")
            .field("configured_references", &self.variables.len())
            .field("resolved_values", &"[redacted]")
            .finish()
    }
}

fn valid_environment_name(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value.is_ascii()
        && !value.contains('=')
        && !value.contains('\0')
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
}

#[cfg(unix)]
fn environment_bytes(value: std::ffi::OsString) -> Result<Vec<u8>, SecretResolverError> {
    use std::os::unix::ffi::OsStringExt;

    Ok(value.into_vec())
}

#[cfg(not(unix))]
fn environment_bytes(value: std::ffi::OsString) -> Result<Vec<u8>, SecretResolverError> {
    value
        .into_string()
        .map(String::into_bytes)
        .map_err(|_value| SecretResolverError::Unavailable)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn configuration_is_explicit_and_debug_is_redacted() -> Result<(), Box<dyn std::error::Error>> {
        const CHILD_FLAG: &str = "MILKDRIFT_SECRET_ENV_TEST_CHILD";
        const SECRET_NAME: &str = "MILKDRIFT_SECRET_ENV_TEST_VALUE";
        const SECRET_VALUE: &str = "subprocess-secret-value";
        let reference = SecretRef::new("secret:test-token")?;
        let resolver = SecretEnvResolver::new(BTreeMap::from([(
            reference.clone(),
            SECRET_NAME.to_owned(),
        )]))?;
        assert_eq!(resolver.environment_name(&reference), Some(SECRET_NAME));
        let debug = format!("{resolver:?}");
        assert!(!debug.contains(SECRET_NAME));
        assert!(
            SecretEnvResolver::new(BTreeMap::from([(reference.clone(), "BAD=NAME".to_owned())]))
                .is_err()
        );
        assert!(
            SecretEnvResolver::new(BTreeMap::from([
                (reference.clone(), SECRET_NAME.to_owned()),
                (SecretRef::new("secret:duplicate")?, SECRET_NAME.to_owned()),
            ]))
            .is_err()
        );

        if env::var_os(CHILD_FLAG).is_some() {
            let resolved = resolver.resolve(&reference)?;
            resolved.expose(|bytes| assert_eq!(bytes, SECRET_VALUE.as_bytes()));
            assert!(matches!(
                resolver.resolve(&SecretRef::new("secret:unmapped")?),
                Err(SecretResolverError::Unavailable)
            ));
            return Ok(());
        }

        let status = std::process::Command::new(env::current_exe()?)
            .args([
                "--exact",
                "tests::configuration_is_explicit_and_debug_is_redacted",
                "--nocapture",
            ])
            .env_clear()
            .env(CHILD_FLAG, "1")
            .env(SECRET_NAME, SECRET_VALUE)
            .status()?;
        assert!(status.success());
        Ok(())
    }
}
