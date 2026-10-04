use milkdrift_persistence::{RunEventEnvelope, RunEventKind};

use crate::RuntimeError;

use super::run::RunProjection;

impl RunProjection {
    pub(super) fn apply_artifact_kind(
        &mut self,
        event: &RunEventEnvelope,
    ) -> Result<(), RuntimeError> {
        let _sequence = event.sequence();
        #[expect(
            clippy::wildcard_enum_match_arm,
            reason = "The exhaustive apply_kind dispatcher selects this event family; other variants indicate an internal routing bug."
        )]
        match event.kind() {
            RunEventKind::ArtifactPublished { metadata } => {
                self.apply_artifact_publication(metadata, event)?;
            }
            _ => unreachable!("central projection dispatch owns artifact publication routing"),
        }
        Ok(())
    }
}
