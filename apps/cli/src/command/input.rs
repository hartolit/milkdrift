//! Local text-file convenience over the shared authenticated upload operation.
use crate::{error::CliError, session::CliSession};
use milkdrift_control_protocol::{
    ArtifactMetadataRead, InputUploadRequest, MAX_INPUT_UPLOAD_BYTES,
};
use std::{collections::BTreeSet, path::Path};

pub(super) async fn upload_text(
    session: &CliSession,
    host: &str,
    identity: &str,
    prefix: &str,
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
        let identity = format!("{identity}:{index}:{name}");
        let request = InputUploadRequest::from_content(
            host.into(),
            format!("{prefix}:{}", blake3::hash(identity.as_bytes())),
            "text/plain".into(),
            "restricted".into(),
            &bytes,
        )
        .map_err(|error| CliError::Invalid(error.to_string()))?;
        inputs.push((name.into(), session.client().upload_input(&request).await?));
    }
    Ok(inputs)
}
