//! Receipt-bound output disclosure. Public outputs have their own permission and never broaden
//! the caller's arbitrary artifact scope or expose a service's internal workspace references.
use super::{PeerService, ServingError, map_execution_persistence};
use milkdrift_authority::{ActorRef, AuthorityOperation};
use milkdrift_peer_protocol::{InvocationOutputChunk, PeerExecutionId};
use milkdrift_persistence::{
    ArtifactReadAuthority, ArtifactReadRequest, EvidenceId, PeerExecutionSnapshot,
};

impl PeerService {
    /// Read a bounded range of an artifact listed by this caller's exact accepted operation.
    /// Archival retains terminal references; unfinished output observations use bounded pages.
    pub fn client_output(
        &self,
        actor: &ActorRef,
        execution: &PeerExecutionId,
        artifact: &str,
        offset: u64,
        maximum: u32,
    ) -> Result<InvocationOutputChunk, ServingError> {
        if maximum == 0 || maximum > 65_536 {
            return Err(ServingError::Protocol(
                "output ranges require 1..=65536 bytes".to_owned(),
            ));
        }
        let snapshot = self.client_inspect(actor, execution)?;
        self.require_client_execution(actor, &snapshot, AuthorityOperation::ReadCapabilityOutput)?;
        let terminal = match &snapshot {
            PeerExecutionSnapshot::Hot(record) => match &record.phase {
                milkdrift_persistence::PeerExecutionPhase::Terminal { sequence, .. } => self
                    .executions
                    .peer_observations(
                        &self.client_caller(actor),
                        execution,
                        sequence.saturating_sub(1),
                        milkdrift_persistence::PageSize::new(1)
                            .map_err(map_execution_persistence)?,
                    )
                    .map_err(map_execution_persistence)?
                    .observations
                    .into_iter()
                    .next(),
                _ => None,
            },
            PeerExecutionSnapshot::Archived(record) => {
                record.disposition.terminal_observation().cloned()
            }
        };
        let reference = terminal
            .as_ref()
            .and_then(|item| item.event.kind().terminal())
            .and_then(|terminal| {
                terminal
                    .outputs()
                    .iter()
                    .find(|value| value.identity() == artifact)
            })
            .cloned()
            .ok_or_else(|| {
                ServingError::NotFound(
                    "artifact is not a retained terminal output of this invocation".to_owned(),
                )
            })?;
        let metadata = self.artifacts.metadata(&reference)?;
        let evidence = EvidenceId::new(format!("output:{execution}:{offset}"))
            .map_err(map_execution_persistence)?;
        let chunk = self.artifacts.read_input_chunk(
            &ArtifactReadRequest::new(
                metadata.reference().clone(),
                offset,
                maximum,
                ArtifactReadAuthority::Authorized {
                    actor: actor.clone(),
                    evidence,
                },
            )
            .map_err(map_execution_persistence)?,
        )?;
        Ok(InvocationOutputChunk {
            execution: execution.clone(),
            metadata,
            offset: chunk.offset,
            bytes: chunk.bytes,
            complete: chunk.end_of_artifact,
        })
    }
}
