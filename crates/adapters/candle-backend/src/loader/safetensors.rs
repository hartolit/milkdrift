//! Allocation-bounded Safetensors JSON decoding and retained tensor facts.

use std::fmt::Formatter;
use std::fs::File;
use std::mem::size_of;

use candle_core::DType;
use domain_contracts::{BackendId, LoadError, ScalarType};
use safetensors::tensor::Dtype as SafeDtype;
use serde::de::{self, DeserializeSeed, IgnoredAny, MapAccess, SeqAccess, Visitor};
use serde::{Deserialize, Deserializer};

use crate::failure::{
    CODE_DUPLICATE_TENSOR, CODE_HEADER_BOUNDS, CODE_HEADER_DECODE, CODE_INSPECTION_ALLOCATION,
    CODE_INSPECTION_INVENTORY_LIMIT, CODE_METADATA_LIMIT, CODE_NUMERIC_OVERFLOW, CODE_TENSOR_LIMIT,
};
use crate::source::CandleShardIdentity;

use super::identity::EstablishedShardIdentity;
use super::{host_memory_failure, invalid_model_failure};

const MAX_FIXED_RANK: usize = 8;
const SHA256_BYTES: usize = 32;

/// One production limit set governs all header-side growth. Values provide
/// realistic Llama headroom while keeping hostile JSON amplification bounded.
#[derive(Clone, Copy, Debug)]
pub(super) struct InspectionLimits {
    pub(super) shards: usize,
    pub(super) per_shard_header_bytes: u64,
    pub(super) aggregate_header_bytes: u64,
    pub(super) tensors: usize,
    pub(super) tensor_name_bytes: usize,
    pub(super) aggregate_tensor_name_bytes: u64,
    pub(super) rank: usize,
    pub(super) shape_dimension_extent: u64,
    pub(super) aggregate_shape_dimensions: usize,
    pub(super) metadata_entries: usize,
    pub(super) metadata_key_bytes: usize,
    pub(super) metadata_value_bytes: usize,
    pub(super) aggregate_metadata_string_bytes: u64,
    pub(super) final_inventory_bytes: u64,
}

impl InspectionLimits {
    pub(super) const PRODUCTION: Self = Self {
        shards: 256,
        per_shard_header_bytes: 8 * 1024 * 1024,
        aggregate_header_bytes: 64 * 1024 * 1024,
        tensors: 16_384,
        tensor_name_bytes: 512,
        aggregate_tensor_name_bytes: 8 * 1024 * 1024,
        rank: 8,
        // This is four times the largest currently reviewed Llama vocabulary
        // dimension and still accommodates long-context auxiliary shapes.
        shape_dimension_extent: 1_048_576,
        aggregate_shape_dimensions: 131_072,
        metadata_entries: 1_024,
        metadata_key_bytes: 256,
        metadata_value_bytes: 4 * 1024,
        aggregate_metadata_string_bytes: 4 * 1024 * 1024,
        final_inventory_bytes: 64 * 1024 * 1024,
    };
}

#[cfg(test)]
thread_local! {
    static TEST_INSPECTION_ALLOCATION_FAILURES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

#[derive(Debug)]
pub(super) struct InspectedShard {
    pub(super) file: File,
    pub(super) file_length: u64,
    pub(super) data_start: u64,
    pub(super) prefix_header_sha256: [u8; SHA256_BYTES],
    pub(super) source_identity: CandleShardIdentity,
    pub(super) established_identity: Option<EstablishedShardIdentity>,
    pub(super) tensors: Vec<InspectedTensor>,
}

#[derive(Debug)]
pub(super) struct InspectedTensor {
    pub(super) name: String,
    pub(super) source_dtype: SourceTensorDType,
    pub(super) shape: TensorShape,
    pub(super) data_start: u64,
    pub(super) source_bytes: u64,
    pub(super) element_count: u64,
    pub(super) required: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct TensorShape {
    dimensions: [usize; MAX_FIXED_RANK],
    rank: u8,
}

impl TensorShape {
    const EMPTY: Self = Self {
        dimensions: [0; MAX_FIXED_RANK],
        rank: 0,
    };

    pub(super) fn as_slice(&self) -> &[usize] {
        self.dimensions
            .get(..usize::from(self.rank))
            .unwrap_or_default()
    }

    #[cfg(test)]
    pub(super) fn from_slice(dimensions: &[usize]) -> Option<Self> {
        let mut shape = Self::EMPTY;
        for dimension in dimensions {
            shape.push(*dimension).ok()?;
        }
        Some(shape)
    }

    fn push(&mut self, dimension: usize) -> Result<(), ()> {
        let rank = usize::from(self.rank);
        let slot = self.dimensions.get_mut(rank).ok_or(())?;
        *slot = dimension;
        self.rank = self.rank.checked_add(1).ok_or(())?;
        Ok(())
    }
}

/// Structural representation of every dtype understood by Safetensors 0.8.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum SourceTensorDType {
    Bool,
    F4,
    F6E2M3,
    F6E3M2,
    U8,
    I8,
    F8E5M2,
    F8E4M3,
    F8E8M0,
    F8E4M3Fnuz,
    F8E5M2Fnuz,
    I16,
    U16,
    F16,
    Bf16,
    I32,
    U32,
    F32,
    C64,
    F64,
    I64,
    U64,
}

impl SourceTensorDType {
    const fn from_safetensors(dtype: SafeDtype) -> Option<Self> {
        match dtype {
            SafeDtype::BOOL => Some(Self::Bool),
            SafeDtype::F4 => Some(Self::F4),
            SafeDtype::F6_E2M3 => Some(Self::F6E2M3),
            SafeDtype::F6_E3M2 => Some(Self::F6E3M2),
            SafeDtype::U8 => Some(Self::U8),
            SafeDtype::I8 => Some(Self::I8),
            SafeDtype::F8_E5M2 => Some(Self::F8E5M2),
            SafeDtype::F8_E4M3 => Some(Self::F8E4M3),
            SafeDtype::F8_E8M0 => Some(Self::F8E8M0),
            SafeDtype::F8_E4M3FNUZ => Some(Self::F8E4M3Fnuz),
            SafeDtype::F8_E5M2FNUZ => Some(Self::F8E5M2Fnuz),
            SafeDtype::I16 => Some(Self::I16),
            SafeDtype::U16 => Some(Self::U16),
            SafeDtype::F16 => Some(Self::F16),
            SafeDtype::BF16 => Some(Self::Bf16),
            SafeDtype::I32 => Some(Self::I32),
            SafeDtype::U32 => Some(Self::U32),
            SafeDtype::F32 => Some(Self::F32),
            SafeDtype::C64 => Some(Self::C64),
            SafeDtype::F64 => Some(Self::F64),
            SafeDtype::I64 => Some(Self::I64),
            SafeDtype::U64 => Some(Self::U64),
            _ => None,
        }
    }

    pub(super) const fn scalar_type(self) -> ScalarType {
        match self {
            Self::F32 => ScalarType::F32,
            Self::F16 => ScalarType::F16,
            Self::Bf16 => ScalarType::Bf16,
            Self::I8 => ScalarType::I8,
            Self::U8 => ScalarType::U8,
            Self::Bool => ScalarType::Other(1),
            Self::F4 => ScalarType::Other(2),
            Self::F6E2M3 => ScalarType::Other(3),
            Self::F6E3M2 => ScalarType::Other(4),
            Self::F8E5M2 => ScalarType::Other(5),
            Self::F8E4M3 => ScalarType::Other(6),
            Self::F8E8M0 => ScalarType::Other(7),
            Self::F8E4M3Fnuz => ScalarType::Other(8),
            Self::F8E5M2Fnuz => ScalarType::Other(9),
            Self::I16 => ScalarType::Other(10),
            Self::U16 => ScalarType::Other(11),
            Self::I32 => ScalarType::Other(12),
            Self::U32 => ScalarType::Other(13),
            Self::C64 => ScalarType::Other(14),
            Self::F64 => ScalarType::Other(15),
            Self::I64 => ScalarType::Other(16),
            Self::U64 => ScalarType::Other(17),
        }
    }

    pub(super) const fn executable_dtype(self) -> Option<DType> {
        match self {
            Self::F32 => Some(DType::F32),
            Self::F16 => Some(DType::F16),
            Self::Bf16 => Some(DType::BF16),
            _ => None,
        }
    }

    pub(super) const fn alignment(self) -> Option<u64> {
        match self {
            Self::F32 => Some(4),
            Self::F16 | Self::Bf16 => Some(2),
            _ => None,
        }
    }

    pub(super) const fn bits_per_element(self) -> u64 {
        match self {
            Self::F4 => 4,
            Self::F6E2M3 | Self::F6E3M2 => 6,
            Self::Bool
            | Self::U8
            | Self::I8
            | Self::F8E5M2
            | Self::F8E4M3
            | Self::F8E8M0
            | Self::F8E4M3Fnuz
            | Self::F8E5M2Fnuz => 8,
            Self::I16 | Self::U16 | Self::F16 | Self::Bf16 => 16,
            Self::I32 | Self::U32 | Self::F32 => 32,
            Self::C64 | Self::F64 | Self::I64 | Self::U64 => 64,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub(super) enum ParseFailure {
    DuplicateTensor,
    HeaderBounds,
    NumericOverflow,
    TensorLimit,
    MetadataLimit,
    InventoryLimit,
    Allocation,
}

pub(super) struct InspectionBudget<'a> {
    limits: &'a InspectionLimits,
    tensor_count: usize,
    aggregate_tensor_name_bytes: u64,
    aggregate_shape_dimensions: usize,
    metadata_entries: usize,
    aggregate_metadata_string_bytes: u64,
    final_inventory_bytes: u64,
    parse_failure: Option<ParseFailure>,
}

impl<'a> InspectionBudget<'a> {
    pub(super) const fn new(limits: &'a InspectionLimits) -> Self {
        Self {
            limits,
            tensor_count: 0,
            aggregate_tensor_name_bytes: 0,
            aggregate_shape_dimensions: 0,
            metadata_entries: 0,
            aggregate_metadata_string_bytes: 0,
            final_inventory_bytes: 0,
            parse_failure: None,
        }
    }

    fn fail<E: de::Error>(&mut self, failure: ParseFailure, message: &'static str) -> E {
        if self.parse_failure.is_none() {
            self.parse_failure = Some(failure);
        }
        E::custom(message)
    }

    fn begin_tensor_name<E: de::Error>(&mut self, length: usize) -> Result<(), E> {
        self.tensor_count = self
            .tensor_count
            .checked_add(1)
            .ok_or_else(|| self.fail(ParseFailure::TensorLimit, "tensor count overflow"))?;
        if self.tensor_count > self.limits.tensors || length > self.limits.tensor_name_bytes {
            return Err(self.fail(ParseFailure::TensorLimit, "tensor limit exceeded"));
        }
        let length_u64 = u64::try_from(length)
            .map_err(|_| self.fail(ParseFailure::TensorLimit, "tensor name length overflow"))?;
        self.aggregate_tensor_name_bytes = self
            .aggregate_tensor_name_bytes
            .checked_add(length_u64)
            .ok_or_else(|| self.fail(ParseFailure::TensorLimit, "tensor names overflow"))?;
        if self.aggregate_tensor_name_bytes > self.limits.aggregate_tensor_name_bytes {
            return Err(self.fail(ParseFailure::TensorLimit, "tensor names limit exceeded"));
        }
        self.add_inventory_for_parse(size_of::<InspectedTensor>())?;
        self.add_inventory_for_parse(length)?;
        Ok(())
    }

    fn add_shape_dimension<E: de::Error>(&mut self) -> Result<(), E> {
        self.aggregate_shape_dimensions = self
            .aggregate_shape_dimensions
            .checked_add(1)
            .ok_or_else(|| self.fail(ParseFailure::TensorLimit, "shape count overflow"))?;
        if self.aggregate_shape_dimensions > self.limits.aggregate_shape_dimensions {
            return Err(self.fail(ParseFailure::TensorLimit, "shape limit exceeded"));
        }
        Ok(())
    }

    fn add_metadata_string<E: de::Error>(
        &mut self,
        length: usize,
        maximum: usize,
        is_key: bool,
    ) -> Result<(), E> {
        if length > maximum {
            return Err(self.fail(
                ParseFailure::MetadataLimit,
                "metadata string limit exceeded",
            ));
        }
        if is_key {
            self.metadata_entries = self
                .metadata_entries
                .checked_add(1)
                .ok_or_else(|| self.fail(ParseFailure::MetadataLimit, "metadata count overflow"))?;
            if self.metadata_entries > self.limits.metadata_entries {
                return Err(self.fail(ParseFailure::MetadataLimit, "metadata count exceeded"));
            }
        }
        let length_u64 = u64::try_from(length)
            .map_err(|_| self.fail(ParseFailure::MetadataLimit, "metadata length overflow"))?;
        self.aggregate_metadata_string_bytes = self
            .aggregate_metadata_string_bytes
            .checked_add(length_u64)
            .ok_or_else(|| self.fail(ParseFailure::MetadataLimit, "metadata bytes overflow"))?;
        if self.aggregate_metadata_string_bytes > self.limits.aggregate_metadata_string_bytes {
            return Err(self.fail(ParseFailure::MetadataLimit, "metadata bytes exceeded"));
        }
        Ok(())
    }

    pub(super) fn add_inventory(
        &mut self,
        backend: BackendId,
        bytes: usize,
    ) -> Result<(), LoadError> {
        let bytes = u64::try_from(bytes)
            .map_err(|_| invalid_model_failure(backend, CODE_NUMERIC_OVERFLOW))?;
        self.final_inventory_bytes = self
            .final_inventory_bytes
            .checked_add(bytes)
            .ok_or_else(|| invalid_model_failure(backend, CODE_NUMERIC_OVERFLOW))?;
        if self.final_inventory_bytes > self.limits.final_inventory_bytes {
            return Err(invalid_model_failure(
                backend,
                CODE_INSPECTION_INVENTORY_LIMIT,
            ));
        }
        Ok(())
    }

    fn add_inventory_for_parse<E: de::Error>(&mut self, bytes: usize) -> Result<(), E> {
        let bytes = u64::try_from(bytes)
            .map_err(|_| self.fail(ParseFailure::InventoryLimit, "inventory overflow"))?;
        self.final_inventory_bytes = self
            .final_inventory_bytes
            .checked_add(bytes)
            .ok_or_else(|| self.fail(ParseFailure::InventoryLimit, "inventory overflow"))?;
        if self.final_inventory_bytes > self.limits.final_inventory_bytes {
            return Err(self.fail(ParseFailure::InventoryLimit, "inventory limit exceeded"));
        }
        Ok(())
    }
}

pub(super) fn parse_header(
    backend: BackendId,
    header: &[u8],
    budget: &mut InspectionBudget<'_>,
) -> Result<Vec<InspectedTensor>, LoadError> {
    let mut deserializer = serde_json::Deserializer::from_slice(header);
    let parsed = HeaderSeed {
        budget: &mut *budget,
    }
    .deserialize(&mut deserializer);
    let Ok(tensors) = parsed else {
        return Err(map_parse_failure(backend, budget.parse_failure));
    };
    if deserializer.end().is_err() {
        return Err(invalid_model_failure(backend, CODE_HEADER_DECODE));
    }
    Ok(tensors)
}

pub(super) fn map_parse_failure(
    backend: BackendId,
    failure_kind: Option<ParseFailure>,
) -> LoadError {
    match failure_kind {
        Some(ParseFailure::DuplicateTensor) => {
            invalid_model_failure(backend, CODE_DUPLICATE_TENSOR)
        }
        Some(ParseFailure::HeaderBounds) => invalid_model_failure(backend, CODE_HEADER_BOUNDS),
        Some(ParseFailure::NumericOverflow) => {
            invalid_model_failure(backend, CODE_NUMERIC_OVERFLOW)
        }
        Some(ParseFailure::TensorLimit) => invalid_model_failure(backend, CODE_TENSOR_LIMIT),
        Some(ParseFailure::MetadataLimit) => invalid_model_failure(backend, CODE_METADATA_LIMIT),
        Some(ParseFailure::InventoryLimit) => {
            invalid_model_failure(backend, CODE_INSPECTION_INVENTORY_LIMIT)
        }
        Some(ParseFailure::Allocation) => host_memory_failure(backend, CODE_INSPECTION_ALLOCATION),
        None => invalid_model_failure(backend, CODE_HEADER_DECODE),
    }
}

struct HeaderSeed<'a, 'limits> {
    budget: &'a mut InspectionBudget<'limits>,
}

impl<'de> DeserializeSeed<'de> for HeaderSeed<'_, '_> {
    type Value = Vec<InspectedTensor>;

    fn deserialize<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_map(HeaderVisitor {
            budget: self.budget,
        })
    }
}

struct HeaderVisitor<'a, 'limits> {
    budget: &'a mut InspectionBudget<'limits>,
}

impl<'de> Visitor<'de> for HeaderVisitor<'_, '_> {
    type Value = Vec<InspectedTensor>;

    fn expecting(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("a bounded Safetensors header object")
    }

    fn visit_map<M>(self, mut map: M) -> Result<Self::Value, M::Error>
    where
        M: MapAccess<'de>,
    {
        let mut tensors: Vec<InspectedTensor> = Vec::new();
        let mut metadata_seen = false;
        while let Some(key) = map.next_key_seed(HeaderKeySeed {
            budget: self.budget,
        })? {
            match key {
                HeaderKey::Metadata => {
                    if metadata_seen {
                        return Err(self.budget.fail(
                            ParseFailure::DuplicateTensor,
                            "duplicate Safetensors metadata key",
                        ));
                    }
                    metadata_seen = true;
                    map.next_value_seed(MetadataSeed {
                        budget: self.budget,
                    })?;
                }
                HeaderKey::Tensor(name) => {
                    reserve_bounded_slot(&mut tensors, self.budget.limits.tensors).map_err(
                        |()| {
                            self.budget.fail(
                                ParseFailure::Allocation,
                                "tensor inventory allocation failed",
                            )
                        },
                    )?;
                    let parsed = map.next_value_seed(TensorInfoSeed {
                        budget: self.budget,
                    })?;
                    let tensor = match build_tensor(name, parsed) {
                        Ok(tensor) => tensor,
                        Err(BuildTensorError::Bounds) => {
                            return Err(self
                                .budget
                                .fail(ParseFailure::HeaderBounds, "tensor byte range is invalid"));
                        }
                        Err(BuildTensorError::NumericOverflow) => {
                            return Err(self.budget.fail(
                                ParseFailure::NumericOverflow,
                                "tensor byte calculation overflowed",
                            ));
                        }
                    };
                    tensors.push(tensor);
                }
            }
        }
        Ok(tensors)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum BuildTensorError {
    Bounds,
    NumericOverflow,
}

fn build_tensor(
    name: String,
    parsed: ParsedTensorInfo,
) -> Result<InspectedTensor, BuildTensorError> {
    let element_count = parsed
        .shape
        .as_slice()
        .iter()
        .try_fold(1_u64, |total, dimension| {
            let dimension =
                u64::try_from(*dimension).map_err(|_| BuildTensorError::NumericOverflow)?;
            total
                .checked_mul(dimension)
                .ok_or(BuildTensorError::NumericOverflow)
        })?;
    let total_bits = element_count
        .checked_mul(parsed.source_dtype.bits_per_element())
        .ok_or(BuildTensorError::NumericOverflow)?;
    if total_bits % 8 != 0 {
        return Err(BuildTensorError::Bounds);
    }
    let source_bytes = total_bits
        .checked_div(8)
        .ok_or(BuildTensorError::NumericOverflow)?;
    let offset_bytes = parsed
        .data_end
        .checked_sub(parsed.data_start)
        .ok_or(BuildTensorError::Bounds)?;
    if offset_bytes != source_bytes {
        return Err(BuildTensorError::Bounds);
    }
    Ok(InspectedTensor {
        name,
        source_dtype: parsed.source_dtype,
        shape: parsed.shape,
        data_start: parsed.data_start,
        source_bytes,
        element_count,
        required: false,
    })
}

enum HeaderKey {
    Metadata,
    Tensor(String),
}

struct HeaderKeySeed<'a, 'limits> {
    budget: &'a mut InspectionBudget<'limits>,
}

impl<'de> DeserializeSeed<'de> for HeaderKeySeed<'_, '_> {
    type Value = HeaderKey;

    fn deserialize<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_identifier(HeaderKeyVisitor {
            budget: self.budget,
        })
    }
}

struct HeaderKeyVisitor<'a, 'limits> {
    budget: &'a mut InspectionBudget<'limits>,
}

impl HeaderKeyVisitor<'_, '_> {
    fn classify<E: de::Error>(&mut self, value: &str) -> Result<HeaderKey, E> {
        if value == "__metadata__" {
            return Ok(HeaderKey::Metadata);
        }
        self.budget.begin_tensor_name(value.len())?;
        let name = checked_string(value).map_err(|()| {
            self.budget
                .fail(ParseFailure::Allocation, "tensor name allocation failed")
        })?;
        Ok(HeaderKey::Tensor(name))
    }
}

impl<'de> Visitor<'de> for HeaderKeyVisitor<'_, '_> {
    type Value = HeaderKey;

    fn expecting(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("a bounded Safetensors tensor name")
    }

    fn visit_str<E>(mut self, value: &str) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        self.classify(value)
    }

    fn visit_borrowed_str<E>(mut self, value: &'de str) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        self.classify(value)
    }
}

#[derive(Clone, Copy, Debug)]
struct ParsedTensorInfo {
    source_dtype: SourceTensorDType,
    shape: TensorShape,
    data_start: u64,
    data_end: u64,
}

struct TensorInfoSeed<'a, 'limits> {
    budget: &'a mut InspectionBudget<'limits>,
}

impl<'de> DeserializeSeed<'de> for TensorInfoSeed<'_, '_> {
    type Value = ParsedTensorInfo;

    fn deserialize<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_map(TensorInfoVisitor {
            budget: self.budget,
        })
    }
}

struct TensorInfoVisitor<'a, 'limits> {
    budget: &'a mut InspectionBudget<'limits>,
}

impl<'de> Visitor<'de> for TensorInfoVisitor<'_, '_> {
    type Value = ParsedTensorInfo;

    fn expecting(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("bounded Safetensors tensor metadata")
    }

    fn visit_map<M>(self, mut map: M) -> Result<Self::Value, M::Error>
    where
        M: MapAccess<'de>,
    {
        let mut dtype = None;
        let mut shape = None;
        let mut offsets = None;
        while let Some(field) = map.next_key::<TensorField>()? {
            match field {
                TensorField::Dtype => {
                    if dtype.is_some() {
                        return Err(de::Error::duplicate_field("dtype"));
                    }
                    let safe_dtype: SafeDtype = map.next_value()?;
                    dtype = Some(
                        SourceTensorDType::from_safetensors(safe_dtype).ok_or_else(|| {
                            de::Error::custom("Safetensors dtype is not understood")
                        })?,
                    );
                }
                TensorField::Shape => {
                    if shape.is_some() {
                        return Err(de::Error::duplicate_field("shape"));
                    }
                    shape = Some(map.next_value_seed(ShapeSeed {
                        budget: self.budget,
                    })?);
                }
                TensorField::DataOffsets => {
                    if offsets.is_some() {
                        return Err(de::Error::duplicate_field("data_offsets"));
                    }
                    offsets = Some(map.next_value_seed(OffsetsSeed)?);
                }
                TensorField::Other => {
                    map.next_value::<IgnoredAny>()?;
                }
            }
        }
        let source_dtype = dtype.ok_or_else(|| de::Error::missing_field("dtype"))?;
        let shape = shape.ok_or_else(|| de::Error::missing_field("shape"))?;
        let (data_start, data_end) =
            offsets.ok_or_else(|| de::Error::missing_field("data_offsets"))?;
        Ok(ParsedTensorInfo {
            source_dtype,
            shape,
            data_start,
            data_end,
        })
    }
}

enum TensorField {
    Dtype,
    Shape,
    DataOffsets,
    Other,
}

impl<'de> Deserialize<'de> for TensorField {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_identifier(TensorFieldVisitor)
    }
}

struct TensorFieldVisitor;

impl Visitor<'_> for TensorFieldVisitor {
    type Value = TensorField;

    fn expecting(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("a Safetensors tensor metadata field")
    }

    fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        Ok(match value {
            "dtype" => TensorField::Dtype,
            "shape" => TensorField::Shape,
            "data_offsets" => TensorField::DataOffsets,
            _ => TensorField::Other,
        })
    }
}

struct ShapeSeed<'a, 'limits> {
    budget: &'a mut InspectionBudget<'limits>,
}

impl<'de> DeserializeSeed<'de> for ShapeSeed<'_, '_> {
    type Value = TensorShape;

    fn deserialize<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_seq(ShapeVisitor {
            budget: self.budget,
        })
    }
}

struct ShapeVisitor<'a, 'limits> {
    budget: &'a mut InspectionBudget<'limits>,
}

impl<'de> Visitor<'de> for ShapeVisitor<'_, '_> {
    type Value = TensorShape;

    fn expecting(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("a bounded Safetensors shape array")
    }

    fn visit_seq<A>(self, mut sequence: A) -> Result<Self::Value, A::Error>
    where
        A: SeqAccess<'de>,
    {
        let mut shape = TensorShape::EMPTY;
        while let Some(dimension) = sequence.next_element::<u64>()? {
            if usize::from(shape.rank) >= self.budget.limits.rank
                || usize::from(shape.rank) >= MAX_FIXED_RANK
            {
                return Err(self
                    .budget
                    .fail(ParseFailure::TensorLimit, "tensor rank limit exceeded"));
            }
            if dimension > self.budget.limits.shape_dimension_extent {
                return Err(self
                    .budget
                    .fail(ParseFailure::TensorLimit, "shape dimension limit exceeded"));
            }
            self.budget.add_shape_dimension()?;
            let dimension = usize::try_from(dimension).map_err(|_| {
                self.budget
                    .fail(ParseFailure::TensorLimit, "shape dimension overflow")
            })?;
            shape.push(dimension).map_err(|()| {
                self.budget
                    .fail(ParseFailure::TensorLimit, "tensor rank limit exceeded")
            })?;
        }
        Ok(shape)
    }
}

struct OffsetsSeed;

impl<'de> DeserializeSeed<'de> for OffsetsSeed {
    type Value = (u64, u64);

    fn deserialize<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_seq(OffsetsVisitor)
    }
}

struct OffsetsVisitor;

impl<'de> Visitor<'de> for OffsetsVisitor {
    type Value = (u64, u64);

    fn expecting(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("exactly two Safetensors data offsets")
    }

    fn visit_seq<A>(self, mut sequence: A) -> Result<Self::Value, A::Error>
    where
        A: SeqAccess<'de>,
    {
        let start = sequence
            .next_element::<u64>()?
            .ok_or_else(|| de::Error::invalid_length(0, &self))?;
        let end = sequence
            .next_element::<u64>()?
            .ok_or_else(|| de::Error::invalid_length(1, &self))?;
        if sequence.next_element::<IgnoredAny>()?.is_some() {
            return Err(de::Error::invalid_length(3, &self));
        }
        Ok((start, end))
    }
}

struct MetadataSeed<'a, 'limits> {
    budget: &'a mut InspectionBudget<'limits>,
}

impl<'de> DeserializeSeed<'de> for MetadataSeed<'_, '_> {
    type Value = ();

    fn deserialize<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_map(MetadataVisitor {
            budget: self.budget,
        })
    }
}

struct MetadataVisitor<'a, 'limits> {
    budget: &'a mut InspectionBudget<'limits>,
}

impl<'de> Visitor<'de> for MetadataVisitor<'_, '_> {
    type Value = ();

    fn expecting(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("bounded string Safetensors metadata")
    }

    fn visit_map<M>(self, mut map: M) -> Result<Self::Value, M::Error>
    where
        M: MapAccess<'de>,
    {
        let mut keys: Vec<String> = Vec::new();
        while let Some(key) = map.next_key_seed(MetadataKeySeed {
            budget: self.budget,
        })? {
            reserve_bounded_slot(&mut keys, self.budget.limits.metadata_entries).map_err(|()| {
                self.budget.fail(
                    ParseFailure::Allocation,
                    "metadata key inventory allocation failed",
                )
            })?;
            keys.push(key);
            map.next_value_seed(MetadataValueSeed {
                budget: self.budget,
            })?;
        }
        keys.sort_unstable();
        if keys.windows(2).any(|pair| pair.first() == pair.get(1)) {
            return Err(self.budget.fail(
                ParseFailure::DuplicateTensor,
                "duplicate Safetensors metadata key",
            ));
        }
        Ok(())
    }
}

struct MetadataKeySeed<'a, 'limits> {
    budget: &'a mut InspectionBudget<'limits>,
}

impl<'de> DeserializeSeed<'de> for MetadataKeySeed<'_, '_> {
    type Value = String;

    fn deserialize<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_identifier(MetadataKeyVisitor {
            budget: self.budget,
        })
    }
}

struct MetadataKeyVisitor<'a, 'limits> {
    budget: &'a mut InspectionBudget<'limits>,
}

impl MetadataKeyVisitor<'_, '_> {
    fn retain<E: de::Error>(&mut self, value: &str) -> Result<String, E> {
        self.budget.add_metadata_string(
            value.len(),
            self.budget.limits.metadata_key_bytes,
            true,
        )?;
        checked_string(value).map_err(|()| {
            self.budget
                .fail(ParseFailure::Allocation, "metadata key allocation failed")
        })
    }
}

impl<'de> Visitor<'de> for MetadataKeyVisitor<'_, '_> {
    type Value = String;

    fn expecting(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("a bounded Safetensors metadata key")
    }

    fn visit_str<E>(mut self, value: &str) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        self.retain(value)
    }

    fn visit_borrowed_str<E>(mut self, value: &'de str) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        self.retain(value)
    }
}

struct MetadataValueSeed<'a, 'limits> {
    budget: &'a mut InspectionBudget<'limits>,
}

impl<'de> DeserializeSeed<'de> for MetadataValueSeed<'_, '_> {
    type Value = ();

    fn deserialize<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_str(MetadataValueVisitor {
            budget: self.budget,
        })
    }
}

struct MetadataValueVisitor<'a, 'limits> {
    budget: &'a mut InspectionBudget<'limits>,
}

impl Visitor<'_> for MetadataValueVisitor<'_, '_> {
    type Value = ();

    fn expecting(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("a bounded Safetensors metadata value")
    }

    fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        self.budget
            .add_metadata_string(value.len(), self.budget.limits.metadata_value_bytes, false)
    }
}

fn checked_string(value: &str) -> Result<String, ()> {
    let mut retained = String::new();
    try_reserve_string(&mut retained, value.len())?;
    retained.push_str(value);
    Ok(retained)
}

fn reserve_bounded_slot<T>(values: &mut Vec<T>, maximum: usize) -> Result<(), ()> {
    if values.len() < values.capacity() {
        return Ok(());
    }
    let remaining = maximum.checked_sub(values.len()).ok_or(())?;
    let additional = remaining.min(64);
    if additional == 0 {
        return Err(());
    }
    try_reserve_vec(values, additional)
}

pub(super) fn try_reserve_vec<T>(values: &mut Vec<T>, additional: usize) -> Result<(), ()> {
    #[cfg(test)]
    if injected_allocation_failure() {
        return Err(());
    }
    values.try_reserve_exact(additional).map_err(|_| ())
}

fn try_reserve_string(value: &mut String, additional: usize) -> Result<(), ()> {
    #[cfg(test)]
    if injected_allocation_failure() {
        return Err(());
    }
    value.try_reserve_exact(additional).map_err(|_| ())
}

#[cfg(test)]
fn injected_allocation_failure() -> bool {
    TEST_INSPECTION_ALLOCATION_FAILURES.with(|remaining| {
        let value = remaining.get();
        match value {
            0 => false,
            1 => {
                remaining.set(0);
                true
            }
            _ => {
                remaining.set(value - 1);
                false
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use std::fs::{self, File};
    use std::io::Write;
    use std::sync::atomic::{AtomicU64, Ordering};

    use domain_contracts::{BackendFailureKind, BackendId, LoadError, ScalarType};

    use super::{
        InspectionBudget, InspectionLimits, SourceTensorDType, TEST_INSPECTION_ALLOCATION_FAILURES,
        map_parse_failure, parse_header,
    };
    use crate::failure::{
        CODE_HEADER_ALLOCATION, CODE_HEADER_LIMIT, CODE_INSPECTION_ALLOCATION,
        CODE_INSPECTION_INVENTORY_LIMIT, CODE_METADATA_LIMIT, CODE_TENSOR_LIMIT,
    };
    use crate::source::CandleWeightShard;

    use super::super::manifest::inspect_weight_shards;

    const BACKEND: BackendId = BackendId::new(11);
    static NEXT_TEMPORARY_FILE: AtomicU64 = AtomicU64::new(0);

    fn backend_code(error: LoadError) -> Option<(BackendFailureKind, u32)> {
        match error {
            LoadError::Backend(failure) => Some((failure.kind, failure.code)),
            _ => None,
        }
    }

    fn required_error<T>(result: Result<T, LoadError>, context: &str) -> Result<LoadError, String> {
        result.err().ok_or_else(|| context.to_owned())
    }

    type ParserLimitCase = (fn(&mut InspectionLimits), &'static [u8], u32);

    #[test]
    fn safetensors_08_dtype_classification_is_complete_and_stable() {
        let direct = [
            (SourceTensorDType::F32, ScalarType::F32),
            (SourceTensorDType::F16, ScalarType::F16),
            (SourceTensorDType::Bf16, ScalarType::Bf16),
            (SourceTensorDType::I8, ScalarType::I8),
            (SourceTensorDType::U8, ScalarType::U8),
        ];
        for (dtype, scalar) in direct {
            assert_eq!(dtype.scalar_type(), scalar);
        }
        for dtype in [
            SourceTensorDType::Bool,
            SourceTensorDType::F4,
            SourceTensorDType::F6E2M3,
            SourceTensorDType::F6E3M2,
            SourceTensorDType::F8E5M2,
            SourceTensorDType::F8E4M3,
            SourceTensorDType::F8E8M0,
            SourceTensorDType::F8E4M3Fnuz,
            SourceTensorDType::F8E5M2Fnuz,
            SourceTensorDType::I16,
            SourceTensorDType::U16,
            SourceTensorDType::I32,
            SourceTensorDType::U32,
            SourceTensorDType::C64,
            SourceTensorDType::F64,
            SourceTensorDType::I64,
            SourceTensorDType::U64,
        ] {
            assert!(matches!(dtype.scalar_type(), ScalarType::Other(_)));
            assert_eq!(dtype.executable_dtype(), None);
        }
    }

    #[test]
    fn bitpacked_widths_are_calculated_without_rounding() -> Result<(), String> {
        let mut limits = InspectionLimits::PRODUCTION;
        let mut budget = InspectionBudget::new(&limits);
        let tensors = parse_header(
            BACKEND,
            br#"{"packed":{"dtype":"F4","shape":[2],"data_offsets":[0,1]}}"#,
            &mut budget,
        )
        .map_err(|error| format!("parse aligned F4 tensor: {error:?}"))?;
        let tensor = tensors
            .first()
            .ok_or_else(|| "aligned F4 tensor was not retained".to_owned())?;
        assert_eq!(tensor.source_bytes, 1);

        limits.tensors = 1;
        let mut budget = InspectionBudget::new(&limits);
        assert!(
            parse_header(
                BACKEND,
                br#"{"packed":{"dtype":"F4","shape":[1],"data_offsets":[0,1]}}"#,
                &mut budget,
            )
            .is_err()
        );
        Ok(())
    }

    #[test]
    fn every_injectable_parser_limit_uses_small_fixtures() -> Result<(), String> {
        let cases: &[ParserLimitCase] = &[
            (
                |limits| limits.tensors = 0,
                br#"{"x":{"dtype":"F32","shape":[1],"data_offsets":[0,4]}}"#,
                CODE_TENSOR_LIMIT,
            ),
            (
                |limits| limits.tensor_name_bytes = 0,
                br#"{"x":{"dtype":"F32","shape":[1],"data_offsets":[0,4]}}"#,
                CODE_TENSOR_LIMIT,
            ),
            (
                |limits| limits.aggregate_tensor_name_bytes = 0,
                br#"{"x":{"dtype":"F32","shape":[1],"data_offsets":[0,4]}}"#,
                CODE_TENSOR_LIMIT,
            ),
            (
                |limits| limits.rank = 0,
                br#"{"x":{"dtype":"F32","shape":[1],"data_offsets":[0,4]}}"#,
                CODE_TENSOR_LIMIT,
            ),
            (
                |limits| limits.shape_dimension_extent = 0,
                br#"{"x":{"dtype":"F32","shape":[1],"data_offsets":[0,4]}}"#,
                CODE_TENSOR_LIMIT,
            ),
            (
                |limits| limits.aggregate_shape_dimensions = 0,
                br#"{"x":{"dtype":"F32","shape":[1],"data_offsets":[0,4]}}"#,
                CODE_TENSOR_LIMIT,
            ),
            (
                |limits| limits.metadata_entries = 0,
                br#"{"__metadata__":{"x":"y"}}"#,
                CODE_METADATA_LIMIT,
            ),
            (
                |limits| limits.metadata_key_bytes = 0,
                br#"{"__metadata__":{"x":"y"}}"#,
                CODE_METADATA_LIMIT,
            ),
            (
                |limits| limits.metadata_value_bytes = 0,
                br#"{"__metadata__":{"x":"y"}}"#,
                CODE_METADATA_LIMIT,
            ),
            (
                |limits| limits.aggregate_metadata_string_bytes = 1,
                br#"{"__metadata__":{"x":"y"}}"#,
                CODE_METADATA_LIMIT,
            ),
            (
                |limits| limits.final_inventory_bytes = 0,
                br#"{"x":{"dtype":"F32","shape":[1],"data_offsets":[0,4]}}"#,
                CODE_INSPECTION_INVENTORY_LIMIT,
            ),
        ];

        for (configure, header, expected_code) in cases {
            let mut limits = InspectionLimits::PRODUCTION;
            configure(&mut limits);
            let mut budget = InspectionBudget::new(&limits);
            let error = required_error(
                parse_header(BACKEND, header, &mut budget),
                "configured structural ceiling must fail",
            )?;
            assert_eq!(
                backend_code(error),
                Some((BackendFailureKind::InvalidModel, *expected_code))
            );
        }
        Ok(())
    }

    #[test]
    fn shard_header_and_final_inventory_limits_use_small_files() -> Result<(), String> {
        let first_path = write_safetensors(br"{}", &[])?;
        let second_path = write_safetensors(br"{}", &[])?;
        let first = CandleWeightShard::unverified(first_path.clone());
        let second = CandleWeightShard::unverified(second_path.clone());

        let mut limits = InspectionLimits::PRODUCTION;
        limits.shards = 0;
        assert!(matches!(
            inspect_weight_shards(BACKEND, std::slice::from_ref(&first), &limits),
            Err(LoadError::CapacityExhausted(_))
        ));

        limits = InspectionLimits::PRODUCTION;
        limits.per_shard_header_bytes = 1;
        let error = required_error(
            inspect_weight_shards(BACKEND, std::slice::from_ref(&first), &limits),
            "per-shard header ceiling must fail",
        )?;
        assert_eq!(
            backend_code(error),
            Some((BackendFailureKind::InvalidModel, CODE_HEADER_LIMIT))
        );

        limits = InspectionLimits::PRODUCTION;
        limits.aggregate_header_bytes = 3;
        let error = required_error(
            inspect_weight_shards(BACKEND, &[first.clone(), second], &limits),
            "aggregate header ceiling must fail",
        )?;
        assert_eq!(
            backend_code(error),
            Some((BackendFailureKind::InvalidModel, CODE_HEADER_LIMIT))
        );

        limits = InspectionLimits::PRODUCTION;
        limits.final_inventory_bytes = 0;
        let error = required_error(
            inspect_weight_shards(BACKEND, std::slice::from_ref(&first), &limits),
            "final inventory ceiling must include each shard",
        )?;
        assert_eq!(
            backend_code(error),
            Some((
                BackendFailureKind::InvalidModel,
                CODE_INSPECTION_INVENTORY_LIMIT,
            ))
        );

        TEST_INSPECTION_ALLOCATION_FAILURES.with(|remaining| remaining.set(3));
        let error = required_error(
            inspect_weight_shards(
                BACKEND,
                std::slice::from_ref(&first),
                &InspectionLimits::PRODUCTION,
            ),
            "third checked allocation is the bounded header buffer",
        )?;
        assert_eq!(
            backend_code(error),
            Some((BackendFailureKind::HostMemory, CODE_HEADER_ALLOCATION))
        );

        fs::remove_file(first_path).map_err(|error| error.to_string())?;
        fs::remove_file(second_path).map_err(|error| error.to_string())?;
        Ok(())
    }

    #[test]
    fn parser_allocation_failure_is_stable() -> Result<(), String> {
        let limits = InspectionLimits::PRODUCTION;
        let mut budget = InspectionBudget::new(&limits);
        TEST_INSPECTION_ALLOCATION_FAILURES.with(|remaining| remaining.set(1));
        let error = required_error(
            parse_header(
                BACKEND,
                br#"{"x":{"dtype":"F32","shape":[1],"data_offsets":[0,4]}}"#,
                &mut budget,
            ),
            "injected parser allocation must fail",
        )?;
        assert_eq!(
            backend_code(error),
            Some((BackendFailureKind::HostMemory, CODE_INSPECTION_ALLOCATION))
        );
        assert_eq!(
            backend_code(map_parse_failure(
                BACKEND,
                Some(super::ParseFailure::Allocation)
            )),
            Some((BackendFailureKind::HostMemory, CODE_INSPECTION_ALLOCATION))
        );
        Ok(())
    }

    fn write_safetensors(header: &[u8], payload: &[u8]) -> Result<std::path::PathBuf, String> {
        let sequence = NEXT_TEMPORARY_FILE.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "milkdrift-candle-manifest-{}-{sequence}.safetensors",
            std::process::id()
        ));
        let mut file = File::create(&path).map_err(|error| error.to_string())?;
        file.write_all(
            &u64::try_from(header.len())
                .map_err(|error| error.to_string())?
                .to_le_bytes(),
        )
        .map_err(|error| error.to_string())?;
        file.write_all(header).map_err(|error| error.to_string())?;
        file.write_all(payload).map_err(|error| error.to_string())?;
        file.sync_all().map_err(|error| error.to_string())?;
        Ok(path)
    }
}
