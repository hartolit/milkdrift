//! One bounded external CLI envelope for results, failures and stream transitions.

use milkdrift_control_protocol::MAX_DOCUMENT_BYTES;
use serde::Serialize;
use serde_json::{Value, json};

use crate::{Cli, error::CliError};

/// Owns a new output until commit or explicit cleanup; Drop only covers interruption.
///
/// Artifact callers verify size and digest before committing; export callers finish encoding.
/// The destination is visible while being written, so this is cleanup ownership, not an atomic
/// rename or a guarantee of cleanup after forced process termination.
pub(crate) struct PendingFile {
    file: Option<std::fs::File>,
    path: std::path::PathBuf,
    finished: bool,
}

impl PendingFile {
    pub(crate) fn create(path: &std::path::Path) -> Result<Self, CliError> {
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt as _;
            options.mode(0o600);
        }
        let file = options.open(path).map_err(|error| {
            CliError::Invalid(format!(
                "output must be a new writable file: {:?}",
                error.kind()
            ))
        })?;
        Ok(Self {
            file: Some(file),
            path: path.to_owned(),
            finished: false,
        })
    }

    pub(crate) fn commit(&mut self) -> std::io::Result<()> {
        let file = self
            .file
            .as_ref()
            .ok_or_else(|| std::io::Error::other("output already closed"))?;
        file.sync_all()?;
        self.finished = true;
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
        if self.finished {
            return outcome;
        }
        let operation = outcome
            .err()
            .unwrap_or_else(|| CliError::Internal("output was not committed".into()));
        match self.abort() {
            Ok(()) => Err(operation),
            Err(error) => Err(CliError::OutputCleanup {
                operation: Box::new(operation),
                cleanup: error.kind(),
            }),
        }
    }

    fn abort(&mut self) -> std::io::Result<()> {
        drop(self.file.take());
        // An explicit failure must remain visible; Drop must not silently retry it afterward.
        self.finished = true;
        match std::fs::remove_file(&self.path) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            result => result,
        }
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
        drop(self.file.take());
        if !self.finished
            && let Err(error) = self.abort()
        {
            #[expect(
                clippy::print_stderr,
                reason = "Cancellation or unwind cannot return a result; report unconfirmed local-file cleanup without revealing the destination path."
            )]
            {
                eprintln!(
                    "milkdrift: incomplete output cleanup unconfirmed ({:?}); destination may remain",
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
        // Replace the destination with a directory so file removal deterministically fails.
        // Close the descriptor first so this fault is also valid on Windows.
        drop(pending.file.take());
        std::fs::remove_file(&output)?;
        std::fs::create_dir(&output)?;
        let error = pending
            .finish::<()>(Err(CliError::Deadline))
            .err()
            .ok_or("cleanup must fail")?;
        assert_eq!(crate::error::exit_code(&error), 9);
        let CliError::OutputCleanup { operation, .. } = error else {
            return Err("cleanup uncertainty was discarded".into());
        };
        assert!(matches!(*operation, CliError::Deadline));
        assert!(output.is_dir());
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
        assert_eq!(value["schema_version"], 2);
        assert_eq!(value["command_id"], "command-1");
        assert_eq!(value["final"], true);
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
