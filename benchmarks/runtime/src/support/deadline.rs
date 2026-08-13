//! One checked synchronous deadline for every benchmark polling loop.

use std::time::{Duration, Instant};

use crate::error::{BenchmarkError, BenchmarkResult};

const POLL_CADENCE: Duration = Duration::from_millis(1);

#[derive(Clone, Copy)]
pub(crate) struct Deadline {
    expires_at: Instant,
    operation: &'static str,
}

impl Deadline {
    pub(crate) fn after(timeout: Duration, operation: &'static str) -> BenchmarkResult<Self> {
        Self::from_start(Instant::now(), timeout, operation)
    }

    pub(crate) fn from_start(
        started_at: Instant,
        timeout: Duration,
        operation: &'static str,
    ) -> BenchmarkResult<Self> {
        let expires_at = started_at.checked_add(timeout).ok_or_else(|| {
            BenchmarkError::new(format!(
                "deadline overflow while preparing to wait for {operation}"
            ))
        })?;
        Ok(Self {
            expires_at,
            operation,
        })
    }

    pub(crate) fn remaining(self, waiting_for: &'static str) -> BenchmarkResult<Duration> {
        self.expires_at
            .checked_duration_since(Instant::now())
            .filter(|remaining| !remaining.is_zero())
            .ok_or_else(|| {
                BenchmarkError::new(format!(
                    "timed out waiting for {waiting_for} under the {} deadline; the deadline is an operational hang bound, not a performance threshold",
                    self.operation,
                ))
            })
    }

    pub(crate) fn wait_for_poll(self, waiting_for: &'static str) -> BenchmarkResult {
        let remaining = self.remaining(waiting_for)?;
        std::thread::sleep(POLL_CADENCE.min(remaining));
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use super::Deadline;

    #[test]
    fn deadline_rejects_instant_overflow_with_operation_context() -> Result<(), &'static str> {
        let Err(error) =
            Deadline::from_start(Instant::now(), Duration::MAX, "overflow regression stage")
        else {
            return Err("Duration::MAX did not overflow a current Instant");
        };
        assert!(error.to_string().contains("overflow regression stage"));
        assert!(error.to_string().contains("overflow"));
        Ok(())
    }

    #[test]
    fn expired_deadline_times_out_without_sleeping() -> Result<(), &'static str> {
        let deadline = Deadline {
            expires_at: Instant::now(),
            operation: "timeout regression stage",
        };
        let Err(error) = deadline.wait_for_poll("timeout regression wait") else {
            return Err("an expired deadline entered a successful wait result");
        };
        assert!(error.to_string().contains("timeout regression stage"));
        assert!(error.to_string().contains("timeout regression wait"));
        assert!(error.to_string().contains("timed out"));
        Ok(())
    }
}
