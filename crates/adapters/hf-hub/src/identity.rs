use std::collections::{BTreeMap, BTreeSet};
use std::fs::File;
use std::io::{self, Read};
use std::path::PathBuf;

use hf_hub::repository::{BlobLfsInfo, RepoTreeEntry};
use hf_hub::{HFRepositorySync, RepoTypeModel};
use sha2::{Digest, Sha256};

use crate::{
    ArtifactContentIdentity, ArtifactContentIdentityAuthority, HubError, ResolvedSafetensorsShard,
};

const SHA256_BYTES: usize = 32;
/// A fixed 64 KiB buffer keeps fallback hashing independent of artifact size.
const HASH_BUFFER_BYTES: usize = 64 * 1024;
const HASH_BUFFER_BYTES_U64: u64 = 64 * 1024;

#[derive(Clone, Copy, Debug)]
pub(crate) struct SelectedWeightMetadata {
    reported_byte_length: u64,
    lfs_content_identity: Option<ArtifactContentIdentity>,
}

pub(crate) fn selected_weight_metadata(
    repository: &HFRepositorySync<RepoTypeModel>,
    commit: &str,
    filenames: &[String],
) -> Result<BTreeMap<String, SelectedWeightMetadata>, HubError> {
    crate::weights::validate_selected_weight_shard_count(filenames.len())?;
    let entries = repository
        .get_paths_info()
        .paths(filenames.to_vec())
        .revision(commit.to_owned())
        .send()
        .map_err(HubError::ArtifactMetadata)?;
    match_selected_weight_metadata(filenames, entries)
}

fn match_selected_weight_metadata(
    filenames: &[String],
    entries: Vec<RepoTreeEntry>,
) -> Result<BTreeMap<String, SelectedWeightMetadata>, HubError> {
    crate::weights::validate_selected_weight_shard_count(filenames.len())?;
    let selected: BTreeSet<&str> = filenames.iter().map(String::as_str).collect();
    if selected.len() != filenames.len() {
        return Err(HubError::DuplicateShardMetadata);
    }

    let mut raw_matches = BTreeMap::new();
    let mut duplicate = false;
    let mut unexpected = false;
    for entry in entries {
        let RepoTreeEntry::File {
            path, size, lfs, ..
        } = entry
        else {
            unexpected = true;
            continue;
        };
        if !selected.contains(path.as_str()) {
            unexpected = true;
            continue;
        }
        if raw_matches.insert(path, (size, lfs)).is_some() {
            duplicate = true;
        }
    }

    // Fixed precedence makes malformed response classification independent of response order.
    if unexpected {
        return Err(HubError::UnexpectedShardMetadata);
    }
    if duplicate {
        return Err(HubError::DuplicateShardMetadata);
    }
    if raw_matches.len() != selected.len() {
        return Err(HubError::MissingShardMetadata);
    }

    let mut matched = BTreeMap::new();
    for filename in filenames {
        let (reported_byte_length, lfs) = raw_matches
            .remove(filename.as_str())
            .ok_or(HubError::MissingShardMetadata)?;
        let lfs_content_identity = lfs_content_identity(reported_byte_length, lfs)?;
        matched.insert(
            filename.clone(),
            SelectedWeightMetadata {
                reported_byte_length,
                lfs_content_identity,
            },
        );
    }
    if !raw_matches.is_empty() {
        return Err(HubError::UnexpectedShardMetadata);
    }
    Ok(matched)
}

fn lfs_content_identity(
    reported_byte_length: u64,
    lfs: Option<BlobLfsInfo>,
) -> Result<Option<ArtifactContentIdentity>, HubError> {
    let Some(BlobLfsInfo {
        size: lfs_byte_length,
        sha256,
        ..
    }) = lfs
    else {
        return Ok(None);
    };
    if lfs_byte_length.is_some_and(|byte_length| byte_length != reported_byte_length) {
        return Err(HubError::ShardLengthMismatch);
    }
    let Some(sha256) = sha256 else {
        return Ok(None);
    };

    Ok(Some(ArtifactContentIdentity {
        byte_length: reported_byte_length,
        sha256: decode_sha256(sha256.as_str())?,
        authority: ArtifactContentIdentityAuthority::HuggingFaceLfs,
    }))
}

pub(crate) fn resolve_weight_shard(
    path: PathBuf,
    metadata: SelectedWeightMetadata,
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

    let content_identity = match metadata.lfs_content_identity {
        Some(identity) => {
            if identity.byte_length != local_byte_length {
                return Err(HubError::ShardLengthMismatch);
            }
            identity
        }
        None => establish_project_content_identity(&mut file, local_byte_length)?,
    };

    Ok(ResolvedSafetensorsShard {
        path,
        content_identity,
    })
}

fn establish_project_content_identity<R: Read>(
    reader: &mut R,
    expected_byte_length: u64,
) -> Result<ArtifactContentIdentity, HubError> {
    let mut hasher = Sha256::new();
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
        hasher.update(chunk);
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

    Ok(ArtifactContentIdentity {
        byte_length: observed_byte_length,
        sha256: hasher.finalize().into(),
        authority: ArtifactContentIdentityAuthority::ProjectEstablished,
    })
}

fn decode_sha256(value: &str) -> Result<[u8; SHA256_BYTES], HubError> {
    if value.len() != SHA256_BYTES * 2 || !value.is_ascii() {
        return Err(HubError::InvalidLfsContentIdentity);
    }
    let mut decoded = [0_u8; SHA256_BYTES];
    for (slot, pair) in decoded.iter_mut().zip(value.as_bytes().chunks_exact(2)) {
        let high = hex_nibble(
            pair.first()
                .copied()
                .ok_or(HubError::InvalidLfsContentIdentity)?,
        )?;
        let low = hex_nibble(
            pair.get(1)
                .copied()
                .ok_or(HubError::InvalidLfsContentIdentity)?,
        )?;
        *slot = (high << 4) | low;
    }
    Ok(decoded)
}

const fn hex_nibble(value: u8) -> Result<u8, HubError> {
    match value {
        b'0'..=b'9' => Ok(value - b'0'),
        b'a'..=b'f' => Ok(value - b'a' + 10),
        b'A'..=b'F' => Ok(value - b'A' + 10),
        _ => Err(HubError::InvalidLfsContentIdentity),
    }
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use hf_hub::repository::{BlobLfsInfo, RepoTreeEntry};
    use sha2::{Digest, Sha256};

    use super::{
        decode_sha256, establish_project_content_identity, match_selected_weight_metadata,
    };
    use crate::{ArtifactContentIdentityAuthority, HubError};

    const SHA256_ABC: &str = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";
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
    fn exact_selected_metadata_classifies_lfs_and_fallback_identities() -> Result<(), HubError> {
        let filenames = vec![FIRST_SHARD.to_owned(), SECOND_SHARD.to_owned()];
        let entries = vec![
            file_entry(SECOND_SHARD, 7, Some(lfs_info(Some(7), Some(SHA256_ABC)))),
            file_entry(FIRST_SHARD, 5, None),
        ];
        let metadata = match_selected_weight_metadata(&filenames, entries)?;
        let trusted = metadata
            .get(SECOND_SHARD)
            .and_then(|metadata| metadata.lfs_content_identity)
            .ok_or(HubError::MissingShardMetadata)?;
        assert_eq!(trusted.byte_length, 7);
        assert_eq!(
            trusted.authority,
            ArtifactContentIdentityAuthority::HuggingFaceLfs
        );
        assert_eq!(trusted.sha256, decode_sha256(SHA256_ABC)?);
        assert!(
            metadata
                .get(FIRST_SHARD)
                .is_some_and(|metadata| metadata.lfs_content_identity.is_none())
        );

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
                    metadata.lfs_content_identity.is_some_and(|identity| {
                        identity.byte_length == 5
                            && identity.authority
                                == ArtifactContentIdentityAuthority::HuggingFaceLfs
                    })
                })
        );

        let missing_sha = match_selected_weight_metadata(
            &[FIRST_SHARD.to_owned()],
            vec![file_entry(FIRST_SHARD, 5, Some(lfs_info(Some(5), None)))],
        )?;
        assert!(
            missing_sha
                .get(FIRST_SHARD)
                .is_some_and(|metadata| metadata.lfs_content_identity.is_none())
        );
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
    fn fallback_hashing_is_project_established_and_length_bound() -> Result<(), HubError> {
        let mut exact = Cursor::new(b"abc".to_vec());
        let identity = establish_project_content_identity(&mut exact, 3)?;
        assert_eq!(identity.byte_length, 3);
        assert_eq!(identity.sha256, decode_sha256(SHA256_ABC)?);
        assert_eq!(
            identity.authority,
            ArtifactContentIdentityAuthority::ProjectEstablished
        );

        let mut truncated = Cursor::new(b"ab".to_vec());
        assert!(matches!(
            establish_project_content_identity(&mut truncated, 3),
            Err(HubError::ShardLengthMismatch)
        ));
        let mut trailing = Cursor::new(b"abcd".to_vec());
        assert!(matches!(
            establish_project_content_identity(&mut trailing, 3),
            Err(HubError::ShardLengthMismatch)
        ));
        Ok(())
    }

    fn file_entry(path: &str, size: u64, lfs: Option<BlobLfsInfo>) -> RepoTreeEntry {
        RepoTreeEntry::File {
            oid: "unused".to_owned(),
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
