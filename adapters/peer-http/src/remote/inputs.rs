//! Freeze origin-owned inputs before entry, then stage them under the exact delegated request.
use milkdrift_capability_host::{AdapterError, AdapterInvocation, AdapterReporter};
use milkdrift_peer_protocol::{
    ArtifactChunk, ArtifactMetadataOffer, ArtifactTransferBinding, ArtifactTransferDecision,
    ArtifactTransferDirection, MAX_ARTIFACT_CHUNK_BYTES, ServingInvocationRequest, TransferId,
};
use milkdrift_persistence::{ArtifactReadAuthority, ArtifactReadRequest, EvidenceId};
use milkdrift_workspace::{ArtifactMetadata, ContentDigest};

use super::RemoteCapabilityAdapter;

pub(super) struct PreparedInput {
    metadata: ArtifactMetadata,
    bytes: Vec<u8>,
}

impl RemoteCapabilityAdapter {
    pub(super) fn prepare_inputs(
        &self,
        invocation: &AdapterInvocation<'_>,
    ) -> Result<Vec<PreparedInput>, AdapterError> {
        let references = invocation
            .request()
            .inputs()
            .iter()
            .filter_map(|input| input.value().artifact())
            .chain(invocation.request().context_manifest())
            .collect::<Vec<_>>();
        if invocation.request().inputs().iter().any(|input| {
            matches!(
                input.value(),
                milkdrift_capability::InvocationValueReference::WorkspaceValue { .. }
            )
        }) {
            return Err(AdapterError::rejected(
                "remote execution requires explicit inline or artifact inputs; workspace values must be selected into portable inputs by the origin",
            ));
        }
        let mut total = 0_u64;
        for reference in &references {
            let size = reference
                .size_bytes()
                .ok_or_else(|| AdapterError::rejected("remote input requires an exact size"))?;
            total = total
                .checked_add(size)
                .filter(|total| *total <= self.relationship.execution_limits.artifact_bytes)
                .ok_or_else(|| {
                    AdapterError::rejected("remote inputs exceed the accepted artifact allowance")
                })?;
            if size > self.relationship.maximum_artifact_bytes {
                return Err(AdapterError::rejected(
                    "remote input exceeds the relationship transfer limit",
                ));
            }
        }
        let mut prepared = Vec::new();
        for reference in references {
            let basis = invocation
                .context()
                .and_then(|context| context.authority())
                .ok_or_else(|| {
                    AdapterError::rejected(
                        "remote input preparation requires the originating execution authority",
                    )
                })?;
            let authority = ArtifactReadAuthority::Authorized {
                actor: basis.actor().clone(),
                evidence: EvidenceId::new(basis.accepted_decision().as_str())
                    .map_err(|error| AdapterError::rejected(error.to_string()))?,
            };
            let metadata = self
                .artifacts
                .metadata(reference)
                .map_err(|error| AdapterError::rejected(error.to_string()))?;
            if !self
                .relationship
                .artifact_sensitivities
                .contains(&metadata.sensitivity())
            {
                return Err(AdapterError::rejected(
                    "remote input sensitivity is outside the relationship grant",
                ));
            }
            let size = metadata.reference().size_bytes();
            let mut bytes = Vec::with_capacity(
                usize::try_from(size)
                    .map_err(|_| AdapterError::rejected("input cannot fit this platform"))?,
            );
            while (bytes.len() as u64) < size {
                let request = ArtifactReadRequest::new(
                    metadata.reference().clone(),
                    bytes.len() as u64,
                    MAX_ARTIFACT_CHUNK_BYTES,
                    authority.clone(),
                )
                .map_err(|error| AdapterError::rejected(error.to_string()))?;
                let chunk = self
                    .artifacts
                    .read_input_chunk(&request)
                    .map_err(|error| AdapterError::rejected(error.to_string()))?;
                let next = (bytes.len() as u64)
                    .checked_add(chunk.bytes.len() as u64)
                    .filter(|next| *next <= size)
                    .ok_or_else(|| AdapterError::rejected("input read exceeded its exact size"))?;
                if chunk.offset != bytes.len() as u64
                    || chunk.bytes.is_empty()
                    || chunk.end_of_artifact != (next == size)
                {
                    return Err(AdapterError::rejected(
                        "input read did not make exact bounded progress",
                    ));
                }
                bytes.extend_from_slice(&chunk.bytes);
            }
            if ContentDigest::for_bytes(&bytes) != metadata.reference().digest() {
                return Err(AdapterError::rejected(
                    "remote input digest differs from its selected bytes",
                ));
            }
            prepared.push(PreparedInput { metadata, bytes });
        }
        Ok(prepared)
    }

    pub(super) fn stage_inputs(
        &self,
        request: &ServingInvocationRequest,
        inputs: Vec<PreparedInput>,
        reporter: &dyn AdapterReporter,
    ) -> Result<(), AdapterError> {
        for input in inputs {
            let identity = serde_json::to_vec(&(
                self.client.local_peer(),
                self.client.remote_peer(),
                &request.request_id,
                input.metadata.reference(),
            ))
            .map_err(|error| AdapterError::rejected(error.to_string()))?;
            let transfer = TransferId::new(format!("input:{}", blake3::hash(&identity)))
                .map_err(|error| AdapterError::rejected(error.to_string()))?;
            let offer = ArtifactMetadataOffer {
                transfer: transfer.clone(),
                direction: ArtifactTransferDirection::Upload,
                artifact: input.metadata.reference().clone(),
                sensitivity: input.metadata.sensitivity(),
                retention: input.metadata.retention().clone(),
                provenance: input.metadata.provenance().clone(),
                source_peer: self.client.local_peer().clone(),
                binding: ArtifactTransferBinding::Input {
                    request: Box::new(request.clone()),
                },
                expires_at_unix_ms: request.deadline_unix_ms,
            };
            let result = (|| {
                let (mut offset, maximum) = match self
                    .client
                    .negotiate_artifact(&offer)
                    .map_err(|error| AdapterError::external_failure(error.to_string()))?
                {
                    ArtifactTransferDecision::AlreadyPresent => return Ok(()),
                    ArtifactTransferDecision::Transfer {
                        next_offset,
                        maximum_chunk_bytes,
                    } if maximum_chunk_bytes > 0 => (
                        next_offset,
                        maximum_chunk_bytes.min(MAX_ARTIFACT_CHUNK_BYTES),
                    ),
                    _ => {
                        return Err(AdapterError::external_failure(
                            "remote input staging was refused",
                        ));
                    }
                };
                while offset < input.bytes.len() as u64 {
                    reporter.heartbeat()?;
                    let start = usize::try_from(offset).map_err(|_| {
                        AdapterError::external_failure("remote input offset overflow")
                    })?;
                    let end = start
                        .saturating_add(maximum as usize)
                        .min(input.bytes.len());
                    let chunk = ArtifactChunk {
                        transfer: transfer.clone(),
                        offset,
                        bytes: input.bytes[start..end].to_vec(),
                        final_chunk: end == input.bytes.len(),
                    };
                    let decision = self
                        .client
                        .write_artifact_chunk(&chunk)
                        .map_err(|error| AdapterError::external_failure(error.to_string()))?;
                    match decision {
                        ArtifactTransferDecision::AlreadyPresent if chunk.final_chunk => {}
                        ArtifactTransferDecision::Transfer { next_offset, .. }
                            if next_offset == end as u64 && !chunk.final_chunk => {}
                        _ => {
                            return Err(AdapterError::external_failure(
                                "remote input staging did not accept the exact chunk",
                            ));
                        }
                    }
                    offset = end as u64;
                }
                if offset != input.bytes.len() as u64 {
                    return Err(AdapterError::external_failure(
                        "remote input resume offset exceeds its exact size",
                    ));
                }
                Ok(())
            })();
            let _ = self.client.abort_artifact(&transfer);
            result?;
        }
        Ok(())
    }
}
