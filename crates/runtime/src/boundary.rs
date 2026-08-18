use std::{
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

use milkdrift_persistence::TimestampMillis;

use crate::RuntimeError;

/// Boundary supplying timestamp facts; replay never calls it.
pub trait BoundaryClock: Send + Sync {
    /// Returns the current epoch-millisecond observation.
    fn now(&self) -> Result<TimestampMillis, RuntimeError>;
}

/// Production wall-clock boundary.
#[derive(Clone, Copy, Debug, Default)]
pub struct SystemBoundaryClock;

impl BoundaryClock for SystemBoundaryClock {
    fn now(&self) -> Result<TimestampMillis, RuntimeError> {
        let elapsed = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|error| {
                RuntimeError::InvalidTransition(format!(
                    "system clock precedes Unix epoch: {error}"
                ))
            })?;
        let millis = u64::try_from(elapsed.as_millis()).map_err(|_| {
            RuntimeError::InvalidTransition("system clock milliseconds exceed u64".to_owned())
        })?;
        Ok(TimestampMillis::new(millis))
    }
}

/// Deterministic mutable clock for scheduler/recovery tests.
#[derive(Debug)]
pub struct ManualClock(AtomicU64);

impl ManualClock {
    /// Starts at an exact recorded timestamp.
    #[must_use]
    pub const fn new(initial_millis: u64) -> Self {
        Self(AtomicU64::new(initial_millis))
    }

    /// Advances by a checked duration and returns the resulting fact.
    pub fn advance(&self, millis: u64) -> Result<TimestampMillis, RuntimeError> {
        let mut current = self.0.load(Ordering::SeqCst);
        loop {
            let next = current.checked_add(millis).ok_or_else(|| {
                RuntimeError::InvalidTransition("manual clock overflow".to_owned())
            })?;
            match self
                .0
                .compare_exchange(current, next, Ordering::SeqCst, Ordering::SeqCst)
            {
                Ok(_) => return Ok(TimestampMillis::new(next)),
                Err(observed) => current = observed,
            }
        }
    }

    /// Sets an exact fact, permitting deterministic expiry scenarios.
    pub fn set(&self, millis: u64) {
        self.0.store(millis, Ordering::SeqCst);
    }
}

impl BoundaryClock for ManualClock {
    fn now(&self) -> Result<TimestampMillis, RuntimeError> {
        Ok(TimestampMillis::new(self.0.load(Ordering::SeqCst)))
    }
}

/// Boundary for stable IDs recorded in durable facts.
pub trait IdGenerator: Send + Sync {
    /// Produces one bounded safe-ASCII identity under a stable semantic kind.
    fn next(&self, kind: &'static str) -> Result<String, RuntimeError>;
}

/// Deterministic monotonic ID generator scoped by an externally stable instance prefix.
#[derive(Debug)]
pub struct SequentialIdGenerator {
    prefix: String,
    next: AtomicU64,
}

impl SequentialIdGenerator {
    /// Creates a generator. Prefix validation prevents path/control text from entering IDs.
    pub fn new(prefix: impl Into<String>, first: u64) -> Result<Self, RuntimeError> {
        let prefix = prefix.into();
        if prefix.is_empty()
            || prefix.len() > 64
            || !prefix.is_ascii()
            || !prefix.as_bytes()[0].is_ascii_alphanumeric()
            || !prefix
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
        {
            return Err(RuntimeError::InvalidCommand(
                "ID generator prefix must be 1..=64 safe ASCII bytes".to_owned(),
            ));
        }
        Ok(Self {
            prefix,
            next: AtomicU64::new(first),
        })
    }
}

impl IdGenerator for SequentialIdGenerator {
    fn next(&self, kind: &'static str) -> Result<String, RuntimeError> {
        if kind.is_empty()
            || !kind
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
        {
            return Err(RuntimeError::InvalidCommand(
                "ID kind must be safe ASCII identity text".to_owned(),
            ));
        }
        let value = self
            .next
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |current| {
                current.checked_add(1)
            })
            .map_err(|_| RuntimeError::InvalidTransition("ID counter overflow".to_owned()))?;
        Ok(format!("{}-{kind}-{value}", self.prefix))
    }
}
