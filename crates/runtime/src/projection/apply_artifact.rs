use milkdrift_persistence::{RunEventEnvelope, RunEventKind};

use crate::RuntimeError;

use super::run::RunProjection;

impl RunProjection {
    pub(super) fn apply_artifact_kind(
        &mut self,
        event: &RunEventEnvelope,
    ) -> Result<(), RuntimeError> {
        let _sequence = event.sequence();
        match event.kind() {
            RunEventKind::ArtifactPublished { metadata } => {
                self.apply_artifact_publication(metadata, event)?;
            }
            _ => unreachable!("central projection dispatch owns artifact publication routing"),
        }
        Ok(())
    }
}
