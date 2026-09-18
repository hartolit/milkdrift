//! Peer admission drain, shutdown, recovery, revocation, and retention lifecycle.

use std::{
    sync::{Arc, atomic::Ordering},
    time::Duration,
};

use milkdrift_capability::PeerId;
use milkdrift_persistence::{
    PageSize, PeerExecutionStatus, PeerRecoveryResult, PeerRetentionRequest, ServingCallerState,
    TimestampMillis,
};

use super::{
    PeerService, PeerWorkerShutdownReport, ServingError, map_execution_persistence,
    relationship_generation,
};

impl PeerService {
    /// Marks all catalogs stale and stops accepting new peer invocations.
    pub fn begin_drain(&self) -> Result<(), ServingError> {
        self.executions
            .set_peer_admission_open(false)
            .map_err(map_execution_persistence)?;
        self.drain.store(1, Ordering::SeqCst);
        self.notify_workers();
        Ok(())
    }

    /// Marks shutdown state for handshake and catalog consumers.
    pub fn begin_shutdown(&self) -> Result<(), ServingError> {
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
        let started = std::time::Instant::now();
        // Stop local claims immediately. A temporarily full owner queue must not turn an
        // otherwise clean shutdown into a failed durable close; retry inside the same budget.
        let admission_closed = loop {
            match self.begin_shutdown() {
                Ok(()) => break true,
                Err(ServingError::Overloaded(_)) if started.elapsed() < timeout => {
                    std::thread::sleep(Duration::from_millis(1));
                }
                Err(_) => break false,
            }
        };
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
            |owner| owner.shutdown(timeout.saturating_sub(started.elapsed())),
        );
        report.clean &= admission_closed;
        report
    }

    /// Revokes one relationship immediately for inbound authentication and protocol actions.
    pub fn revoke_peer(&self, peer: &PeerId) -> Result<(), ServingError> {
        let Some(relationship) = self.relationships.get(peer) else {
            return Err(ServingError::NotFound(
                "peer relationship is not configured".to_owned(),
            ));
        };
        self.executions
            .configure_peer_relationship(&ServingCallerState {
                caller: self.peer_caller(peer),
                generation: relationship_generation(relationship).saturating_add(1),
                enabled: false,
                expires_at_unix_ms: relationship.expires_at_unix_ms,
                maximum_active: u32::from(relationship.maximum_concurrent),
            })
            .map_err(map_execution_persistence)?;
        self.revoked_peers
            .lock()
            .map_err(|_| ServingError::Unavailable("peer revocation state unavailable".to_owned()))?
            .insert(peer.clone());
        self.catalogs
            .lock()
            .map_err(|_| ServingError::Unavailable("catalog cache unavailable".to_owned()))?
            .remove(peer);
        Ok(())
    }

    /// Recovers bounded prior-owner claims. Pre-entry work requeues; entered work becomes uncertain.
    pub fn recover(self: &Arc<Self>, maximum: usize) -> Result<(), ServingError> {
        let configured = usize::from(self.config.workers.recovery_page);
        let bounded = maximum.min(configured).max(1);
        let limit = PageSize::new(u32::try_from(bounded).unwrap_or(u32::MAX))
            .map_err(|error| ServingError::Protocol(error.to_string()))?;
        recover_claim_pages(|| {
            self.executions
                .recover_peer_claims(self.now()?, limit)
                .map_err(map_execution_persistence)
        })?;
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
    pub fn maintain_retention(&self) -> Result<PeerExecutionStatus, ServingError> {
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
                    .map_err(|error| ServingError::Protocol(error.to_string()))?,
            })
            .map_err(map_execution_persistence)?;
        self.executions
            .peer_execution_status()
            .map_err(map_execution_persistence)
    }

    /// Returns redacted serving execution accounting for daemon health projection.
    pub fn execution_status(&self) -> Result<PeerExecutionStatus, ServingError> {
        self.executions
            .peer_execution_status()
            .map_err(map_execution_persistence)
    }
}

fn recover_claim_pages(
    mut recover_page: impl FnMut() -> Result<PeerRecoveryResult, ServingError>,
) -> Result<(), ServingError> {
    loop {
        let recovered = recover_page()?;
        if !recovered.more {
            return Ok(());
        }
        if recovered.requeued == 0 && recovered.uncertain == 0 {
            return Err(ServingError::Unavailable(
                "peer claim recovery reported more work without progress".to_owned(),
            ));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{PeerRecoveryResult, ServingError, recover_claim_pages};

    #[test]
    fn recovery_refuses_an_empty_continuation_before_requesting_another_page() {
        let mut calls = 0;
        let result = recover_claim_pages(|| {
            calls += 1;
            if calls > 1 {
                return Err(ServingError::Unavailable("unexpected next page".to_owned()));
            }
            Ok(PeerRecoveryResult {
                more: true,
                ..PeerRecoveryResult::default()
            })
        });
        assert_eq!(calls, 1);
        assert!(matches!(result, Err(ServingError::Unavailable(reason))
            if reason == "peer claim recovery reported more work without progress"));
    }

    #[test]
    fn recovery_exhausts_requeued_and_uncertain_pages_and_accepts_an_empty_frontier() {
        let mut pages = [
            PeerRecoveryResult {
                requeued: 1,
                uncertain: 0,
                more: true,
            },
            PeerRecoveryResult {
                requeued: 0,
                uncertain: 1,
                more: true,
            },
            PeerRecoveryResult::default(),
        ]
        .into_iter();
        assert!(
            recover_claim_pages(|| {
                pages
                    .next()
                    .ok_or_else(|| ServingError::Unavailable("past the frontier".to_owned()))
            })
            .is_ok()
        );
        assert!(pages.next().is_none());
        assert!(recover_claim_pages(|| Ok(PeerRecoveryResult::default())).is_ok());
    }
}
