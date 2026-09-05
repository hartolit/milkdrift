use super::support::{
    AtomicUsize, Condvar, Duration, FaultInjector, FaultPoint, Mutex, Ordering, PeerClock,
    PeerClockError, injected_failure,
};

#[derive(Debug)]
struct ControlledClockState {
    observed_at_unix_ms: u64,
    last_unix_ms: u64,
    available: bool,
    unavailable_observations: u64,
}

#[derive(Debug)]
pub(super) struct ControlledPeerClock {
    state: Mutex<ControlledClockState>,
    changed: Condvar,
}

impl ControlledPeerClock {
    pub(super) fn new(observed_at_unix_ms: u64) -> Self {
        Self {
            state: Mutex::new(ControlledClockState {
                observed_at_unix_ms,
                last_unix_ms: 0,
                available: true,
                unavailable_observations: 0,
            }),
            changed: Condvar::new(),
        }
    }

    pub(super) fn set(&self, observed_at_unix_ms: u64) -> Result<(), PeerClockError> {
        self.state
            .lock()
            .map_err(|_| PeerClockError::Unavailable)?
            .observed_at_unix_ms = observed_at_unix_ms;
        Ok(())
    }

    pub(super) fn set_available(&self, available: bool) -> Result<(), PeerClockError> {
        self.state
            .lock()
            .map_err(|_| PeerClockError::Unavailable)?
            .available = available;
        self.changed.notify_all();
        Ok(())
    }

    pub(super) fn wait_for_unavailable_observations(
        &self,
        minimum: u64,
        timeout: Duration,
    ) -> Result<bool, PeerClockError> {
        let state = self.state.lock().map_err(|_| PeerClockError::Unavailable)?;
        let (state, timed) = self
            .changed
            .wait_timeout_while(state, timeout, |state| {
                state.unavailable_observations < minimum
            })
            .map_err(|_| PeerClockError::Unavailable)?;
        Ok(state.unavailable_observations >= minimum && !timed.timed_out())
    }
}

impl PeerClock for ControlledPeerClock {
    fn now_unix_ms(&self) -> Result<u64, PeerClockError> {
        let mut state = self.state.lock().map_err(|_| PeerClockError::Unavailable)?;
        if !state.available {
            state.unavailable_observations = state.unavailable_observations.saturating_add(1);
            self.changed.notify_all();
            return Err(PeerClockError::Unavailable);
        }
        if state.observed_at_unix_ms < state.last_unix_ms {
            return Err(PeerClockError::MovedBackwards);
        }
        state.last_unix_ms = state.observed_at_unix_ms;
        Ok(state.observed_at_unix_ms)
    }
}

pub(super) struct FailOnce {
    point: FaultPoint,
    remaining: AtomicUsize,
}

impl FailOnce {
    pub(super) fn new(point: FaultPoint) -> Self {
        Self {
            point,
            remaining: AtomicUsize::new(1),
        }
    }

    pub(super) fn triggered(&self) -> bool {
        self.remaining.load(Ordering::SeqCst) == 0
    }
}

impl FaultInjector for FailOnce {
    fn check(&self, point: FaultPoint) -> Result<(), milkdrift_persistence::PersistenceError> {
        if point == self.point && self.remaining.swap(0, Ordering::SeqCst) == 1 {
            Err(injected_failure(point))
        } else {
            Ok(())
        }
    }
}
