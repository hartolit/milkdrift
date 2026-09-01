//! Peer admission drain, shutdown, recovery, revocation, and retention lifecycle.

use std::{
    sync::{Arc, atomic::Ordering},
    time::Duration,
};

use milkdrift_authority::PeerId;
use milkdrift_persistence::{
    PageSize, PeerExecutionStatus, PeerRelationshipState, PeerRetentionRequest, TimestampMillis,
};

use super::{
    PeerHttpError, PeerService, PeerWorkerShutdownReport, map_execution_persistence,
    relationship_generation,
};

impl PeerService {
    /// Marks all catalogs stale and stops accepting new peer invocations.
    pub fn begin_drain(&self) -> Result<(), PeerHttpError> {
        self.executions
            .set_peer_admission_open(false)
            .map_err(map_execution_persistence)?;
        self.drain.store(1, Ordering::SeqCst);
        self.notify_workers();
        Ok(())
    }

    /// Marks shutdown state for handshake and catalog consumers.
    pub fn begin_shutdown(&self) -> Result<(), PeerHttpError> {
        let closed = self
            .executions
            .set_peer_admission_open(false)
            .map_err(map_execution_persistence);
        self.drain.store(2, Ordering::SeqCst);
        self.notify_workers();
        closed
    }

    /// Stops durable claims and joins the fixed worker owner up to the supplied deadline.
    pub fn shutdown_workers(&self, timeout: Duration) -> PeerWorkerShutdownReport {
        let admission_closed = self.begin_shutdown().is_ok();
        let Ok(mut workers) = self.workers.lock() else {
            return PeerWorkerShutdownReport {
                clean: false,
                joined: 0,
                retained_workers: self.config.workers.threads,
            };
        };
        let mut report = workers.as_mut().map_or(
            PeerWorkerShutdownReport {
                clean: true,
                joined: 0,
                retained_workers: 0,
            },
            |owner| owner.shutdown(timeout),
        );
        report.clean &= admission_closed;
        report
    }

    /// Revokes one relationship immediately for inbound authentication and protocol actions.
    pub fn revoke_peer(&self, peer: &PeerId) -> Result<(), PeerHttpError> {
        let Some(relationship) = self.relationships.get(peer) else {
            return Err(PeerHttpError::NotFound(
                "peer relationship is not configured".to_owned(),
            ));
        };
        self.executions
            .configure_peer_relationship(&PeerRelationshipState {
                peer: peer.clone(),
                generation: relationship_generation(relationship).saturating_add(1),
                enabled: false,
                expires_at_unix_ms: relationship.expires_at_unix_ms,
                maximum_active: u32::from(relationship.maximum_concurrent),
            })
            .map_err(map_execution_persistence)?;
        self.revoked_peers
            .lock()
            .map_err(|_| {
                PeerHttpError::Unavailable("peer revocation state unavailable".to_owned())
            })?
            .insert(peer.clone());
        self.catalogs
            .lock()
            .map_err(|_| PeerHttpError::Unavailable("catalog cache unavailable".to_owned()))?
            .remove(peer);
        Ok(())
    }

    /// Recovers bounded prior-owner claims. Pre-entry work requeues; entered work becomes uncertain.
    pub fn recover(self: &Arc<Self>, maximum: usize) -> Result<(), PeerHttpError> {
        let configured = usize::from(self.config.workers.recovery_page);
        let bounded = maximum.min(configured).max(1);
        let limit = PageSize::new(u32::try_from(bounded).unwrap_or(u32::MAX))
            .map_err(|error| PeerHttpError::Protocol(error.to_string()))?;
        loop {
            let recovered = self
                .executions
                .recover_peer_claims(self.now()?, limit)
                .map_err(map_execution_persistence)?;
            if !recovered.more {
                break;
            }
        }
        self.executions
            .verify_peer_execution_integrity()
            .map_err(map_execution_persistence)?;
        self.maintain_retention()?;
        self.executions
            .verify_peer_execution_integrity()
            .map_err(map_execution_persistence)?;
        self.executions
            .set_peer_admission_open(true)
            .map_err(map_execution_persistence)?;
        self.drain.store(0, Ordering::SeqCst);
        self.notify_workers();
        Ok(())
    }

    /// Compacts one bounded page beyond the configured hot observation horizon.
    pub fn maintain_retention(&self) -> Result<PeerExecutionStatus, PeerHttpError> {
        let now = self.now()?;
        let retention_ms = u64::try_from(self.config.workers.observation_hot_retention.as_millis())
            .unwrap_or(u64::MAX);
        self.executions
            .archive_peer_executions(&PeerRetentionRequest {
                terminal_before_or_at: TimestampMillis::new(
                    now.saturating_sub(retention_ms).max(1),
                ),
                archived_at: TimestampMillis::new(now),
                limit: PageSize::new(self.config.workers.archive_batch_size)
                    .map_err(|error| PeerHttpError::Protocol(error.to_string()))?,
            })
            .map_err(map_execution_persistence)?;
        self.executions
            .peer_execution_status()
            .map_err(map_execution_persistence)
    }

    /// Returns redacted serving execution accounting for daemon health projection.
    pub fn execution_status(&self) -> Result<PeerExecutionStatus, PeerHttpError> {
        self.executions
            .peer_execution_status()
            .map_err(map_execution_persistence)
    }
}
