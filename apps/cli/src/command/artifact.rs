use std::{fs, io::Write as _, path::Path};

use milkdrift_workspace::ContentDigest;
use serde_json::json;

use crate::{ArtifactCommand, error::CliError, session::CliSession};

pub(super) async fn execute(
    session: &CliSession,
    command: &ArtifactCommand,
) -> Result<(), CliError> {
    match command {
        ArtifactCommand::Metadata { artifact } => session.output(
            "artifact.metadata",
            &session.client().artifact_metadata(artifact).await?,
        ),
        ArtifactCommand::Get {
            artifact,
            output: destination,
        } => download(session, artifact, destination).await,
    }
}

async fn download(
    session: &CliSession,
    artifact: &str,
    destination: &Path,
) -> Result<(), CliError> {
    let metadata = session.client().artifact_metadata(artifact).await?;
    let expected_digest = ContentDigest::from_hex(&metadata.digest).map_err(|_| {
        CliError::Internal("artifact metadata has an invalid content digest".to_owned())
    })?;
    let mut file = crate::session::create_new_destination(destination, "artifact")?;
    let result = async {
        let mut offset = 0_u64;
        let mut digest = blake3::Hasher::new();
        while offset < metadata.size {
            let end = offset
                .saturating_add(1_048_576 - 1)
                .min(metadata.size.saturating_sub(1));
            let range = session
                .client()
                .artifact_range(artifact, offset, end)
                .await?;
            if range.bytes.is_empty()
                || range.start != offset
                || range.complete_size != metadata.size
                || range.end
                    != offset.saturating_add(
                        u64::try_from(range.bytes.len())
                            .unwrap_or(0)
                            .saturating_sub(1),
                    )
            {
                return Err(CliError::Internal(
                    "artifact range identity or size was inconsistent".to_owned(),
                ));
            }
            file.write_all(&range.bytes).map_err(|error| {
                CliError::Internal(format!("artifact write failed: {:?}", error.kind()))
            })?;
            digest.update(&range.bytes);
            offset = offset.saturating_add(u64::try_from(range.bytes.len()).unwrap_or(0));
        }
        if offset != metadata.size || digest.finalize().as_bytes() != expected_digest.as_bytes() {
            return Err(CliError::Internal(
                "downloaded artifact size or digest did not match metadata".to_owned(),
            ));
        }
        file.sync_all().map_err(|error| {
            CliError::Internal(format!("artifact flush failed: {:?}", error.kind()))
        })?;
        Ok::<(), CliError>(())
    }
    .await;
    if result.is_err() {
        drop(file);
        let _ = fs::remove_file(destination);
    }
    result?;
    session.output(
        "artifact.get",
        &json!({
            "artifact_id": metadata.artifact_id,
            "digest": metadata.digest,
            "size": metadata.size,
            "destination": destination,
        }),
    )
}
