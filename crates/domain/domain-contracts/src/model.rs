//! Model metadata, capabilities, memory plans, and sequence configuration.

use core::num::NonZeroU32;

use crate::{BackendId, ByteCount, DeviceId, ModelHandle};

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

/// Scalar representation used for configuration declarations, observed tensors, or execution.
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

/// Fixed-size set of scalar categories observed or accepted by a portable boundary.
///
/// Every [`ScalarType::Other`] value occupies the same `Other` category bit. The
/// six low-order bits returned by [`Self::bits`] correspond, in order, to
/// [`ScalarType::F32`], [`ScalarType::F16`], [`ScalarType::Bf16`],
/// [`ScalarType::I8`], [`ScalarType::U8`], and [`ScalarType::Other`].
#[repr(transparent)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct ScalarTypeSet(u8);

impl ScalarTypeSet {
    /// Empty scalar-type set.
    pub const EMPTY: Self = Self(0);

    /// Creates a set containing one scalar category.
    #[must_use]
    pub const fn from_scalar(scalar_type: ScalarType) -> Self {
        Self(Self::scalar_bit(scalar_type))
    }

    /// Returns the raw category bits.
    #[must_use]
    pub const fn bits(self) -> u8 {
        self.0
    }

    /// Returns whether the set contains no scalar categories.
    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    /// Returns whether the set contains the category for `scalar_type`.
    #[must_use]
    pub const fn contains(self, scalar_type: ScalarType) -> bool {
        (self.0 & Self::scalar_bit(scalar_type)) != 0
    }

    /// Inserts the category for `scalar_type`.
    pub const fn insert(&mut self, scalar_type: ScalarType) {
        self.0 |= Self::scalar_bit(scalar_type);
    }

    /// Returns the union of two scalar-type sets.
    #[must_use]
    pub const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    /// Returns whether every category in this set is present in `other`.
    #[must_use]
    pub const fn is_subset_of(self, other: Self) -> bool {
        (self.0 & other.0) == self.0
    }

    const fn scalar_bit(scalar_type: ScalarType) -> u8 {
        match scalar_type {
            ScalarType::F32 => 1 << 0,
            ScalarType::F16 => 1 << 1,
            ScalarType::Bf16 => 1 << 2,
            ScalarType::I8 => 1 << 3,
            ScalarType::U8 => 1 << 4,
            ScalarType::Other(_) => 1 << 5,
        }
    }
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

/// Deterministic byte accounting for a named ownership or reservation phase.
///
/// The field or operation carrying a footprint defines its exact meaning, such
/// as final post-load ownership, a loading transaction peak, or a sequence
/// logical-payload reservation. "Exact" means that the declared phase's checked
/// arithmetic is exact; it does not turn a conservative reservation bound into
/// an instantaneous allocation measurement. Rates and other planning
/// coefficients belong outside the footprint.
///
/// Portable footprints exclude physical RSS/VRAM, allocator rounding,
/// fragmentation and pools, CUDA context/driver allocations, native workspaces,
/// and serialized headers, configuration, and other metadata unless a carrying
/// field explicitly defines otherwise.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MemoryFootprint {
    host_weight_bytes: ByteCount,
    device_weight_bytes: ByteCount,
    host_working_bytes: ByteCount,
    device_working_bytes: ByteCount,
}

impl MemoryFootprint {
    /// Empty deterministic ownership.
    pub const ZERO: Self = Self {
        host_weight_bytes: ByteCount::ZERO,
        device_weight_bytes: ByteCount::ZERO,
        host_working_bytes: ByteCount::ZERO,
        device_working_bytes: ByteCount::ZERO,
    };

    /// Returns a footprint containing only host-resident weights.
    #[must_use]
    pub const fn host_weights(bytes: ByteCount) -> Self {
        Self::ZERO.with_host_weight_bytes(bytes)
    }

    /// Returns a footprint containing only device-resident weights.
    #[must_use]
    pub const fn device_weights(bytes: ByteCount) -> Self {
        Self::ZERO.with_device_weight_bytes(bytes)
    }

    /// Returns a footprint containing only host working payload.
    #[must_use]
    pub const fn host_working(bytes: ByteCount) -> Self {
        Self::ZERO.with_host_working_bytes(bytes)
    }

    /// Returns a footprint containing only device working payload.
    #[must_use]
    pub const fn device_working(bytes: ByteCount) -> Self {
        Self::ZERO.with_device_working_bytes(bytes)
    }

    /// Replaces the host-resident weight component.
    #[must_use]
    pub const fn with_host_weight_bytes(mut self, bytes: ByteCount) -> Self {
        self.host_weight_bytes = bytes;
        self
    }

    /// Replaces the device-resident weight component.
    #[must_use]
    pub const fn with_device_weight_bytes(mut self, bytes: ByteCount) -> Self {
        self.device_weight_bytes = bytes;
        self
    }

    /// Replaces the host working component.
    #[must_use]
    pub const fn with_host_working_bytes(mut self, bytes: ByteCount) -> Self {
        self.host_working_bytes = bytes;
        self
    }

    /// Replaces the device working component.
    #[must_use]
    pub const fn with_device_working_bytes(mut self, bytes: ByteCount) -> Self {
        self.device_working_bytes = bytes;
        self
    }

    /// Returns host-resident weight ownership.
    #[must_use]
    pub const fn host_weight_bytes(self) -> ByteCount {
        self.host_weight_bytes
    }

    /// Returns device-resident weight ownership.
    #[must_use]
    pub const fn device_weight_bytes(self) -> ByteCount {
        self.device_weight_bytes
    }

    /// Returns host working payload ownership or reservation.
    #[must_use]
    pub const fn host_working_bytes(self) -> ByteCount {
        self.host_working_bytes
    }

    /// Returns device working payload ownership or reservation.
    #[must_use]
    pub const fn device_working_bytes(self) -> ByteCount {
        self.device_working_bytes
    }

    /// Returns the exact component-wise sum, or `None` if any component overflows.
    #[must_use]
    pub const fn checked_add(self, other: Self) -> Option<Self> {
        match (
            self.host_weight_bytes.checked_add(other.host_weight_bytes),
            self.device_weight_bytes
                .checked_add(other.device_weight_bytes),
            self.host_working_bytes
                .checked_add(other.host_working_bytes),
            self.device_working_bytes
                .checked_add(other.device_working_bytes),
        ) {
            (
                Some(host_weight_bytes),
                Some(device_weight_bytes),
                Some(host_working_bytes),
                Some(device_working_bytes),
            ) => Some(Self {
                host_weight_bytes,
                device_weight_bytes,
                host_working_bytes,
                device_working_bytes,
            }),
            _ => None,
        }
    }

    /// Returns the exact component-wise difference, or `None` if any component underflows.
    #[must_use]
    pub const fn checked_sub(self, other: Self) -> Option<Self> {
        match (
            self.host_weight_bytes.checked_sub(other.host_weight_bytes),
            self.device_weight_bytes
                .checked_sub(other.device_weight_bytes),
            self.host_working_bytes
                .checked_sub(other.host_working_bytes),
            self.device_working_bytes
                .checked_sub(other.device_working_bytes),
        ) {
            (
                Some(host_weight_bytes),
                Some(device_weight_bytes),
                Some(host_working_bytes),
                Some(device_working_bytes),
            ) => Some(Self {
                host_weight_bytes,
                device_weight_bytes,
                host_working_bytes,
                device_working_bytes,
            }),
            _ => None,
        }
    }

    /// Returns the component-wise maximum of two ownership phases.
    #[must_use]
    pub const fn component_max(self, other: Self) -> Self {
        Self {
            host_weight_bytes: self
                .host_weight_bytes
                .component_max(other.host_weight_bytes),
            device_weight_bytes: self
                .device_weight_bytes
                .component_max(other.device_weight_bytes),
            host_working_bytes: self
                .host_working_bytes
                .component_max(other.host_working_bytes),
            device_working_bytes: self
                .device_working_bytes
                .component_max(other.device_working_bytes),
        }
    }

    /// Returns whether every component is at least the corresponding required component.
    #[must_use]
    pub const fn contains_components(self, required: Self) -> bool {
        self.host_weight_bytes.contains(required.host_weight_bytes)
            && self
                .device_weight_bytes
                .contains(required.device_weight_bytes)
            && self
                .host_working_bytes
                .contains(required.host_working_bytes)
            && self
                .device_working_bytes
                .contains(required.device_working_bytes)
    }

    /// Returns the exact host byte total, or `None` on overflow.
    #[must_use]
    pub const fn checked_host_bytes(self) -> Option<ByteCount> {
        self.host_weight_bytes.checked_add(self.host_working_bytes)
    }

    /// Returns the exact device byte total, or `None` on overflow.
    #[must_use]
    pub const fn checked_device_bytes(self) -> Option<ByteCount> {
        self.device_weight_bytes
            .checked_add(self.device_working_bytes)
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
    host_bytes: ByteCount,
    device_bytes: ByteCount,
}

impl MemoryBudget {
    /// Empty admission budget.
    pub const ZERO: Self = Self {
        host_bytes: ByteCount::ZERO,
        device_bytes: ByteCount::ZERO,
    };

    /// An unbounded-by-policy budget in both portable memory domains.
    pub const UNLIMITED: Self = Self {
        host_bytes: ByteCount::MAX,
        device_bytes: ByteCount::MAX,
    };

    /// Replaces the host-memory limit.
    #[must_use]
    pub const fn with_host_bytes(mut self, bytes: ByteCount) -> Self {
        self.host_bytes = bytes;
        self
    }

    /// Replaces the device-memory limit.
    #[must_use]
    pub const fn with_device_bytes(mut self, bytes: ByteCount) -> Self {
        self.device_bytes = bytes;
        self
    }

    /// Returns the host-memory limit.
    #[must_use]
    pub const fn host_bytes(self) -> ByteCount {
        self.host_bytes
    }

    /// Returns the device-memory limit.
    #[must_use]
    pub const fn device_bytes(self) -> ByteCount {
        self.device_bytes
    }
}

/// Immutable model metadata exposed after inspection or load.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ModelMetadata {
    /// Model architecture family.
    pub architecture: ModelArchitecture,
    /// Scalar type declared by immutable model configuration, when recognized.
    ///
    /// This is evidence about producer intent only. It does not prove tensor
    /// homogeneity, report any scalar type read from serialized tensor headers,
    /// or select the scalar type used for backend execution. `None` means that
    /// inspection found no recognized, trustworthy configuration declaration.
    pub configuration_declared_scalar_type: Option<ScalarType>,
    /// Scalar categories read directly from serialized tensor headers.
    ///
    /// This set records observed artifact facts and can contain multiple scalar
    /// categories. It is independent of both the optional configuration
    /// declaration and the scalar type later selected for backend execution.
    pub observed_tensor_scalar_types: ScalarTypeSet,
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
    /// Inspection-phase, device-independent deterministic tensor footprint estimate.
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

/// Exact transaction plan exposed by a prepared model load.
///
/// [`Self::final_footprint`] and [`Self::loading_peak_footprint`] are exact,
/// explicitly named byte-ownership phases, not estimates or rates.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LoadPlan {
    /// Exact caller configuration accepted and bound to this load transaction.
    pub accepted_configuration: LoadConfiguration,
    /// Model descriptor accepted by the backend for this transaction.
    pub descriptor: ModelDescriptor,
    /// Scalar type selected for materialized backend execution tensors.
    pub execution_scalar_type: ScalarType,
    /// Exact final post-materialization ownership claimed for the loaded model.
    pub final_footprint: MemoryFootprint,
    /// Exact component-wise ownership peak for this loading transaction.
    ///
    /// This phase includes final ownership and transient tensor staging,
    /// conversion, or duplicate-residency headroom required by the selected
    /// loading algorithm.
    pub loading_peak_footprint: MemoryFootprint,
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

/// Backend-owned memory reservation for one accepted sequence.
///
/// Persistent payload is the maximum sequence-owned logical payload retained
/// between backend calls. Transient payload is additional logical payload or
/// source-transfer headroom that may be live during sequence creation or one
/// permitted backend call. Their checked sum is the aggregate reservation E0
/// admits before native sequence creation.
///
/// Caller-owned logits, sampling, history, stop, output, and terminal-state
/// buffers are not part of this value. Physical RSS/VRAM, allocator rounding,
/// fragmentation and pools, accelerator contexts, native library workspaces,
/// and other non-deterministic observations are also outside this contract.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SequenceReservation {
    persistent_footprint: MemoryFootprint,
    transient_footprint: MemoryFootprint,
    total_footprint: MemoryFootprint,
}

impl SequenceReservation {
    /// Constructs a reservation only when its aggregate can be represented.
    #[must_use]
    pub const fn checked(
        persistent_footprint: MemoryFootprint,
        transient_footprint: MemoryFootprint,
    ) -> Option<Self> {
        if persistent_footprint.checked_host_bytes().is_none()
            || persistent_footprint.checked_device_bytes().is_none()
            || transient_footprint.checked_host_bytes().is_none()
            || transient_footprint.checked_device_bytes().is_none()
        {
            return None;
        }
        match persistent_footprint.checked_add(transient_footprint) {
            Some(total_footprint)
                if total_footprint.checked_host_bytes().is_some()
                    && total_footprint.checked_device_bytes().is_some() =>
            {
                Some(Self {
                    persistent_footprint,
                    transient_footprint,
                    total_footprint,
                })
            }
            None => None,
            Some(_) => None,
        }
    }

    /// Returns the persistent payload retained between calls.
    #[must_use]
    pub const fn persistent_footprint(self) -> MemoryFootprint {
        self.persistent_footprint
    }

    /// Returns the additional transient payload covered by the reservation.
    #[must_use]
    pub const fn transient_footprint(self) -> MemoryFootprint {
        self.transient_footprint
    }

    /// Returns the constructor-owned checked aggregate reservation.
    #[must_use]
    pub const fn total_footprint(self) -> MemoryFootprint {
        self.total_footprint
    }
}

/// Cold-path sequence reservation plan.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SequencePlan {
    /// Accepted sequence configuration.
    pub configuration: SequenceConfiguration,
    /// Checked backend-owned reservation for the accepted sequence plan.
    ///
    /// Component arithmetic is exact, while each component may deliberately be
    /// a documented conservative upper bound rather than an instantaneous
    /// allocation measurement.
    pub reservation: SequenceReservation,
    /// Required caller-owned logits elements for each decode operation.
    pub logits_capacity: usize,
}
