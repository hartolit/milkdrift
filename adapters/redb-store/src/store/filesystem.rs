use super::{File, Path, PersistenceError, error, fs};
pub(crate) fn prepare_owned_directory(
    path: &Path,
    family: &'static str,
) -> Result<(), PersistenceError> {
    let existed = match fs::symlink_metadata(path) {
        Ok(metadata) => {
            validate_owned_directory_type(&metadata, family)?;
            true
        }
        Err(cause) if cause.kind() == std::io::ErrorKind::NotFound => false,
        Err(cause) => return Err(error::io(cause)),
    };
    if !existed {
        fs::create_dir_all(path).map_err(error::io)?;
        let metadata = fs::symlink_metadata(path).map_err(error::io)?;
        validate_owned_directory_type(&metadata, family)?;
        if let Some(parent) = path.parent() {
            sync_owned_directory(parent)?;
        }
    }
    Ok(())
}

pub(crate) fn sync_owned_directory(path: &Path) -> Result<(), PersistenceError> {
    let metadata = fs::symlink_metadata(path).map_err(error::io)?;
    validate_owned_directory_type(&metadata, "storage directory")?;
    let directory = open_directory_no_follow(path)?;
    verify_opened_identity(path, &directory, true)?;
    directory.sync_all().map_err(error::io)
}

#[cfg(unix)]
pub(crate) fn open_directory_no_follow(path: &Path) -> Result<File, PersistenceError> {
    use rustix::fs::{Mode, OFlags};

    rustix::fs::open(
        path,
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .map(File::from)
    .map_err(|cause| {
        if cause == rustix::io::Errno::LOOP {
            error::corruption("storage directory changed into a symlink while opening")
        } else {
            error::io(cause.into())
        }
    })
}

#[cfg(not(unix))]
pub(crate) fn open_directory_no_follow(path: &Path) -> Result<File, PersistenceError> {
    File::open(path).map_err(error::io)
}

pub(crate) fn verify_opened_identity(
    path: &Path,
    opened: &File,
    expect_directory: bool,
) -> Result<(), PersistenceError> {
    let opened_metadata = opened.metadata().map_err(error::io)?;
    let path_metadata = fs::symlink_metadata(path).map_err(error::io)?;
    let expected_type = if expect_directory {
        opened_metadata.is_dir() && path_metadata.file_type().is_dir()
    } else {
        opened_metadata.is_file() && path_metadata.file_type().is_file()
    };
    if !expected_type || path_metadata.file_type().is_symlink() {
        return Err(error::corruption(
            "storage path changed type or became a symlink while opening",
        ));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;

        if opened_metadata.dev() != path_metadata.dev()
            || opened_metadata.ino() != path_metadata.ino()
        {
            return Err(error::corruption(
                "storage path identity changed while opening",
            ));
        }
    }
    Ok(())
}

pub(crate) fn validate_owned_directory_type(
    metadata: &fs::Metadata,
    family: &'static str,
) -> Result<(), PersistenceError> {
    if !metadata.file_type().is_dir() || metadata.file_type().is_symlink() {
        return Err(error::corruption(format!(
            "{family} must be an owned directory, not a symlink or special file"
        )));
    }
    Ok(())
}

pub(crate) fn ensure_regular_file_or_absent(
    path: &Path,
    family: &'static str,
) -> Result<(), PersistenceError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_file() => Ok(()),
        Ok(_) => Err(error::corruption(format!(
            "{family} must be a regular file, not a symlink or special file"
        ))),
        Err(cause) if cause.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(cause) => Err(error::io(cause)),
    }
}
