//! Materialize only observed outputs through authorized, bounded core transfers.
use std::collections::BTreeSet;

use milkdrift_capability::InvocationEventKind;
use milkdrift_capability_host::{AdapterError, AdapterReporter};
use milkdrift_peer_protocol::{
    ArtifactTransferDecision, ArtifactTransferDirection, PeerExecutionId, PeerObservation,
};

use super::{Lifecycle, Ordering, RemoteCapabilityAdapter};

impl RemoteCapabilityAdapter {
    pub(super) fn import_output(
        &self,
        execution: &PeerExecutionId,
        observation: &PeerObservation,
        deadline: u64,
        imported: &mut BTreeSet<String>,
        total_bytes: &mut u64,
        reporter: &dyn AdapterReporter,
    ) -> Result<(), AdapterError> {
        let InvocationEventKind::Output { reference, .. } = observation.event.kind() else {
            return Ok(());
        };
        if imported.contains(reference.identity()) {
            return Ok(());
        }
        let size = reference
            .size_bytes()
            .ok_or_else(|| AdapterError::external_failure("peer output lacks an exact size"))?;
        let next_total = total_bytes
            .checked_add(size)
            .filter(|total| *total <= self.relationship.execution_limits.artifact_bytes)
            .ok_or_else(|| {
                AdapterError::external_failure(
                    "peer outputs exceed the accepted artifact allowance",
                )
            })?;
        if imported.len() >= self.relationship.execution_limits.observations as usize {
            return Err(AdapterError::external_failure(
                "peer output count exceeds the observation allowance",
            ));
        }
        let download = self
            .client
            .output_artifact_offer(execution, observation.sequence)
            .map_err(|error| AdapterError::external_failure(error.to_string()))?;
        if download.artifact.artifact().as_str() != reference.identity()
            || download.artifact.digest().to_hex() != reference.digest()
            || download.artifact.size_bytes() != size
            || Some(download.artifact.media_type().as_str()) != reference.media_type()
            || !self
                .relationship
                .artifact_sensitivities
                .contains(&download.sensitivity)
            || download.expires_at_unix_ms > deadline
        {
            return Err(AdapterError::external_failure(
                "peer output metadata contradicts the exact observation or transfer scope",
            ));
        }
        let peer = self.client.remote_peer();
        let transfer = download.transfer.clone();
        let result = (|| {
            let remote_limit = match self
                .client
                .negotiate_artifact(&download)
                .map_err(|error| AdapterError::external_failure(error.to_string()))?
            {
                ArtifactTransferDecision::Transfer {
                    maximum_chunk_bytes,
                    ..
                } if maximum_chunk_bytes > 0 => maximum_chunk_bytes,
                _ => {
                    return Err(AdapterError::external_failure(
                        "peer output download was refused",
                    ));
                }
            };
            let mut upload = download.clone();
            upload.direction = ArtifactTransferDirection::Upload;
            let (mut offset, local_limit) = match self
                .artifacts
                .negotiate(peer, &upload, self.relationship.maximum_artifact_bytes)
                .map_err(|error| AdapterError::external_failure(error.to_string()))?
            {
                ArtifactTransferDecision::AlreadyPresent => return Ok(()),
                ArtifactTransferDecision::Transfer {
                    next_offset,
                    maximum_chunk_bytes,
                } if maximum_chunk_bytes > 0 => (next_offset, maximum_chunk_bytes),
                _ => {
                    return Err(AdapterError::external_failure(
                        "core peer output publication was refused",
                    ));
                }
            };
            let chunk_limit = remote_limit
                .min(local_limit)
                .min(milkdrift_peer_protocol::MAX_ARTIFACT_CHUNK_BYTES);
            while offset < size {
                if self.lifecycle.load(Ordering::SeqCst) == Lifecycle::Stopped as u8 {
                    return Err(AdapterError::external_failure(
                        "remote adapter shutdown interrupted output transfer",
                    ));
                }
                if self
                    .clock
                    .now_unix_ms()
                    .map_err(|error| AdapterError::external_failure(error.to_string()))?
                    > deadline
                {
                    return Err(AdapterError::external_failure(
                        "peer output transfer deadline elapsed",
                    ));
                }
                // Large outputs can span many bounded HTTP requests. Keep the runtime
                // lease alive and stop transferring if its durable owner refuses renewal.
                reporter.heartbeat()?;
                let chunk = self
                    .client
                    .read_artifact_chunk(&transfer, offset, chunk_limit)
                    .map_err(|error| AdapterError::external_failure(error.to_string()))?;
                if chunk.transfer != transfer || chunk.offset != offset || chunk.bytes.is_empty() {
                    return Err(AdapterError::external_failure(
                        "peer output transfer made no exact progress",
                    ));
                }
                let next = offset
                    .checked_add(chunk.bytes.len() as u64)
                    .filter(|next| *next <= size)
                    .ok_or_else(|| {
                        AdapterError::external_failure("peer output chunk exceeds its exact size")
                    })?;
                if chunk.final_chunk != (next == size) {
                    return Err(AdapterError::external_failure(
                        "peer output final marker contradicts its size",
                    ));
                }
                let decision = self
                    .artifacts
                    .write_chunk(peer, &chunk, chunk_limit)
                    .map_err(|error| AdapterError::external_failure(error.to_string()))?;
                match decision {
                    ArtifactTransferDecision::AlreadyPresent if next == size => (),
                    ArtifactTransferDecision::Transfer { next_offset, .. }
                        if next_offset == next && next < size => {}
                    _ => {
                        return Err(AdapterError::external_failure(
                            "peer output publication did not accept the exact chunk",
                        ));
                    }
                }
                offset = next;
            }
            Ok(())
        })();
        // Both owners bound staging independently. Every exit releases transport state;
        // aborting a completed transfer does not remove its durable core artifact.
        let _ = self.client.abort_artifact(&transfer);
        let _ = self.artifacts.abort(peer, &transfer);
        result?;
        imported.insert(reference.identity().to_owned());
        *total_bytes = next_total;
        Ok(())
    }
}
