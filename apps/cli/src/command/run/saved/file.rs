//! Atomic promotion and bounded private reads of a single operator-owned recovery record.
use crate::error::CliError;
use milkdrift_control_client::SavedRunRequest;
use milkdrift_control_protocol::{MAX_DOCUMENT_BYTES, decode_json, encode_json};
use std::{
    fs,
    io::{Read as _, Write as _},
    path::{Path, PathBuf},
};

pub(super) struct PendingRequest {
    temp: tempfile::NamedTempFile,
    path: PathBuf,
}

impl PendingRequest {
    pub(super) fn new(path: &Path) -> Result<Self, CliError> {
        if path == Path::new("-")
            || path.file_name().is_none()
            || fs::symlink_metadata(path).is_ok()
        {
            return Err(CliError::Invalid(
                "request file must be a new explicit destination".into(),
            ));
        }
        let parent = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        let temp = tempfile::NamedTempFile::new_in(parent).map_err(io_error)?;
        Ok(Self {
            temp,
            path: path.to_owned(),
        })
    }

    pub(super) fn commit(mut self, saved: &SavedRunRequest) -> Result<(), CliError> {
        let bytes = encode_json(saved).map_err(|error| CliError::Invalid(error.to_string()))?;
        self.temp.write_all(&bytes).map_err(io_error)?;
        self.temp.as_file().sync_all().map_err(io_error)?;
        self.temp
            .persist_noclobber(&self.path)
            .map_err(|error| io_error(error.error))?;
        #[cfg(unix)]
        {
            let parent = self
                .path
                .parent()
                .filter(|parent| !parent.as_os_str().is_empty())
                .unwrap_or(Path::new("."));
            fs::File::open(parent)
                .and_then(|file| file.sync_all())
                .map_err(io_error)?;
        }
        Ok(())
    }
}

pub(super) fn read(path: &Path) -> Result<SavedRunRequest, CliError> {
    let before = fs::symlink_metadata(path).map_err(io_error)?;
    if !before.is_file() {
        return Err(CliError::Invalid(
            "request file must be a regular file, not a link".into(),
        ));
    }
    let file = fs::File::open(path).map_err(io_error)?;
    let metadata = file.metadata().map_err(io_error)?;
    if !metadata.is_file() || metadata.len() > MAX_DOCUMENT_BYTES as u64 {
        return Err(CliError::Invalid(
            "request file exceeds the document bound".into(),
        ));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt as _;
        if metadata.mode() & 0o077 != 0
            || before.dev() != metadata.dev()
            || before.ino() != metadata.ino()
        {
            return Err(CliError::Invalid(
                "request file must be private and unchanged while opening".into(),
            ));
        }
    }
    let mut bytes = Vec::new();
    file.take(MAX_DOCUMENT_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(io_error)?;
    let saved: SavedRunRequest =
        decode_json(&bytes).map_err(|error| CliError::Invalid(error.to_string()))?;
    saved.validate()?;
    Ok(saved)
}

fn io_error(error: std::io::Error) -> CliError {
    CliError::Invalid(format!(
        "private request file unavailable: {:?}",
        error.kind()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn saved() -> Result<SavedRunRequest, Box<dyn std::error::Error>> {
        Ok(serde_json::from_value(serde_json::json!({
            "schema_version":1,
            "authority":{"host":"host:one","actor":"human:one","grant_id":"grant:one","grant_revision":1,"grant_digest":"digest","revocation_generation":0,"operations":[]},
            "request":{"protocol":milkdrift_control_protocol::ProtocolVersion::CURRENT,"command_id":"start-one","expected_sequence":0,"expected_revision":"revision","reason":"start once","evidence":[],"command":{"type":"start_run","run_id":"one","workflow_id":"workflow","revision_id":"revision","inputs":[{"name":"brief","artifact_id":"input:one"}]}}
        }))?)
    }

    #[test]
    fn request_is_private_atomic_bounded_and_never_overwrites()
    -> Result<(), Box<dyn std::error::Error>> {
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("request.json");
        drop(PendingRequest::new(&path)?);
        assert!(
            !path.exists(),
            "interrupted preparation exposed a partial request"
        );
        let record = saved()?;
        PendingRequest::new(&path)?.commit(&record)?;
        assert_eq!(read(&path)?, record);
        assert!(PendingRequest::new(&path).is_err());
        let mut unsupported = record.clone();
        unsupported.schema_version = 2;
        fs::write(&path, serde_json::to_vec(&unsupported)?)?;
        assert!(read(&path).is_err());
        let mut wrong_operation = record.clone();
        wrong_operation.request.command = milkdrift_control_protocol::Command::CancelRun {
            run_id: "one".into(),
        };
        fs::write(&path, serde_json::to_vec(&wrong_operation)?)?;
        assert!(read(&path).is_err());
        let raced = directory.path().join("raced.json");
        let pending = PendingRequest::new(&raced)?;
        fs::write(&raced, b"another writer")?;
        assert!(pending.commit(&record).is_err());
        assert_eq!(fs::read(&raced)?, b"another writer");
        fs::write(&path, b"{broken")?;
        assert!(read(&path).is_err());
        fs::write(&path, vec![b' '; MAX_DOCUMENT_BYTES + 1])?;
        assert!(read(&path).is_err());
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn exposed_and_linked_request_records_are_refused() -> Result<(), Box<dyn std::error::Error>> {
        use std::os::unix::fs::{PermissionsExt as _, symlink};
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("request.json");
        PendingRequest::new(&path)?.commit(&saved()?)?;
        assert_eq!(fs::metadata(&path)?.permissions().mode() & 0o777, 0o600);
        let link = directory.path().join("link.json");
        symlink(&path, &link)?;
        assert!(read(&link).is_err());
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644))?;
        assert!(read(&path).is_err());
        Ok(())
    }
}
