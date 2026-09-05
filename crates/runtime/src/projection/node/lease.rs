//! Lease ownership, timers, and retry admission state.

use milkdrift_capability::ErrorClass;
use milkdrift_persistence::{
    AttemptId, LeaseId, NodeExecutionId, Reason, RecoveryClassification, RunSequence, TimerId,
    TimestampMillis, WorkerId,
};
use serde::{Deserialize, Serialize};

/// State of a durable lease.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub enum LeaseState {
    /// Lease remains the active ownership fact.
    Active,
    /// Lease expired with the recorded recovery classification.
    Expired(RecoveryClassification),
    /// An expired lease was superseded by a new lease for the same attempt.
    Superseded(LeaseId),
    /// The attempt reached a terminal or evidence-resolved boundary.
    Completed,
}

/// Read model for one immutable worker lease.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub struct LeaseProjection {
    pub(in crate::projection) lease: LeaseId,
    pub(in crate::projection) execution: NodeExecutionId,
    pub(in crate::projection) attempt: AttemptId,
    pub(in crate::projection) worker: WorkerId,
    pub(in crate::projection) expires_at: TimestampMillis,
    pub(in crate::projection) state: LeaseState,
}

impl LeaseProjection {
    /// Stable lease identity.
    #[must_use]
    pub const fn lease(&self) -> &LeaseId {
        &self.lease
    }

    /// Owning logical execution.
    #[must_use]
    pub const fn execution(&self) -> &NodeExecutionId {
        &self.execution
    }

    /// Owning immutable attempt.
    #[must_use]
    pub const fn attempt(&self) -> &AttemptId {
        &self.attempt
    }

    /// Worker/controller holding this lease.
    #[must_use]
    pub const fn worker(&self) -> &WorkerId {
        &self.worker
    }

    /// Latest recorded expiration fact.
    #[must_use]
    pub const fn expires_at(&self) -> TimestampMillis {
        self.expires_at
    }

    /// Current lease state.
    #[must_use]
    pub const fn state(&self) -> &LeaseState {
        &self.state
    }

    /// Returns whether this is the attempt's active lease.
    #[must_use]
    pub const fn is_active(&self) -> bool {
        matches!(self.state, LeaseState::Active)
    }

    /// Returns whether the lease no longer owns work.
    #[must_use]
    pub const fn is_completed(&self) -> bool {
        !self.is_active()
    }
}

/// Origin and state of a durable timer.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub enum TimerPurpose {
    /// General workflow wait, optionally attached to a node execution.
    Wait {
        /// Waiting execution when the timer is node-owned.
        execution: Option<NodeExecutionId>,
    },
    /// Retry backoff for one reserved next attempt.
    Retry {
        /// Reserved next attempt admitted by this timer.
        attempt: AttemptId,
    },
}

/// State of a durable timer.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub enum TimerState {
    /// Registered and not yet fired.
    Pending,
    /// Fired at the recorded boundary-clock observation.
    Fired {
        /// Boundary-clock observation proving the deadline elapsed.
        observed_at: TimestampMillis,
    },
    /// Explicitly cancelled before firing.
    Cancelled,
}

/// Durable cancellation fact for a timer.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub struct TimerCancellationProjection {
    pub(in crate::projection) reason: Reason,
    pub(in crate::projection) sequence: RunSequence,
}

impl TimerCancellationProjection {
    /// Bounded causal cancellation rationale.
    #[must_use]
    pub const fn reason(&self) -> &Reason {
        &self.reason
    }

    /// Sequence at which cancellation became durable.
    #[must_use]
    pub const fn sequence(&self) -> RunSequence {
        self.sequence
    }
}

/// Durable timer read model.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub struct TimerProjection {
    pub(in crate::projection) timer: TimerId,
    pub(in crate::projection) purpose: TimerPurpose,
    pub(in crate::projection) fire_at: TimestampMillis,
    pub(in crate::projection) state: TimerState,
    pub(in crate::projection) cancellation: Option<TimerCancellationProjection>,
}

impl TimerProjection {
    /// Stable timer identity.
    #[must_use]
    pub const fn timer(&self) -> &TimerId {
        &self.timer
    }

    /// Workflow wait or retry purpose.
    #[must_use]
    pub const fn purpose(&self) -> &TimerPurpose {
        &self.purpose
    }

    /// Exact registered deadline.
    #[must_use]
    pub const fn fire_at(&self) -> TimestampMillis {
        self.fire_at
    }

    /// Current timer state.
    #[must_use]
    pub const fn state(&self) -> TimerState {
        self.state
    }

    /// Explicit cancellation fact, when present.
    #[must_use]
    pub const fn cancellation(&self) -> Option<&TimerCancellationProjection> {
        self.cancellation.as_ref()
    }

    /// Returns whether the timer is still pending.
    #[must_use]
    pub const fn is_pending(&self) -> bool {
        matches!(self.state, TimerState::Pending)
    }

    /// Returns whether the timer fired.
    #[must_use]
    pub const fn is_completed(&self) -> bool {
        matches!(self.state, TimerState::Fired { .. } | TimerState::Cancelled)
    }
}

/// Current retry admission state.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub enum RetryState {
    /// Backoff timer has not fired.
    Waiting,
    /// Backoff timer fired and the next attempt may be scheduled.
    Ready,
    /// The next attempt was scheduled.
    Scheduled,
    /// Structured cancellation prevented the reserved attempt from dispatching.
    Cancelled,
}

/// Immutable retry decision and its current admission state.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub struct RetryProjection {
    pub(in crate::projection) execution: NodeExecutionId,
    pub(in crate::projection) previous_attempt: AttemptId,
    pub(in crate::projection) next_attempt: AttemptId,
    pub(in crate::projection) attempt_number: u32,
    pub(in crate::projection) timer: TimerId,
    pub(in crate::projection) fire_at: TimestampMillis,
    pub(in crate::projection) error_class: ErrorClass,
    pub(in crate::projection) reason: Reason,
    pub(in crate::projection) state: RetryState,
}

impl RetryProjection {
    /// Owning logical execution.
    #[must_use]
    pub const fn execution(&self) -> &NodeExecutionId {
        &self.execution
    }

    /// Completed or authority-released prior attempt.
    #[must_use]
    pub const fn previous_attempt(&self) -> &AttemptId {
        &self.previous_attempt
    }

    /// Reserved immutable next attempt.
    #[must_use]
    pub const fn next_attempt(&self) -> &AttemptId {
        &self.next_attempt
    }

    /// One-based number of the next attempt.
    #[must_use]
    pub const fn attempt_number(&self) -> u32 {
        self.attempt_number
    }

    /// Durable backoff timer.
    #[must_use]
    pub const fn timer(&self) -> &TimerId {
        &self.timer
    }

    /// Recorded deadline including deterministic or recorded jitter.
    #[must_use]
    pub const fn fire_at(&self) -> TimestampMillis {
        self.fire_at
    }

    /// Failure class selected by retry policy.
    #[must_use]
    pub const fn error_class(&self) -> ErrorClass {
        self.error_class
    }

    /// Bounded policy rationale.
    #[must_use]
    pub const fn reason(&self) -> &Reason {
        &self.reason
    }

    /// Current retry state.
    #[must_use]
    pub const fn state(&self) -> RetryState {
        self.state
    }

    /// Returns whether retry admission remains pending.
    #[must_use]
    pub const fn is_pending(&self) -> bool {
        matches!(self.state, RetryState::Waiting | RetryState::Ready)
    }

    /// Returns whether the retry attempt was scheduled.
    #[must_use]
    pub const fn is_completed(&self) -> bool {
        matches!(self.state, RetryState::Scheduled | RetryState::Cancelled)
    }
}
