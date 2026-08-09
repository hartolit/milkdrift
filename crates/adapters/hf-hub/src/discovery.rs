use std::collections::BTreeSet;

use hf_hub::repository::{FileMetadataInfo, RepoTreeEntry};
use hf_hub::{HFRepositorySync, RepoTypeModel};

use crate::{CONFIG_FILE, HubError, HubStructuralLimit};

/// Supported model repositories are expected to remain far below 4,096 tree entries.
const MAX_REPOSITORY_ENTRIES: usize = 4_096;
/// Hub paths for the supported layouts are normally well below 1,024 UTF-8 bytes.
const MAX_REPOSITORY_ENTRY_PATH_BYTES: usize = 1_024;
/// Four MiB bounds total owned repository path data with substantial model-layout headroom.
const MAX_REPOSITORY_AGGREGATE_PATH_BYTES: usize = 4 * 1_024 * 1_024;

const REPOSITORY_TREE_LIMITS: RepositoryTreeLimits = RepositoryTreeLimits {
    entry_count: MAX_REPOSITORY_ENTRIES,
    per_path_bytes: MAX_REPOSITORY_ENTRY_PATH_BYTES,
    aggregate_path_bytes: MAX_REPOSITORY_AGGREGATE_PATH_BYTES,
};

#[derive(Clone, Copy)]
struct RepositoryTreeLimits {
    entry_count: usize,
    per_path_bytes: usize,
    aggregate_path_bytes: usize,
}

#[derive(Default)]
struct RepositoryTreeCollection {
    entry_count: usize,
    aggregate_path_bytes: usize,
    file_paths: BTreeSet<String>,
}

pub(crate) fn resolve_commit_and_files(
    repository: &HFRepositorySync<RepoTypeModel>,
    requested_revision: &str,
) -> Result<(String, BTreeSet<String>), HubError> {
    let FileMetadataInfo { commit_hash, .. } = repository
        .get_file_metadata()
        .filepath(CONFIG_FILE)
        .revision(requested_revision)
        .send()
        .map_err(HubError::RepositoryInfo)?;
    let commit = canonical_commit(commit_hash.as_str())?.to_owned();
    let request_limit =
        MAX_REPOSITORY_ENTRIES
            .checked_add(1)
            .ok_or(HubError::StructuralLimitExceeded(
                HubStructuralLimit::RepositoryEntries,
            ))?;
    let entries = repository
        .list_tree()
        .revision(commit.clone())
        .recursive(true)
        .limit(request_limit)
        .send()
        .map_err(HubError::RepositoryInfo)?;
    let files = collect_available_files(entries)?;
    Ok((commit, files))
}

fn canonical_commit(commit: &str) -> Result<&str, HubError> {
    if commit.len() == 40
        && commit
            .bytes()
            .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
    {
        Ok(commit)
    } else {
        Err(HubError::InvalidCommit)
    }
}

fn collect_available_files<I>(entries: I) -> Result<BTreeSet<String>, HubError>
where
    I: IntoIterator<Item = RepoTreeEntry>,
{
    collect_available_files_with_limits(entries, REPOSITORY_TREE_LIMITS)
}

fn collect_available_files_with_limits<I>(
    entries: I,
    limits: RepositoryTreeLimits,
) -> Result<BTreeSet<String>, HubError>
where
    I: IntoIterator<Item = RepoTreeEntry>,
{
    entries
        .into_iter()
        .try_fold(
            RepositoryTreeCollection::default(),
            |mut collection, entry| {
                collection.entry_count = collection.entry_count.checked_add(1).ok_or(
                    HubError::StructuralLimitExceeded(HubStructuralLimit::RepositoryEntries),
                )?;
                if collection.entry_count > limits.entry_count {
                    return Err(HubError::StructuralLimitExceeded(
                        HubStructuralLimit::RepositoryEntries,
                    ));
                }

                let (path, is_file) = match entry {
                    RepoTreeEntry::File { path, .. } => (path, true),
                    RepoTreeEntry::Directory { path, .. } => (path, false),
                };
                let path_bytes = path.len();
                if path_bytes > limits.per_path_bytes {
                    return Err(HubError::StructuralLimitExceeded(
                        HubStructuralLimit::RepositoryEntryPathBytes,
                    ));
                }
                collection.aggregate_path_bytes = collection
                    .aggregate_path_bytes
                    .checked_add(path_bytes)
                    .ok_or(HubError::StructuralLimitExceeded(
                        HubStructuralLimit::RepositoryAggregatePathBytes,
                    ))?;
                if collection.aggregate_path_bytes > limits.aggregate_path_bytes {
                    return Err(HubError::StructuralLimitExceeded(
                        HubStructuralLimit::RepositoryAggregatePathBytes,
                    ));
                }

                if is_file && !collection.file_paths.insert(path) {
                    return Err(HubError::DuplicateRepositoryFilePath);
                }
                Ok(collection)
            },
        )
        .map(|collection| collection.file_paths)
}

#[cfg(test)]
mod tests {
    use hf_hub::repository::RepoTreeEntry;

    use super::{
        MAX_REPOSITORY_AGGREGATE_PATH_BYTES, MAX_REPOSITORY_ENTRIES,
        MAX_REPOSITORY_ENTRY_PATH_BYTES, RepositoryTreeLimits, canonical_commit,
        collect_available_files, collect_available_files_with_limits,
    };
    use crate::{HubError, HubStructuralLimit};

    const VALID_COMMIT: &str = "0123456789abcdef0123456789abcdef01234567";

    #[test]
    fn commit_validation_requires_canonical_lowercase_forty_hex() -> Result<(), HubError> {
        assert_eq!(canonical_commit(VALID_COMMIT)?, VALID_COMMIT);
        for invalid in [
            String::new(),
            "0".repeat(39),
            "0".repeat(41),
            "g".repeat(40),
            VALID_COMMIT.to_ascii_uppercase(),
            format!(" {VALID_COMMIT}"),
        ] {
            let error = canonical_commit(invalid.as_str())
                .err()
                .ok_or(HubError::InvalidCommit)?;
            assert!(matches!(error, HubError::InvalidCommit));
            if !invalid.is_empty() {
                assert!(!error.to_string().contains(invalid.as_str()));
            }
        }
        Ok(())
    }

    #[test]
    fn repository_entry_count_boundary_and_sentinel_are_enforced() {
        let at_limit = (0..MAX_REPOSITORY_ENTRIES)
            .map(|index| directory_entry(format!("directory-{index}").as_str()));
        assert!(collect_available_files(at_limit).is_ok());

        let sentinel = (0..=MAX_REPOSITORY_ENTRIES)
            .map(|index| directory_entry(format!("directory-{index}").as_str()));
        assert!(matches!(
            collect_available_files(sentinel),
            Err(HubError::StructuralLimitExceeded(
                HubStructuralLimit::RepositoryEntries
            ))
        ));
    }

    #[test]
    fn repository_entry_path_limit_is_enforced() {
        let at_limit = "x".repeat(MAX_REPOSITORY_ENTRY_PATH_BYTES);
        assert!(collect_available_files([file_entry(at_limit.as_str())]).is_ok());

        let oversized = "x".repeat(MAX_REPOSITORY_ENTRY_PATH_BYTES + 1);
        assert!(matches!(
            collect_available_files([file_entry(oversized.as_str())]),
            Err(HubError::StructuralLimitExceeded(
                HubStructuralLimit::RepositoryEntryPathBytes
            ))
        ));
    }

    #[test]
    fn repository_aggregate_path_limit_is_enforced_independently() {
        let maximum_path = "x".repeat(MAX_REPOSITORY_ENTRY_PATH_BYTES);
        let at_limit = std::iter::repeat_with(|| directory_entry(maximum_path.as_str()))
            .take(MAX_REPOSITORY_ENTRIES);
        assert!(collect_available_files(at_limit).is_ok());
        assert_eq!(
            MAX_REPOSITORY_AGGREGATE_PATH_BYTES,
            MAX_REPOSITORY_ENTRIES * MAX_REPOSITORY_ENTRY_PATH_BYTES
        );

        let limits = RepositoryTreeLimits {
            entry_count: 2,
            per_path_bytes: 4,
            aggregate_path_bytes: 7,
        };
        assert!(matches!(
            collect_available_files_with_limits([file_entry("aaaa"), file_entry("bbbb")], limits),
            Err(HubError::StructuralLimitExceeded(
                HubStructuralLimit::RepositoryAggregatePathBytes
            ))
        ));
    }

    #[test]
    fn duplicate_file_paths_are_rejected() {
        assert!(matches!(
            collect_available_files([
                file_entry("model.safetensors"),
                file_entry("model.safetensors"),
            ]),
            Err(HubError::DuplicateRepositoryFilePath)
        ));
    }

    #[test]
    fn only_file_paths_enter_the_available_set() -> Result<(), HubError> {
        let files = collect_available_files([
            directory_entry("weights"),
            file_entry("weights/model.safetensors"),
        ])?;
        assert_eq!(
            files,
            ["weights/model.safetensors".to_owned()]
                .into_iter()
                .collect()
        );
        Ok(())
    }

    fn file_entry(path: &str) -> RepoTreeEntry {
        RepoTreeEntry::File {
            oid: "unused".to_owned(),
            size: 0,
            path: path.to_owned(),
            lfs: None,
            last_commit: None,
            xet_hash: None,
            security: None,
        }
    }

    fn directory_entry(path: &str) -> RepoTreeEntry {
        RepoTreeEntry::Directory {
            oid: "unused".to_owned(),
            path: path.to_owned(),
            last_commit: None,
        }
    }
}
