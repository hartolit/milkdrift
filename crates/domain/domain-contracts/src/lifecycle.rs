//! Deterministic loaded-model lifecycle and bounded drain semantics.

use core::num::NonZeroU64;

use crate::capacity::{CapacityExhausted, CapacityResource};
use crate::error::BackendFailure;
use crate::generation::CancellationReason;
use crate::time::MonotonicMillis;

/// Non-zero hard timeout applied to draining active model work.
#[repr(transparent)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct DrainTimeout(NonZeroU64);

impl DrainTimeout {
    /// Creates a drain timeout from a validated non-zero millisecond value.
    #[must_use]
    pub const fn new(milliseconds: NonZeroU64) -> Self {
        Self(milliseconds)
    }

    /// Creates a drain timeout from milliseconds.
    ///
    /// # Errors
    ///
    /// Returns [`LifecycleError::ZeroDrainTimeout`] when `milliseconds` is zero.
    pub const fn from_millis(milliseconds: u64) -> Result<Self, LifecycleError> {
        match NonZeroU64::new(milliseconds) {
            Some(value) => Ok(Self(value)),
            None => Err(LifecycleError::ZeroDrainTimeout),
        }
    }

    /// Returns the timeout in milliseconds.
    #[must_use]
    pub const fn as_millis(self) -> u64 {
        self.0.get()
    }
}

/// Active bounded drain window measured against a monotonic clock.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DrainWindow {
    started_at: MonotonicMillis,
    timeout: DrainTimeout,
}

impl DrainWindow {
    /// Starts a drain window.
    #[must_use]
    pub const fn new(started_at: MonotonicMillis, timeout: DrainTimeout) -> Self {
        Self {
            started_at,
            timeout,
        }
    }

    /// Returns the monotonic start timestamp.
    #[must_use]
    pub const fn started_at(self) -> MonotonicMillis {
        self.started_at
    }

    /// Returns the hard timeout.
    #[must_use]
    pub const fn timeout(self) -> DrainTimeout {
        self.timeout
    }

    /// Returns whether the hard timeout has elapsed.
    #[must_use]
    pub const fn has_expired(self, now: MonotonicMillis) -> bool {
        now.elapsed_since(self.started_at) >= self.timeout.as_millis()
    }
}

/// Model unload policy selected by the owning runtime.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnloadPolicy {
    /// Reject unload while any request remains active.
    RejectIfBusy,
    /// Force-cancel all active requests before releasing model resources.
    CancelActive,
    /// Allow active requests to finish until the hard timeout, then automatically
    /// escalate to `CancelActive` with `CancellationReason::DrainTimeout`.
    Drain {
        /// Mandatory non-zero hard timeout.
        timeout: DrainTimeout,
    },
}

/// Phase in which an unrecoverable lifecycle failure occurred.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LifecycleFailurePhase {
    /// Loading model resources.
    Loading,
    /// Serving active requests.
    Active,
    /// Synchronizing or releasing model resources.
    Unloading,
}

/// Stable model lifecycle state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ModelLifecycleState {
    /// No model resources are resident.
    Absent,
    /// Model resources are being loaded.
    Loading,
    /// Model is resident with no active requests.
    Ready,
    /// Model is resident and serving requests.
    Active {
        /// Number of active requests.
        active_requests: u32,
    },
    /// Runtime is allowing active work to finish within a bounded window.
    Draining {
        /// Number of active requests not yet finished.
        active_requests: u32,
        /// Mandatory hard timeout window.
        window: DrainWindow,
    },
    /// Runtime has requested cancellation and is waiting for safe boundaries.
    Cancelling {
        /// Number of active requests not yet finished.
        active_requests: u32,
        /// Cancellation reason propagated to each request.
        reason: CancellationReason,
    },
    /// Model has no active requests and backend resources may be released.
    Unloading,
    /// Lifecycle cannot continue until the owner explicitly clears the failure.
    Failed {
        /// Phase that failed.
        phase: LifecycleFailurePhase,
        /// Stable backend failure description.
        failure: BackendFailure,
    },
}

/// Coarse action the runtime must perform after a state transition.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LifecycleAction {
    /// No external action is required.
    None,
    /// Signal every active request to cancel at its next safe boundary.
    CancelActive {
        /// Cancellation reason supplied to active requests.
        reason: CancellationReason,
    },
    /// Synchronize and release the exclusively owned backend model.
    ReleaseModel,
    /// Model was already absent or unload completed.
    UnloadComplete,
}

/// Invalid lifecycle transition or bounded resource failure.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum LifecycleError {
    /// Requested transition is invalid for the current state.
    InvalidTransition,
    /// Unload was rejected because work is active.
    Busy {
        /// Number of active requests preventing unload.
        active_requests: u32,
    },
    /// Drain timeout must be non-zero.
    ZeroDrainTimeout,
    /// A bounded lifecycle counter was exhausted.
    CapacityExhausted(CapacityExhausted),
}

/// Pure lifecycle state machine owned by the inference runtime.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ModelLifecycle {
    state: ModelLifecycleState,
}

impl Default for ModelLifecycle {
    fn default() -> Self {
        Self::new()
    }
}

impl ModelLifecycle {
    /// Creates an absent model lifecycle.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            state: ModelLifecycleState::Absent,
        }
    }

    /// Returns the current state.
    #[must_use]
    pub const fn state(&self) -> ModelLifecycleState {
        self.state
    }

    /// Begins loading from the absent state.
    ///
    /// # Errors
    ///
    /// Returns [`LifecycleError::InvalidTransition`] unless the model is absent.
    pub fn begin_load(&mut self) -> Result<LifecycleAction, LifecycleError> {
        if self.state != ModelLifecycleState::Absent {
            return Err(LifecycleError::InvalidTransition);
        }
        self.state = ModelLifecycleState::Loading;
        Ok(LifecycleAction::None)
    }

    /// Completes a successful load.
    ///
    /// # Errors
    ///
    /// Returns [`LifecycleError::InvalidTransition`] unless loading is in progress.
    pub fn complete_load(&mut self) -> Result<LifecycleAction, LifecycleError> {
        if self.state != ModelLifecycleState::Loading {
            return Err(LifecycleError::InvalidTransition);
        }
        self.state = ModelLifecycleState::Ready;
        Ok(LifecycleAction::None)
    }

    /// Marks the lifecycle failed and prevents further progress until cleared.
    pub const fn fail(
        &mut self,
        phase: LifecycleFailurePhase,
        failure: BackendFailure,
    ) -> LifecycleAction {
        self.state = ModelLifecycleState::Failed { phase, failure };
        LifecycleAction::None
    }

    /// Clears a failed model after the owner has externally reclaimed resources.
    ///
    /// # Errors
    ///
    /// Returns [`LifecycleError::InvalidTransition`] unless the lifecycle has failed.
    pub const fn clear_failure(&mut self) -> Result<LifecycleAction, LifecycleError> {
        match self.state {
            ModelLifecycleState::Failed { .. } => {
                self.state = ModelLifecycleState::Absent;
                Ok(LifecycleAction::UnloadComplete)
            }
            _ => Err(LifecycleError::InvalidTransition),
        }
    }

    /// Registers one active generation request.
    ///
    /// # Errors
    ///
    /// Returns [`LifecycleError::InvalidTransition`] unless the model is ready or
    /// active, or [`LifecycleError::CapacityExhausted`] if the request count is full.
    pub fn start_request(&mut self) -> Result<LifecycleAction, LifecycleError> {
        match self.state {
            ModelLifecycleState::Ready => {
                self.state = ModelLifecycleState::Active { active_requests: 1 };
                Ok(LifecycleAction::None)
            }
            ModelLifecycleState::Active { active_requests } => {
                let Some(next) = active_requests.checked_add(1) else {
                    return Err(LifecycleError::CapacityExhausted(CapacityExhausted::new(
                        CapacityResource::ActiveRequests,
                        u64::from(u32::MAX) + 1,
                        u64::from(active_requests),
                    )));
                };
                self.state = ModelLifecycleState::Active {
                    active_requests: next,
                };
                Ok(LifecycleAction::None)
            }
            _ => Err(LifecycleError::InvalidTransition),
        }
    }

    /// Registers completion of one active request.
    ///
    /// # Errors
    ///
    /// Returns [`LifecycleError::InvalidTransition`] unless the model has an active,
    /// draining, or cancelling request to finish.
    pub const fn finish_request(&mut self) -> Result<LifecycleAction, LifecycleError> {
        match self.state {
            ModelLifecycleState::Active { active_requests } => {
                if active_requests == 1 {
                    self.state = ModelLifecycleState::Ready;
                } else {
                    self.state = ModelLifecycleState::Active {
                        active_requests: active_requests - 1,
                    };
                }
                Ok(LifecycleAction::None)
            }
            ModelLifecycleState::Draining {
                active_requests,
                window,
            } => {
                if active_requests == 1 {
                    self.state = ModelLifecycleState::Unloading;
                    Ok(LifecycleAction::ReleaseModel)
                } else {
                    self.state = ModelLifecycleState::Draining {
                        active_requests: active_requests - 1,
                        window,
                    };
                    Ok(LifecycleAction::None)
                }
            }
            ModelLifecycleState::Cancelling {
                active_requests,
                reason,
            } => {
                if active_requests == 1 {
                    self.state = ModelLifecycleState::Unloading;
                    Ok(LifecycleAction::ReleaseModel)
                } else {
                    self.state = ModelLifecycleState::Cancelling {
                        active_requests: active_requests - 1,
                        reason,
                    };
                    Ok(LifecycleAction::None)
                }
            }
            _ => Err(LifecycleError::InvalidTransition),
        }
    }

    /// Requests unloading under an explicit policy.
    ///
    /// # Errors
    ///
    /// Returns [`LifecycleError::Busy`] when busy-model rejection is requested, or
    /// [`LifecycleError::InvalidTransition`] when unloading cannot begin.
    pub const fn request_unload(
        &mut self,
        policy: UnloadPolicy,
        now: MonotonicMillis,
    ) -> Result<LifecycleAction, LifecycleError> {
        match self.state {
            ModelLifecycleState::Absent => Ok(LifecycleAction::UnloadComplete),
            ModelLifecycleState::Ready => {
                self.state = ModelLifecycleState::Unloading;
                Ok(LifecycleAction::ReleaseModel)
            }
            ModelLifecycleState::Active { active_requests } => match policy {
                UnloadPolicy::RejectIfBusy => Err(LifecycleError::Busy { active_requests }),
                UnloadPolicy::CancelActive => {
                    let reason = CancellationReason::ModelUnload;
                    self.state = ModelLifecycleState::Cancelling {
                        active_requests,
                        reason,
                    };
                    Ok(LifecycleAction::CancelActive { reason })
                }
                UnloadPolicy::Drain { timeout } => {
                    self.state = ModelLifecycleState::Draining {
                        active_requests,
                        window: DrainWindow::new(now, timeout),
                    };
                    Ok(LifecycleAction::None)
                }
            },
            ModelLifecycleState::Loading
            | ModelLifecycleState::Draining { .. }
            | ModelLifecycleState::Cancelling { .. }
            | ModelLifecycleState::Unloading
            | ModelLifecycleState::Failed { .. } => Err(LifecycleError::InvalidTransition),
        }
    }

    /// Polls timeout-driven lifecycle transitions.
    ///
    /// An expired drain window unconditionally escalates to forced cancellation.
    ///
    /// # Errors
    ///
    /// Returns [`LifecycleError::InvalidTransition`] while unloading or after a failure.
    pub const fn poll(&mut self, now: MonotonicMillis) -> Result<LifecycleAction, LifecycleError> {
        match self.state {
            ModelLifecycleState::Draining {
                active_requests,
                window,
            } if window.has_expired(now) => {
                let reason = CancellationReason::DrainTimeout;
                self.state = ModelLifecycleState::Cancelling {
                    active_requests,
                    reason,
                };
                Ok(LifecycleAction::CancelActive { reason })
            }
            ModelLifecycleState::Draining { .. }
            | ModelLifecycleState::Ready
            | ModelLifecycleState::Active { .. }
            | ModelLifecycleState::Cancelling { .. }
            | ModelLifecycleState::Loading
            | ModelLifecycleState::Absent => Ok(LifecycleAction::None),
            ModelLifecycleState::Unloading | ModelLifecycleState::Failed { .. } => {
                Err(LifecycleError::InvalidTransition)
            }
        }
    }

    /// Completes backend destruction and returns to the absent state.
    ///
    /// # Errors
    ///
    /// Returns [`LifecycleError::InvalidTransition`] unless unloading is in progress.
    pub fn complete_unload(&mut self) -> Result<LifecycleAction, LifecycleError> {
        if self.state != ModelLifecycleState::Unloading {
            return Err(LifecycleError::InvalidTransition);
        }
        self.state = ModelLifecycleState::Absent;
        Ok(LifecycleAction::UnloadComplete)
    }
}
