//! Quarantined user-space threading, bounded channels, and monotonic time.

#![forbid(unsafe_code)]

mod output;
mod text_output;
mod token_output;

pub use output::{
    OutputConsumer, OutputInitializationError, OutputProducer, OutputPullError, OutputPushError,
    output_accumulator,
};
pub use text_output::{
    TextOutputBatch, TextOutputConsumer, TextOutputCursor, TextOutputInitializationError,
    TextOutputProducer, TextOutputRecord, TextOutputRecordKind, TextRange, text_output_accumulator,
};
pub use token_output::{
    TokenOutputBatch, TokenOutputConsumer, TokenOutputCursor, TokenOutputInitializationError,
    TokenOutputProducer, TokenOutputRecord, TokenOutputRecordKind, TokenRange,
    token_output_accumulator,
};

use std::error::Error;
use std::fmt::{self, Display, Formatter};
use std::io;
use std::num::NonZeroUsize;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use domain_contracts::MonotonicMillis;

/// Validated non-zero capacity for one bounded host channel.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ChannelCapacity(NonZeroUsize);

impl ChannelCapacity {
    /// Creates a validated channel capacity.
    #[must_use]
    pub const fn new(capacity: NonZeroUsize) -> Self {
        Self(capacity)
    }

    /// Returns the number of messages the channel can retain.
    #[must_use]
    pub const fn get(self) -> usize {
        self.0.get()
    }
}

/// Non-blocking bounded-send failure that retains ownership of the message.
#[derive(Debug)]
pub enum TrySendError<T> {
    /// The bounded channel has no remaining capacity.
    Full(T),
    /// The receiving side has been dropped.
    Disconnected(T),
}

/// Bounded-send failure after waiting for a configured duration.
#[derive(Debug)]
pub enum SendTimeoutError<T> {
    /// Capacity did not become available before the timeout.
    Timeout(T),
    /// The receiving side has been dropped.
    Disconnected(T),
}

/// Non-blocking receive failure.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TryReceiveError {
    /// The channel currently contains no message.
    Empty,
    /// Every sender has been dropped and no messages remain.
    Disconnected,
}

/// Receive failure after waiting for a configured duration.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReceiveTimeoutError {
    /// No message arrived before the timeout.
    Timeout,
    /// Every sender has been dropped and no messages remain.
    Disconnected,
}

/// Sending side of a bounded MPMC host channel.
pub struct BoundedSender<T> {
    inner: flume::Sender<T>,
}

impl<T> Clone for BoundedSender<T> {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
        }
    }
}

impl<T> BoundedSender<T> {
    /// Attempts to send without blocking.
    ///
    /// # Errors
    ///
    /// Returns [`TrySendError::Full`] when the bounded channel has no remaining
    /// capacity or [`TrySendError::Disconnected`] when the receiver was dropped.
    /// In either case, the error returns ownership of the unsent message.
    pub fn try_send(&self, message: T) -> Result<(), TrySendError<T>> {
        self.inner.try_send(message).map_err(|error| match error {
            flume::TrySendError::Full(message) => TrySendError::Full(message),
            flume::TrySendError::Disconnected(message) => TrySendError::Disconnected(message),
        })
    }

    /// Waits up to `timeout` for bounded capacity to become available.
    ///
    /// # Errors
    ///
    /// Returns [`SendTimeoutError::Timeout`] when capacity does not become
    /// available before `timeout`, or [`SendTimeoutError::Disconnected`] when the
    /// receiver is dropped. In either case, the error returns ownership of the
    /// unsent message.
    pub fn send_timeout(&self, message: T, timeout: Duration) -> Result<(), SendTimeoutError<T>> {
        self.inner
            .send_timeout(message, timeout)
            .map_err(|error| match error {
                flume::SendTimeoutError::Timeout(message) => SendTimeoutError::Timeout(message),
                flume::SendTimeoutError::Disconnected(message) => {
                    SendTimeoutError::Disconnected(message)
                }
            })
    }

    /// Returns whether the receiving side has been dropped.
    #[must_use]
    pub fn is_disconnected(&self) -> bool {
        self.inner.is_disconnected()
    }

    /// Returns the current number of queued messages.
    #[must_use]
    pub fn len(&self) -> usize {
        self.inner.len()
    }

    /// Returns whether no messages are currently queued.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }
}

/// Receiving side of a bounded MPMC host channel.
pub struct BoundedReceiver<T> {
    inner: flume::Receiver<T>,
}

impl<T> BoundedReceiver<T> {
    /// Attempts to receive without blocking.
    ///
    /// # Errors
    ///
    /// Returns [`TryReceiveError::Empty`] when no message is currently available
    /// or [`TryReceiveError::Disconnected`] when all senders have been dropped and
    /// no messages remain.
    pub fn try_receive(&self) -> Result<T, TryReceiveError> {
        self.inner.try_recv().map_err(|error| match error {
            flume::TryRecvError::Empty => TryReceiveError::Empty,
            flume::TryRecvError::Disconnected => TryReceiveError::Disconnected,
        })
    }

    /// Waits up to `timeout` for the next message.
    ///
    /// # Errors
    ///
    /// Returns [`ReceiveTimeoutError::Timeout`] when no message arrives before
    /// `timeout`, or [`ReceiveTimeoutError::Disconnected`] when all senders have
    /// been dropped and no messages remain.
    pub fn receive_timeout(&self, timeout: Duration) -> Result<T, ReceiveTimeoutError> {
        self.inner
            .recv_timeout(timeout)
            .map_err(|error| match error {
                flume::RecvTimeoutError::Timeout => ReceiveTimeoutError::Timeout,
                flume::RecvTimeoutError::Disconnected => ReceiveTimeoutError::Disconnected,
            })
    }

    /// Returns whether every sender has been dropped.
    #[must_use]
    pub fn is_disconnected(&self) -> bool {
        self.inner.is_disconnected()
    }

    /// Returns the current number of queued messages.
    #[must_use]
    pub fn len(&self) -> usize {
        self.inner.len()
    }

    /// Returns whether no messages are currently queued.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }
}

/// Creates one bounded MPMC channel.
#[must_use]
pub fn bounded<T>(capacity: ChannelCapacity) -> (BoundedSender<T>, BoundedReceiver<T>) {
    let (sender, receiver) = flume::bounded(capacity.get());
    (
        BoundedSender { inner: sender },
        BoundedReceiver { inner: receiver },
    )
}

/// Monotonic process-local clock used for lifecycle deadlines.
#[derive(Clone, Debug)]
pub struct MonotonicClock {
    origin: Instant,
}

impl Default for MonotonicClock {
    fn default() -> Self {
        Self::new()
    }
}

impl MonotonicClock {
    /// Starts a clock at a process-local monotonic origin.
    #[must_use]
    pub fn new() -> Self {
        Self {
            origin: Instant::now(),
        }
    }

    /// Returns elapsed milliseconds from this clock's origin.
    #[must_use]
    pub fn now(&self) -> MonotonicMillis {
        let elapsed = self.origin.elapsed().as_millis();
        let milliseconds = u64::try_from(elapsed).unwrap_or(u64::MAX);
        MonotonicMillis::new(milliseconds)
    }
}

/// Failure to create one named host thread.
#[derive(Debug)]
pub struct ThreadSpawnError(io::Error);

impl Display for ThreadSpawnError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        write!(formatter, "failed to spawn host thread: {}", self.0)
    }
}

impl Error for ThreadSpawnError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        Some(&self.0)
    }
}

/// Host thread terminated by unwinding.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ThreadPanicked;

impl Display for ThreadPanicked {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        formatter.write_str("host thread panicked")
    }
}

impl Error for ThreadPanicked {}

/// Join handle whose panic payload never crosses the adapter boundary.
pub struct HostThread<T> {
    inner: JoinHandle<T>,
}

impl<T> HostThread<T> {
    /// Reports whether the host thread has completed without blocking.
    #[must_use]
    pub fn is_finished(&self) -> bool {
        self.inner.is_finished()
    }

    /// Waits for thread completion.
    ///
    /// # Errors
    ///
    /// Returns [`ThreadPanicked`] when the host thread terminates by unwinding.
    pub fn join(self) -> Result<T, ThreadPanicked> {
        self.inner.join().map_err(|_| ThreadPanicked)
    }
}

/// Spawns one named user-space thread.
///
/// # Errors
///
/// Returns [`ThreadSpawnError`] when the operating system cannot create the
/// thread.
pub fn spawn_named<T, F>(name: &str, task: F) -> Result<HostThread<T>, ThreadSpawnError>
where
    T: Send + 'static,
    F: FnOnce() -> T + Send + 'static,
{
    let handle = thread::Builder::new()
        .name(name.to_owned())
        .spawn(task)
        .map_err(ThreadSpawnError)?;
    Ok(HostThread { inner: handle })
}

/// Cooperatively yields the current host thread's remaining time slice.
pub fn yield_now() {
    thread::yield_now();
}
