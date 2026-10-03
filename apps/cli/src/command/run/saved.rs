//! One explicit recovery file per requested execution; no automatic client archive.
use crate::{RunCommand, error::CliError, session::CliSession};
use milkdrift_control_client::SavedRunRequest;
use milkdrift_control_protocol::{
    Command, InputUploadRequest, MAX_DOCUMENT_BYTES, MAX_INPUT_UPLOAD_BYTES, RunInput,
};
use serde_json::json;
use std::path::Path;

mod file;

pub(super) async fn execute(session: &CliSession, command: &RunCommand) -> Result<(), CliError> {
    let (saved, path, prepare_only, wait) = match command {
        RunCommand::Start {
            run,
            workflow,
            revision,
            inputs,
            input,
            request_file,
            prepare_only,
            wait,
        } => {
            // Prepare a private temporary file before uploading or submitting anything.
            let destination = file::PendingRequest::new(request_file)?;
            let mut supplied: Vec<RunInput> = match inputs {
                Some(path) => serde_json::from_value(
                    session
                        .read_json(path, MAX_DOCUMENT_BYTES, "run inputs")
                        .await?,
                )
                .map_err(|error| CliError::Invalid(error.to_string()))?,
                None => Vec::new(),
            };
            let request = session.command_request_with_revision(
                Command::StartRun {
                    run_id: run.clone(),
                    workflow_id: workflow.clone(),
                    revision_id: revision.clone(),
                    inputs: vec![],
                },
                revision,
            )?;
            let mut saved = session.client().prepare_run(request).await?;
            let mut names: std::collections::BTreeSet<String> =
                supplied.iter().map(|input| input.name.clone()).collect();
            if names.len() != supplied.len() {
                return Err(CliError::Invalid("run input names must be distinct".into()));
            }
            for (index, specification) in input.iter().enumerate() {
                let (name, source) = specification
                    .split_once('=')
                    .filter(|(name, path)| !name.is_empty() && !path.is_empty())
                    .ok_or_else(|| CliError::Invalid("--input requires NAME=FILE".into()))?;
                if !names.insert(name.into()) {
                    return Err(CliError::Invalid("run input names must be distinct".into()));
                }
                let bytes = session
                    .read_bounded(Path::new(source), MAX_INPUT_UPLOAD_BYTES, "run text input")
                    .await?;
                std::str::from_utf8(&bytes).map_err(|_| CliError::Invalid("--input requires UTF-8 text; use artifact upload and --inputs for other media".into()))?;
                let identity = format!("{}:{index}:{name}", saved.request.command_id);
                let upload = InputUploadRequest::from_content(
                    saved.authority.host.clone(),
                    format!("run-input:{}", blake3::hash(identity.as_bytes())),
                    "text/plain".into(),
                    "restricted".into(),
                    &bytes,
                )
                .map_err(|error| CliError::Invalid(error.to_string()))?;
                let artifact = session.client().upload_input(&upload).await?;
                supplied.push(RunInput {
                    name: name.into(),
                    artifact_id: artifact.artifact_id,
                });
            }
            if let Command::StartRun { inputs, .. } = &mut saved.request.command {
                *inputs = supplied;
            }
            saved.validate()?;
            destination.commit(&saved)?;
            (saved, request_file.as_path(), *prepare_only, *wait)
        }
        RunCommand::Reconnect { file, wait } => {
            let saved = file::read(file)?;
            // Global identity/guard overrides cannot silently replace fields in a saved request.
            if session.cli().expected_sequence.is_some()
                || session.cli().expected_revision.is_some()
                || !session.cli().evidence.is_empty()
            {
                return Err(CliError::Invalid(
                    "reconnect uses the saved guards and evidence".into(),
                ));
            }
            if session
                .cli()
                .command_id
                .as_ref()
                .is_some_and(|id| id != &saved.request.command_id)
                || (session.cli().reason != "operator CLI command"
                    && session.cli().reason != saved.request.reason)
            {
                return Err(CliError::Invalid(
                    "reconnect uses the saved command identity and reason".into(),
                ));
            }
            (saved, file.as_path(), false, *wait)
        }
        _ => return Err(CliError::Internal("expected start or reconnect".into())),
    };
    if prepare_only {
        return session.output("run.prepared", &identity(&saved, path));
    }
    if wait {
        announce(session, &saved, path)?;
    }
    let accepted = session.client().submit_saved_run(&saved).await?;
    let mut value =
        serde_json::to_value(accepted).map_err(|error| CliError::Internal(error.to_string()))?;
    value["recovery"] = identity(&saved, path);
    if wait {
        let Command::StartRun { run_id, .. } = &saved.request.command else {
            return Err(CliError::Internal("saved start missing".into()));
        };
        super::wait(
            session,
            run_id,
            &crate::TerminalFilter::Any,
            crate::DEFAULT_RUN_POLL_MS,
            crate::DEFAULT_RUN_MAX_POLLS,
        )
        .await
    } else {
        session.output("run.start", &value)
    }
}

fn announce(session: &CliSession, saved: &SavedRunRequest, path: &Path) -> Result<(), CliError> {
    if session.cli().json {
        println!(
            "{}",
            crate::output::encode(
                "run.prepared",
                Some(&saved.request.command_id),
                "prepared",
                identity(saved, path),
                serde_json::Value::Null,
                false
            )?
        );
    } else {
        session.output("run.prepared", &identity(saved, path))?;
    }
    use std::io::Write as _;
    std::io::stdout()
        .flush()
        .map_err(|error| CliError::Internal(error.to_string()))
}

pub(super) fn identity(saved: &SavedRunRequest, path: &Path) -> serde_json::Value {
    let Command::StartRun {
        run_id,
        workflow_id,
        revision_id,
        ..
    } = &saved.request.command
    else {
        return serde_json::Value::Null;
    };
    json!({"file":path,"host":saved.authority.host,"actor":saved.authority.actor,"command_id":saved.request.command_id,"run_id":run_id,"workflow_id":workflow_id,"revision_id":revision_id})
}
