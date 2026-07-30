//! Typed GGUF source and CPU execution configuration.

use std::error::Error;
use std::fmt::{self, Display, Formatter};
use std::num::{NonZeroI32, NonZeroU32};
use std::path::{Path, PathBuf};

use crate::digest::Sha256Digest;

const MAX_NATIVE_COUNT: u32 = 2_147_483_647;
const DEFAULT_MAXIMUM_HEADER_BYTES: u64 = 512 * 1024 * 1024;
const DEFAULT_MAXIMUM_METADATA_ENTRIES: u64 = 65_536;
const DEFAULT_MAXIMUM_STRING_BYTES: u64 = 16 * 1024 * 1024;
const DEFAULT_MAXIMUM_ARRAY_ELEMENTS: u64 = 4_000_000;

/// Bounded resource limits for cold GGUF header inspection.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GgufInspectionLimits {
    /// Maximum bytes inspected before tensor data begins.
    pub maximum_header_bytes: u64,
    /// Maximum number of metadata entries accepted from one file.
    pub maximum_metadata_entries: u64,
    /// Maximum bytes accepted for one GGUF string.
    pub maximum_string_bytes: u64,
    /// Maximum elements accepted in one metadata array.
    pub maximum_array_elements: u64,
}

impl Default for GgufInspectionLimits {
    fn default() -> Self {
        Self {
            maximum_header_bytes: DEFAULT_MAXIMUM_HEADER_BYTES,
            maximum_metadata_entries: DEFAULT_MAXIMUM_METADATA_ENTRIES,
            maximum_string_bytes: DEFAULT_MAXIMUM_STRING_BYTES,
            maximum_array_elements: DEFAULT_MAXIMUM_ARRAY_ELEMENTS,
        }
    }
}

/// CPU context configuration prepared before model loading.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GgufExecutionConfiguration {
    context_tokens_per_sequence: NonZeroU32,
    maximum_prefill_batch: NonZeroU32,
    micro_batch_tokens: NonZeroU32,
    maximum_sequences: NonZeroU32,
    threads: NonZeroI32,
    batch_threads: NonZeroI32,
    use_mmap: bool,
    use_mlock: bool,
}

impl GgufExecutionConfiguration {
    /// Creates a validated CPU execution configuration.
    ///
    /// # Errors
    ///
    /// Returns [`SourceError::InvalidExecutionConfiguration`] when batch,
    /// sequence, or thread bounds are inconsistent. Returns
    /// [`SourceError::ContextCapacityOverflow`] when the aggregate context
    /// capacity exceeds `u32`.
    pub fn new(
        context_tokens_per_sequence: NonZeroU32,
        maximum_prefill_batch: NonZeroU32,
        micro_batch_tokens: NonZeroU32,
        maximum_sequences: NonZeroU32,
        threads: NonZeroI32,
        batch_threads: NonZeroI32,
    ) -> Result<Self, SourceError> {
        if maximum_prefill_batch.get() > context_tokens_per_sequence.get()
            || micro_batch_tokens.get() > maximum_prefill_batch.get()
            || context_tokens_per_sequence.get() > MAX_NATIVE_COUNT
            || maximum_prefill_batch.get() > MAX_NATIVE_COUNT
            || maximum_sequences.get() > MAX_NATIVE_COUNT
            || threads.get() <= 0
            || batch_threads.get() <= 0
        {
            return Err(SourceError::InvalidExecutionConfiguration);
        }
        context_tokens_per_sequence
            .get()
            .checked_mul(maximum_sequences.get())
            .ok_or(SourceError::ContextCapacityOverflow)?;

        Ok(Self {
            context_tokens_per_sequence,
            maximum_prefill_batch,
            micro_batch_tokens,
            maximum_sequences,
            threads,
            batch_threads,
            use_mmap: true,
            use_mlock: false,
        })
    }

    /// Enables or disables memory-mapped model loading.
    #[must_use]
    pub const fn with_mmap(mut self, enabled: bool) -> Self {
        self.use_mmap = enabled;
        self
    }

    /// Enables or disables locking mapped model pages in host memory.
    #[must_use]
    pub const fn with_mlock(mut self, enabled: bool) -> Self {
        self.use_mlock = enabled;
        self
    }

    /// Returns the maximum retained tokens for one logical sequence.
    #[must_use]
    pub const fn context_tokens_per_sequence(self) -> NonZeroU32 {
        self.context_tokens_per_sequence
    }

    /// Returns the maximum prompt tokens accepted by one prefill call.
    #[must_use]
    pub const fn maximum_prefill_batch(self) -> NonZeroU32 {
        self.maximum_prefill_batch
    }

    /// Returns the physical micro-batch token bound.
    #[must_use]
    pub const fn micro_batch_tokens(self) -> NonZeroU32 {
        self.micro_batch_tokens
    }

    /// Returns the number of concurrently resident logical sequence slots.
    #[must_use]
    pub const fn maximum_sequences(self) -> NonZeroU32 {
        self.maximum_sequences
    }

    /// Returns the CPU thread count used for token decoding.
    #[must_use]
    pub const fn threads(self) -> NonZeroI32 {
        self.threads
    }

    /// Returns the CPU thread count used for prompt batches.
    #[must_use]
    pub const fn batch_threads(self) -> NonZeroI32 {
        self.batch_threads
    }

    /// Returns whether model weights should be memory mapped.
    #[must_use]
    pub const fn use_mmap(self) -> bool {
        self.use_mmap
    }

    /// Returns whether mapped model pages should be locked in host memory.
    #[must_use]
    pub const fn use_mlock(self) -> bool {
        self.use_mlock
    }

    pub(crate) fn total_context_tokens(self) -> Result<NonZeroU32, SourceError> {
        let total = self
            .context_tokens_per_sequence
            .get()
            .checked_mul(self.maximum_sequences.get())
            .ok_or(SourceError::ContextCapacityOverflow)?;
        NonZeroU32::new(total).ok_or(SourceError::InvalidExecutionConfiguration)
    }
}

/// Failure while constructing a typed GGUF source.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SourceError {
    /// Batch, micro-batch, sequence, or thread bounds are inconsistent.
    InvalidExecutionConfiguration,
    /// Total context capacity overflowed `u32`.
    ContextCapacityOverflow,
    /// Inspection limits contain an unusable zero bound.
    InvalidInspectionLimits,
}

impl Display for SourceError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidExecutionConfiguration => {
                formatter.write_str("GGUF execution configuration is inconsistent")
            }
            Self::ContextCapacityOverflow => {
                formatter.write_str("GGUF total context capacity exceeds u32")
            }
            Self::InvalidInspectionLimits => {
                formatter.write_str("GGUF inspection limits must be non-zero")
            }
        }
    }
}

impl Error for SourceError {}

/// One local GGUF model file and its prepared execution bounds.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GgufSource {
    path: PathBuf,
    execution: GgufExecutionConfiguration,
    inspection_limits: GgufInspectionLimits,
    expected_digest: Option<Sha256Digest>,
}

impl GgufSource {
    /// Creates a GGUF source with bounded default metadata limits.
    #[must_use]
    pub fn new(path: impl Into<PathBuf>, execution: GgufExecutionConfiguration) -> Self {
        Self {
            path: path.into(),
            execution,
            inspection_limits: GgufInspectionLimits::default(),
            expected_digest: None,
        }
    }

    /// Creates a GGUF source that requires one immutable SHA-256 identity.
    #[must_use]
    pub fn new_verified(
        path: impl Into<PathBuf>,
        execution: GgufExecutionConfiguration,
        expected_digest: Sha256Digest,
    ) -> Self {
        Self {
            path: path.into(),
            execution,
            inspection_limits: GgufInspectionLimits::default(),
            expected_digest: Some(expected_digest),
        }
    }

    /// Requires this source to match one immutable SHA-256 identity.
    #[must_use]
    pub const fn with_expected_digest(mut self, expected_digest: Sha256Digest) -> Self {
        self.expected_digest = Some(expected_digest);
        self
    }

    /// Replaces the metadata inspection limits after validating non-zero bounds.
    ///
    /// # Errors
    ///
    /// Returns [`SourceError::InvalidInspectionLimits`] when any bound is zero.
    pub fn with_inspection_limits(
        mut self,
        limits: GgufInspectionLimits,
    ) -> Result<Self, SourceError> {
        if limits.maximum_header_bytes == 0
            || limits.maximum_metadata_entries == 0
            || limits.maximum_string_bytes == 0
            || limits.maximum_array_elements == 0
        {
            return Err(SourceError::InvalidInspectionLimits);
        }
        self.inspection_limits = limits;
        Ok(self)
    }

    /// Returns the local GGUF path.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Returns the prepared CPU execution configuration.
    #[must_use]
    pub const fn execution(&self) -> GgufExecutionConfiguration {
        self.execution
    }

    /// Returns the bounded metadata inspection limits.
    #[must_use]
    pub const fn inspection_limits(&self) -> GgufInspectionLimits {
        self.inspection_limits
    }

    /// Returns the required SHA-256 identity, or `None` for a legacy unverified source.
    #[must_use]
    pub const fn expected_digest(&self) -> Option<Sha256Digest> {
        self.expected_digest
    }
}
