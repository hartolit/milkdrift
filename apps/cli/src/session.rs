//! Load one credential and negotiate before routing a command through the shared client.
//!
//! Stable request identities remain attached to daemon-owned receipts. Local
//! confirmation expresses operator intent; the daemon still owns authorization and acceptance.
//! Document reads share a single-stdin-use rule so one command cannot consume it twice.

use std::{
    cell::Cell,
    env, fs,
    io::{self, BufRead as _, IsTerminal as _, Read as _, Write as _},
    path::Path,
    time::{SystemTime, UNIX_EPOCH},
};

use milkdrift_control_client::{BearerCredential, ClientConfig, ControlClient};
use milkdrift_control_protocol::{Cursor, MAX_DOCUMENT_BYTES};
use serde::Serialize;
use serde_json::{Value, json};

use crate::{Cli, error::CliError};

pub(crate) struct CliSession {
    cli: Cli,
    client: ControlClient,
    stdin_consumed: Cell<bool>,
}

impl CliSession {
    pub(crate) async fn connect(cli: Cli) -> Result<Self, CliError> {
        let token_file = cli.token_file.clone();
        let token_env = cli.token_env.clone();
        let credential =
            tokio::task::spawn_blocking(move || load_credential(token_file.as_deref(), &token_env))
                .await
                .map_err(|_| CliError::Internal("credential reader failed".to_owned()))??;
        let mut config = ClientConfig::new(cli.endpoint.clone());
        config.request_timeout = std::time::Duration::from_secs(cli.timeout_secs.unwrap_or(60));
        let client = ControlClient::new(config, credential)?;
        client.negotiate().await?;
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

    pub(crate) fn cursor(value: Option<&str>) -> Result<Option<Cursor>, CliError> {
        value
            .map(|value| serde_json::from_value(Value::String(value.to_owned())))
            .transpose()
            .map_err(|error| CliError::Invalid(error.to_string()))
    }

    pub(crate) async fn read_json(
        &self,
        path: &Path,
        maximum: usize,
        kind: &str,
    ) -> Result<Value, CliError> {
        let bytes = self
            .read_bounded(path, maximum.min(MAX_DOCUMENT_BYTES), kind)
            .await?;
        milkdrift_control_protocol::decode_json(&bytes)
            .map_err(|error| CliError::Invalid(error.to_string()))
    }

    pub(crate) async fn confirm(&self, operation: &str) -> Result<(), CliError> {
        confirm(&self.cli, operation).await
    }

    pub(crate) fn output<T: Serialize>(&self, kind: &str, value: &T) -> Result<(), CliError> {
        crate::output::success(&self.cli, kind, value)
    }

    pub(crate) fn stream_status(
        &self,
        retryable: bool,
        _error: &impl std::fmt::Display,
    ) -> Result<(), CliError> {
        if self.cli.json {
            crate::output::line(format_args!(
                "{}",
                crate::output::encode(
                    self.cli.operation(),
                    self.cli.command_id.as_deref(),
                    "reconnecting",
                    Value::Null,
                    json!({"classification":"unavailable", "code":"unavailable",
                        "daemon_code":null, "detail":"observation transport interrupted",
                        "retryable": retryable}),
                    false
                )?
            ))?;
        } else {
            writeln!(
                io::stderr().lock(),
                "observation stream reconnecting within configured bounds"
            )?;
        }
        Ok(())
    }
    pub(crate) async fn read_bounded(
        &self,
        path: &Path,
        maximum: usize,
        kind: &str,
    ) -> Result<Vec<u8>, CliError> {
        if path == Path::new("-") && self.stdin_consumed.replace(true) {
            return Err(CliError::Invalid(
                "stdin may supply only one bounded document".to_owned(),
            ));
        }
        crate::input::read_bounded(path, maximum, kind).await
    }
}
pub(crate) fn generated_command_id() -> String {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|duration| u64::try_from(duration.as_millis()).ok())
        .unwrap_or(0);
    format!("cli-{millis}-{}", std::process::id())
}

fn load_credential(
    token_file: Option<&Path>,
    token_env: &str,
) -> Result<BearerCredential, CliError> {
    let mut value = if let Some(path) = token_file {
        let metadata = fs::metadata(path)
            .map_err(|_| CliError::Invalid("credential file unavailable".to_owned()))?;
        if !metadata.is_file() || metadata.len() > 4_097 {
            return Err(CliError::Invalid(
                "credential file is not a bounded regular file".to_owned(),
            ));
        }
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
        env::var(token_env).map_err(|_| {
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

async fn confirm(cli: &Cli, operation: &str) -> Result<(), CliError> {
    if cli.yes {
        return Ok(());
    }
    if cli.json || !io::stdin().is_terminal() || !io::stdout().is_terminal() {
        return Err(CliError::Invalid(
            "high-risk noninteractive commands require --yes".to_owned(),
        ));
    }
    write!(io::stderr().lock(), "Confirm {operation}? Type 'yes': ")?;
    io::stderr()
        .flush()
        .map_err(|error| CliError::Internal(error.to_string()))?;
    let answer = tokio::task::spawn_blocking(|| {
        let mut answer = String::new();
        io::stdin()
            .lock()
            .take(5)
            .read_line(&mut answer)
            .map(|_| answer)
    })
    .await
    .map_err(|error| CliError::Internal(error.to_string()))?
    .map_err(|error| CliError::Internal(error.to_string()))?;
    if answer.trim() == "yes" {
        Ok(())
    } else {
        Err(CliError::Invalid("operation was not confirmed".to_owned()))
    }
}
#[cfg(test)]
mod tests {
    use clap::Parser as _;

    use super::confirm;
    use crate::Cli;

    #[tokio::test]
    async fn high_risk_json_mode_requires_yes() -> Result<(), Box<dyn std::error::Error>> {
        let cli = Cli::try_parse_from(["milkdrift", "--json", "daemon", "health"])?;
        assert!(confirm(&cli, "test").await.is_err());
        Ok(())
    }

    #[test]
    fn artifact_destination_must_be_new() -> Result<(), Box<dyn std::error::Error>> {
        let directory = tempfile::tempdir()?;
        let destination = directory.path().join("artifact.bin");
        crate::output::PendingFile::create(&destination)?.commit()?;
        assert!(crate::output::PendingFile::create(&destination).is_err());
        Ok(())
    }
}
