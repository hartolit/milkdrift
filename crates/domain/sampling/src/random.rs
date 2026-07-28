//! Small deterministic pseudo-random generator with no external state.

/// `SplitMix64` generator suitable for deterministic local sampling.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SplitMix64 {
    state: u64,
}

impl SplitMix64 {
    /// Creates a generator from an explicit seed.
    #[must_use]
    pub const fn new(seed: u64) -> Self {
        Self { state: seed }
    }

    /// Produces the next 64 random bits.
    pub const fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut value = self.state;
        value = (value ^ (value >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        value = (value ^ (value >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        value ^ (value >> 31)
    }

    /// Produces a uniformly distributed value in `[0, 1)` using 24 random bits.
    #[allow(clippy::cast_precision_loss)]
    pub const fn next_unit_f32(&mut self) -> f32 {
        const SCALE: f32 = 1.0 / 16_777_216.0;
        let bits = (self.next_u64() >> 40) as u32;
        // Every 24-bit integer is exactly representable by f32, so this cast is lossless.
        bits as f32 * SCALE
    }
}
