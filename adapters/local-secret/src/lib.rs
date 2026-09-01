//! Explicit local environment- and restricted-file-backed secret resolution.
//!
//! Configuration maps opaque references to exact local sources. The resolver never
//! enumerates the ambient environment, retains resolved values, or accepts relative
//! file paths whose meaning could change with the process working directory.

use std::{
    collections::BTreeMap,
    env, fmt,
    fs::File,
    io::{Read as _, Take},
    path::{Path, PathBuf},
};

use milkdrift_authority::{SecretRef, SensitiveSecret};
use milkdrift_capability_host::{SecretResolver, SecretResolverError};
use thiserror::Error;

const MAX_ENVIRONMENT_NAME_BYTES: usize = 128;
const MAX_SECRET_BYTES: usize = 4_096;
const MAX_SECRET_FILE_BYTES: u64 = MAX_SECRET_BYTES as u64 + 1;

/// Invalid non-secret local source configuration.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum LocalSecretConfigError {
    /// An environment variable name is empty, too large, or platform-unsafe.
    #[error("invalid environment variable name for secret reference")]
    InvalidEnvironmentName,
    /// A file source is not an absolute path.
    #[error("secret file source must use an absolute path")]
    InvalidFilePath,
}

#[derive(Clone, Eq, PartialEq)]
enum SourceKind {
    Environment(String),
    File(PathBuf),
}

/// One validated local source containing no resolved secret value.
#[derive(Clone, Eq, PartialEq)]
pub struct LocalSecretSource(SourceKind);

impl LocalSecretSource {
    /// Creates an exact environment-variable source without reading its value.
    pub fn environment(variable: impl Into<String>) -> Result<Self, LocalSecretConfigError> {
        let variable = variable.into();
        if !valid_environment_name(&variable) {
            return Err(LocalSecretConfigError::InvalidEnvironmentName);
        }
        Ok(Self(SourceKind::Environment(variable)))
    }

    /// Creates an absolute restricted-file source without reading its value.
    pub fn file(path: impl Into<PathBuf>) -> Result<Self, LocalSecretConfigError> {
        let path = path.into();
        if !path.is_absolute() {
            return Err(LocalSecretConfigError::InvalidFilePath);
        }
        Ok(Self(SourceKind::File(path)))
    }

    fn resolve(&self) -> Result<SensitiveSecret, SecretResolverError> {
        let bytes = match &self.0 {
            SourceKind::Environment(variable) => env::var_os(variable)
                .map(environment_bytes)
                .transpose()?
                .ok_or(SecretResolverError::Unavailable)?,
            SourceKind::File(path) => read_restricted_file(path)?,
        };
        if bytes.is_empty() || bytes.len() > MAX_SECRET_BYTES {
            return Err(SecretResolverError::Unavailable);
        }
        Ok(SensitiveSecret::new(bytes))
    }
}

impl fmt::Debug for LocalSecretSource {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let kind = match self.0 {
            SourceKind::Environment(_) => "environment",
            SourceKind::File(_) => "file",
        };
        formatter
            .debug_struct("LocalSecretSource")
            .field("kind", &kind)
            .field("location", &"[redacted]")
            .finish()
    }
}

/// Resolver from explicitly configured opaque references to local sources.
pub struct LocalSecretResolver {
    sources: BTreeMap<SecretRef, LocalSecretSource>,
}

impl LocalSecretResolver {
    /// Installs one exact validated reference-to-source mapping.
    #[must_use]
    pub fn new(sources: BTreeMap<SecretRef, LocalSecretSource>) -> Self {
        Self { sources }
    }
}

impl SecretResolver for LocalSecretResolver {
    fn resolve(&self, reference: &SecretRef) -> Result<SensitiveSecret, SecretResolverError> {
        self.sources
            .get(reference)
            .ok_or(SecretResolverError::Unavailable)?
            .resolve()
    }
}

impl fmt::Debug for LocalSecretResolver {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("LocalSecretResolver")
            .field("configured_references", &self.sources.len())
            .field("sources", &"[redacted]")
            .field("resolved_values", &"[redacted]")
            .finish()
    }
}

fn valid_environment_name(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_ENVIRONMENT_NAME_BYTES
        && value.is_ascii()
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
}

fn read_restricted_file(path: &Path) -> Result<Vec<u8>, SecretResolverError> {
    let file = File::open(path).map_err(|_error| SecretResolverError::Unavailable)?;
    let metadata = file
        .metadata()
        .map_err(|_error| SecretResolverError::Unavailable)?;
    if !metadata.is_file() || metadata.len() > MAX_SECRET_FILE_BYTES {
        return Err(SecretResolverError::Unavailable);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;

        if metadata.permissions().mode() & 0o077 != 0 {
            return Err(SecretResolverError::Unavailable);
        }
    }

    let mut bytes = Vec::with_capacity(
        usize::try_from(metadata.len())
            .unwrap_or(MAX_SECRET_BYTES)
            .min(MAX_SECRET_BYTES),
    );
    let mut bounded: Take<File> = file.take(MAX_SECRET_FILE_BYTES + 1);
    bounded
        .read_to_end(&mut bytes)
        .map_err(|_error| SecretResolverError::Unavailable)?;
    if bytes.len() as u64 > MAX_SECRET_FILE_BYTES {
        return Err(SecretResolverError::Unavailable);
    }
    if bytes.last() == Some(&b'\n') {
        bytes.pop();
        if bytes.last() == Some(&b'\r') {
            bytes.pop();
        }
    }
    Ok(bytes)
}

#[cfg(unix)]
fn environment_bytes(value: std::ffi::OsString) -> Result<Vec<u8>, SecretResolverError> {
    use std::os::unix::ffi::OsStringExt as _;

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
    use std::{collections::BTreeMap, fs};

    #[cfg(unix)]
    fn restrict(path: &Path) -> std::io::Result<()> {
        use std::os::unix::fs::PermissionsExt as _;

        fs::set_permissions(path, fs::Permissions::from_mode(0o600))
    }

    #[cfg(not(unix))]
    fn restrict(_path: &Path) -> std::io::Result<()> {
        Ok(())
    }

    #[test]
    fn environment_configuration_is_explicit_bounded_and_redacted()
    -> Result<(), Box<dyn std::error::Error>> {
        const CHILD_FLAG: &str = "MILKDRIFT_LOCAL_SECRET_TEST_CHILD";
        const SECRET_NAME: &str = "MILKDRIFT_LOCAL_SECRET_TEST_VALUE";
        const SECRET_VALUE: &str = "subprocess-secret-value";
        let reference = SecretRef::new("secret:test-token")?;
        let source = LocalSecretSource::environment(SECRET_NAME)?;
        let resolver = LocalSecretResolver::new(BTreeMap::from([(reference.clone(), source)]));
        let debug = format!("{resolver:?}");
        assert!(!debug.contains(SECRET_NAME));
        assert!(LocalSecretSource::environment("BAD=NAME").is_err());
        assert!(LocalSecretSource::environment("x".repeat(129)).is_err());

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
                "tests::environment_configuration_is_explicit_bounded_and_redacted",
                "--nocapture",
            ])
            .env_clear()
            .env(CHILD_FLAG, "1")
            .env(SECRET_NAME, SECRET_VALUE)
            .status()?;
        assert!(status.success());
        Ok(())
    }

    #[test]
    fn restricted_file_resolution_rotates_and_enforces_bounds()
    -> Result<(), Box<dyn std::error::Error>> {
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("token");
        fs::write(&path, b"first-token\r\n")?;
        restrict(&path)?;
        let reference = SecretRef::new("credential:operator")?;
        let resolver = LocalSecretResolver::new(BTreeMap::from([(
            reference.clone(),
            LocalSecretSource::file(&path)?,
        )]));

        resolver
            .resolve(&reference)?
            .expose(|bytes| assert_eq!(bytes, b"first-token"));
        fs::write(&path, b"second-token")?;
        restrict(&path)?;
        resolver
            .resolve(&reference)?
            .expose(|bytes| assert_eq!(bytes, b"second-token"));

        fs::write(&path, vec![b'x'; MAX_SECRET_FILE_BYTES as usize + 1])?;
        restrict(&path)?;
        assert!(matches!(
            resolver.resolve(&reference),
            Err(SecretResolverError::Unavailable)
        ));
        assert!(!format!("{resolver:?}").contains(path.to_string_lossy().as_ref()));
        assert!(LocalSecretSource::file("relative-token").is_err());
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn file_source_rejects_group_or_other_access() -> Result<(), Box<dyn std::error::Error>> {
        use std::os::unix::fs::PermissionsExt as _;

        let directory = tempfile::tempdir()?;
        let path = directory.path().join("token");
        fs::write(&path, b"secret")?;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o640))?;
        let reference = SecretRef::new("secret:file")?;
        let resolver = LocalSecretResolver::new(BTreeMap::from([(
            reference.clone(),
            LocalSecretSource::file(&path)?,
        )]));
        assert!(matches!(
            resolver.resolve(&reference),
            Err(SecretResolverError::Unavailable)
        ));
        Ok(())
    }
}
