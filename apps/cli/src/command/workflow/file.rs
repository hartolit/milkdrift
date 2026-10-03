//! A cooperating editor holds an OS lock from read through reply and atomic replacement.
//! Temporary files are never promoted until complete; changed bytes and symlinks refuse.

use crate::error::CliError;
use milkdrift_control_protocol::{BlueprintDraft, MAX_DOCUMENT_BYTES, decode_json, encode_json};
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read as _, Write as _},
    path::{Path, PathBuf},
};

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Document {
    schema_version: u32,
    draft: BlueprintDraft,
}

pub(super) struct DraftFile {
    path: PathBuf,
    original: Option<Vec<u8>>,
    _lock: File,
}
fn invalid(message: impl ToString) -> CliError {
    CliError::Invalid(message.to_string())
}
fn read(path: &Path) -> Result<Vec<u8>, CliError> {
    let metadata = fs::symlink_metadata(path).map_err(invalid)?;
    if !metadata.is_file() || metadata.len() > MAX_DOCUMENT_BYTES as u64 {
        return Err(invalid(
            "draft must be a bounded regular file, not a symlink",
        ));
    }
    let mut bytes = Vec::new();
    File::open(path)
        .map_err(invalid)?
        .take(MAX_DOCUMENT_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(invalid)?;
    if bytes.len() > MAX_DOCUMENT_BYTES {
        return Err(invalid("draft exceeds document bound"));
    }
    Ok(bytes)
}
impl DraftFile {
    pub(super) fn open(
        path: &Path,
        creating: bool,
        expected: Option<&str>,
    ) -> Result<Self, CliError> {
        let parent = path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."))
            .canonicalize()
            .map_err(invalid)?;
        let name = path
            .file_name()
            .ok_or_else(|| invalid("draft requires a file name"))?;
        let path = parent.join(name);
        let mut lock_name = name.to_os_string();
        lock_name.push(".lock");
        let lock_path = parent.join(lock_name);
        match fs::symlink_metadata(&lock_path) {
            Ok(metadata) if !metadata.is_file() => {
                return Err(invalid("draft lock must be a regular file"));
            }
            Err(error) if error.kind() != std::io::ErrorKind::NotFound => {
                return Err(invalid(error));
            }
            _ => {}
        }
        let lock = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(lock_path)
            .map_err(invalid)?;
        lock.try_lock().map_err(|_| {
            invalid("draft is being edited by another session; retry after it finishes")
        })?;
        let original = if creating {
            if fs::symlink_metadata(&path).is_ok() {
                return Err(invalid("draft destination already exists"));
            }
            None
        } else {
            Some(read(&path)?)
        };
        let file = Self {
            path,
            original,
            _lock: lock,
        };
        if expected.is_some_and(|expected| expected != file.token()) {
            return Err(invalid(
                "stale draft edit token; inspect the current file before editing",
            ));
        }
        Ok(file)
    }
    pub(super) fn token(&self) -> String {
        self.original.as_ref().map_or_else(String::new, |bytes| {
            blake3::hash(bytes).to_hex().to_string()
        })
    }
    pub(super) fn draft(&self) -> Result<BlueprintDraft, CliError> {
        let document: Document = decode_json(
            self.original
                .as_deref()
                .ok_or_else(|| invalid("draft does not exist"))?,
        )
        .map_err(invalid)?;
        if document.schema_version != 1 {
            return Err(invalid("unsupported draft schema"));
        }
        Ok(document.draft)
    }
    pub(super) fn replace(&self, draft: &BlueprintDraft) -> Result<String, CliError> {
        let bytes = encode_json(&Document {
            schema_version: 1,
            draft: draft.clone(),
        })
        .map_err(invalid)?;
        let mut temporary = tempfile::NamedTempFile::new_in(
            self.path
                .parent()
                .ok_or_else(|| invalid("draft parent missing"))?,
        )
        .map_err(invalid)?;
        temporary.write_all(&bytes).map_err(invalid)?;
        temporary.as_file().sync_all().map_err(invalid)?;
        if let Some(original) = &self.original {
            if &read(&self.path)? != original {
                return Err(invalid(
                    "draft changed since this edit began; no file was replaced",
                ));
            }
            temporary.persist(&self.path).map_err(invalid)?;
        } else {
            temporary.persist_noclobber(&self.path).map_err(invalid)?;
        }
        #[cfg(unix)]
        File::open(
            self.path
                .parent()
                .ok_or_else(|| invalid("draft parent missing"))?,
        )
        .and_then(|parent| parent.sync_all())
        .map_err(invalid)?;
        Ok(blake3::hash(&bytes).to_hex().to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn atomic_drafts_preserve_old_bytes_refuse_stale_and_interrupted_edits()
    -> Result<(), Box<dyn std::error::Error>> {
        let root = tempfile::tempdir()?;
        let path = root.path().join("notes.json");
        let draft = BlueprintDraft {
            workflow_id: "notes".into(),
            base_revision: None,
            mutations: vec![],
        };
        let file = DraftFile::open(&path, true, None)?;
        let token = file.replace(&draft)?;
        assert!(DraftFile::open(&path, false, None).is_err());
        drop(file);
        let original = fs::read(&path)?;
        assert!(DraftFile::open(&path, true, None).is_err());
        assert!(DraftFile::open(&path, false, Some("old-token")).is_err());
        {
            let file = DraftFile::open(&path, false, Some(&token))?;
            assert_eq!(file.draft()?, draft);
            // Simulate interruption after writing an unpromoted temporary file.
            let mut temporary = tempfile::NamedTempFile::new_in(root.path())?;
            temporary.write_all(b"partial")?;
            assert_eq!(fs::read(&path)?, original);
            assert_eq!(file.replace(&draft)?, token);
        }
        let file = DraftFile::open(&path, false, None)?;
        fs::write(&path, b"external edit")?;
        assert!(file.replace(&draft).is_err());
        assert_eq!(fs::read(&path)?, b"external edit");
        Ok(())
    }
    #[cfg(unix)]
    #[test]
    fn draft_symlinks_are_never_followed() -> Result<(), Box<dyn std::error::Error>> {
        let root = tempfile::tempdir()?;
        let target = root.path().join("target");
        fs::write(&target, b"keep")?;
        let link = root.path().join("link");
        std::os::unix::fs::symlink(&target, &link)?;
        assert!(DraftFile::open(&link, false, None).is_err());
        assert!(DraftFile::open(&link, true, None).is_err());
        assert_eq!(fs::read(target)?, b"keep");
        Ok(())
    }
}
