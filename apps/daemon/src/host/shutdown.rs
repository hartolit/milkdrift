//! Ordered peer/effect drain while the durable owner still serves final calls.
use super::{
    DaemonHost, HostError, Owner, PublicFailure, health::Lifecycle, health::SharedHealth,
    read_model::bounded,
};
use crate::config::ShutdownEffectPolicy;
use milkdrift_capability_host::{EffectShutdownMode, EffectWorkerHost};
use milkdrift_control_protocol::ErrorCode;
use milkdrift_peer_http::PeerWorkerShutdownReport;
use std::{sync::atomic::Ordering, time::Duration};
use tracing::{info, warn};

impl DaemonHost {
    /// Closes durable admission on the owner before graceful HTTP shutdown begins.
    pub(crate) async fn begin_draining(&self) -> Result<(), HostError> {
        self.mutating_admission.store(false, Ordering::SeqCst);
        let durable = self
            .dispatch(false, |owner| owner.begin_peer_drain())
            .await
            .map_err(|error| HostError::Shutdown(error.message));
        self.health.set_lifecycle(Lifecycle::Draining);
        let registries = self.peer_registries.values().cloned().collect::<Vec<_>>();
        tokio::task::spawn_blocking(move || {
            for registry in registries {
                let _ = registry.disconnect();
            }
        })
        .await
        .map_err(|_| HostError::Shutdown("peer disconnect task failed".to_owned()))?;
        durable
    }

    /// Runs ordered shutdown and joins the owner thread.
    pub async fn shutdown(&self) -> Result<(), HostError> {
        let shutdown_started = std::time::Instant::now();
        let drain_error = self.begin_draining().await.err();
        let peer_shutdown = if let Some(service) = &self.peer_service {
            let service = service.clone();
            let deadline = self
                .shutdown_deadline
                .saturating_sub(shutdown_started.elapsed());
            Some(
                match tokio::task::spawn_blocking(move || service.shutdown_workers(deadline)).await
                {
                    Ok(report) => report,
                    Err(_) => PeerWorkerShutdownReport {
                        clean: false,
                        joined: 0,
                        retained_workers: 1,
                    },
                },
            )
        } else {
            None
        };
        let effect_shutdown = match self
            .dispatch_draining(|owner| owner.take_effect_workers_for_shutdown())
            .await
        {
            Ok((workers, mode)) => {
                let deadline = self
                    .shutdown_deadline
                    .saturating_sub(shutdown_started.elapsed());
                let health = self.health.clone();
                match tokio::task::spawn_blocking(move || {
                    shutdown_effect_workers(&workers, mode, deadline, &health)
                })
                .await
                {
                    Ok(outcome) => outcome,
                    Err(_) => EffectShutdownOutcome::failed(),
                }
            }
            Err(error) => {
                warn!(
                    phase = "draining",
                    code = "effect_worker_take",
                    "{}",
                    bounded(&error.message)
                );
                self.health.failure("effect worker shutdown failed");
                EffectShutdownOutcome::failed()
            }
        };
        let health = self.health.clone();
        let result = match self
            .dispatch(true, move |owner| {
                owner.shutdown(&health, peer_shutdown, effect_shutdown)
            })
            .await
        {
            Ok(result) => result,
            Err(error) => {
                self.health.set_lifecycle(Lifecycle::Failed);
                return Err(HostError::Shutdown(error.message));
            }
        };
        let join = self
            .join
            .lock()
            .map_err(|_| HostError::Shutdown("owner join state is unavailable".to_owned()))?
            .take();
        if let Some(join) = join {
            tokio::task::spawn_blocking(move || join.join())
                .await
                .map_err(|_| HostError::Shutdown("owner join task failed".to_owned()))?
                .map_err(|_| HostError::Shutdown("runtime owner panicked".to_owned()))?;
        }
        match result {
            ShutdownOutcome {
                clean: true,
                unresolved: 0,
            } => match drain_error {
                Some(error) => Err(error),
                None => Ok(()),
            },
            ShutdownOutcome { clean, unresolved } => {
                self.health.set_lifecycle(Lifecycle::Failed);
                Err(HostError::Shutdown(format!(
                    "shutdown retained or could not resolve {unresolved} invocation(s); clean={clean}"
                )))
            }
        }
    }
}

pub(super) struct ShutdownOutcome {
    clean: bool,
    unresolved: u32,
}

pub(super) struct EffectShutdownOutcome {
    clean: bool,
    unresolved_invocations: usize,
    outstanding_effects: usize,
}

impl EffectShutdownOutcome {
    pub(super) const fn failed() -> Self {
        Self {
            clean: false,
            unresolved_invocations: 1,
            outstanding_effects: 1,
        }
    }
}

pub(super) fn shutdown_effect_workers(
    workers: &EffectWorkerHost,
    mode: EffectShutdownMode,
    deadline: Duration,
    health: &SharedHealth,
) -> EffectShutdownOutcome {
    match workers.shutdown(mode, deadline) {
        Ok(result) => {
            let execution_work = result
                .health
                .queued_executions
                .saturating_add(result.health.active_executions);
            let cancellation_work = result
                .health
                .queued_cancellations
                .saturating_add(result.health.active_cancellations);
            EffectShutdownOutcome {
                clean: result.clean,
                unresolved_invocations: result
                    .unresolved_invocations
                    .len()
                    .max(execution_work)
                    .saturating_add(cancellation_work),
                outstanding_effects: execution_work.saturating_add(cancellation_work),
            }
        }
        Err(error) => {
            health.failure("effect worker shutdown failed");
            warn!(
                phase = "draining",
                code = "effect_worker_shutdown",
                "{}",
                bounded(&error.to_string())
            );
            let outstanding = workers.health().map_or(1, |value| {
                value
                    .queued_executions
                    .saturating_add(value.active_executions)
                    .saturating_add(value.queued_cancellations)
                    .saturating_add(value.active_cancellations)
                    .max(1)
            });
            EffectShutdownOutcome {
                clean: false,
                unresolved_invocations: outstanding,
                outstanding_effects: outstanding,
            }
        }
    }
}

impl Owner {
    pub(super) fn take_effect_workers_for_shutdown(
        &mut self,
    ) -> Result<(EffectWorkerHost, EffectShutdownMode), PublicFailure> {
        self.runtime.begin_shutdown();
        let mode = match self.shutdown.effect_policy {
            ShutdownEffectPolicy::Drain => EffectShutdownMode::Drain,
            ShutdownEffectPolicy::Cancel => EffectShutdownMode::Cancel,
            ShutdownEffectPolicy::Retain => EffectShutdownMode::Retain,
        };
        self.effect_workers
            .take()
            .map(|workers| (workers, mode))
            .ok_or_else(|| {
                PublicFailure::new(
                    ErrorCode::Unavailable,
                    "effect worker owner is unavailable",
                    true,
                )
            })
    }

    pub(super) fn shutdown(
        &mut self,
        health: &SharedHealth,
        peer_shutdown: Option<PeerWorkerShutdownReport>,
        mut effect_shutdown: EffectShutdownOutcome,
    ) -> Result<ShutdownOutcome, PublicFailure> {
        info!(phase = "draining", "runtime owner closing admission");
        health.set_lifecycle(Lifecycle::Draining);
        self.runtime.begin_shutdown();
        if self.effect_workers.take().is_some() {
            health.failure("effect worker owner was not transferred before shutdown");
            effect_shutdown.clean = false;
            effect_shutdown.unresolved_invocations =
                effect_shutdown.unresolved_invocations.saturating_add(1);
            effect_shutdown.outstanding_effects =
                effect_shutdown.outstanding_effects.saturating_add(1);
        }
        let peer_retained = peer_shutdown.map_or(0, |report| report.retained_workers);
        let clean = effect_shutdown.clean && peer_retained == 0 && !self.request_panicked;
        health.set_active_effects(if clean {
            0
        } else {
            u32::try_from(effect_shutdown.outstanding_effects).unwrap_or(u32::MAX)
        });
        health.set_lifecycle(if clean {
            Lifecycle::Stopped
        } else {
            Lifecycle::Failed
        });
        let unresolved = effect_shutdown
            .unresolved_invocations
            .saturating_add(usize::from(peer_retained));
        info!(
            phase = if clean { "stopped" } else { "failed" },
            clean, unresolved, "runtime owner shutdown completed"
        );
        Ok(ShutdownOutcome {
            clean,
            unresolved: u32::try_from(unresolved).unwrap_or(u32::MAX),
        })
    }
}
