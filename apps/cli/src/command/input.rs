//! Local text-file convenience over the shared authenticated upload operation.
use crate::{error::CliError, session::CliSession};
use milkdrift_control_protocol::{
    ArtifactMetadataRead, InputUploadRequest, MAX_INPUT_UPLOAD_BYTES,
};
use std::{collections::BTreeSet, path::Path};

#[derive(Clone, Copy)]
pub(super) enum UploadOperation {
    Run,
    Invocation,
}

impl UploadOperation {
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

pub(super) async fn upload_text(
    session: &CliSession,
    host: &str,
    identity: &str,
    operation: UploadOperation,
    specifications: &[String],
    names: &mut BTreeSet<String>,
) -> Result<Vec<(String, ArtifactMetadataRead)>, CliError> {
    let mut inputs = Vec::new();
    for (index, specification) in specifications.iter().enumerate() {
        let (name, source) = specification
            .split_once('=')
            .filter(|(name, path)| !name.is_empty() && !path.is_empty())
            .ok_or_else(|| CliError::Invalid("--input requires NAME=FILE".into()))?;
        if !names.insert(name.into()) {
            return Err(CliError::Invalid("input names must be distinct".into()));
        }
        let bytes = session
            .read_bounded(Path::new(source), MAX_INPUT_UPLOAD_BYTES, "text input")
            .await?;
        std::str::from_utf8(&bytes).map_err(|_| {
            CliError::Invalid(
                "--input requires UTF-8 text; use artifact upload and --inputs for other media"
                    .into(),
            )
        })?;
        let request = InputUploadRequest::from_content(
            host.into(),
            operation.identity(identity, index, name)?,
            "text/plain".into(),
            "restricted".into(),
            &bytes,
        )
        .map_err(|error| CliError::Invalid(error.to_string()))?;
        inputs.push((name.into(), session.client().upload_input(&request).await?));
    }
    Ok(inputs)
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
