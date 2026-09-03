use std::{
    panic::{AssertUnwindSafe, catch_unwind},
    sync::{Arc, Condvar, Mutex, Weak},
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

use milkdrift_persistence::{PeerClaimOutcome, WorkerId};

use crate::{
    PeerHttpError,
    config::PeerWorkerConfig,
    service::{PeerService, PeerUncertainty, PeerWorkerRecovery, PeerWorkerRun},
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
}

impl PeerDispatchWorkers {
    pub(crate) fn start(
        service: Weak<PeerService>,
        config: PeerWorkerConfig,
    ) -> Result<Self, PeerHttpError> {
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
    ) -> Result<Self, PeerHttpError> {
        let signal = Arc::new(DispatchSignal::default());
        let mut handles = Vec::with_capacity(usize::from(config.threads));
        for index in 0..config.threads {
            let worker = WorkerId::new(format!("peer-worker-{index}"))
                .map_err(|error| PeerHttpError::Configuration(error.to_string()))?;
            let service = service.clone();
            let thread_signal = signal.clone();
            let task =
                Box::new(move || worker_loop(service, thread_signal, worker, config.poll_interval));
            let spawn = spawn_thread(format!("milkdrift-peer-worker-{index}"), task);
            match spawn {
                Ok(handle) => handles.push(handle),
                Err(error) => {
                    signal.stop();
                    for handle in handles {
                        let _ = handle.join();
                    }
                    return Err(PeerHttpError::Unavailable(format!(
                        "peer worker owner failed to spawn: {error}"
                    )));
                }
            }
        }
        Ok(Self { signal, handles })
    }

    pub(crate) fn notify(&self) {
        self.signal.notify();
    }

    pub(crate) fn shutdown(&mut self, timeout: Duration) -> super::PeerWorkerShutdownReport {
        self.signal.stop();
        let deadline = Instant::now() + timeout;
        let mut joined = 0_u16;
        while !self.handles.is_empty() && Instant::now() < deadline {
            let mut index = 0;
            while index < self.handles.len() {
                if self.handles[index].is_finished() {
                    let handle = self.handles.swap_remove(index);
                    let _ = handle.join();
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
            clean: self.handles.is_empty(),
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
                    {
                        let _ = service.recover_worker(recovery, &worker);
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
                let calls = calls.clone();
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
        assert!(matches!(result, Err(PeerHttpError::Unavailable(_))));
        assert_eq!(finished.load(Ordering::SeqCst), 1);
    }
}
