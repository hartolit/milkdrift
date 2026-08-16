pub(crate) use std::collections::HashMap;
pub(crate) use std::fs;
pub(crate) use std::io::{Read, Seek, SeekFrom, Write};
pub(crate) use std::num::NonZeroU32;
pub(crate) use std::path::{Path, PathBuf};
pub(crate) use std::sync::atomic::{AtomicU64, Ordering};

pub(crate) use candle_backend::{
    CandleExpectedContentIdentity, CandleLlamaLoader, CandleLlamaModel, CandleLlamaPreparedLoad,
    CandleLlamaSequence, CandleLlamaSource, CandleWeightShard,
};
pub(crate) use candle_core::{DType, Device, Tensor};
pub(crate) use domain_contracts::{
    BackendFailureKind, BackendId, BackendSequence, ByteCount, CancellationStatus,
    CapacityResource, DecodeBuffers, DecodeInput, DecodeOutcome, DeviceId, DeviceKind,
    ExecutionDevice, LoadConfiguration, LoadError, LoadFailureStage, LoadPlan, LoadedModel,
    MemoryBudget, MemoryFootprint, ModelGeneration, ModelHandle, ModelId, ModelLoader,
    PrefillBuffers, PrefillInput, PrefillOutcome, PreparedLoad, ScalarType, ScalarTypeSet,
    SequenceConfiguration, SequenceId, SequenceState, TensorFailureLocation, TokenId,
    decode_checked, prefill_checked,
};
pub(crate) use serde_json::{Map as JsonMap, Value as JsonValue, json};
pub(crate) use sha2::{Digest, Sha256};

pub(crate) const BACKEND: BackendId = BackendId::new(1);
pub(crate) const REQUIRED_ELEMENTS: u64 = 920;
pub(crate) const VOCABULARY_SIZE: usize = 16;
pub(crate) const PER_SHARD_HEADER_LIMIT: u64 = 8 * 1024 * 1024;
pub(crate) const F32_SEQUENCE_PERSISTENT_BYTES: u64 = 1_504;
pub(crate) const F32_SEQUENCE_TRANSIENT_BYTES: u64 = 6_052;
pub(crate) const F32_SEQUENCE_HOST_WORKING_BYTES: u64 = 7_556;
pub(crate) const HALF_SEQUENCE_PERSISTENT_BYTES: u64 = 864;
pub(crate) const HALF_SEQUENCE_TRANSIENT_BYTES: u64 = 6_180;
pub(crate) const HALF_SEQUENCE_HOST_WORKING_BYTES: u64 = 7_044;

pub(crate) const CPU_F32_FINAL: MemoryFootprint = host_weights(3_680);
pub(crate) const CPU_F32_LOADING_PEAK: MemoryFootprint =
    host_weights(3_680).with_host_working_bytes(ByteCount::from_u64(65_763));
pub(crate) const CPU_F16_FINAL: MemoryFootprint = host_weights(1_840);
pub(crate) const CPU_F16_LOADING_PEAK: MemoryFootprint =
    host_weights(1_840).with_host_working_bytes(ByteCount::from_u64(65_649));
pub(crate) const CPU_MIXED_F16_F32_LOADING_PEAK: MemoryFootprint =
    host_weights(1_840).with_host_working_bytes(ByteCount::from_u64(65_665));
pub(crate) const CPU_BF16_TO_F32_LOADING_PEAK: MemoryFootprint =
    host_weights(3_680).with_host_working_bytes(ByteCount::from_u64(65_632));
pub(crate) const CPU_MIXED_BF16_F32_LOADING_PEAK: MemoryFootprint =
    host_weights(3_680).with_host_working_bytes(ByteCount::from_u64(65_664));

const fn host_weights(value: u64) -> MemoryFootprint {
    MemoryFootprint::host_weights(ByteCount::from_u64(value))
}

pub(crate) static NEXT_FIXTURE: AtomicU64 = AtomicU64::new(0);

pub(crate) type TestResult<T = ()> = Result<T, String>;

pub(crate) const SOURCE_IDENTITY_MISMATCH_CODE: u32 = 32;
pub(crate) const SOURCE_IDENTITY_LENGTH_CODE: u32 = 45;
pub(crate) const REQUIRED_TENSOR_CODE: u32 = 26;
pub(crate) const UNSUPPORTED_SCALAR_CODE: u32 = 21;
pub(crate) const MODEL_NORM_NAME_HASH: u64 = 0xd6d6_e5af_d116_04e4;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RequiredProfile {
    F32,
    F16,
    Bf16,
    MixedF16F32,
    MixedBf16F32,
    MixedF16Bf16,
    UnsupportedU8,
    InvalidNormShape,
}

impl RequiredProfile {
    pub(crate) fn dtype_for(self, name: &str) -> DType {
        match self {
            Self::F32 | Self::InvalidNormShape => DType::F32,
            Self::F16 => DType::F16,
            Self::Bf16 => DType::BF16,
            Self::MixedF16F32 => {
                if name == "model.norm.weight" {
                    DType::F32
                } else {
                    DType::F16
                }
            }
            Self::MixedBf16F32 => {
                if name == "model.norm.weight" {
                    DType::F32
                } else {
                    DType::BF16
                }
            }
            Self::MixedF16Bf16 => {
                if name == "model.norm.weight" {
                    DType::F16
                } else {
                    DType::BF16
                }
            }
            Self::UnsupportedU8 => {
                if name == "model.norm.weight" {
                    DType::U8
                } else {
                    DType::F32
                }
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ConfigDeclaration {
    Absent,
    F32,
    F16,
    Bf16,
    Unsupported,
    Conflict,
}

impl ConfigDeclaration {
    pub(crate) fn recognized(self) -> TestResult<Option<ScalarType>> {
        match self {
            Self::Absent => Ok(None),
            Self::F32 => Ok(Some(ScalarType::F32)),
            Self::F16 => Ok(Some(ScalarType::F16)),
            Self::Bf16 => Ok(Some(ScalarType::Bf16)),
            Self::Unsupported | Self::Conflict => {
                Err("test requested a recognized value for an invalid declaration".to_owned())
            }
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct ExtraTensor {
    name: &'static str,
    dtype: &'static str,
    elements: usize,
    bytes_per_element: usize,
}

impl ExtraTensor {
    pub(crate) const fn new(
        name: &'static str,
        dtype: &'static str,
        elements: usize,
        bytes_per_element: usize,
    ) -> Self {
        Self {
            name,
            dtype,
            elements,
            bytes_per_element,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) enum PreparedMutation {
    Payload,
    SameLengthHeader,
    Truncate,
    Extend,
}

mod assertions;
mod fixture;
mod model;
mod source;

pub(crate) use assertions::*;
pub(crate) use fixture::*;
pub(crate) use model::*;
pub(crate) use source::*;
