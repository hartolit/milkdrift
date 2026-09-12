//! One bounded external CLI envelope for results, failures and stream transitions.

use milkdrift_control_protocol::MAX_DOCUMENT_BYTES;
use serde::Serialize;
use serde_json::{Value, json};

use crate::{Cli, error::CliError};

/// Removes a newly created file on drop unless the caller commits a complete result.
///
/// Artifact callers verify size and digest before committing; export callers finish encoding.
/// The destination is visible while being written, so this is cleanup ownership, not an atomic
/// rename or a guarantee of cleanup after forced process termination.
pub(crate) struct PendingFile {
    file: Option<std::fs::File>,
    path: std::path::PathBuf,
    committed: bool,
}

impl PendingFile {
    pub(crate) fn create(path: &std::path::Path) -> Result<Self, CliError> {
        let file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)
            .map_err(|error| {
                CliError::Invalid(format!(
                    "output must be a new writable file: {:?}",
                    error.kind()
                ))
            })?;
        Ok(Self {
            file: Some(file),
            path: path.to_owned(),
            committed: false,
        })
    }

    pub(crate) fn commit(mut self) -> std::io::Result<()> {
        let file = self
            .file
            .take()
            .ok_or_else(|| std::io::Error::other("output already closed"))?;
        file.sync_all()?;
        self.committed = true;
        Ok(())
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
        if !self.committed {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

pub(crate) const JSON_OUTPUT_SCHEMA_VERSION: u32 = 2;

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
        println!(
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
        );
    } else {
        println!("{kind}");
        human_value(&value);
    }
    Ok(())
}

fn human_value(value: &Value) {
    match value {
        Value::Array(items) => {
            for item in items {
                human_value(item);
            }
        }
        Value::Object(fields) => {
            for key in [
                "command_id",
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
            ] {
                if let Some(value) = fields.get(key).filter(|value| !value.is_null()) {
                    println!("  {key}: {value}");
                }
            }
            for key in ["controller_accounting", "accounting"] {
                if let Some(accounting) = fields.get(key) {
                    println!("  cumulative controller accounting: {accounting}");
                }
            }
            for key in ["summary", "value", "items"] {
                if let Some(value) = fields.get(key) {
                    human_value(value);
                }
            }
        }
        _ => println!("  {value}"),
    }
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
