//! Advance eligible work and retire old operational detail between owner requests.
//!
//! Each pass refreshes capability health, maintains receipt/peer retention, ticks the scheduler,
//! and notifies effect workers. Archival preserves replay identity; health failures report the
//! affected maintenance boundary without manufacturing new workflow outcomes.
use super::{Owner, health::SharedHealth, read_model::bounded};
use std::sync::Weak;
use tracing::warn;

impl Owner {
    pub(super) fn maintenance(&self, health: &SharedHealth) {
        if !self.recovery_controls
            && let Err(error) = self.refresh_capability_health()
        {
            warn!(
                outcome = "error",
                code = "capability_health",
                "{}",
                bounded(&error.message)
            );
            health.failure("bounded capability health refresh failed");
        }
        if self.recovery_controls {
            return;
        }
        if let Some(service) = self.peer_service.as_ref().and_then(Weak::upgrade) {
            match service.maintain_retention() {
                Ok(status) => health.peer_status(status),
                Err(error) => {
                    warn!(
                        outcome = "error",
                        code = "peer_execution_archive",
                        "{}",
                        bounded(&error.to_string())
                    );
                    health.peer_failure();
                }
            }
        }
        if let Some(workers) = &self.effect_workers {
            if let Err(error) = workers.poll() {
                warn!(
                    outcome = "error",
                    code = "effect_poll",
                    "{}",
                    bounded(&error.to_string())
                );
            }
            if let Ok(worker_health) = workers.health() {
                let active = worker_health
                    .active_executions
                    .saturating_add(worker_health.active_cancellations);
                health.set_active_effects(u32::try_from(active).unwrap_or(u32::MAX));
            }
        }
    }
}
