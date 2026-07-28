//! Validated runtime and host-worker limits.

use std::num::{NonZeroU32, NonZeroU64, NonZeroUsize};
use std::time::Duration;

use domain_contracts::MemoryBudget;
use host_runtime::ChannelCapacity;

const DEFAULT_TOKEN_OUTPUT_CAPACITY: NonZeroUsize = match NonZeroUsize::new(256) {
    Some(value) => value,
    None => NonZeroUsize::MIN,
};
const DEFAULT_TOKEN_OUTPUT_RECORD_CAPACITY: NonZeroUsize = match NonZeroUsize::new(512) {
    Some(value) => value,
    None => NonZeroUsize::MIN,
};
const DEFAULT_CLEANUP_MAXIMUM_ATTEMPTS: NonZeroU32 = match NonZeroU32::new(3) {
    Some(value) => value,
    None => NonZeroU32::MIN,
};

/// Deterministic total-attempt limit for explicit backend cleanup.
///
/// The initial failed cleanup operation counts as attempt one. Maintenance may
/// retry only while the retained attempt count is below `maximum_attempts`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CleanupRetryPolicy {
    /// Maximum total cleanup attempts for one retained resource.
    pub maximum_attempts: NonZeroU32,
}

impl CleanupRetryPolicy {
    /// Creates a cleanup retry policy from an already validated non-zero limit.
    #[must_use]
    pub const fn new(maximum_attempts: NonZeroU32) -> Self {
        Self { maximum_attempts }
    }
}

impl Default for CleanupRetryPolicy {
    fn default() -> Self {
        Self::new(DEFAULT_CLEANUP_MAXIMUM_ATTEMPTS)
    }
}

/// Hard model, request, aggregate memory, and cleanup bounds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RuntimeLimits {
    /// Maximum concurrently loaded model instances.
    pub maximum_loaded_models: NonZeroU32,
    /// Maximum active request-owned sequences across all models.
    pub maximum_active_requests: NonZeroU32,
    /// Aggregate resident memory budget enforced by the registry.
    pub memory_budget: MemoryBudget,
    /// Bounded total-attempt policy for explicit cleanup.
    pub cleanup_retry: CleanupRetryPolicy,
}

impl RuntimeLimits {
    /// Creates runtime limits with the default bounded cleanup policy.
    #[must_use]
    pub const fn new(
        maximum_loaded_models: NonZeroU32,
        maximum_active_requests: NonZeroU32,
        memory_budget: MemoryBudget,
    ) -> Self {
        Self {
            maximum_loaded_models,
            maximum_active_requests,
            memory_budget,
            cleanup_retry: CleanupRetryPolicy::new(DEFAULT_CLEANUP_MAXIMUM_ATTEMPTS),
        }
    }

    /// Overrides the total-attempt cleanup policy.
    #[must_use]
    pub const fn with_cleanup_retry_policy(mut self, cleanup_retry: CleanupRetryPolicy) -> Self {
        self.cleanup_retry = cleanup_retry;
        self
    }
}

/// Bounded host-worker transport and lifecycle polling configuration.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HostedRuntimeConfiguration {
    /// Maximum queued runtime commands.
    pub command_capacity: ChannelCapacity,
    /// Maximum queued runtime events.
    pub event_capacity: ChannelCapacity,
    /// Maximum unpublished token identifiers in the pull accumulator.
    pub token_output_capacity: NonZeroUsize,
    /// Maximum unpublished token/state records in the pull accumulator.
    pub token_output_record_capacity: NonZeroUsize,
    poll_interval_milliseconds: NonZeroU64,
}

impl HostedRuntimeConfiguration {
    /// Creates a hosted configuration.
    #[must_use]
    pub const fn new(
        command_capacity: NonZeroUsize,
        event_capacity: NonZeroUsize,
        poll_interval_milliseconds: NonZeroU64,
    ) -> Self {
        Self {
            command_capacity: ChannelCapacity::new(command_capacity),
            event_capacity: ChannelCapacity::new(event_capacity),
            token_output_capacity: DEFAULT_TOKEN_OUTPUT_CAPACITY,
            token_output_record_capacity: DEFAULT_TOKEN_OUTPUT_RECORD_CAPACITY,
            poll_interval_milliseconds,
        }
    }

    /// Overrides pull-oriented token and state capacities.
    #[must_use]
    pub const fn with_token_output_capacity(
        mut self,
        tokens: NonZeroUsize,
        records: NonZeroUsize,
    ) -> Self {
        self.token_output_capacity = tokens;
        self.token_output_record_capacity = records;
        self
    }

    /// Returns the lifecycle polling and bounded-send interval.
    #[must_use]
    pub const fn poll_interval(self) -> Duration {
        Duration::from_millis(self.poll_interval_milliseconds.get())
    }
}
