//! Monotonic time values used by deterministic lifecycle policies.

/// Milliseconds measured from an implementation-defined monotonic epoch.
#[repr(transparent)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct MonotonicMillis(u64);

impl MonotonicMillis {
    /// Creates a monotonic timestamp.
    #[must_use]
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    /// Returns the raw monotonic millisecond count.
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }

    /// Returns elapsed milliseconds using saturating arithmetic.
    #[must_use]
    pub const fn elapsed_since(self, earlier: Self) -> u64 {
        self.0.saturating_sub(earlier.0)
    }
}
