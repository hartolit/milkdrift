use std::fs::File;
use std::io;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use crate::bounded::{BoundedReadError, read_bounded};
use crate::configuration::MAX_CONFIG_BYTES;
use crate::identity::{SelectedArtifactIdentity, SelectedArtifactMetadata, verify_git_blob_bytes};
use crate::{
    ArtifactContentIdentity, ArtifactContentIdentityAuthority, ArtifactContentKind, HubError,
    HubStructuralLimit, ResolvedContentArtifact,
};

/// Thirty-two MiB accommodates large Llama tokenizer vocabularies and merge tables while
/// bounding retained JSON bytes and downstream parser work.
pub(crate) const MAX_TOKENIZER_BYTES: u64 = 32 * 1024 * 1024;

impl ArtifactContentKind {
    pub(crate) const fn maximum_bytes(self) -> u64 {
        match self {
            Self::Configuration => MAX_CONFIG_BYTES,
            Self::Tokenizer => MAX_TOKENIZER_BYTES,
            Self::WeightIndex => crate::weights::MAX_WEIGHT_INDEX_BYTES,
        }
    }

    pub(crate) const fn structural_limit(self) -> HubStructuralLimit {
        match self {
            Self::Configuration => HubStructuralLimit::ConfigurationBytes,
            Self::Tokenizer => HubStructuralLimit::TokenizerBytes,
            Self::WeightIndex => HubStructuralLimit::WeightIndexBytes,
        }
    }
}

pub(crate) fn resolve_content_artifact(
    path: PathBuf,
    metadata: SelectedArtifactMetadata,
    kind: ArtifactContentKind,
) -> Result<(ResolvedContentArtifact, Vec<u8>), HubError> {
    let bytes = read_exact_bounded_file(&path, kind, metadata.reported_byte_length)?;
    let observed_byte_length = observed_byte_length(bytes.as_slice(), kind)?;
    let observed_sha256: [u8; 32] = Sha256::digest(bytes.as_slice()).into();

    let content_identity = match metadata.identity {
        SelectedArtifactIdentity::HuggingFaceLfs(identity) => {
            if identity.byte_length != observed_byte_length || identity.sha256 != observed_sha256 {
                return Err(HubError::ContentIdentityMismatch(kind));
            }
            identity
        }
        SelectedArtifactIdentity::HuggingFaceGitBlob(expected_sha1) => {
            verify_git_blob_bytes(bytes.as_slice(), expected_sha1)
                .map_err(|()| HubError::ContentIdentityMismatch(kind))?;
            ArtifactContentIdentity {
                byte_length: observed_byte_length,
                sha256: observed_sha256,
                authority: ArtifactContentIdentityAuthority::HuggingFaceGitBlob,
            }
        }
    };

    Ok((
        ResolvedContentArtifact {
            path,
            content_identity,
            kind,
        },
        bytes,
    ))
}

pub(crate) fn read_verified_content_bytes(
    artifact: &ResolvedContentArtifact,
) -> Result<Vec<u8>, HubError> {
    let bytes = read_exact_bounded_file(
        &artifact.path,
        artifact.kind,
        artifact.content_identity.byte_length,
    )?;
    let observed_sha256: [u8; 32] = Sha256::digest(bytes.as_slice()).into();
    if observed_sha256 != artifact.content_identity.sha256 {
        return Err(HubError::ContentIdentityMismatch(artifact.kind));
    }
    Ok(bytes)
}

fn read_exact_bounded_file(
    path: &Path,
    kind: ArtifactContentKind,
    expected_byte_length: u64,
) -> Result<Vec<u8>, HubError> {
    if expected_byte_length > kind.maximum_bytes() {
        return Err(HubError::StructuralLimitExceeded(kind.structural_limit()));
    }

    let file = File::open(path).map_err(|error| read_error(kind, error))?;
    let metadata = file.metadata().map_err(|error| read_error(kind, error))?;
    if !metadata.is_file() {
        return Err(HubError::InvalidContentFile(kind));
    }
    let local_byte_length = metadata.len();
    if local_byte_length != expected_byte_length {
        return Err(HubError::ContentIdentityMismatch(kind));
    }

    let bytes =
        read_bounded(file, local_byte_length, kind.maximum_bytes()).map_err(
            |error| match error {
                BoundedReadError::Io(error) => read_error(kind, error),
                BoundedReadError::Limit => {
                    HubError::StructuralLimitExceeded(kind.structural_limit())
                }
            },
        )?;
    if observed_byte_length(bytes.as_slice(), kind)? != expected_byte_length {
        return Err(HubError::ContentIdentityMismatch(kind));
    }
    Ok(bytes)
}

fn observed_byte_length(bytes: &[u8], kind: ArtifactContentKind) -> Result<u64, HubError> {
    u64::try_from(bytes.len()).map_err(|_| HubError::ContentIdentityMismatch(kind))
}

fn read_error(kind: ArtifactContentKind, error: io::Error) -> HubError {
    match kind {
        ArtifactContentKind::Configuration => HubError::ReadConfiguration(error),
        ArtifactContentKind::Tokenizer => HubError::ReadTokenizer(error),
        ArtifactContentKind::WeightIndex => HubError::ReadIndex(error),
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::io::Cursor;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    use sha1::Sha1;
    use sha2::{Digest, Sha256};

    use super::{MAX_TOKENIZER_BYTES, resolve_content_artifact};
    use crate::bounded::{BoundedReadError, read_bounded};
    use crate::identity::{SelectedArtifactIdentity, SelectedArtifactMetadata};
    use crate::{
        ArtifactContentIdentity, ArtifactContentIdentityAuthority, ArtifactContentKind, HubError,
        HubStructuralLimit,
    };

    static NEXT_TEST_FILE: AtomicU64 = AtomicU64::new(0);

    #[test]
    fn configuration_mutation_after_resolution_is_rejected() -> Result<(), String> {
        assert_mutation_rejected(
            ArtifactContentKind::Configuration,
            br#"{"model_type":"llama"}"#,
            br#"{"model_type":"other"}"#,
        )
    }

    #[test]
    fn tokenizer_mutation_after_resolution_is_rejected() -> Result<(), String> {
        assert_mutation_rejected(
            ArtifactContentKind::Tokenizer,
            br#"{"version":"1.0","model":{}}"#,
            br#"{"version":"2.0","model":{}}"#,
        )
    }

    #[test]
    fn matching_upstream_identity_retains_hugging_face_authority() -> Result<(), String> {
        let kind = ArtifactContentKind::Tokenizer;
        let bytes = br#"{"version":"1.0","model":{}}"#;
        let path = temporary_file_path(kind);
        let _cleanup = Cleanup(path.clone());
        fs::write(&path, bytes).map_err(|error| error.to_string())?;
        let byte_length = u64::try_from(bytes.len()).map_err(|error| error.to_string())?;
        let sha256 = Sha256::digest(bytes).into();
        let (artifact, resolved_bytes) = resolve_content_artifact(
            path,
            SelectedArtifactMetadata {
                reported_byte_length: byte_length,
                identity: SelectedArtifactIdentity::HuggingFaceLfs(ArtifactContentIdentity {
                    byte_length,
                    sha256,
                    authority: ArtifactContentIdentityAuthority::HuggingFaceLfs,
                }),
            },
            kind,
        )
        .map_err(|error| error.to_string())?;

        assert_eq!(resolved_bytes, bytes);
        assert_eq!(
            artifact.content_identity.authority,
            ArtifactContentIdentityAuthority::HuggingFaceLfs
        );
        Ok(())
    }

    #[test]
    fn tokenizer_json_limit_is_enforced_before_reading() {
        assert!(matches!(
            read_bounded(
                Cursor::new(Vec::<u8>::new()),
                MAX_TOKENIZER_BYTES + 1,
                MAX_TOKENIZER_BYTES,
            ),
            Err(BoundedReadError::Limit)
        ));
    }

    fn assert_mutation_rejected(
        kind: ArtifactContentKind,
        original: &[u8],
        mutation: &[u8],
    ) -> Result<(), String> {
        let path = temporary_file_path(kind);
        let _cleanup = Cleanup(path.clone());
        fs::write(&path, original).map_err(|error| error.to_string())?;
        let reported_byte_length =
            u64::try_from(original.len()).map_err(|error| error.to_string())?;
        let (artifact, resolved_bytes) = resolve_content_artifact(
            path.clone(),
            SelectedArtifactMetadata {
                reported_byte_length,
                identity: SelectedArtifactIdentity::HuggingFaceGitBlob(git_blob_sha1(original)?),
            },
            kind,
        )
        .map_err(|error| error.to_string())?;
        assert_eq!(resolved_bytes, original);
        assert_eq!(
            artifact.content_identity.authority,
            ArtifactContentIdentityAuthority::HuggingFaceGitBlob
        );
        assert_eq!(
            artifact
                .read_verified_bytes()
                .map_err(|error| error.to_string())?,
            original
        );

        fs::write(&path, mutation).map_err(|error| error.to_string())?;
        assert!(matches!(
            artifact.read_verified_bytes(),
            Err(HubError::ContentIdentityMismatch(actual)) if actual == kind
        ));
        Ok(())
    }

    fn git_blob_sha1(bytes: &[u8]) -> Result<[u8; 20], String> {
        let byte_length = u64::try_from(bytes.len()).map_err(|error| error.to_string())?;
        let mut hasher = Sha1::new();
        hasher.update(b"blob ");
        hasher.update(byte_length.to_string().as_bytes());
        hasher.update([0]);
        hasher.update(bytes);
        Ok(hasher.finalize().into())
    }

    fn temporary_file_path(kind: ArtifactContentKind) -> PathBuf {
        let sequence = NEXT_TEST_FILE.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!(
            "milkdrift-hf-hub-{}-{kind:?}-{sequence}.json",
            std::process::id()
        ))
    }

    struct Cleanup(PathBuf);

    impl Drop for Cleanup {
        fn drop(&mut self) {
            let _ignored = fs::remove_file(&self.0);
        }
    }

    #[test]
    fn public_kind_maps_to_the_expected_structural_limit() {
        assert_eq!(
            ArtifactContentKind::Configuration.structural_limit(),
            HubStructuralLimit::ConfigurationBytes
        );
        assert_eq!(
            ArtifactContentKind::Tokenizer.structural_limit(),
            HubStructuralLimit::TokenizerBytes
        );
        assert_eq!(
            ArtifactContentKind::WeightIndex.structural_limit(),
            HubStructuralLimit::WeightIndexBytes
        );
    }
}
