use std::collections::{BTreeMap, BTreeSet};
use std::fs::File;
use std::io::{self, Read};
use std::path::PathBuf;

use hf_hub::repository::{BlobLfsInfo, RepoTreeEntry};
use hf_hub::{HFRepositorySync, RepoTypeModel};
use sha1::Sha1;
use sha2::{Digest, Sha256};

use crate::{
    ArtifactContentIdentity, ArtifactContentIdentityAuthority, HubError, ResolvedSafetensorsShard,
};

const SHA256_BYTES: usize = 32;
const GIT_SHA1_BYTES: usize = 20;
/// A fixed 64 KiB buffer keeps non-LFS verification independent of artifact size.
const HASH_BUFFER_BYTES: usize = 64 * 1024;
const HASH_BUFFER_BYTES_U64: u64 = 64 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SelectedArtifactIdentity {
    HuggingFaceLfs(ArtifactContentIdentity),
    HuggingFaceGitBlob([u8; GIT_SHA1_BYTES]),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct SelectedArtifactMetadata {
    pub(crate) reported_byte_length: u64,
    pub(crate) identity: SelectedArtifactIdentity,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct SelectedContentMetadata {
    pub(crate) configuration: SelectedArtifactMetadata,
    pub(crate) tokenizer: SelectedArtifactMetadata,
    pub(crate) weight_index: Option<SelectedArtifactMetadata>,
}

pub(crate) fn selected_weight_metadata(
    repository: &HFRepositorySync<RepoTypeModel>,
    commit: &str,
    filenames: &[String],
) -> Result<BTreeMap<String, SelectedArtifactMetadata>, HubError> {
    crate::weights::validate_selected_weight_shard_count(filenames.len())?;
    let entries = repository
        .get_paths_info()
        .paths(filenames.to_vec())
        .revision(commit.to_owned())
        .send()
        .map_err(HubError::ArtifactMetadata)?;
    match_selected_weight_metadata(filenames, entries)
}

pub(crate) fn selected_content_metadata(
    repository: &HFRepositorySync<RepoTypeModel>,
    commit: &str,
    include_weight_index: bool,
) -> Result<SelectedContentMetadata, HubError> {
    let mut filenames = vec![
        crate::CONFIG_FILE.to_owned(),
        crate::TOKENIZER_FILE.to_owned(),
    ];
    if include_weight_index {
        filenames.push(crate::WEIGHT_INDEX_FILE.to_owned());
    }
    let entries = repository
        .get_paths_info()
        .paths(filenames.clone())
        .revision(commit.to_owned())
        .send()
        .map_err(HubError::ArtifactMetadata)?;
    let mut matched =
        match_selected_metadata(filenames.as_slice(), entries, SelectedMetadataKind::Content)?;
    let configuration = matched
        .remove(crate::CONFIG_FILE)
        .ok_or(HubError::InvalidContentMetadata)?;
    let tokenizer = matched
        .remove(crate::TOKENIZER_FILE)
        .ok_or(HubError::InvalidContentMetadata)?;
    let weight_index = if include_weight_index {
        Some(
            matched
                .remove(crate::WEIGHT_INDEX_FILE)
                .ok_or(HubError::InvalidContentMetadata)?,
        )
    } else {
        None
    };
    if !matched.is_empty() {
        return Err(HubError::InvalidContentMetadata);
    }
    Ok(SelectedContentMetadata {
        configuration,
        tokenizer,
        weight_index,
    })
}

#[derive(Clone, Copy)]
enum SelectedMetadataKind {
    Content,
    Weight,
}

impl SelectedMetadataKind {
    const fn duplicate_error(self) -> HubError {
        match self {
            Self::Content => HubError::InvalidContentMetadata,
            Self::Weight => HubError::DuplicateShardMetadata,
        }
    }

    const fn missing_error(self) -> HubError {
        match self {
            Self::Content => HubError::InvalidContentMetadata,
            Self::Weight => HubError::MissingShardMetadata,
        }
    }

    const fn unexpected_error(self) -> HubError {
        match self {
            Self::Content => HubError::InvalidContentMetadata,
            Self::Weight => HubError::UnexpectedShardMetadata,
        }
    }

    const fn length_error(self) -> HubError {
        match self {
            Self::Content => HubError::InvalidContentMetadata,
            Self::Weight => HubError::ShardLengthMismatch,
        }
    }
}

fn match_selected_weight_metadata(
    filenames: &[String],
    entries: Vec<RepoTreeEntry>,
) -> Result<BTreeMap<String, SelectedArtifactMetadata>, HubError> {
    crate::weights::validate_selected_weight_shard_count(filenames.len())?;
    match_selected_metadata(filenames, entries, SelectedMetadataKind::Weight)
}

fn match_selected_metadata(
    filenames: &[String],
    entries: Vec<RepoTreeEntry>,
    kind: SelectedMetadataKind,
) -> Result<BTreeMap<String, SelectedArtifactMetadata>, HubError> {
    let selected: BTreeSet<&str> = filenames.iter().map(String::as_str).collect();
    if selected.len() != filenames.len() {
        return Err(kind.duplicate_error());
    }

    let mut raw_matches = BTreeMap::new();
    let mut duplicate = false;
    let mut unexpected = false;
    for entry in entries {
        let RepoTreeEntry::File {
            oid,
            path,
            size,
            lfs,
            ..
        } = entry
        else {
            unexpected = true;
            continue;
        };
        if !selected.contains(path.as_str()) {
            unexpected = true;
            continue;
        }
        if raw_matches.insert(path, (oid, size, lfs)).is_some() {
            duplicate = true;
        }
    }

    // Fixed precedence makes malformed response classification independent of response order.
    if unexpected {
        return Err(kind.unexpected_error());
    }
    if duplicate {
        return Err(kind.duplicate_error());
    }
    if raw_matches.len() != selected.len() {
        return Err(kind.missing_error());
    }

    let mut matched = BTreeMap::new();
    for filename in filenames {
        let (oid, reported_byte_length, lfs) = raw_matches
            .remove(filename.as_str())
            .ok_or_else(|| kind.missing_error())?;
        let identity = selected_artifact_identity(reported_byte_length, oid.as_str(), lfs, kind)?;
        matched.insert(
            filename.clone(),
            SelectedArtifactMetadata {
                reported_byte_length,
                identity,
            },
        );
    }
    if !raw_matches.is_empty() {
        return Err(kind.unexpected_error());
    }
    Ok(matched)
}

fn selected_artifact_identity(
    reported_byte_length: u64,
    oid: &str,
    lfs: Option<BlobLfsInfo>,
    kind: SelectedMetadataKind,
) -> Result<SelectedArtifactIdentity, HubError> {
    let Some(BlobLfsInfo {
        size: lfs_byte_length,
        sha256,
        ..
    }) = lfs
    else {
        return decode_git_sha1(oid).map(SelectedArtifactIdentity::HuggingFaceGitBlob);
    };
    if lfs_byte_length.is_some_and(|byte_length| byte_length != reported_byte_length) {
        return Err(kind.length_error());
    }
    let sha256 = sha256.ok_or(HubError::InvalidLfsContentIdentity)?;

    Ok(SelectedArtifactIdentity::HuggingFaceLfs(
        ArtifactContentIdentity {
            byte_length: reported_byte_length,
            sha256: decode_sha256(sha256.as_str())?,
            authority: ArtifactContentIdentityAuthority::HuggingFaceLfs,
        },
    ))
}

pub(crate) fn resolve_weight_shard(
    path: PathBuf,
    metadata: SelectedArtifactMetadata,
) -> Result<ResolvedSafetensorsShard, HubError> {
    let mut file = File::open(&path).map_err(HubError::ReadWeight)?;
    let local_metadata = file.metadata().map_err(HubError::ReadWeight)?;
    if !local_metadata.is_file() {
        return Err(HubError::InvalidWeightFile);
    }
    let local_byte_length = local_metadata.len();
    if local_byte_length != metadata.reported_byte_length {
        return Err(HubError::ShardLengthMismatch);
    }

    let content_identity = match metadata.identity {
        SelectedArtifactIdentity::HuggingFaceLfs(identity) => {
            if identity.byte_length != local_byte_length {
                return Err(HubError::ShardLengthMismatch);
            }
            identity
        }
        SelectedArtifactIdentity::HuggingFaceGitBlob(expected_sha1) => {
            establish_git_blob_content_identity(&mut file, local_byte_length, expected_sha1)?
        }
    };

    Ok(ResolvedSafetensorsShard {
        path,
        content_identity,
    })
}

fn establish_git_blob_content_identity<R: Read>(
    reader: &mut R,
    expected_byte_length: u64,
    expected_git_sha1: [u8; GIT_SHA1_BYTES],
) -> Result<ArtifactContentIdentity, HubError> {
    let mut sha256 = Sha256::new();
    let mut git_sha1 = git_blob_hasher(expected_byte_length);
    let mut buffer = vec![0_u8; HASH_BUFFER_BYTES].into_boxed_slice();
    let mut observed_byte_length = 0_u64;
    let mut remaining = expected_byte_length;

    while remaining > 0 {
        let chunk_length = usize::try_from(remaining.min(HASH_BUFFER_BYTES_U64))
            .map_err(|_| HubError::ShardLengthMismatch)?;
        let chunk = buffer
            .get_mut(..chunk_length)
            .ok_or(HubError::ShardLengthMismatch)?;
        match reader.read_exact(chunk) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::UnexpectedEof => {
                return Err(HubError::ShardLengthMismatch);
            }
            Err(error) => return Err(HubError::ReadWeight(error)),
        }
        sha256.update(&*chunk);
        git_sha1.update(&*chunk);
        let chunk_length =
            u64::try_from(chunk_length).map_err(|_| HubError::ShardLengthMismatch)?;
        observed_byte_length = observed_byte_length
            .checked_add(chunk_length)
            .ok_or(HubError::ShardLengthMismatch)?;
        remaining = remaining
            .checked_sub(chunk_length)
            .ok_or(HubError::ShardLengthMismatch)?;
    }

    let mut trailing = [0_u8; 1];
    match reader.read(&mut trailing) {
        Ok(0) => {}
        Ok(_) => return Err(HubError::ShardLengthMismatch),
        Err(error) => return Err(HubError::ReadWeight(error)),
    }
    if observed_byte_length != expected_byte_length {
        return Err(HubError::ShardLengthMismatch);
    }
    let observed_git_sha1: [u8; GIT_SHA1_BYTES] = git_sha1.finalize().into();
    if observed_git_sha1 != expected_git_sha1 {
        return Err(HubError::ShardContentIdentityMismatch);
    }

    Ok(ArtifactContentIdentity {
        byte_length: observed_byte_length,
        sha256: sha256.finalize().into(),
        authority: ArtifactContentIdentityAuthority::HuggingFaceGitBlob,
    })
}

pub(crate) fn verify_git_blob_bytes(
    bytes: &[u8],
    expected_git_sha1: [u8; GIT_SHA1_BYTES],
) -> Result<(), ()> {
    let byte_length = u64::try_from(bytes.len()).map_err(|_| ())?;
    let mut hasher = git_blob_hasher(byte_length);
    hasher.update(bytes);
    let observed: [u8; GIT_SHA1_BYTES] = hasher.finalize().into();
    if observed == expected_git_sha1 {
        Ok(())
    } else {
        Err(())
    }
}

fn git_blob_hasher(byte_length: u64) -> Sha1 {
    let mut hasher = Sha1::new();
    hasher.update(b"blob ");
    hasher.update(byte_length.to_string().as_bytes());
    hasher.update([0]);
    hasher
}

fn decode_git_sha1(value: &str) -> Result<[u8; GIT_SHA1_BYTES], HubError> {
    decode_hex::<GIT_SHA1_BYTES>(value).map_err(|()| HubError::InvalidGitContentIdentity)
}

fn decode_sha256(value: &str) -> Result<[u8; SHA256_BYTES], HubError> {
    decode_hex::<SHA256_BYTES>(value).map_err(|()| HubError::InvalidLfsContentIdentity)
}

fn decode_hex<const N: usize>(value: &str) -> Result<[u8; N], ()> {
    if value.len() != N * 2 || !value.is_ascii() {
        return Err(());
    }
    let mut decoded = [0_u8; N];
    for (slot, pair) in decoded.iter_mut().zip(value.as_bytes().chunks_exact(2)) {
        let high = hex_nibble(pair.first().copied().ok_or(())?)?;
        let low = hex_nibble(pair.get(1).copied().ok_or(())?)?;
        *slot = (high << 4) | low;
    }
    Ok(decoded)
}

const fn hex_nibble(value: u8) -> Result<u8, ()> {
    match value {
        b'0'..=b'9' => Ok(value - b'0'),
        b'a'..=b'f' => Ok(value - b'a' + 10),
        b'A'..=b'F' => Ok(value - b'A' + 10),
        _ => Err(()),
    }
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use hf_hub::repository::{BlobLfsInfo, RepoTreeEntry};
    use sha2::{Digest, Sha256};

    use super::{
        SelectedArtifactIdentity, decode_git_sha1, decode_sha256,
        establish_git_blob_content_identity, match_selected_weight_metadata,
    };
    use crate::{ArtifactContentIdentityAuthority, HubError};

    const SHA256_ABC: &str = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";
    const GIT_BLOB_ABC: &str = "f2ba8f84ab5c1bce84a7b441cb1959cfc7093b7f";
    const FIRST_SHARD: &str = "model-00001-of-00002.safetensors";
    const SECOND_SHARD: &str = "model-00002-of-00002.safetensors";

    #[test]
    fn lfs_sha256_decoding_requires_exact_hex() -> Result<(), HubError> {
        let expected: [u8; 32] = Sha256::digest(b"abc").into();
        assert_eq!(decode_sha256(SHA256_ABC)?, expected);
        assert_eq!(
            decode_sha256(SHA256_ABC.to_ascii_uppercase().as_str())?,
            expected
        );
        for invalid in [
            "abc".to_owned(),
            "g".repeat(64),
            "a".repeat(63),
            "a".repeat(65),
            "é".repeat(32),
        ] {
            assert!(matches!(
                decode_sha256(invalid.as_str()),
                Err(HubError::InvalidLfsContentIdentity)
            ));
        }
        Ok(())
    }

    #[test]
    fn exact_selected_metadata_classifies_lfs_and_git_blob_identities() -> Result<(), HubError> {
        let filenames = vec![FIRST_SHARD.to_owned(), SECOND_SHARD.to_owned()];
        let entries = vec![
            file_entry(SECOND_SHARD, 7, Some(lfs_info(Some(7), Some(SHA256_ABC)))),
            file_entry(FIRST_SHARD, 5, None),
        ];
        let metadata = match_selected_weight_metadata(&filenames, entries)?;
        let trusted = metadata
            .get(SECOND_SHARD)
            .and_then(|metadata| match metadata.identity {
                SelectedArtifactIdentity::HuggingFaceLfs(identity) => Some(identity),
                SelectedArtifactIdentity::HuggingFaceGitBlob(_) => None,
            })
            .ok_or(HubError::MissingShardMetadata)?;
        assert_eq!(trusted.byte_length, 7);
        assert_eq!(
            trusted.authority,
            ArtifactContentIdentityAuthority::HuggingFaceLfs
        );
        assert_eq!(trusted.sha256, decode_sha256(SHA256_ABC)?);
        let git_blob_abc = decode_git_sha1(GIT_BLOB_ABC)?;
        assert!(metadata.get(FIRST_SHARD).is_some_and(|metadata| {
            metadata.identity == SelectedArtifactIdentity::HuggingFaceGitBlob(git_blob_abc)
        }));

        let outer_size_is_exact = match_selected_weight_metadata(
            &[FIRST_SHARD.to_owned()],
            vec![file_entry(
                FIRST_SHARD,
                5,
                Some(lfs_info(None, Some(SHA256_ABC))),
            )],
        )?;
        assert!(
            outer_size_is_exact
                .get(FIRST_SHARD)
                .is_some_and(|metadata| {
                    matches!(
                        metadata.identity,
                        SelectedArtifactIdentity::HuggingFaceLfs(identity)
                            if identity.byte_length == 5
                                && identity.authority
                                    == ArtifactContentIdentityAuthority::HuggingFaceLfs
                    )
                })
        );

        assert!(matches!(
            match_selected_weight_metadata(
                &[FIRST_SHARD.to_owned()],
                vec![file_entry(FIRST_SHARD, 5, Some(lfs_info(Some(5), None)))],
            ),
            Err(HubError::InvalidLfsContentIdentity)
        ));
        Ok(())
    }

    #[test]
    fn selected_metadata_rejects_cardinality_type_and_identity_failures() {
        let filenames = vec!["model.safetensors".to_owned()];
        assert!(matches!(
            match_selected_weight_metadata(&filenames, Vec::new()),
            Err(HubError::MissingShardMetadata)
        ));

        let entry = file_entry("model.safetensors", 3, None);
        assert!(matches!(
            match_selected_weight_metadata(&filenames, vec![entry.clone(), entry]),
            Err(HubError::DuplicateShardMetadata)
        ));
        assert!(matches!(
            match_selected_weight_metadata(
                &filenames,
                vec![file_entry("other.safetensors", 3, None)]
            ),
            Err(HubError::UnexpectedShardMetadata)
        ));
        assert!(matches!(
            match_selected_weight_metadata(
                &filenames,
                vec![RepoTreeEntry::Directory {
                    oid: "unused".to_owned(),
                    path: "model.safetensors".to_owned(),
                    last_commit: None,
                }]
            ),
            Err(HubError::UnexpectedShardMetadata)
        ));
        assert!(matches!(
            match_selected_weight_metadata(
                &filenames,
                vec![file_entry(
                    "model.safetensors",
                    3,
                    Some(lfs_info(Some(3), Some("not-a-sha256"))),
                )]
            ),
            Err(HubError::InvalidLfsContentIdentity)
        ));
        assert!(matches!(
            match_selected_weight_metadata(
                &filenames,
                vec![file_entry(
                    "model.safetensors",
                    3,
                    Some(lfs_info(Some(4), Some(SHA256_ABC))),
                )]
            ),
            Err(HubError::ShardLengthMismatch)
        ));
        let mut invalid_git = file_entry("model.safetensors", 3, None);
        if let RepoTreeEntry::File { oid, .. } = &mut invalid_git {
            *oid = "not-a-git-object-id".to_owned();
        }
        assert!(matches!(
            match_selected_weight_metadata(&filenames, vec![invalid_git]),
            Err(HubError::InvalidGitContentIdentity)
        ));
    }

    #[test]
    fn metadata_error_precedence_does_not_depend_on_response_order() {
        let filenames = vec!["model.safetensors".to_owned()];
        let selected = file_entry("model.safetensors", 3, None);
        let unexpected = file_entry("other.safetensors", 3, None);
        for entries in [
            vec![selected.clone(), selected.clone(), unexpected.clone()],
            vec![unexpected.clone(), selected.clone(), selected.clone()],
        ] {
            assert!(matches!(
                match_selected_weight_metadata(&filenames, entries),
                Err(HubError::UnexpectedShardMetadata)
            ));
        }
    }

    #[test]
    fn git_blob_verification_establishes_sha256_and_is_length_bound() -> Result<(), HubError> {
        let expected_git_sha1 = decode_git_sha1(GIT_BLOB_ABC)?;
        let mut exact = Cursor::new(b"abc".to_vec());
        let identity = establish_git_blob_content_identity(&mut exact, 3, expected_git_sha1)?;
        assert_eq!(identity.byte_length, 3);
        assert_eq!(identity.sha256, decode_sha256(SHA256_ABC)?);
        assert_eq!(
            identity.authority,
            ArtifactContentIdentityAuthority::HuggingFaceGitBlob
        );

        let mut truncated = Cursor::new(b"ab".to_vec());
        assert!(matches!(
            establish_git_blob_content_identity(&mut truncated, 3, expected_git_sha1),
            Err(HubError::ShardLengthMismatch)
        ));
        let mut trailing = Cursor::new(b"abcd".to_vec());
        assert!(matches!(
            establish_git_blob_content_identity(&mut trailing, 3, expected_git_sha1),
            Err(HubError::ShardLengthMismatch)
        ));
        let mut changed = Cursor::new(b"abd".to_vec());
        assert!(matches!(
            establish_git_blob_content_identity(&mut changed, 3, expected_git_sha1),
            Err(HubError::ShardContentIdentityMismatch)
        ));
        Ok(())
    }

    fn file_entry(path: &str, size: u64, lfs: Option<BlobLfsInfo>) -> RepoTreeEntry {
        RepoTreeEntry::File {
            oid: GIT_BLOB_ABC.to_owned(),
            size,
            path: path.to_owned(),
            lfs,
            last_commit: None,
            xet_hash: None,
            security: None,
        }
    }

    fn lfs_info(size: Option<u64>, sha256: Option<&str>) -> BlobLfsInfo {
        BlobLfsInfo {
            size,
            sha256: sha256.map(str::to_owned),
            pointer_size: None,
        }
    }
}
