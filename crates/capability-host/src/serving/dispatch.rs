//! Own fixed serving threads while durable storage owns the accepted queue and claims.
//!
//! Notifications only wake workers; they are not execution authority. A worker retains one failed
//! recovery transition and retries it before claiming other work, avoiding a second adapter entry.

use std::{
    panic::{AssertUnwindSafe, catch_unwind},
    sync::{Arc, Condvar, Mutex, Weak},
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

use milkdrift_persistence::{PeerClaimOutcome, WorkerId};

use super::{
    PeerService, PeerUncertainty, PeerWorkerRecovery, PeerWorkerRun, ServingError,
    config::PeerWorkerConfig,
};

#[derive(Default)]
struct SignalState {
    generation: u64,
    stop: bool,
}

#[derive(Default)]
struct DispatchSignal {
    state: Mutex<SignalState>,
    changed: Condvar,
}

impl DispatchSignal {
    fn notify(&self) {
        if let Ok(mut state) = self.state.lock() {
            state.generation = state.generation.wrapping_add(1);
            self.changed.notify_all();
        }
    }

    fn stop(&self) {
        if let Ok(mut state) = self.state.lock() {
            state.stop = true;
            state.generation = state.generation.wrapping_add(1);
            self.changed.notify_all();
        }
    }

    fn wait(&self, observed_generation: &mut u64, timeout: Duration) -> bool {
        let Ok(state) = self.state.lock() else {
            return true;
        };
        if state.stop {
            return true;
        }
        if state.generation != *observed_generation {
            *observed_generation = state.generation;
            return false;
        }
        let Ok((state, _timeout)) = self.changed.wait_timeout(state, timeout) else {
            return true;
        };
        *observed_generation = state.generation;
        state.stop
    }
}

pub(crate) struct PeerDispatchWorkers {
    signal: Arc<DispatchSignal>,
    handles: Vec<JoinHandle<()>>,
    panicked: bool,
}

impl PeerDispatchWorkers {
    pub(crate) fn start(
        service: Weak<PeerService>,
        config: PeerWorkerConfig,
    ) -> Result<Self, ServingError> {
        Self::start_with(service, config, |name, task| {
            thread::Builder::new().name(name).spawn(task)
        })
    }

    fn start_with(
        service: Weak<PeerService>,
        config: PeerWorkerConfig,
        mut spawn_thread: impl FnMut(
            String,
            Box<dyn FnOnce() + Send>,
        ) -> std::io::Result<JoinHandle<()>>,
    ) -> Result<Self, ServingError> {
        let signal = Arc::new(DispatchSignal::default());
        let mut handles = Vec::with_capacity(usize::from(config.threads));
        for index in 0..config.threads {
            let worker = WorkerId::new(format!("peer-worker-{index}"))
                .map_err(|error| ServingError::Configuration(error.to_string()))?;
            let service = service.clone();
            let thread_signal = signal.clone();
            let task =
                Box::new(move || worker_loop(service, thread_signal, worker, config.poll_interval));
            let spawn = spawn_thread(format!("milkdrift-peer-worker-{index}"), task);
            match spawn {
                Ok(handle) => handles.push(handle),
                Err(error) => {
                    signal.stop();
                    let mut panicked = false;
                    for handle in handles {
                        panicked |= handle.join().is_err();
                    }
                    return Err(ServingError::Unavailable(format!(
                        "peer worker owner failed to spawn: {error}; started worker panicked: {panicked}"
                    )));
                }
            }
        }
        Ok(Self {
            signal,
            handles,
            panicked: false,
        })
    }

    pub(crate) fn notify(&self) {
        self.signal.notify();
    }

    pub(crate) fn shutdown(&mut self, timeout: Duration) -> super::PeerWorkerShutdownReport {
        self.signal.stop();
        let started = Instant::now();
        let mut joined = 0_u16;
        while !self.handles.is_empty() && started.elapsed() < timeout {
            let mut index = 0;
            while let Some(handle) = self.handles.get(index) {
                if handle.is_finished() {
                    let handle = self.handles.swap_remove(index);
                    self.panicked |= handle.join().is_err();
                    joined = joined.saturating_add(1);
                } else {
                    index += 1;
                }
            }
            if !self.handles.is_empty() {
                thread::yield_now();
            }
        }
        super::PeerWorkerShutdownReport {
            clean: self.handles.is_empty() && !self.panicked,
            joined,
            retained_workers: u16::try_from(self.handles.len()).unwrap_or(u16::MAX),
        }
    }
}

fn worker_loop(
    weak_service: Weak<PeerService>,
    signal: Arc<DispatchSignal>,
    worker: WorkerId,
    poll_interval: Duration,
) {
    let mut observed_generation = 0;
    let mut pending_recovery: Option<PeerWorkerRecovery> = None;
    loop {
        let Some(service) = weak_service.upgrade() else {
            return;
        };
        if let Some(recovery) = pending_recovery.as_mut() {
            if service.recover_worker(recovery, &worker).is_ok() {
                pending_recovery = None;
            } else {
                drop(service);
                if signal.wait(&mut observed_generation, poll_interval) {
                    if let Some(service) = weak_service.upgrade()
                        && let Some(recovery) = pending_recovery.as_mut()
                        && let Err(error) = service.recover_worker(recovery, &worker)
                    {
                        tracing::error!(%error, "peer worker stopped with durable recovery still incomplete");
                    }
                    return;
                }
                continue;
            }
        }
        if service.worker_claims_enabled() {
            match service.claim_for_worker(&worker) {
                Ok(PeerClaimOutcome::Claimed(record))
                | Ok(PeerClaimOutcome::CancellationRequested(record)) => {
                    let recovery_record = record.clone();
                    let result = catch_unwind(AssertUnwindSafe(|| service.run_claimed(record)));
                    match result {
                        Ok(PeerWorkerRun::Settled) => {}
                        Ok(PeerWorkerRun::Recover(recovery)) => {
                            pending_recovery = Some(recovery);
                        }
                        Err(_) => {
                            pending_recovery = Some(PeerWorkerRecovery::inspect(
                                recovery_record,
                                PeerUncertainty::WorkerPanicked,
                            ));
                        }
                    }
                    continue;
                }
                Ok(PeerClaimOutcome::Empty) => {}
                Err(_error) => {
                    // Persistence remains authoritative; a bounded poll retries without spinning.
                }
            }
        }
        drop(service);
        if signal.wait(&mut observed_generation, poll_interval) {
            return;
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;

    #[test]
    #[expect(
        clippy::panic,
        reason = "Injects an abnormal worker exit to verify shutdown never reports it as clean, including repeat calls."
    )]
    fn panicked_worker_cannot_be_reported_as_clean_on_later_shutdown() {
        let handle = thread::spawn(|| panic!("injected peer worker panic"));
        let mut workers = PeerDispatchWorkers {
            signal: Arc::new(DispatchSignal::default()),
            handles: vec![handle],
            panicked: false,
        };
        let report = workers.shutdown(Duration::MAX);
        assert_eq!(report.joined, 1);
        assert_eq!(report.retained_workers, 0);
        assert!(!report.clean);
        assert!(!workers.shutdown(Duration::ZERO).clean);
    }

    #[test]
    fn partial_spawn_failure_stops_and_joins_started_workers() {
        let calls = Arc::new(AtomicUsize::new(0));
        let finished = Arc::new(AtomicUsize::new(0));
        let result = PeerDispatchWorkers::start_with(
            Weak::new(),
            PeerWorkerConfig {
                threads: 2,
                maximum_global_active: 2,
                maximum_dispatch_queue: 2,
                maximum_hot_terminal_records: 2,
                archive_batch_size: 1,
                observation_hot_retention: Duration::from_millis(1),
                recovery_page: 2,
                poll_interval: Duration::from_millis(1),
            },
            {
                let finished = finished.clone();
                move |name, task| {
                    if calls.fetch_add(1, Ordering::SeqCst) == 1 {
                        return Err(std::io::Error::other("deterministic spawn failure"));
                    }
                    let finished = finished.clone();
                    thread::Builder::new().name(name).spawn(move || {
                        task();
                        finished.fetch_add(1, Ordering::SeqCst);
                    })
                }
            },
        );
        assert!(matches!(result, Err(ServingError::Unavailable(_))));
        assert_eq!(finished.load(Ordering::SeqCst), 1);
    }
}
