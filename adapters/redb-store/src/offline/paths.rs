//! File-owner boundary for offline copies. Reject links and non-owned destinations.
use crate::error;
use milkdrift_persistence::PersistenceError;
use std::{
    fs::{self, File, OpenOptions},
    io::Read,
    path::{Component, Path},
};

fn ordinary(metadata: &fs::Metadata, is_directory: bool) -> bool {
    let kind = if is_directory {
        metadata.is_dir()
    } else {
        metadata.is_file()
    };
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt as _;
        kind && metadata.file_attributes() & 0x400 == 0
    }
    #[cfg(not(windows))]
    {
        kind && !metadata.file_type().is_symlink()
    }
}

pub(super) fn directory(path: &Path) -> Result<(), PersistenceError> {
    // Validate ancestors as well as the final component: a junction must not lead
    // copying outside the stated root. File-owner authority excludes hostile renames.
    for ancestor in path.ancestors().filter(|p| !p.as_os_str().is_empty()) {
        if !ordinary(&fs::symlink_metadata(ancestor).map_err(error::io)?, true) {
            return Err(error::corruption(
                "offline paths must use ordinary directories without links or reparse points",
            ));
        }
    }
    Ok(())
}

pub(super) fn read_file(path: &Path) -> Result<File, PersistenceError> {
    directory(
        path.parent()
            .ok_or_else(|| error::corruption("file has no parent"))?,
    )?;
    if !ordinary(&fs::symlink_metadata(path).map_err(error::io)?, false) {
        return Err(error::corruption(
            "offline source must be a regular non-link file",
        ));
    }
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.custom_flags(rustix::fs::OFlags::NOFOLLOW.bits() as i32);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt as _;
        options.custom_flags(0x0020_0000);
    }
    let file = options.open(path).map_err(error::io)?;
    crate::store::filesystem::verify_opened_identity(path, &file, false)?;
    if !ordinary(&file.metadata().map_err(error::io)?, false) {
        return Err(error::corruption("offline source changed file type"));
    }
    Ok(file)
}

pub(super) fn new_file(path: &Path) -> Result<File, PersistenceError> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600);
    }
    options.open(path).map_err(error::io)
}

pub(super) fn create_directory(path: &Path) -> Result<(), PersistenceError> {
    #[cfg_attr(not(unix), allow(unused_mut))]
    let mut builder = fs::DirBuilder::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt as _;
        builder.mode(0o700);
    }
    builder.create(path).map_err(error::io)
}

pub(super) fn private_directory(path: &Path) -> Result<(), PersistenceError> {
    directory(path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt as _;
        let metadata = fs::metadata(path).map_err(error::io)?;
        if metadata.uid() != rustix::process::geteuid().as_raw() || metadata.mode() & 0o077 != 0 {
            return Err(error::corruption(
                "offline destination parent must be owned by the current user with mode 0700",
            ));
        }
    }
    #[cfg(windows)]
    {
        // Safe OS ACL inspection via the installed system PowerShell. Pass the path
        // as data, never interpolate it into executable text. Reject broad inherited
        // access before any sensitive copy is written. Administrator/System are allowed.
        use std::os::windows::process::CommandExt as _;
        let system = std::env::var_os("SystemRoot")
            .ok_or_else(|| error::corruption("SystemRoot is unavailable for ACL inspection"))?;
        let script = "$ErrorActionPreference='Stop'; $acl=[System.IO.Directory]::GetAccessControl($env:MILKDRIFT_OFFLINE_ACL_PATH); $me=[System.Security.Principal.WindowsIdentity]::GetCurrent().User.Value; $allowed=@($me,'S-1-5-18','S-1-5-32-544'); if ($allowed -notcontains $acl.GetOwner([System.Security.Principal.SecurityIdentifier]).Value) {exit 2}; foreach($rule in $acl.GetAccessRules($true,$true,[System.Security.Principal.SecurityIdentifier])) {if($rule.AccessControlType -eq 'Allow' -and $allowed -notcontains $rule.IdentityReference.Value) {exit 3}}; exit 0";
        let status = std::process::Command::new(
            Path::new(&system).join("System32/WindowsPowerShell/v1.0/powershell.exe"),
        )
        .args(["-NoProfile", "-NonInteractive", "-Command", script])
        .env("MILKDRIFT_OFFLINE_ACL_PATH", path)
        .creation_flags(0x0800_0000)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map_err(error::io)?;
        if !status.success() {
            return Err(error::corruption(
                "offline destination parent ACL must be owned by and allow only the current user, SYSTEM or Administrators; ACL inspection failed or broader access was found",
            ));
        }
    }
    #[cfg(not(any(unix, windows)))]
    return Err(error::corruption(
        "offline ownership is unsupported on this platform",
    ));
    Ok(())
}

pub(super) fn hash_file(path: &Path) -> Result<(u64, String), PersistenceError> {
    let mut file = read_file(path)?;
    let mut buffer = vec![0; super::COPY_CHUNK_BYTES];
    let mut digest = blake3::Hasher::new();
    let mut size = 0u64;
    loop {
        let count = file.read(&mut buffer).map_err(error::io)?;
        if count == 0 {
            break;
        }
        size = size
            .checked_add(count as u64)
            .ok_or_else(|| error::corruption("file size overflow"))?;
        digest.update(&buffer[..count]);
    }
    Ok((size, digest.finalize().to_hex().to_string()))
}

pub(super) fn relative(path: &str) -> Result<(), PersistenceError> {
    if path.len() > 512
        || path.is_empty()
        || path.contains('\\')
        || path.contains(':')
        || Path::new(path)
            .components()
            .any(|c| !matches!(c, Component::Normal(_)))
    {
        return Err(error::corruption("backup contains an unsafe relative path"));
    }
    Ok(())
}

#[cfg(any(test, feature = "test-admin"))]
pub(crate) fn private_test_directory() -> Result<tempfile::TempDir, PersistenceError> {
    let directory = tempfile::tempdir().map_err(error::io)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(directory.path(), std::fs::Permissions::from_mode(0o700))
            .map_err(error::io)?;
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt as _;
        let script = "$ErrorActionPreference='Stop'; $me=[System.Security.Principal.WindowsIdentity]::GetCurrent().User; $acl=New-Object System.Security.AccessControl.DirectorySecurity; $acl.SetOwner($me); $acl.SetAccessRuleProtection($true,$false); $rule=New-Object System.Security.AccessControl.FileSystemAccessRule($me,'FullControl','ContainerInherit,ObjectInherit','None','Allow'); $acl.AddAccessRule($rule); [System.IO.Directory]::SetAccessControl($env:MILKDRIFT_OFFLINE_ACL_PATH,$acl)";
        let system = std::env::var_os("SystemRoot")
            .ok_or_else(|| error::corruption("SystemRoot missing"))?;
        let status = std::process::Command::new(
            Path::new(&system).join("System32/WindowsPowerShell/v1.0/powershell.exe"),
        )
        .args(["-NoProfile", "-NonInteractive", "-Command", script])
        .env("MILKDRIFT_OFFLINE_ACL_PATH", directory.path())
        .creation_flags(0x0800_0000)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .status()
        .map_err(error::io)?;
        if !status.success() {
            return Err(error::corruption("private fixture ACL setup failed"));
        }
    }
    private_directory(directory.path())?;
    Ok(directory)
}
