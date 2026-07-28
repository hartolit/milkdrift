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

/// Scalar representation used by model tensors or execution buffers.
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
    /// GGUF-defined quantization code.
    Gguf(u16),
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

/// Estimated or observed memory footprint in bytes.
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
    /// Weight scalar type.
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
    /// Estimated memory footprint.
    pub estimated_footprint: MemoryFootprint,
}

/// Cold-path configuration for loading one model instance.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LoadConfiguration {
    /// Handle assigned by the owning inference runtime.
    pub handle: ModelHandle,
    /// Target device selected by an adapter or application.
    pub device: DeviceId,
    /// Device category.
    pub device_kind: DeviceKind,
    /// Hard admission-control budget.
    pub memory_budget: MemoryBudget,
}

/// Validated load plan produced before allocating model resources.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LoadPlan {
    /// Model descriptor accepted by the backend.
    pub descriptor: ModelDescriptor,
    /// Expected resource footprint after load.
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
