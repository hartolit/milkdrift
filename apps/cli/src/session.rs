//! One connected CLI session and its shared input, envelope, confirmation, and output policy.

use std::{
    cell::Cell,
    env, fs,
    io::{self, Read as _, Write as _},
    path::Path,
    time::{SystemTime, UNIX_EPOCH},
};

use milkdrift_control_client::{BearerCredential, ClientConfig, ControlClient};
use milkdrift_control_protocol::{
    Command, CommandRequest, Cursor, LayoutDocument, MAX_DOCUMENT_BYTES, MAX_LAYOUT_BYTES,
    PageRequest, ProtocolVersion,
};
use milkdrift_prompt_sequence::{
    MAX_INLINE_PROMPT_BYTES, MAX_PROMPT_SEQUENCE_DOCUMENT_BYTES, PromptSequenceDocument,
};
use serde::Serialize;
use serde_json::{Value, json};

use crate::{Cli, error::CliError};

const JSON_OUTPUT_SCHEMA_VERSION: u32 = 1;

pub(crate) struct CliSession {
    cli: Cli,
    client: ControlClient,
    stdin_consumed: Cell<bool>,
}

impl CliSession {
    pub(crate) async fn connect(cli: Cli) -> Result<Self, CliError> {
        let credential = load_credential(&cli)?;
        let client = ControlClient::new(ClientConfig::new(cli.endpoint.clone()), credential)?;
        let _ = client.negotiate().await?;
        Ok(Self {
            cli,
            client,
            stdin_consumed: Cell::new(false),
        })
    }

    pub(crate) const fn cli(&self) -> &Cli {
        &self.cli
    }

    pub(crate) const fn client(&self) -> &ControlClient {
        &self.client
    }

    pub(crate) fn command_request(&self, command: Command) -> Result<CommandRequest, CliError> {
        command_request(&self.cli, command, None)
    }

    pub(crate) fn command_request_with_revision(
        &self,
        command: Command,
        required_revision: &str,
    ) -> Result<CommandRequest, CliError> {
        command_request(&self.cli, command, Some(required_revision))
    }

    pub(crate) fn page_request(
        &self,
        limit: u32,
        cursor: Option<&str>,
    ) -> Result<PageRequest, CliError> {
        let cursor = self.cursor(cursor)?;
        let page = PageRequest { cursor, limit };
        page.validate()
            .map_err(|error| CliError::Invalid(error.to_string()))?;
        Ok(page)
    }

    pub(crate) fn cursor(&self, value: Option<&str>) -> Result<Option<Cursor>, CliError> {
        value
            .map(|value| serde_json::from_value(Value::String(value.to_owned())))
            .transpose()
            .map_err(|error| CliError::Invalid(error.to_string()))
    }

    pub(crate) fn read_json(
        &self,
        path: &Path,
        maximum: usize,
        kind: &str,
    ) -> Result<Value, CliError> {
        let bytes = self.read_bounded(path, maximum.min(MAX_DOCUMENT_BYTES), kind)?;
        milkdrift_control_protocol::decode_json(&bytes)
            .map_err(|error| CliError::Invalid(error.to_string()))
    }

    pub(crate) fn read_prompt_sequence(&self, path: &Path) -> Result<Value, CliError> {
        serde_json::to_value(self.read_prompt_sequence_document(path)?)
            .map_err(|error| CliError::Internal(error.to_string()))
    }

    pub(crate) fn read_prompt_sequence_document(
        &self,
        path: &Path,
    ) -> Result<PromptSequenceDocument, CliError> {
        let bytes = self.read_bounded(
            path,
            MAX_PROMPT_SEQUENCE_DOCUMENT_BYTES.min(MAX_DOCUMENT_BYTES),
            "prompt-sequence document",
        )?;
        PromptSequenceDocument::from_bytes(&bytes)
            .map_err(|error| CliError::Invalid(error.to_string()))
    }

    pub(crate) fn read_remediation_prompt(&self, path: &Path) -> Result<String, CliError> {
        let bytes = self.read_bounded(path, MAX_INLINE_PROMPT_BYTES, "remediation prompt")?;
        if bytes.is_empty() || bytes.len() > MAX_INLINE_PROMPT_BYTES {
            return Err(CliError::Invalid(format!(
                "remediation prompt must contain 1..={MAX_INLINE_PROMPT_BYTES} bytes"
            )));
        }
        String::from_utf8(bytes)
            .map_err(|_| CliError::Invalid("remediation prompt is not UTF-8".to_owned()))
    }

    pub(crate) fn read_layout(&self, path: &Path) -> Result<LayoutDocument, CliError> {
        let bytes = self.read_bounded(path, MAX_LAYOUT_BYTES, "layout document")?;
        milkdrift_control_protocol::decode_json(&bytes)
            .map_err(|error| CliError::Invalid(error.to_string()))
    }

    pub(crate) fn write_exact_document(
        &self,
        destination: Option<&Path>,
        bytes: &[u8],
    ) -> Result<(), CliError> {
        if let Some(destination) = destination {
            if destination == Path::new("-") {
                return Err(CliError::Invalid(
                    "use --document, not --output -, to emit a document to stdout".to_owned(),
                ));
            }
            let mut file = create_new_destination(destination, "document")?;
            if let Err(error) = file.write_all(bytes).and_then(|()| file.sync_all()) {
                drop(file);
                let _ = fs::remove_file(destination);
                return Err(CliError::Internal(format!(
                    "canonical document write failed: {:?}",
                    error.kind()
                )));
            }
            return Ok(());
        }
        let mut stdout = io::stdout().lock();
        stdout
            .write_all(bytes)
            .and_then(|()| stdout.flush())
            .map_err(|error| {
                CliError::Internal(format!(
                    "canonical document output failed: {:?}",
                    error.kind()
                ))
            })
    }

    pub(crate) fn confirm(&self, operation: &str) -> Result<(), CliError> {
        confirm(&self.cli, operation)
    }

    pub(crate) fn output<T: Serialize>(&self, kind: &str, value: &T) -> Result<(), CliError> {
        println!("{}", encode_output(&self.cli, kind, value)?);
        Ok(())
    }

    pub(crate) fn stream_status(
        &self,
        retryable: bool,
        error: &impl std::fmt::Display,
    ) -> Result<(), CliError> {
        if self.cli.json {
            println!(
                "{}",
                serde_json::to_string(&json!({
                "schema_version": JSON_OUTPUT_SCHEMA_VERSION,
                "type": "stream_status",
                "status": "reconnecting",
                "retryable": retryable
                }))
                .map_err(|encode| CliError::Internal(encode.to_string()))?
            );
        } else {
            eprintln!("observation stream: {error}; reconnecting when permitted");
        }
        Ok(())
    }

    fn read_bounded(&self, path: &Path, maximum: usize, kind: &str) -> Result<Vec<u8>, CliError> {
        let mut bytes = Vec::new();
        if path == Path::new("-") {
            if self.stdin_consumed.replace(true) {
                return Err(CliError::Invalid(
                    "stdin may supply only one bounded document".to_owned(),
                ));
            }
            let limit = u64::try_from(maximum).unwrap_or(u64::MAX).saturating_add(1);
            io::stdin()
                .lock()
                .take(limit)
                .read_to_end(&mut bytes)
                .map_err(|error| {
                    CliError::Invalid(format!("{kind} stdin read failed: {:?}", error.kind()))
                })?;
        } else {
            let file = fs::File::open(path).map_err(|error| {
                CliError::Invalid(format!("{kind} file read failed: {:?}", error.kind()))
            })?;
            let metadata = file.metadata().map_err(|error| {
                CliError::Invalid(format!("{kind} metadata read failed: {:?}", error.kind()))
            })?;
            if !metadata.is_file() {
                return Err(CliError::Invalid(format!(
                    "{kind} input must be a regular file or -"
                )));
            }
            if metadata.len() > u64::try_from(maximum).unwrap_or(u64::MAX) {
                return Err(CliError::Invalid(format!("{kind} exceeds {maximum} bytes")));
            }
            file.take(u64::try_from(maximum).unwrap_or(u64::MAX).saturating_add(1))
                .read_to_end(&mut bytes)
                .map_err(|error| {
                    CliError::Invalid(format!("{kind} file read failed: {:?}", error.kind()))
                })?;
        }
        if bytes.len() > maximum {
            return Err(CliError::Invalid(format!("{kind} exceeds {maximum} bytes")));
        }
        Ok(bytes)
    }
}

pub(crate) fn create_new_destination(destination: &Path, kind: &str) -> Result<fs::File, CliError> {
    if destination.file_name().is_none() {
        return Err(CliError::Invalid(format!(
            "{kind} destination must name a file"
        )));
    }
    fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(destination)
        .map_err(|error| {
            CliError::Invalid(format!(
                "{kind} destination must not already exist and must be writable: {:?}",
                error.kind()
            ))
        })
}

pub(crate) fn safe_identity(value: &str) -> Result<(), CliError> {
    if value.is_empty()
        || value.len() > 256
        || !value.is_ascii()
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':'))
    {
        return Err(CliError::Invalid("resource identity is invalid".to_owned()));
    }
    Ok(())
}

fn command_request(
    cli: &Cli,
    command: Command,
    required_revision: Option<&str>,
) -> Result<CommandRequest, CliError> {
    if let (Some(selected), Some(required)) = (&cli.expected_revision, required_revision)
        && selected != required
    {
        return Err(CliError::Invalid(
            "--expected-revision conflicts with the exact revision required by this document"
                .to_owned(),
        ));
    }
    let request = CommandRequest {
        protocol: ProtocolVersion::CURRENT,
        command_id: cli.command_id.clone().unwrap_or_else(generated_command_id),
        expected_sequence: cli.expected_sequence,
        expected_revision: cli
            .expected_revision
            .clone()
            .or_else(|| required_revision.map(str::to_owned)),
        reason: cli.reason.clone(),
        evidence: cli.evidence.clone(),
        command,
    };
    request
        .validate()
        .map_err(|error| CliError::Invalid(error.to_string()))?;
    Ok(request)
}

fn generated_command_id() -> String {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|duration| u64::try_from(duration.as_millis()).ok())
        .unwrap_or(0);
    format!("cli-{millis}-{}", std::process::id())
}

fn load_credential(cli: &Cli) -> Result<BearerCredential, CliError> {
    let mut value = if let Some(path) = &cli.token_file {
        let file = fs::File::open(path).map_err(|error| {
            CliError::Invalid(format!("credential file unavailable: {:?}", error.kind()))
        })?;
        let metadata = file.metadata().map_err(|error| {
            CliError::Invalid(format!(
                "credential file metadata unavailable: {:?}",
                error.kind()
            ))
        })?;
        if !metadata.is_file() || metadata.len() > 4_097 {
            return Err(CliError::Invalid(
                "credential file is not a bounded regular file".to_owned(),
            ));
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            if metadata.permissions().mode() & 0o077 != 0 {
                return Err(CliError::Invalid(
                    "credential file must not be accessible by group or other users".to_owned(),
                ));
            }
        }
        let mut bytes = Vec::new();
        file.take(4_098).read_to_end(&mut bytes).map_err(|error| {
            CliError::Invalid(format!("credential file read failed: {:?}", error.kind()))
        })?;
        if bytes.len() > 4_097 {
            return Err(CliError::Invalid(
                "credential file is not a bounded regular file".to_owned(),
            ));
        }
        String::from_utf8(bytes)
            .map_err(|_| CliError::Invalid("credential file is not UTF-8".to_owned()))?
    } else {
        env::var(&cli.token_env).map_err(|_| {
            CliError::Invalid(
                "configured credential environment reference is unavailable".to_owned(),
            )
        })?
    };
    if value.ends_with('\n') {
        value.pop();
        if value.ends_with('\r') {
            value.pop();
        }
    }
    BearerCredential::new(value).map_err(CliError::from)
}

fn confirm(cli: &Cli, operation: &str) -> Result<(), CliError> {
    if cli.yes {
        return Ok(());
    }
    if cli.json {
        return Err(CliError::Invalid(
            "high-risk JSON-mode commands require --yes".to_owned(),
        ));
    }
    eprint!("Confirm {operation}? Type 'yes': ");
    io::stderr()
        .flush()
        .map_err(|error| CliError::Internal(error.to_string()))?;
    let mut answer = String::new();
    io::stdin()
        .read_line(&mut answer)
        .map_err(|error| CliError::Internal(error.to_string()))?;
    if answer.trim() == "yes" {
        Ok(())
    } else {
        Err(CliError::Invalid("operation was not confirmed".to_owned()))
    }
}

fn encode_output<T: Serialize>(cli: &Cli, kind: &str, value: &T) -> Result<String, CliError> {
    let document = json!({
        "schema_version": JSON_OUTPUT_SCHEMA_VERSION,
        "type": kind,
        "value": value,
    });
    if cli.json {
        serde_json::to_string(&document)
    } else {
        serde_json::to_string_pretty(&document)
    }
    .map_err(|error| CliError::Internal(error.to_string()))
}

#[cfg(test)]
mod tests {
    use clap::Parser as _;
    use milkdrift_control_protocol::Command;
    use serde_json::json;

    use super::{JSON_OUTPUT_SCHEMA_VERSION, command_request, confirm, encode_output};
    use crate::Cli;

    #[test]
    fn command_envelope_preserves_operator_owned_fields() -> Result<(), Box<dyn std::error::Error>>
    {
        let cli = Cli::try_parse_from([
            "milkdrift",
            "--command-id",
            "command-fixed",
            "--reason",
            "fixed reason",
            "--expected-sequence",
            "41",
            "daemon",
            "health",
        ])?;
        let request = command_request(
            &cli,
            Command::PauseRun {
                run_id: "run-one".to_owned(),
            },
            None,
        )?;
        assert_eq!(request.command_id, "command-fixed");
        assert_eq!(request.reason, "fixed reason");
        assert_eq!(request.expected_sequence, Some(41));
        assert!(request.expected_revision.is_none());
        assert!(request.evidence.is_empty());
        assert!(matches!(request.command, Command::PauseRun { run_id } if run_id == "run-one"));
        Ok(())
    }

    #[test]
    fn command_envelope_accepts_exact_revision_and_bounded_evidence()
    -> Result<(), Box<dyn std::error::Error>> {
        let cli = Cli::try_parse_from([
            "milkdrift",
            "--command-id",
            "command-fixed",
            "--expected-revision",
            "revision-one",
            "--evidence",
            "artifact=artifact-one",
            "daemon",
            "health",
        ])?;
        let request = command_request(
            &cli,
            Command::PauseRun {
                run_id: "run-one".to_owned(),
            },
            None,
        )?;
        assert_eq!(request.expected_revision.as_deref(), Some("revision-one"));
        assert_eq!(request.evidence[0].kind, "artifact");
        assert_eq!(request.evidence[0].id, "artifact-one");
        Ok(())
    }

    #[test]
    fn json_output_schema_is_stable_and_has_no_control_characters()
    -> Result<(), Box<dyn std::error::Error>> {
        let cli = Cli::try_parse_from(["milkdrift", "--json", "daemon", "health"])?;
        let value = encode_output(&cli, "fixture", &json!({"ok": true}))?;
        assert_eq!(
            value,
            format!(
                r#"{{"schema_version":{JSON_OUTPUT_SCHEMA_VERSION},"type":"fixture","value":{{"ok":true}}}}"#
            )
        );
        assert!(!value.contains('\u{1b}'));
        Ok(())
    }

    #[test]
    fn high_risk_json_mode_requires_yes() -> Result<(), Box<dyn std::error::Error>> {
        let cli = Cli::try_parse_from(["milkdrift", "--json", "daemon", "health"])?;
        assert!(confirm(&cli, "test").is_err());
        Ok(())
    }

    #[test]
    fn artifact_destination_must_be_new() -> Result<(), Box<dyn std::error::Error>> {
        let directory = tempfile::tempdir()?;
        let destination = directory.path().join("artifact.bin");
        drop(super::create_new_destination(&destination, "artifact")?);
        assert!(super::create_new_destination(&destination, "artifact").is_err());
        Ok(())
    }
}
