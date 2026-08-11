//! Frontend-neutral retained model ownership and cleanup state.

use domain_contracts::{MemoryFootprint, ModelHandle};

use crate::ApplicationFailure;

/// Concrete byte ownership reported through the application boundary.
///
/// These values describe deterministic runtime accounting, not sampled RSS or
/// whole-device memory use.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ApplicationMemoryFootprint {
    /// Host bytes retaining model weights.
    pub host_weight_bytes: u64,
    /// Device-local bytes retaining model weights.
    pub device_weight_bytes: u64,
    /// Host bytes retaining model working storage.
    pub host_working_bytes: u64,
    /// Device-local bytes retaining model working storage.
    pub device_working_bytes: u64,
}

impl From<MemoryFootprint> for ApplicationMemoryFootprint {
    fn from(value: MemoryFootprint) -> Self {
        Self {
            host_weight_bytes: value.host_weight_bytes,
            device_weight_bytes: value.device_weight_bytes,
            host_working_bytes: value.host_working_bytes,
            device_working_bytes: value.device_working_bytes,
        }
    }
}

/// Checked conservative evidence for ownership whose exact upper bound is not verified.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ApplicationConservativeFootprint {
    /// Every component and aggregate total was represented exactly.
    Known(ApplicationMemoryFootprint),
    /// Checked arithmetic could not represent at least one component or aggregate total.
    Overflow,
}

/// Application-relevant origin of retained model ownership.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ApplicationRetainedModelResource {
    /// A failed prepared load retains partial or complete preparation resources.
    FailedLoad {
        /// Generation-safe identity assigned to the failed load transaction.
        handle: ModelHandle,
    },
    /// A verified loaded model remains owned while ordinary unload is retried.
    LoadedModel {
        /// Exact verified generation being released.
        handle: ModelHandle,
    },
    /// A complete lower model contradicted its admitted contract.
    IncompatibleModel {
        /// Generation-safe identity assigned to the incompatible lower model.
        handle: ModelHandle,
    },
    /// A load was in flight when the worker disconnected before ownership was reported.
    UnconfirmedLoad,
    /// Model ownership exists, but the worker stopped before E1 received its exact identity.
    UnconfirmedModel,
}

/// Certainty of lower model ownership retained by the application lifecycle.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ApplicationRetainedOwnership {
    /// A named lower ownership phase still has an exact byte footprint.
    Exact(ApplicationMemoryFootprint),
    /// A contract-violating lower owner has no verified exact upper bound.
    Unverified {
        /// Exact loading reservation accepted before materialization.
        accepted_loading_peak: ApplicationMemoryFootprint,
        /// Contradictory footprint reported by the retained lower owner.
        reported_footprint: ApplicationMemoryFootprint,
        /// Checked component-wise conservative evidence.
        conservative_footprint: ApplicationConservativeFootprint,
    },
    /// The endpoint disappeared before ownership certainty could be observed.
    Unknown,
}

/// Current cleanup disposition for retained model ownership.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ApplicationModelCleanupDisposition {
    /// Cleanup or a correlated unload is currently progressing.
    Pending,
    /// Lower cleanup remains retryable under its bounded attempt policy.
    LowerRetryable {
        /// Attempts already performed, including the initial failed cleanup.
        attempts: u32,
        /// Maximum total lower cleanup attempts.
        maximum_attempts: u32,
    },
    /// Lower cleanup exhausted its bounded attempt policy.
    LowerExhausted {
        /// Attempts already performed.
        attempts: u32,
        /// Maximum total lower cleanup attempts.
        maximum_attempts: u32,
    },
    /// E1 could not submit or inspect cleanup after bounded coordination attempts.
    ///
    /// A headless host may call `ApplicationRuntime::retry_model_cleanup` to retry
    /// application coordination. This does not reset lower cleanup exhaustion.
    CoordinationRetryAvailable {
        /// E1 submission attempts already performed.
        attempts: u8,
        /// Maximum attempts in one automatic E1 coordination round.
        maximum_attempts: u8,
    },
    /// The inference worker disconnected without proving release.
    WorkerDisconnected,
    /// Terminal shutdown deliberately retained ownership until process exit.
    RetainedUntilProcessExit,
}

/// Durable frontend-neutral state for one retained model owner.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ApplicationRetainedModel {
    resource: ApplicationRetainedModelResource,
    ownership: ApplicationRetainedOwnership,
    cleanup: ApplicationModelCleanupDisposition,
    primary_failure: ApplicationFailure,
    cleanup_failure: Option<ApplicationFailure>,
}

impl ApplicationRetainedModel {
    pub(crate) fn new(
        resource: ApplicationRetainedModelResource,
        ownership: ApplicationRetainedOwnership,
        cleanup: ApplicationModelCleanupDisposition,
        primary_failure: ApplicationFailure,
        cleanup_failure: Option<ApplicationFailure>,
    ) -> Self {
        Self {
            resource,
            ownership,
            cleanup,
            primary_failure,
            cleanup_failure,
        }
    }

    /// Returns the model owner or transaction whose release remains unresolved.
    #[must_use]
    pub const fn resource(&self) -> ApplicationRetainedModelResource {
        self.resource
    }

    /// Returns exact, unverified, or unknown lower ownership evidence.
    #[must_use]
    pub const fn ownership(&self) -> ApplicationRetainedOwnership {
        self.ownership
    }

    /// Returns lower cleanup, E1 coordination, disconnection, or terminal disposition.
    #[must_use]
    pub const fn cleanup(&self) -> ApplicationModelCleanupDisposition {
        self.cleanup
    }

    /// Returns the independently preserved primary operation failure.
    #[must_use]
    pub const fn primary_failure(&self) -> &ApplicationFailure {
        &self.primary_failure
    }

    /// Returns the independently preserved cleanup or coordination failure, when present.
    #[must_use]
    pub const fn cleanup_failure(&self) -> Option<&ApplicationFailure> {
        self.cleanup_failure.as_ref()
    }

    pub(crate) fn set_cleanup(
        &mut self,
        cleanup: ApplicationModelCleanupDisposition,
        failure: Option<ApplicationFailure>,
    ) {
        self.cleanup = cleanup;
        if failure.is_some() {
            self.cleanup_failure = failure;
        }
    }
}
