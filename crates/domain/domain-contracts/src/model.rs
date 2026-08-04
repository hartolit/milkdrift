//! Model metadata, capabilities, memory plans, and sequence configuration.

use core::num::NonZeroU32;

use crate::{BackendId, DeviceId, ModelHandle};

/// Model architecture family understood by adapters and schedulers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum ModelArchitecture {
    /// Decoder-only Llama-family transformer.
    Llama,
    /// Decoder-only Mistral-family transformer.
    Mistral,
    /// Decoder-only Gemma-family transformer.
    Gemma,
    /// Decoder-only Qwen-family transformer.
    Qwen,
    /// Backend-defined architecture code.
    Other(u32),
}

/// Scalar representation used by source tensors or selected backend execution tensors.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum ScalarType {
    /// IEEE-754 32-bit floating point.
    F32,
    /// IEEE-754 16-bit floating point.
    F16,
    /// Brain floating point.
    Bf16,
    /// Signed 8-bit integer.
    I8,
    /// Unsigned 8-bit integer.
    U8,
    /// Backend-defined scalar representation.
    Other(u16),
}

/// Stable quantization description.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum QuantizationFormat {
    /// Model is not quantized.
    None,
    /// Generic signed 8-bit quantization.
    Int8,
    /// Generic signed 4-bit quantization.
    Int4,
    /// Backend-defined quantization code.
    Other(u16),
}

/// Execution device category.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum DeviceKind {
    /// Host CPU execution.
    Cpu,
    /// CUDA-compatible GPU execution.
    Cuda,
    /// Apple Metal execution.
    Metal,
    /// Other user-space accelerator.
    Accelerator(u16),
}

/// Backend-visible identity of the device that executes a loaded model.
///
/// Device identifiers are interpreted within the selected backend and device
/// kind. In particular, a CUDA ordinal is not a globally permanent hardware
/// identity.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ExecutionDevice {
    /// Backend-visible device identifier or ordinal.
    pub id: DeviceId,
    /// Execution device category.
    pub kind: DeviceKind,
}

impl ExecutionDevice {
    /// Creates one backend-visible execution-device identity.
    #[must_use]
    pub const fn new(id: DeviceId, kind: DeviceKind) -> Self {
        Self { id, kind }
    }
}

/// Compact capability bitset.
#[repr(transparent)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct CapabilitySet(u64);

impl CapabilitySet {
    /// Empty capability set.
    pub const EMPTY: Self = Self(0);
    /// Backend supports prompt prefill.
    pub const PREFILL: Self = Self(1 << 0);
    /// Backend supports incremental decode.
    pub const INCREMENTAL_DECODE: Self = Self(1 << 1);
    /// One model instance may own more than one sequence.
    pub const MULTIPLE_SEQUENCES: Self = Self(1 << 2);
    /// Backend supports batched sequence execution.
    pub const BATCHED_EXECUTION: Self = Self(1 << 3);
    /// Sequence cache can be reset and reused.
    pub const SEQUENCE_RESET: Self = Self(1 << 4);
    /// Backend can synchronize pending device work explicitly.
    pub const EXPLICIT_SYNCHRONIZATION: Self = Self(1 << 5);
    /// Backend guarantees no heap allocation after sequence preparation.
    pub const ALLOCATION_FREE_HOT_PATH: Self = Self(1 << 6);

    /// Creates a capability set from raw stable bits.
    #[must_use]
    pub const fn from_bits(bits: u64) -> Self {
        Self(bits)
    }

    /// Returns the raw capability bits.
    #[must_use]
    pub const fn bits(self) -> u64 {
        self.0
    }

    /// Returns whether all bits in `required` are present.
    #[must_use]
    pub const fn contains(self, required: Self) -> bool {
        (self.0 & required.0) == required.0
    }

    /// Returns the union of two capability sets.
    #[must_use]
    pub const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }
}

/// Backend capability report used for validation and admission control.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ModelCapabilities {
    /// Supported operation bits.
    pub operations: CapabilitySet,
    /// Maximum context tokens accepted by one sequence.
    pub maximum_context_tokens: u32,
    /// Maximum concurrently resident sequences.
    pub maximum_sequences: u32,
    /// Maximum prefill batch size.
    pub maximum_prefill_batch: u32,
}

/// Planned or accounted memory footprint in bytes.
///
/// This value is an admission and accounting quantity, not a measurement of
/// physical memory currently allocated or available on a device.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MemoryFootprint {
    /// Host-resident model weights.
    pub host_weight_bytes: u64,
    /// Device-resident model weights.
    pub device_weight_bytes: u64,
    /// Host working memory excluding weights.
    pub host_working_bytes: u64,
    /// Device working memory excluding weights and sequence caches.
    pub device_working_bytes: u64,
    /// Sequence cache bytes required per token.
    pub cache_bytes_per_token: u64,
}

impl MemoryFootprint {
    /// Returns the non-cache host byte total using saturating arithmetic.
    #[must_use]
    pub const fn host_bytes(self) -> u64 {
        self.host_weight_bytes
            .saturating_add(self.host_working_bytes)
    }

    /// Returns the non-cache device byte total using saturating arithmetic.
    #[must_use]
    pub const fn device_bytes(self) -> u64 {
        self.device_weight_bytes
            .saturating_add(self.device_working_bytes)
    }
}

/// Memory domain used by planning and aggregate admission failures.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MemoryKind {
    /// Host-addressable memory.
    Host,
    /// Device-local memory.
    Device,
}

/// Admission-control budget supplied by the engine.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MemoryBudget {
    /// Maximum host bytes available for the operation.
    pub host_bytes: u64,
    /// Maximum device bytes available for the operation.
    pub device_bytes: u64,
}

/// Immutable model metadata exposed after inspection or load.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ModelMetadata {
    /// Model architecture family.
    pub architecture: ModelArchitecture,
    /// Scalar type stored in the source weight tensors.
    ///
    /// This metadata remains source-derived and does not imply the scalar type
    /// selected later for execution.
    pub scalar_type: ScalarType,
    /// Weight quantization format.
    pub quantization: QuantizationFormat,
    /// Vocabulary size and required logits length.
    pub vocabulary_size: u32,
    /// Native maximum context length.
    pub context_length: u32,
}

/// Model description produced without taking ownership of backend resources.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ModelDescriptor {
    /// Backend that inspected the source.
    pub backend: BackendId,
    /// Model metadata.
    pub metadata: ModelMetadata,
    /// Backend capability report.
    pub capabilities: ModelCapabilities,
    /// Device-independent estimated accounting footprint.
    pub estimated_footprint: MemoryFootprint,
}

/// Cold-path configuration for loading one model instance.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LoadConfiguration {
    /// Handle assigned by the owning inference runtime.
    pub handle: ModelHandle,
    /// Explicit execution device requested by the caller.
    pub execution_device: ExecutionDevice,
    /// Hard admission-control budget.
    pub memory_budget: MemoryBudget,
}

/// Validated load plan produced before allocating model resources.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LoadPlan {
    /// Model descriptor accepted by the backend, including source scalar metadata.
    pub descriptor: ModelDescriptor,
    /// Scalar type selected for backend execution tensors.
    pub execution_scalar_type: ScalarType,
    /// Expected accounting footprint accepted for the load.
    pub expected_footprint: MemoryFootprint,
}

/// Validated configuration for one inference sequence.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SequenceConfiguration {
    /// Maximum token positions retained by the sequence.
    pub maximum_tokens: NonZeroU32,
    /// Maximum prompt tokens accepted by one prefill call.
    pub maximum_prefill_batch: NonZeroU32,
}

impl SequenceConfiguration {
    /// Creates a sequence configuration from validated non-zero bounds.
    #[must_use]
    pub const fn new(maximum_tokens: NonZeroU32, maximum_prefill_batch: NonZeroU32) -> Self {
        Self {
            maximum_tokens,
            maximum_prefill_batch,
        }
    }
}

/// Cold-path sequence creation plan.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SequencePlan {
    /// Accepted sequence configuration.
    pub configuration: SequenceConfiguration,
    /// Expected sequence-specific memory footprint.
    pub expected_footprint: MemoryFootprint,
    /// Required logits elements for each decode operation.
    pub logits_capacity: usize,
}
