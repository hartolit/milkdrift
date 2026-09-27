//! Scratch is disposable only after verified exit and container fencing. Unknown work stays charged.
use crate::{platform::private_directory, rejected};
use milkdrift_capability_host::managed::ManagedError;
use std::{
    fs,
    path::{Path, PathBuf},
};

const MAX_DIRECTORIES: usize = 32;
const MAX_BYTES: u64 = 1_073_741_824;
const MAX_ENTRIES: usize = 4096;
const MAX_DEPTH: u8 = 8;

pub(super) fn prepare(
    root: &Path,
    identity: &str,
    reserved_bytes: u64,
) -> Result<PathBuf, ManagedError> {
    reserve(root, identity, reserved_bytes, MAX_DIRECTORIES, MAX_BYTES)
}

fn reserve(
    root: &Path,
    identity: &str,
    reserved_bytes: u64,
    maximum_directories: usize,
    maximum_bytes: u64,
) -> Result<PathBuf, ManagedError> {
    private_directory(root)?;
    if !milkdrift_contracts::is_canonical_blake3_digest(identity) {
        return Err(rejected("verification scratch identity is invalid"));
    }
    let mut directories = 0;
    let mut entries = 0;
    let mut bytes = 0_u64;
    for entry in fs::read_dir(root).map_err(rejected)? {
        entries += 1;
        if entries > MAX_ENTRIES {
            return Err(rejected(
                "verification scratch inventory exceeds its inspection bound",
            ));
        }
        let entry = entry.map_err(rejected)?;
        let name = entry.file_name();
        let Some(name) = name
            .to_str()
            .and_then(|name| name.strip_prefix("verification-"))
        else {
            continue;
        };
        if !milkdrift_contracts::is_canonical_blake3_digest(name) {
            return Err(rejected(
                "unrecognized verification scratch requires inspection",
            ));
        }
        directories += 1;
        if directories >= maximum_directories {
            return Err(rejected(
                "retained verification scratch directory limit reached; inspect and clean settled scratch",
            ));
        }
        private_directory(&entry.path())?;
        measure(&entry.path(), 0, &mut entries, &mut bytes, maximum_bytes)?;
    }
    if bytes
        .checked_add(reserved_bytes)
        .is_none_or(|size| size > maximum_bytes)
    {
        return Err(rejected(
            "retained verification scratch byte limit reached; inspect and clean settled scratch",
        ));
    }
    let directory = root.join(format!("verification-{identity}"));
    let mut builder = fs::DirBuilder::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(&directory).map_err(|_| {
        rejected("verification scratch already exists; prior observation requires inspection")
    })?;
    Ok(directory)
}

fn measure(
    path: &Path,
    depth: u8,
    entries: &mut usize,
    bytes: &mut u64,
    maximum_bytes: u64,
) -> Result<(), ManagedError> {
    if depth > MAX_DEPTH {
        return Err(rejected(
            "verification scratch nesting exceeds its inspection bound",
        ));
    }
    for entry in fs::read_dir(path).map_err(rejected)? {
        *entries += 1;
        if *entries > MAX_ENTRIES {
            return Err(rejected(
                "verification scratch entries exceed their inspection bound",
            ));
        }
        let entry = entry.map_err(rejected)?;
        let metadata = fs::symlink_metadata(entry.path()).map_err(rejected)?;
        if metadata.is_dir() {
            measure(&entry.path(), depth + 1, entries, bytes, maximum_bytes)?;
        } else if metadata.is_file() {
            *bytes = bytes
                .checked_add(metadata.len())
                .ok_or_else(|| rejected("verification scratch byte count overflow"))?;
            if *bytes > maximum_bytes {
                return Err(rejected(
                    "retained verification scratch exceeds its byte limit",
                ));
            }
        } else {
            return Err(rejected(
                "verification scratch contains a link or special file; inspect before cleanup",
            ));
        }
    }
    Ok(())
}

pub(super) fn remove(directory: &Path) -> Result<(), ManagedError> {
    private_directory(directory)?;
    let mut entries = 0;
    let mut bytes = 0;
    measure(directory, 0, &mut entries, &mut bytes, MAX_BYTES)?;
    fs::remove_dir_all(directory).map_err(rejected)?;
    let parent = directory
        .parent()
        .ok_or_else(|| rejected("verification scratch parent absent"))?;
    fs::File::open(parent)
        .and_then(|file| file.sync_all())
        .map_err(rejected)
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;
    type Result = std::result::Result<(), Box<dyn std::error::Error>>;
    fn identity(number: usize) -> String {
        format!("b3_{number:064x}")
    }
    fn private_root() -> std::result::Result<tempfile::TempDir, Box<dyn std::error::Error>> {
        use std::os::unix::fs::PermissionsExt;
        let root = tempfile::tempdir()?;
        fs::set_permissions(root.path(), fs::Permissions::from_mode(0o700))?;
        Ok(root)
    }

    #[test]
    fn settled_scratch_turns_over_but_retained_bytes_and_directories_block_admission() -> Result {
        let root = private_root()?;
        for number in 0..6 {
            let directory = reserve(root.path(), &identity(number), 4, 1, 4)?;
            fs::write(directory.join("candidate"), b"data")?;
            assert!(reserve(root.path(), &identity(number + 1), 0, 1, 4).is_err());
            assert!(reserve(root.path(), &identity(number + 1), 1, 2, 4).is_err());
            let next = reserve(root.path(), &identity(number + 1), 0, 2, 4)?;
            remove(&next)?;
            remove(&directory)?;
        }
        assert_eq!(fs::read_dir(root.path())?.count(), 0);
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn scratch_cleanup_refuses_links_and_does_not_touch_their_targets() -> Result {
        let root = private_root()?;
        let outside = tempfile::tempdir()?;
        fs::write(outside.path().join("evidence"), b"retain")?;
        let directory = reserve(root.path(), &identity(1), 4, 2, 8)?;
        std::os::unix::fs::symlink(outside.path(), directory.join("foreign"))?;
        assert!(remove(&directory).is_err());
        assert!(reserve(root.path(), &identity(2), 0, 2, 8).is_err());
        assert_eq!(fs::read(outside.path().join("evidence"))?, b"retain");
        assert!(reserve(root.path(), "../outside", 0, 2, 8).is_err());
        Ok(())
    }
}
