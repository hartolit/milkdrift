use super::{FEEDBACK_BYTES, adapter_error, rejected};
use crate::AdapterError;
use milkdrift_blueprint::TaskTool;
use milkdrift_capability::BoundedJson;
use milkdrift_model::ToolDefinition;
use milkdrift_persistence::NativePathRoot;
use std::{
    fs::File,
    io::{Read, Write},
    path::{Component, Path, PathBuf},
};

pub(super) fn definition(tool: &TaskTool) -> Result<ToolDefinition, AdapterError> {
    let write = matches!(tool, TaskTool::WriteFile);
    let properties = if write {
        serde_json::json!({"path":{"type":"string"},"text":{"type":"string"}})
    } else {
        serde_json::json!({"path":{"type":"string"}})
    };
    ToolDefinition::new(if write { "write_file" } else { "read_file" }, if write { "Atomically write bounded UTF-8 text to an existing declared writable project folder. Parent directories must exist." } else { "Read a bounded regular UTF-8 file under a declared project folder. Oversized files are refused without truncation." }, BoundedJson::new(serde_json::json!({"type":"object","properties":properties,"required":if write {vec!["path","text"]} else {vec!["path"]},"additionalProperties":false})).map_err(adapter_error)?).map_err(adapter_error)
}

fn arguments(
    value: &serde_json::Value,
    write: bool,
) -> Result<(&Path, Option<&str>), AdapterError> {
    let object = value
        .as_object()
        .filter(|object| object.len() == if write { 2 } else { 1 })
        .ok_or_else(|| rejected("file tool has unexpected arguments"))?;
    let path = object
        .get("path")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| rejected("file tool requires an absolute path"))?;
    let path = Path::new(path);
    if !path.is_absolute()
        || path.as_os_str().len() > 4096
        || path
            .components()
            .any(|part| matches!(part, Component::ParentDir | Component::CurDir))
    {
        return Err(rejected(
            "file tool path is not a bounded absolute normal path",
        ));
    }
    let text = if write {
        Some(
            object
                .get("text")
                .and_then(serde_json::Value::as_str)
                .filter(|text| text.len() as u64 <= FEEDBACK_BYTES)
                .ok_or_else(|| rejected("file tool text exceeds its exact UTF-8 byte allowance"))?,
        )
    } else {
        None
    };
    Ok((path, text))
}

#[cfg(target_os = "linux")]
fn open_claimed(
    roots: &[NativePathRoot],
    requested: &Path,
    write: bool,
    directory: bool,
) -> Result<File, AdapterError> {
    use rustix::fs::{Mode, OFlags, ResolveFlags};
    use std::os::unix::fs::MetadataExt;
    for root in roots {
        if write && !root.write {
            continue;
        }
        let Ok(relative) = requested
            .strip_prefix(&root.requested)
            .or_else(|_| requested.strip_prefix(&root.physical))
        else {
            continue;
        };
        // Pin the directory whose identity was durably claimed. Replacing its path or retargeting
        // an alias cannot change this descriptor or the scope of the following relative open.
        let pinned = File::from(
            rustix::fs::open(
                &root.physical,
                OFlags::RDONLY | OFlags::CLOEXEC | OFlags::DIRECTORY | OFlags::NONBLOCK,
                Mode::empty(),
            )
            .map_err(adapter_error)?,
        );
        let metadata = pinned.metadata().map_err(adapter_error)?;
        if format!("unix:{}:{}", metadata.dev(), metadata.ino()) != root.identity {
            return Err(rejected(
                "native file tool root differs from its durable claim",
            ));
        }
        let relative = if relative.as_os_str().is_empty() {
            Path::new(".")
        } else {
            relative
        };
        let mut flags = OFlags::RDONLY | OFlags::CLOEXEC | OFlags::NONBLOCK;
        if directory {
            flags |= OFlags::DIRECTORY;
        }
        return rustix::fs::openat2(
            &pinned,
            relative,
            flags,
            Mode::empty(),
            ResolveFlags::BENEATH | ResolveFlags::NO_SYMLINKS,
        )
        .map(File::from)
        .map_err(adapter_error);
    }
    Err(rejected(
        "file tool path is outside its declared task claims",
    ))
}

// Linux descriptor paths keep the already opened object pinned while another process renames
// parent directories. No content is read before checking the opened file's physical location.
#[cfg(target_os = "linux")]
fn descriptor_path(file: &File) -> PathBuf {
    use std::os::fd::AsRawFd;
    PathBuf::from(format!("/proc/self/fd/{}", file.as_raw_fd()))
}

#[cfg(target_os = "linux")]
pub(super) fn read(
    roots: &[NativePathRoot],
    value: &serde_json::Value,
) -> Result<String, AdapterError> {
    use std::os::unix::fs::MetadataExt;
    let (path, _) = arguments(value, false)?;
    let file = open_claimed(roots, path, false, false)?;
    let metadata = file.metadata().map_err(adapter_error)?;
    if !metadata.is_file() || metadata.nlink() != 1 || metadata.len() > FEEDBACK_BYTES {
        return Err(rejected(
            "file tool requires a bounded regular file with one link",
        ));
    }
    let mut bytes = Vec::new();
    file.take(
        FEEDBACK_BYTES
            .checked_add(1)
            .ok_or_else(|| rejected("file bound overflow"))?,
    )
    .read_to_end(&mut bytes)
    .map_err(adapter_error)?;
    if bytes.len() as u64 > FEEDBACK_BYTES {
        return Err(rejected("file grew beyond its exact read allowance"));
    }
    String::from_utf8(bytes).map_err(adapter_error)
}

#[cfg(target_os = "linux")]
pub(super) fn write(
    roots: &[NativePathRoot],
    value: &serde_json::Value,
) -> Result<String, AdapterError> {
    let (path, text) = arguments(value, true)?;
    let parent = path
        .parent()
        .ok_or_else(|| rejected("file tool target requires a parent"))?;
    let directory = open_claimed(roots, parent, true, true)?;
    let pinned = descriptor_path(&directory);
    let target = pinned.join(
        path.file_name()
            .ok_or_else(|| rejected("file tool target name is absent"))?,
    );
    let text = text.ok_or_else(|| rejected("file write content is absent"))?;
    let mut temporary = tempfile::NamedTempFile::new_in(&pinned).map_err(adapter_error)?;
    temporary
        .write_all(text.as_bytes())
        .and_then(|()| temporary.as_file().sync_all())
        .map_err(adapter_error)?;
    temporary.persist(&target).map_err(|error| {
        AdapterError::external_failure(format!(
            "file replacement outcome failed: {}",
            error.error.kind()
        ))
    })?;
    directory.sync_all().map_err(|error| {
        AdapterError::external_failure(format!(
            "file replacement durability uncertain: {}",
            error.kind()
        ))
    })?;
    serde_json::to_string(&serde_json::json!({"path":path,"bytes":text.len(),"digest":format!("b3_{}",blake3::hash(text.as_bytes()))})).map_err(adapter_error)
}

#[cfg(not(target_os = "linux"))]
pub(super) fn read(
    _roots: &[NativePathRoot],
    _value: &serde_json::Value,
) -> Result<String, AdapterError> {
    Err(rejected(
        "native model file tools require the qualified Linux descriptor path implementation",
    ))
}
#[cfg(not(target_os = "linux"))]
pub(super) fn write(
    _roots: &[NativePathRoot],
    _value: &serde_json::Value,
) -> Result<String, AdapterError> {
    Err(rejected(
        "native model file tools require the qualified Linux descriptor path implementation",
    ))
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;
    use std::os::unix::fs::MetadataExt;
    fn context(
        root: &Path,
        write: bool,
    ) -> Result<Vec<NativePathRoot>, Box<dyn std::error::Error>> {
        let metadata = root.metadata()?;
        Ok(vec![NativePathRoot {
            requested: root.to_path_buf(),
            physical: root.canonicalize()?,
            identity: format!("unix:{}:{}", metadata.dev(), metadata.ino()),
            write,
        }])
    }
    #[test]
    fn native_tools_preserve_exact_bytes_and_refuse_escape_and_oversize()
    -> Result<(), Box<dyn std::error::Error>> {
        let root = tempfile::tempdir()?;
        let outside = tempfile::tempdir()?;
        let context = context(root.path(), true)?;
        let target = root.path().join("result.txt");
        let text = "æ\nexact bytes\n";
        write(&context, &serde_json::json!({"path":target,"text":text}))?;
        assert_eq!(read(&context, &serde_json::json!({"path":target}))?, text);
        std::fs::write(outside.path().join("secret"), "hidden")?;
        std::os::unix::fs::symlink(outside.path(), root.path().join("escape"))?;
        assert!(
            read(
                &context,
                &serde_json::json!({"path":root.path().join("escape/secret")})
            )
            .is_err()
        );
        assert!(
            write(
                &context,
                &serde_json::json!({"path":root.path().join("escape/new"),"text":"bad"})
            )
            .is_err()
        );
        assert!(!outside.path().join("new").exists());
        assert!(
            write(
                &context,
                &serde_json::json!({"path":target,"text":"x".repeat(usize::try_from(FEEDBACK_BYTES)?.checked_add(1).ok_or("test byte bound overflow")?)})
            )
            .is_err()
        );
        Ok(())
    }

    #[test]
    fn retargeted_alias_cannot_expand_a_durable_claim_between_authorization_and_file_open()
    -> Result<(), Box<dyn std::error::Error>> {
        let root = tempfile::tempdir()?;
        let original = root.path().join("original");
        let outside = root.path().join("outside");
        std::fs::create_dir(&original)?;
        std::fs::create_dir(&outside)?;
        std::fs::write(original.join("report.txt"), "accepted bytes")?;
        std::fs::write(outside.join("report.txt"), "private bytes")?;
        let alias = root.path().join("alias");
        std::os::unix::fs::symlink(&original, &alias)?;
        let claims = context(&alias, true)?;
        // Simulate the interval after the host checked the claim and before a file opens.
        std::fs::remove_file(&alias)?;
        std::os::unix::fs::symlink(&outside, &alias)?;
        assert_eq!(
            read(
                &claims,
                &serde_json::json!({"path":alias.join("report.txt")})
            )?,
            "accepted bytes"
        );
        write(
            &claims,
            &serde_json::json!({"path":alias.join("new.txt"),"text":"accepted write"}),
        )?;
        assert_eq!(
            std::fs::read_to_string(original.join("new.txt"))?,
            "accepted write"
        );
        assert!(!outside.join("new.txt").exists());
        std::fs::rename(&original, root.path().join("retained"))?;
        std::fs::create_dir(&original)?;
        std::fs::write(original.join("report.txt"), "replacement bytes")?;
        assert!(
            read(
                &claims,
                &serde_json::json!({"path":alias.join("report.txt")})
            )
            .is_err()
        );
        assert!(
            write(
                &claims,
                &serde_json::json!({"path":alias.join("replacement.txt"),"text":"refused"})
            )
            .is_err()
        );
        assert!(!original.join("replacement.txt").exists());
        Ok(())
    }
}
