//! Local text-file convenience over the shared authenticated upload operation.
use crate::{error::CliError, session::CliSession};
use milkdrift_control_protocol::{
    ArtifactMetadataRead, InputUploadRequest, MAX_INPUT_UPLOAD_BYTES,
};
use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
};

#[derive(Clone, Copy)]
pub(super) enum UploadOperation {
    Run,
    Invocation,
}

impl UploadOperation {
    fn maximum(self) -> usize {
        match self {
            Self::Run => milkdrift_blueprint::WorkflowInterface::MAX_FIELDS,
            Self::Invocation => milkdrift_capability::InvocationRequest::MAX_INPUTS,
        }
    }

    fn validate_name(self, name: &str) -> Result<(), CliError> {
        match self {
            Self::Run => milkdrift_blueprint::FieldId::new(name)
                .map(|_| ())
                .map_err(|error| CliError::Invalid(error.to_string())),
            Self::Invocation => milkdrift_capability::InputReference::validate_name(name)
                .map_err(|error| CliError::Invalid(error.to_string())),
        }
    }

    fn namespace(self) -> &'static str {
        match self {
            Self::Run => "run-input",
            Self::Invocation => "invocation-input",
        }
    }

    fn identity(self, command: &str, position: usize, name: &str) -> Result<String, CliError> {
        let namespace = self.namespace();
        // The fixed tuple binds the operation and each field independently. Legal
        // colons in a command or input name must never become field separators.
        let bytes =
            serde_json::to_vec(&("milkdrift.cli.input.v2", namespace, command, position, name))
                .map_err(|error| CliError::Internal(error.to_string()))?;
        Ok(format!("{namespace}:{}", blake3::hash(&bytes)))
    }
}

struct TextInput {
    name: String,
    source: PathBuf,
    digest: blake3::Hash,
    stdin: Option<Vec<u8>>,
}

/// Retain only bounded names, paths and digests, plus at most one bounded stdin value.
/// Files are read again before upload; preflight does not freeze external writers.
pub(super) struct TextInputs {
    operation: UploadOperation,
    inputs: Vec<TextInput>,
}

impl TextInputs {
    pub(super) fn names(&self) -> impl Iterator<Item = &str> {
        self.inputs.iter().map(|input| input.name.as_str())
    }

    pub(super) async fn preflight(
        session: &CliSession,
        operation: UploadOperation,
        specifications: &[String],
        mut names: BTreeSet<String>,
    ) -> Result<Self, CliError> {
        if names.len().saturating_add(specifications.len()) > operation.maximum() {
            return Err(CliError::Invalid("too many supplied inputs".into()));
        }
        for name in &names {
            operation.validate_name(name)?;
        }
        let mut sources = Vec::new();
        let mut stdin = false;
        for specification in specifications {
            let (name, source) = specification
                .split_once('=')
                .filter(|(name, path)| !name.is_empty() && !path.is_empty())
                .ok_or_else(|| CliError::Invalid("--input requires NAME=FILE".into()))?;
            operation.validate_name(name)?;
            if !names.insert(name.into()) {
                return Err(CliError::Invalid("input names must be distinct".into()));
            }
            if source == "-" && std::mem::replace(&mut stdin, true) {
                return Err(CliError::Invalid(
                    "stdin may supply only one bounded document".into(),
                ));
            }
            sources.push((name.to_owned(), PathBuf::from(source)));
        }
        let mut inputs = Vec::new();
        for (name, source) in sources {
            let bytes = session
                .read_bounded(&source, MAX_INPUT_UPLOAD_BYTES, "text input")
                .await?;
            std::str::from_utf8(&bytes).map_err(|_| {
                CliError::Invalid(
                    "--input requires UTF-8 text; use artifact upload and --inputs for other media"
                        .into(),
                )
            })?;
            let digest = blake3::hash(&bytes);
            let stdin = (source == Path::new("-")).then_some(bytes);
            inputs.push(TextInput {
                name,
                source,
                digest,
                stdin,
            });
        }
        Ok(Self { operation, inputs })
    }

    pub(super) async fn upload(
        self,
        session: &CliSession,
        host: &str,
        identity: &str,
    ) -> Result<Vec<(String, ArtifactMetadataRead)>, CliError> {
        let mut uploaded = Vec::new();
        for (index, input) in self.inputs.into_iter().enumerate() {
            let bytes = match input.stdin {
                Some(bytes) => bytes,
                None => {
                    session
                        .read_bounded(&input.source, MAX_INPUT_UPLOAD_BYTES, "text input")
                        .await?
                }
            };
            if blake3::hash(&bytes) != input.digest {
                return Err(CliError::Invalid("text input changed after preflight; restore original bytes before exact preparation recovery".into()));
            }
            let request = InputUploadRequest::from_content(
                host.into(),
                self.operation.identity(identity, index, &input.name)?,
                "text/plain".into(),
                "restricted".into(),
                &bytes,
            )
            .map_err(|error| CliError::Invalid(error.to_string()))?;
            // Flush the exact upload identity before sending: a lost reply or process
            // interruption may follow a remote commit. No content or credentials are emitted.
            progress(
                session,
                identity,
                "input.uploading",
                serde_json::json!({
                    "host": host, "identity": identity, "name": input.name, "position": index,
                    "upload_id": request.upload_id, "digest": request.digest, "size": bytes.len(),
                    "recovery": "Retain this record. If preparation stops, repeat it with the same host, actor, identity, ordered names and original bytes. Committed uploads remain retained and consume capacity; never delete them as local cleanup. If a complete request file exists, recover with that exact file instead."
                }),
            )?;
            let artifact = session.client().upload_input(&request).await?;
            if artifact.digest != request.digest
                || artifact.size != bytes.len() as u64
                || artifact.content_type != request.media_type
                || artifact.sensitivity != request.sensitivity
            {
                return Err(CliError::Internal("upload reply contradicts the supplied content or classification; recover the pending upload identity".into()));
            }
            progress(
                session,
                identity,
                "input.uploaded",
                serde_json::json!({
                    "host": host, "identity": identity, "name": input.name, "position": index,
                    "upload_id": request.upload_id, "artifact": artifact
                }),
            )?;
            uploaded.push((input.name, artifact));
        }
        Ok(uploaded)
    }
}

pub(super) fn progress(
    session: &CliSession,
    identity: &str,
    kind: &str,
    value: serde_json::Value,
) -> Result<(), CliError> {
    use std::io::Write as _;
    if session.cli().json {
        crate::output::line(format_args!(
            "{}",
            crate::output::encode(
                kind,
                Some(identity),
                "preparing",
                value,
                serde_json::Value::Null,
                false
            )?
        ))?;
    } else {
        crate::output::line(format_args!(
            "{kind}\n{}",
            serde_json::to_string_pretty(&value)
                .map_err(|error| CliError::Internal(error.to_string()))?
        ))?;
    }
    std::io::stdout().flush().map_err(CliError::from)
}

#[cfg(test)]
mod tests {
    use super::*;
    type TestResult = Result<(), Box<dyn std::error::Error>>;

    #[test]
    fn upload_fields_and_operation_have_unambiguous_boundaries() -> TestResult {
        for operation in [UploadOperation::Run, UploadOperation::Invocation] {
            let first = operation.identity("x", 0, "z:1:y")?;
            assert_ne!(first, operation.identity("x:0:z", 1, "y")?);
            assert_eq!(first, operation.identity("x", 0, "z:1:y")?);
            let expected = serde_json::to_vec(&(
                "milkdrift.cli.input.v2",
                operation.namespace(),
                "x",
                0,
                "z:1:y",
            ))?;
            assert_eq!(
                first,
                format!("{}:{}", operation.namespace(), blake3::hash(&expected))
            );
        }
        // Challenge field boundaries over a finite punctuation-bearing set;
        // this establishes the encoding examples, not a collision-proof hash.
        let mut ids = BTreeSet::new();
        for operation in [UploadOperation::Run, UploadOperation::Invocation] {
            for command in ["x", "x:0:z", "x:", "x::", "x/1", "x.1"] {
                for position in [0, 1, 11, 255] {
                    for name in ["y", "z:1:y", ":", "z::", "z.y", "z-y"] {
                        assert!(ids.insert(operation.identity(command, position, name)?));
                    }
                }
            }
        }
        Ok(())
    }

    #[test]
    fn maximum_valid_fields_keep_a_bounded_upload_identity() -> TestResult {
        // CommandRequest and PeerRequestId permit 192 bytes. Run fields permit
        // 96 bytes; invocation inputs permit 128. Neither permits empty names.
        for (operation, name_limit) in [
            (UploadOperation::Run, 96),
            (UploadOperation::Invocation, 128),
        ] {
            let command = format!("x{}", ":".repeat(191));
            let name = format!("n{}", ":".repeat(name_limit - 1));
            if matches!(operation, UploadOperation::Invocation) {
                milkdrift_peer_protocol::PeerRequestId::new(&command)?;
            } else {
                milkdrift_blueprint::FieldId::new(&name)?;
                milkdrift_control_protocol::CommandRequest {
                    protocol: milkdrift_control_protocol::ProtocolVersion::CURRENT,
                    command_id: command.clone(),
                    expected_sequence: None,
                    expected_revision: None,
                    reason: "encoding bound".into(),
                    evidence: vec![],
                    command: milkdrift_control_protocol::Command::CancelRun {
                        run_id: "run".into(),
                    },
                }
                .validate()?;
            }
            milkdrift_capability::InputReference::new(
                &name,
                milkdrift_capability::InvocationValueReference::Artifact {
                    reference: milkdrift_capability::ArtifactReference::new(
                        "artifact",
                        "a".repeat(64),
                        None,
                        None,
                    )?,
                },
            )?;
            let identity = operation.identity(&command, 255, &name)?;
            assert_eq!(identity.len(), operation.namespace().len() + 1 + 64);
        }
        Ok(())
    }
}
