//! Selected-only materialization and exact manifest artifact reads.

use milkdrift_capability::{
    ArtifactReference as CapabilityArtifactReference, BoundedJson, InputReference,
    InvocationValueReference,
};
use milkdrift_model::{ContextManifest, ContextManifestDocument, ContextSource};
use milkdrift_persistence::{ArtifactReadAuthority, ArtifactReadRequest};
use milkdrift_workspace::{ArtifactReference, ContentDigest, WorkspaceValue};

use super::{
    ContextBuildError, exact_event, persistence, summarize_context_event, workspace_artifact,
};

/// Loads only the selected sources after the manifest artifact is durable.
pub fn materialize_selected_context(
    store: &dyn crate::RuntimeStore,
    manifest: &ContextManifest,
) -> Result<Vec<InputReference>, ContextBuildError> {
    let mut inputs = Vec::new();
    for entry in manifest.entries() {
        if matches!(entry.source(), ContextSource::DirectInput { .. }) {
            continue;
        }
        let value = match entry.source() {
            ContextSource::Artifact { reference } => InvocationValueReference::Artifact {
                reference: capability_artifact(reference)?,
            },
            ContextSource::WorkspaceValue { reference } => {
                let value = store.value(reference).map_err(persistence)?.ok_or(
                    ContextBuildError::RequiredUnavailable("selected workspace value"),
                )?;
                let bytes = workspace_bytes(value.value())?;
                verify_materialized(entry, &bytes)?;
                InvocationValueReference::WorkspaceValue {
                    identity: serde_json::to_string(reference)
                        .map_err(|error| ContextBuildError::Policy(error.to_string()))?,
                    version: reference.version().get().to_string(),
                }
            }
            ContextSource::Event { event, sequence } => {
                let envelope = exact_event(store, manifest.run(), *sequence, event)?;
                let summary = summarize_context_event(&envelope)?;
                let bytes = serde_json::to_vec(summary.value())
                    .map_err(|error| ContextBuildError::Policy(error.to_string()))?;
                verify_materialized(entry, &bytes)?;
                InvocationValueReference::Inline { value: summary }
            }
            ContextSource::NodeExecution {
                node,
                execution,
                attempt,
                event_sequence,
            } => {
                let value = BoundedJson::new(serde_json::json!({
                    "node": node,
                    "execution": execution,
                    "attempt": attempt,
                    "event_sequence": event_sequence,
                }))
                .map_err(|error| ContextBuildError::Policy(error.to_string()))?;
                let bytes = serde_json::to_vec(value.value())
                    .map_err(|error| ContextBuildError::Policy(error.to_string()))?;
                verify_materialized(entry, &bytes)?;
                InvocationValueReference::Inline { value }
            }
            ContextSource::DirectInput { .. } => unreachable!(),
        };
        if let InvocationValueReference::Artifact { reference } = &value {
            let durable = workspace_artifact(reference)?;
            let metadata = store
                .metadata(durable.artifact())
                .map_err(persistence)?
                .filter(|metadata| metadata.reference() == &durable)
                .ok_or(ContextBuildError::RequiredUnavailable(
                    "selected artifact changed or disappeared",
                ))?;
            if metadata.reference().digest() != entry.content_digest()
                || metadata.reference().size_bytes() != entry.selected_artifact_bytes()
            {
                return Err(ContextBuildError::RequiredUnavailable(
                    "selected artifact integrity",
                ));
            }
        }
        inputs.push(
            InputReference::new(
                format!(
                    "{}{:04}",
                    milkdrift_capability::CONTEXT_ITEM_INPUT_PREFIX,
                    entry.ordinal()
                ),
                value,
            )
            .map_err(|error| ContextBuildError::Policy(error.to_string()))?,
        );
    }
    Ok(inputs)
}

/// Reads and verifies one exact manifest artifact through bounded artifact reads.
pub fn read_context_manifest(
    store: &dyn crate::RuntimeStore,
    reference: &CapabilityArtifactReference,
    authority: ArtifactReadAuthority,
) -> Result<ContextManifest, ContextBuildError> {
    let reference = workspace_artifact(reference)?;
    let capacity = usize::try_from(reference.size_bytes())
        .map_err(|_| ContextBuildError::AccountingOverflow)?;
    let mut bytes = Vec::with_capacity(capacity);
    let mut offset = 0_u64;
    while offset < reference.size_bytes() {
        let maximum = u32::try_from(
            (reference.size_bytes() - offset)
                .min(milkdrift_persistence::MAX_ARTIFACT_CHUNK_BYTES as u64),
        )
        .map_err(|_| ContextBuildError::AccountingOverflow)?;
        let chunk = store
            .read_chunk(
                &ArtifactReadRequest::new(reference.clone(), offset, maximum, authority.clone())
                    .map_err(persistence)?,
            )
            .map_err(persistence)?;
        if chunk.offset != offset || chunk.bytes.is_empty() {
            return Err(ContextBuildError::Persistence(
                "manifest reader made no exact progress".to_owned(),
            ));
        }
        offset = offset
            .checked_add(
                u64::try_from(chunk.bytes.len())
                    .map_err(|_| ContextBuildError::AccountingOverflow)?,
            )
            .ok_or(ContextBuildError::AccountingOverflow)?;
        bytes.extend_from_slice(&chunk.bytes);
    }
    if !reference.verifies(&bytes) {
        return Err(ContextBuildError::Persistence(
            "manifest content contradicts its reference".to_owned(),
        ));
    }
    ContextManifestDocument::from_json(&bytes)
        .map(|document| document.body().clone())
        .map_err(Into::into)
}

fn verify_materialized(
    entry: &milkdrift_model::ContextManifestEntry,
    bytes: &[u8],
) -> Result<(), ContextBuildError> {
    if ContentDigest::for_bytes(bytes) != entry.content_digest()
        || u64::try_from(bytes.len()) != Ok(entry.selected_bytes())
    {
        return Err(ContextBuildError::RequiredUnavailable(
            "selected content integrity",
        ));
    }
    Ok(())
}

fn workspace_bytes(value: &WorkspaceValue) -> Result<Vec<u8>, ContextBuildError> {
    match value {
        WorkspaceValue::Json(value) => serde_json::to_vec(value.value())
            .map_err(|error| ContextBuildError::Policy(error.to_string())),
        WorkspaceValue::Artifact(_) => Err(ContextBuildError::RequiredUnavailable(
            "workspace source changed from inline to artifact",
        )),
    }
}

fn capability_artifact(
    reference: &ArtifactReference,
) -> Result<CapabilityArtifactReference, ContextBuildError> {
    CapabilityArtifactReference::new(
        reference.artifact().as_str(),
        reference.digest().to_hex(),
        Some(reference.media_type().as_str().to_owned()),
        Some(reference.size_bytes()),
    )
    .map_err(|error| ContextBuildError::Policy(error.to_string()))
}
