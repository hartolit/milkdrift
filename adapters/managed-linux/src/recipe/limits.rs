use crate::rejected;
use milkdrift_capability_host::managed::ManagedError;
use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant};

/// Independent hard limits for one container. Temporary files consume this container's memory;
/// these caps do not reserve host resources or combine the worker and model into one cgroup.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContainerLimits {
    /// Resident memory ceiling in bytes, with swap disabled.
    pub memory_bytes: u64,
    /// CPU quota in hundredths of a CPU; 250 permits two and a half CPUs of execution time.
    pub cpu_percent: u32,
    /// Maximum processes and threads, including descendants.
    pub pids: u32,
    /// Size ceiling for the writable /tmp filesystem, within memory_bytes.
    pub temporary_bytes: u64,
}

impl ContainerLimits {
    pub(crate) fn validate(&self, field: &str) -> Result<(), ManagedError> {
        // OCI/Linux memory limits are signed bytes; zero would disable the limit.
        if self.memory_bytes == 0 || self.memory_bytes > i64::MAX as u64 {
            return Err(rejected(format!(
                "{field}.memory_bytes must be positive signed-64-bit bytes"
            )));
        }
        if self.cpu_percent == 0 {
            return Err(rejected(format!("{field}.cpu_percent must be positive")));
        }
        if self.pids == 0 {
            return Err(rejected(format!("{field}.pids must be positive")));
        }
        if self.temporary_bytes == 0 || self.temporary_bytes > self.memory_bytes {
            return Err(rejected(format!(
                "{field}.temporary_bytes must be positive and no greater than memory_bytes"
            )));
        }
        Ok(())
    }
    pub(crate) fn cpus(&self) -> String {
        format!("{}.{:02}", self.cpu_percent / 100, self.cpu_percent % 100)
    }
    pub(crate) fn temporary_mount(&self) -> String {
        format!("/tmp:rw,nodev,nosuid,size={}", self.temporary_bytes)
    }
    #[cfg(any(target_os = "linux", test))]
    pub(crate) fn validate_page_alignment(
        &self,
        page_size: u64,
        field: &str,
    ) -> Result<(), ManagedError> {
        // Linux memory and tmpfs limits are page-granular. Refuse rounding of operator intent;
        // use the actual target host page size, not an assumed x86 page size in the recipe reader.
        for (name, bytes) in [
            ("memory_bytes", self.memory_bytes),
            ("temporary_bytes", self.temporary_bytes),
        ] {
            if !bytes.is_multiple_of(page_size) {
                return Err(rejected(format!(
                    "{field}.{name} must be a multiple of this host's {page_size}-byte page size"
                )));
            }
        }
        Ok(())
    }
}

/// Bounded installation work for an owned model. These are operator-selected waits, not promises
/// that an interrupted service or request has stopped; lifecycle recovery retains that uncertainty.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ServiceTimeouts {
    /// Wait for supervised startup and healthy readiness.
    pub startup_ms: u64,
    /// Grace period for the service to stop before systemd/Podman force termination.
    pub shutdown_ms: u64,
    /// Deadline for reading and hashing the approved local model file during preparation.
    pub model_verification_ms: u64,
}
impl ServiceTimeouts {
    pub(crate) fn validate(&self) -> Result<(), ManagedError> {
        for (name, value) in [
            ("startup_ms", self.startup_ms),
            ("shutdown_ms", self.shutdown_ms),
            ("model_verification_ms", self.model_verification_ms),
        ] {
            validate_timeout(value, &format!("model_service.timeouts.{name}"))?;
        }
        // podman stop accepts signed whole seconds; rounding up preserves the requested grace.
        if self.shutdown_ms.div_ceil(1000) > i32::MAX as u64 {
            return Err(rejected(
                "model_service.timeouts.shutdown_ms exceeds Podman's signed seconds range",
            ));
        }
        // systemd parse_time rejects values >= USEC_INFINITY / the suffix multiplier.
        // Match that parser boundary, including its excluded last partial millisecond.
        if self.startup_ms >= u64::MAX / 1000 {
            return Err(rejected(
                "model_service.timeouts.startup_ms exceeds systemd's finite microseconds range",
            ));
        }
        Ok(())
    }
    pub(crate) fn stop_seconds(&self) -> u64 {
        self.shutdown_ms.div_ceil(1000)
    }
}

pub(crate) fn validate_timeout(value: u64, field: &str) -> Result<(), ManagedError> {
    if value == 0
        || value > i64::MAX as u64
        || Instant::now()
            .checked_add(Duration::from_millis(value))
            .is_none()
    {
        return Err(rejected(format!(
            "{field} must be a positive, representable finite duration in milliseconds"
        )));
    }
    Ok(())
}
