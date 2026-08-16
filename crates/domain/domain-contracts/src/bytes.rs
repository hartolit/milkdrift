//! Portable deterministic byte counts.

use core::fmt;

/// An exact, portable count of bytes.
///
/// This type deliberately provides only named checked arithmetic. Converting to
/// a platform allocation size is explicit and checked at the allocation edge.
#[repr(transparent)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ByteCount(u64);

impl ByteCount {
    /// Zero bytes.
    pub const ZERO: Self = Self(0);

    /// Largest byte count representable by the portable contract.
    pub const MAX: Self = Self(u64::MAX);

    /// Creates an exact byte count from its portable raw representation.
    #[must_use]
    pub const fn from_u64(bytes: u64) -> Self {
        Self(bytes)
    }

    /// Returns the raw representation for display, serialization, or an external boundary.
    #[must_use]
    pub const fn as_u64(self) -> u64 {
        self.0
    }

    /// Returns whether this count is zero.
    #[must_use]
    pub const fn is_zero(self) -> bool {
        self.0 == 0
    }

    /// Returns the exact sum, or `None` on overflow.
    #[must_use]
    pub const fn checked_add(self, other: Self) -> Option<Self> {
        match self.0.checked_add(other.0) {
            Some(bytes) => Some(Self(bytes)),
            None => None,
        }
    }

    /// Returns the exact difference, or `None` on underflow.
    #[must_use]
    pub const fn checked_sub(self, other: Self) -> Option<Self> {
        match self.0.checked_sub(other.0) {
            Some(bytes) => Some(Self(bytes)),
            None => None,
        }
    }

    /// Multiplies this byte count by an independent dimensionless count.
    #[must_use]
    pub const fn checked_mul_count(self, count: u64) -> Option<Self> {
        match self.0.checked_mul(count) {
            Some(bytes) => Some(Self(bytes)),
            None => None,
        }
    }

    /// Returns the larger of two byte-count components.
    #[must_use]
    pub const fn component_max(self, other: Self) -> Self {
        if self.0 >= other.0 { self } else { other }
    }

    /// Returns whether this count is at least the required count.
    #[must_use]
    pub const fn contains(self, required: Self) -> bool {
        self.0 >= required.0
    }

    /// Converts to a platform allocation or slice capacity when representable.
    #[must_use]
    pub fn checked_to_usize(self) -> Option<usize> {
        usize::try_from(self.0).ok()
    }
}

impl fmt::Display for ByteCount {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}
