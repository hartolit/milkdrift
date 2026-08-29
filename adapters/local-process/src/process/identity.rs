use std::{
    fs::{self, File, Metadata},
    io::{self, Read},
    path::{Path, PathBuf},
};

use crate::config::{
    ExecutableIdentityDeclaration, ExecutablePlatformEvidence, MAX_EXECUTABLE_BYTES,
    ProcessProfile, ProcessProfileError, VerifiedExecutableIdentity,
};

const HASH_BUFFER_BYTES: usize = 64 * 1024;

pub(super) struct ExecutableBinding {
    pub(super) canonical_path: PathBuf,
    pub(super) evidence: VerifiedExecutableIdentity,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum IdentityFailure {
    PathUnavailable,
    PathResolutionChanged,
    PathOutsideAuthorizedRoot,
    NotRegularFile,
    NotExecutable,
    ExecutableTooLarge,
    SizeMismatch,
    ContentDigestMismatch,
    PlatformMetadataMismatch,
    ReadDenied,
    ReadFailed,
}

impl IdentityFailure {
    pub(super) const fn code(self) -> &'static str {
        match self {
            Self::PathUnavailable => "tool_path_unavailable",
            Self::PathResolutionChanged => "tool_path_resolution_changed",
            Self::PathOutsideAuthorizedRoot => "tool_path_outside_authorized_root",
            Self::NotRegularFile => "tool_source_not_regular_file",
            Self::NotExecutable => "tool_source_not_executable",
            Self::ExecutableTooLarge => "tool_source_too_large",
            Self::SizeMismatch => "tool_size_mismatch",
            Self::ContentDigestMismatch => "tool_content_digest_mismatch",
            Self::PlatformMetadataMismatch => "tool_platform_metadata_mismatch",
            Self::ReadDenied => "tool_identity_read_denied",
            Self::ReadFailed => "tool_identity_read_failed",
        }
    }
}

pub(super) fn bind(profile: &ProcessProfile) -> Result<ExecutableBinding, ProcessProfileError> {
    let canonical_path = profile.executable.canonicalize().map_err(|error| {
        ProcessProfileError::Invalid(format!(
            "configured executable identity cannot be resolved: {}",
            io_failure(error.kind()).code()
        ))
    })?;
    let evidence = observe(
        &profile.executable,
        &canonical_path,
        &profile.implementation,
    )
    .map_err(|failure| {
        ProcessProfileError::Invalid(format!(
            "configured executable identity is invalid: {}",
            failure.code()
        ))
    })?;
    Ok(ExecutableBinding {
        canonical_path,
        evidence,
    })
}

pub(super) fn revalidate(
    profile: &ProcessProfile,
    expected_path: &Path,
    expected: &VerifiedExecutableIdentity,
    executable_roots: &[PathBuf],
) -> Result<VerifiedExecutableIdentity, IdentityFailure> {
    let canonical_path = profile
        .executable
        .canonicalize()
        .map_err(|error| io_failure(error.kind()))?;
    if canonical_path != expected_path {
        return Err(IdentityFailure::PathResolutionChanged);
    }
    if !executable_roots
        .iter()
        .any(|root| canonical_path.starts_with(root))
    {
        return Err(IdentityFailure::PathOutsideAuthorizedRoot);
    }
    let observed = observe(
        &profile.executable,
        &canonical_path,
        &profile.implementation,
    )?;
    if observed != *expected {
        return Err(if observed.content_digest != expected.content_digest {
            IdentityFailure::ContentDigestMismatch
        } else if observed.size_bytes != expected.size_bytes {
            IdentityFailure::SizeMismatch
        } else if observed.platform != expected.platform {
            IdentityFailure::PlatformMetadataMismatch
        } else {
            IdentityFailure::PathResolutionChanged
        });
    }
    Ok(observed)
}

fn observe(
    configured_path: &Path,
    canonical_path: &Path,
    declaration: &ExecutableIdentityDeclaration,
) -> Result<VerifiedExecutableIdentity, IdentityFailure> {
    let mut file = File::open(canonical_path).map_err(|error| io_failure(error.kind()))?;
    let before = file.metadata().map_err(|error| io_failure(error.kind()))?;
    validate_metadata(&before)?;
    if before.len() > MAX_EXECUTABLE_BYTES {
        return Err(IdentityFailure::ExecutableTooLarge);
    }
    if before.len() != declaration.size_bytes {
        return Err(IdentityFailure::SizeMismatch);
    }

    let mut hasher = blake3::Hasher::new();
    let mut total = 0_u64;
    let mut buffer = [0_u8; HASH_BUFFER_BYTES];
    loop {
        let count = file
            .read(&mut buffer)
            .map_err(|error| io_failure(error.kind()))?;
        if count == 0 {
            break;
        }
        total = total
            .checked_add(u64::try_from(count).map_err(|_| IdentityFailure::ExecutableTooLarge)?)
            .ok_or(IdentityFailure::ExecutableTooLarge)?;
        if total > MAX_EXECUTABLE_BYTES {
            return Err(IdentityFailure::ExecutableTooLarge);
        }
        hasher.update(&buffer[..count]);
    }
    if total != before.len() || total != declaration.size_bytes {
        return Err(IdentityFailure::SizeMismatch);
    }
    let content_digest = format!("b3_{}", hasher.finalize());
    if content_digest != declaration.content_digest {
        return Err(IdentityFailure::ContentDigestMismatch);
    }

    let after = file.metadata().map_err(|error| io_failure(error.kind()))?;
    let path_metadata = fs::metadata(canonical_path).map_err(|error| io_failure(error.kind()))?;
    validate_metadata(&after)?;
    validate_metadata(&path_metadata)?;
    if !same_observation(&before, &after) || !same_open_file(&after, &path_metadata) {
        return Err(IdentityFailure::PlatformMetadataMismatch);
    }
    let resolved_again = configured_path
        .canonicalize()
        .map_err(|error| io_failure(error.kind()))?;
    if resolved_again != canonical_path {
        return Err(IdentityFailure::PathResolutionChanged);
    }

    let configured_path_digest = path_digest(b"configured", configured_path)?;
    let canonical_path_digest = path_digest(b"canonical", canonical_path)?;
    let platform = platform_evidence(&after);
    let identity_digest = identity_digest(
        &configured_path_digest,
        &canonical_path_digest,
        &content_digest,
        total,
        declaration.package_revision.as_deref(),
        &platform,
    );
    Ok(VerifiedExecutableIdentity {
        identity_digest,
        configured_path_digest,
        canonical_path_digest,
        content_digest,
        size_bytes: total,
        package_revision: declaration.package_revision.clone(),
        documentation_reference: declaration.documentation_reference.clone(),
        platform,
    })
}

fn validate_metadata(metadata: &Metadata) -> Result<(), IdentityFailure> {
    if !metadata.file_type().is_file() || metadata.file_type().is_symlink() {
        return Err(IdentityFailure::NotRegularFile);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if metadata.permissions().mode() & 0o111 == 0 {
            return Err(IdentityFailure::NotExecutable);
        }
    }
    Ok(())
}

fn path_digest(label: &[u8], path: &Path) -> Result<String, IdentityFailure> {
    let value = path
        .to_str()
        .ok_or(IdentityFailure::PathResolutionChanged)?;
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"milkdrift.process-executable-path.v1\0");
    hasher.update(label);
    hasher.update(b"\0");
    hasher.update(value.as_bytes());
    Ok(format!("b3_{}", hasher.finalize()))
}

fn identity_digest(
    configured_path_digest: &str,
    canonical_path_digest: &str,
    content_digest: &str,
    size_bytes: u64,
    package_revision: Option<&str>,
    platform: &ExecutablePlatformEvidence,
) -> String {
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"milkdrift.local-executable-identity.v1\0");
    for value in [
        configured_path_digest,
        canonical_path_digest,
        content_digest,
        package_revision.unwrap_or(""),
    ] {
        hasher.update(value.as_bytes());
        hasher.update(b"\0");
    }
    hasher.update(&size_bytes.to_le_bytes());
    hasher.update(&platform.unix_mode.unwrap_or_default().to_le_bytes());
    format!("b3_{}", hasher.finalize())
}

fn platform_evidence(metadata: &Metadata) -> ExecutablePlatformEvidence {
    ExecutablePlatformEvidence {
        regular_file: metadata.file_type().is_file(),
        #[cfg(unix)]
        unix_mode: {
            use std::os::unix::fs::PermissionsExt;
            Some(metadata.permissions().mode() & 0o7777)
        },
        #[cfg(not(unix))]
        unix_mode: None,
    }
}

fn same_observation(left: &Metadata, right: &Metadata) -> bool {
    left.len() == right.len() && platform_evidence(left) == platform_evidence(right)
}

#[cfg(unix)]
fn same_open_file(left: &Metadata, right: &Metadata) -> bool {
    use std::os::unix::fs::MetadataExt;
    left.dev() == right.dev() && left.ino() == right.ino()
}

#[cfg(not(unix))]
fn same_open_file(left: &Metadata, right: &Metadata) -> bool {
    same_observation(left, right)
}

fn io_failure(kind: io::ErrorKind) -> IdentityFailure {
    match kind {
        io::ErrorKind::NotFound => IdentityFailure::PathUnavailable,
        io::ErrorKind::PermissionDenied => IdentityFailure::ReadDenied,
        _ => IdentityFailure::ReadFailed,
    }
}
