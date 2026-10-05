//! One bounded external CLI envelope for results, failures and stream transitions.

use milkdrift_control_protocol::MAX_DOCUMENT_BYTES;
use serde::Serialize;
use serde_json::{Value, json};

use crate::{Cli, error::CliError};

#[cfg(test)]
#[path = "output/files_tests.rs"]
mod files_tests;

pub(crate) fn staging_directory(parent: &std::path::Path) -> std::io::Result<tempfile::TempDir> {
    let mut builder = tempfile::Builder::new();
    builder.prefix(".milkdrift-output-");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        builder.permissions(std::fs::Permissions::from_mode(0o700));
    }
    builder.tempdir_in(parent)
}

pub(crate) fn finish_staging<T>(
    staging: tempfile::TempDir,
    outcome: Result<T, CliError>,
) -> Result<T, CliError> {
    match (outcome, staging.close()) {
        (outcome, Ok(())) => outcome,
        (outcome, Err(error)) => Err(CliError::OutputCleanup {
            operation: Box::new(outcome.err().unwrap_or_else(|| {
                CliError::Internal("output published; staging cleanup failed".into())
            })),
            cleanup: error.kind(),
        }),
    }
}

/// Verifies in private staging, then publishes a complete create-only output.
///
/// Artifact callers verify size and digest before committing; export callers finish encoding.
/// The parent and private staging directory must remain under the operator's control. Other
/// writers may compete for the final name: publication never clobbers it and cleanup never
/// unlinks it. Success describes publication, not what another writer does afterward.
pub(crate) struct PendingFile {
    file: Option<tempfile::NamedTempFile>,
    staging: Option<tempfile::TempDir>,
    path: std::path::PathBuf,
    published: bool,
    committed: bool,
    finished: bool,
}

impl PendingFile {
    pub(crate) fn create(path: &std::path::Path) -> Result<Self, CliError> {
        if path == std::path::Path::new("-") || path.file_name().is_none() {
            return Err(CliError::Invalid(
                "output requires an explicit file name".into(),
            ));
        }
        match std::fs::symlink_metadata(path) {
            Ok(_) => {
                return Err(CliError::Invalid(
                    "output destination already exists".into(),
                ));
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
        let parent = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or(std::path::Path::new("."))
            .canonicalize()?;
        let path = parent.join(
            path.file_name()
                .ok_or_else(|| CliError::Invalid("output requires a file name".into()))?,
        );
        let staging = staging_directory(&parent)?;
        let file = tempfile::NamedTempFile::new_in(staging.path()).map_err(|error| {
            CliError::Invalid(format!(
                "output must be a new writable file: {:?}",
                error.kind()
            ))
        })?;
        Ok(Self {
            file: Some(file),
            staging: Some(staging),
            path,
            published: false,
            committed: false,
            finished: false,
        })
    }

    pub(crate) fn commit(&mut self) -> std::io::Result<()> {
        self.commit_with(std::fs::File::sync_all, |parent| {
            #[cfg(unix)]
            std::fs::File::open(parent)?.sync_all()?;
            #[cfg(not(unix))]
            let _ = parent;
            Ok(())
        })
    }

    // Keep the two durability boundaries injectable for deterministic failure tests.
    fn commit_with(
        &mut self,
        sync_file: impl FnOnce(&std::fs::File) -> std::io::Result<()>,
        sync_parent: impl FnOnce(&std::path::Path) -> std::io::Result<()>,
    ) -> std::io::Result<()> {
        let file = self
            .file
            .as_ref()
            .ok_or_else(|| std::io::Error::other("output already closed"))?;
        sync_file(file.as_file())?;
        let file = self
            .file
            .take()
            .ok_or_else(|| std::io::Error::other("output already closed"))?;
        match file.persist_noclobber(&self.path) {
            Ok(file) => drop(file),
            Err(error) => {
                self.file = Some(error.file);
                return Err(error.error);
            }
        }
        self.published = true;
        sync_parent(
            self.path
                .parent()
                .ok_or_else(|| std::io::Error::other("output parent absent"))?,
        )?;
        self.committed = true;
        Ok(())
    }

    pub(crate) fn write_complete(mut self, bytes: &[u8]) -> Result<(), CliError> {
        use std::io::Write as _;
        let outcome = self
            .write_all(bytes)
            .and_then(|()| self.commit())
            .map_err(|error| {
                CliError::Internal(format!("output write failed: {:?}", error.kind()))
            });
        self.finish(outcome)
    }

    pub(crate) fn finish<T>(mut self, outcome: Result<T, CliError>) -> Result<T, CliError> {
        let outcome = match outcome {
            Ok(value) if self.committed => Ok(value),
            Ok(_) => Err(CliError::Internal("output was not committed".into())),
            Err(error) if self.published && !self.committed => Err(CliError::OutputPublished {
                operation: Box::new(error),
            }),
            Err(error) => Err(error),
        };
        match (outcome, self.abort()) {
            (outcome, Ok(())) => outcome,
            (outcome, Err(error)) => Err(CliError::OutputCleanup {
                operation: Box::new(outcome.err().unwrap_or_else(|| {
                    CliError::Internal("output published; staging cleanup failed".into())
                })),
                cleanup: error.kind(),
            }),
        }
    }

    fn abort(&mut self) -> std::io::Result<()> {
        // An explicit failure must remain visible; Drop must not silently retry it afterward.
        self.finished = true;
        let file = self
            .file
            .take()
            .map(tempfile::NamedTempFile::close)
            .transpose();
        let directory = self
            .staging
            .take()
            .map(tempfile::TempDir::close)
            .transpose();
        file.and(directory).map(|_| ())
    }
}

impl std::io::Write for PendingFile {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.file
            .as_mut()
            .ok_or_else(|| std::io::Error::other("output closed"))?
            .write(bytes)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.file
            .as_mut()
            .ok_or_else(|| std::io::Error::other("output closed"))?
            .flush()
    }
}

impl Drop for PendingFile {
    fn drop(&mut self) {
        if !self.finished
            && let Err(error) = self.abort()
        {
            #[expect(
                clippy::print_stderr,
                reason = "Cancellation or unwind cannot return a result; report unconfirmed local-file cleanup without revealing the destination path."
            )]
            {
                eprintln!(
                    "milkdrift: private output staging cleanup unconfirmed ({:?}); staging may remain",
                    error.kind()
                );
            }
        }
    }
}

pub(crate) const JSON_OUTPUT_SCHEMA_VERSION: u32 = 2;

pub(crate) fn line(arguments: std::fmt::Arguments<'_>) -> Result<(), CliError> {
    use std::io::Write as _;
    let mut stdout = std::io::stdout().lock();
    writeln!(stdout, "{arguments}")?;
    stdout.flush()?;
    Ok(())
}

pub(crate) fn encode(
    operation: &str,
    command_id: Option<&str>,
    status: &str,
    value: Value,
    error: Value,
    finished: bool,
) -> Result<String, CliError> {
    let document = json!({
        "schema_version": JSON_OUTPUT_SCHEMA_VERSION,
        "type": operation,
        "status": status,
        "command_id": command_id.filter(|id| id.len() <= 192 && id.is_ascii() && !id.bytes().any(|b| b.is_ascii_control())),
        "value": value,
        "error": error,
        "final": finished,
    });
    let encoded =
        serde_json::to_string(&document).map_err(|error| CliError::Internal(error.to_string()))?;
    if encoded.len() > MAX_DOCUMENT_BYTES + 4096 {
        return Err(CliError::Internal(
            "CLI output exceeds document bound".to_owned(),
        ));
    }
    Ok(encoded)
}

pub(crate) fn success<T: Serialize>(cli: &Cli, kind: &str, value: &T) -> Result<(), CliError> {
    let value =
        serde_json::to_value(value).map_err(|error| CliError::Internal(error.to_string()))?;
    if cli.json {
        crate::output::line(format_args!(
            "{}",
            encode(
                if cli.is_follow() {
                    kind
                } else {
                    cli.operation()
                },
                cli.command_id.as_deref(),
                "success",
                value,
                Value::Null,
                !cli.is_follow()
            )?
        ))?;
    } else {
        crate::output::line(format_args!("{kind}"))?;
        if kind.starts_with("learning.") {
            // Fixed criteria, missing measurements and reasons must survive human presentation.
            crate::output::line(format_args!(
                "{}",
                serde_json::to_string_pretty(&value)
                    .map_err(|error| CliError::Internal(error.to_string()))?
            ))?;
        } else {
            human_value(&value)?;
        }
    }
    Ok(())
}

fn human_value(value: &Value) -> Result<(), CliError> {
    match value {
        Value::Array(items) => {
            for item in items {
                human_value(item)?;
            }
        }
        Value::Object(fields) => {
            if let Some(workflow) = fields.get("workflow") {
                crate::output::line(format_args!(
                    "  {}",
                    serde_json::to_string_pretty(workflow)
                        .map_err(|error| CliError::Internal(error.to_string()))?
                ))?;
            }
            for key in [
                "file",
                "edit_token",
                "saved",
                "base_revision",
                "lineage_sequence",
                "name",
                "inputs",
                "outputs",
                "copied_from",
                "pending_mutations",
                "generation",
                "available",
                "health",
                "command_id",
                "host",
                "actor",
                "workflow_id",
                "request_id",
                "execution",
                "type",
                "status",
                "origin",
                "replayed",
                "code",
                "detail",
                "closed",
                "next_sequence",
                "disposition",
                "terminal_boundary",
                "result_type",
                "run_id",
                "revision_id",
                "capability_id",
                "provider_profile",
                "artifact_id",
                "proposal_id",
                "execution_id",
                "attempt_id",
                "lifecycle",
                "terminal",
                "terminal_detail",
                "state",
                "sequence",
                "uncertainty_count",
                "next_cursor",
                "output",
                "destination",
                "reached_bound",
                "cycle_eligible",
                "last_assessment_sequence",
                "checkpoint_id",
                "node",
                "execution",
                "classification",
                "action",
                "reason",
                "approved",
                "proposed_revision",
                "proposal_digest",
            ] {
                if let Some(value) = fields.get(key).filter(|value| !value.is_null()) {
                    crate::output::line(format_args!("  {key}: {value}"))?;
                }
            }
            for key in ["controller_accounting", "accounting"] {
                if let Some(accounting) = fields.get(key) {
                    crate::output::line(format_args!(
                        "  cumulative controller accounting: {accounting}"
                    ))?;
                }
            }
            for key in [
                "summary",
                "recovery",
                "value",
                "items",
                "catalog",
                "entries",
                "descriptor",
                "acceptance",
                "observations",
                "event",
                "kind",
                "final_observation",
                "history",
                "impact",
                "classification",
                "reconciliation",
            ] {
                if let Some(value) = fields.get(key) {
                    human_value(value)?;
                }
            }
        }
        _ => crate::output::line(format_args!("  {value}"))?,
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unfinished_output_is_removed_and_committed_output_survives()
    -> Result<(), Box<dyn std::error::Error>> {
        use std::io::Write as _;
        let directory = tempfile::tempdir()?;
        let output = directory.path().join("artifact");
        {
            let mut pending = PendingFile::create(&output)?;
            pending.write_all(b"partial")?;
        }
        assert!(!output.exists());
        let mut pending = PendingFile::create(&output)?;
        pending.write_all(b"complete")?;
        pending.commit()?;
        assert_eq!(std::fs::read(&output)?, b"complete");
        assert!(PendingFile::create(&output).is_err());
        assert_eq!(std::fs::read(&output)?, b"complete");
        Ok(())
    }

    #[test]
    fn failed_output_reports_cleanup_uncertainty_without_losing_the_operation()
    -> Result<(), Box<dyn std::error::Error>> {
        use std::io::Write as _;
        let directory = tempfile::tempdir()?;
        let output = directory.path().join("artifact");
        let mut pending = PendingFile::create(&output)?;
        pending.write_all(b"partial")?;
        // Inject a failure only in owned staging, after closing the descriptor so
        // this is also valid on Windows. Never manipulate the final destination.
        drop(pending.file.take());
        let staging = pending
            .staging
            .as_ref()
            .ok_or("staging absent")?
            .path()
            .to_owned();
        std::fs::remove_dir(&staging)?;
        std::fs::write(&staging, b"owned cleanup fault")?;
        let error = pending
            .finish::<()>(Err(CliError::Deadline))
            .err()
            .ok_or("cleanup must fail")?;
        assert_eq!(crate::error::exit_code(&error), 9);
        let CliError::OutputCleanup { operation, .. } = error else {
            return Err("cleanup uncertainty was discarded".into());
        };
        assert!(matches!(*operation, CliError::Deadline));
        assert!(!output.exists());
        assert_eq!(std::fs::read(&staging)?, b"owned cleanup fault");
        Ok(())
    }

    #[test]
    fn ordinary_failure_explicitly_removes_uncommitted_output()
    -> Result<(), Box<dyn std::error::Error>> {
        use std::io::Write as _;
        let directory = tempfile::tempdir()?;
        let output = directory.path().join("artifact");
        let mut pending = PendingFile::create(&output)?;
        pending.write_all(b"partial")?;
        assert!(matches!(
            pending.finish::<()>(Err(CliError::Deadline)),
            Err(CliError::Deadline)
        ));
        assert!(!output.exists());
        PendingFile::create(&output)?.write_complete(b"complete")?;
        assert_eq!(std::fs::read(&output)?, b"complete");
        Ok(())
    }

    #[test]
    fn envelopes_are_single_bounded_control_free_documents()
    -> Result<(), Box<dyn std::error::Error>> {
        let encoded = encode(
            "fixture",
            Some("command-1"),
            "success",
            json!({"detail":"a\n\u{1b}[31m"}),
            Value::Null,
            true,
        )?;
        assert!(!encoded.chars().any(char::is_control));
        let value: Value = serde_json::from_str(&encoded)?;
        assert_eq!(
            value
                .get("schema_version")
                .ok_or("missing schema_version")?,
            2
        );
        assert_eq!(
            value.get("command_id").ok_or("missing command_id")?,
            "command-1"
        );
        assert_eq!(value.get("final").ok_or("missing final")?, true);
        assert!(
            encode(
                "fixture",
                None,
                "success",
                json!("x".repeat(MAX_DOCUMENT_BYTES + 4096)),
                Value::Null,
                true
            )
            .is_err()
        );
        Ok(())
    }
}
