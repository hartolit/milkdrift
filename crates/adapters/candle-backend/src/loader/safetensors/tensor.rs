use std::fmt::Formatter;

use safetensors::tensor::Dtype as SafeDtype;
use serde::de::{self, DeserializeSeed, IgnoredAny, MapAccess, SeqAccess, Visitor};
use serde::{Deserialize, Deserializer};

use super::dtype::SourceTensorDType;
use super::limits::{InspectionBudget, ParseFailure, checked_string};

const MAX_FIXED_RANK: usize = 8;

#[derive(Debug)]
pub(crate) struct InspectedTensor {
    pub(crate) name: String,
    pub(crate) source_dtype: SourceTensorDType,
    pub(crate) shape: TensorShape,
    pub(crate) data_start: u64,
    pub(crate) source_bytes: u64,
    pub(crate) element_count: u64,
    pub(crate) required: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct TensorShape {
    dimensions: [usize; MAX_FIXED_RANK],
    rank: u8,
}

impl TensorShape {
    const EMPTY: Self = Self {
        dimensions: [0; MAX_FIXED_RANK],
        rank: 0,
    };

    pub(crate) fn as_slice(&self) -> &[usize] {
        self.dimensions
            .get(..usize::from(self.rank))
            .unwrap_or_default()
    }

    #[cfg(test)]
    pub(crate) fn from_slice(dimensions: &[usize]) -> Option<Self> {
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
pub(crate) enum BuildTensorError {
    Bounds,
    NumericOverflow,
}

pub(crate) fn build_tensor(
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

pub(crate) enum HeaderKey {
    Metadata,
    Tensor(String),
}

pub(crate) struct HeaderKeySeed<'a, 'limits> {
    pub(crate) budget: &'a mut InspectionBudget<'limits>,
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
pub(crate) struct ParsedTensorInfo {
    source_dtype: SourceTensorDType,
    shape: TensorShape,
    data_start: u64,
    data_end: u64,
}

pub(crate) struct TensorInfoSeed<'a, 'limits> {
    pub(crate) budget: &'a mut InspectionBudget<'limits>,
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
