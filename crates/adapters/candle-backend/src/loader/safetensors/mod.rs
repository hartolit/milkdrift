//! Allocation-bounded Safetensors JSON decoding and retained tensor facts.

use std::fmt::Formatter;
use std::fs::File;

use domain_contracts::{BackendId, LoadError};
use serde::Deserializer;
use serde::de::{DeserializeSeed, MapAccess, Visitor};

use crate::failure::{
    CODE_DUPLICATE_TENSOR, CODE_HEADER_BOUNDS, CODE_HEADER_DECODE, CODE_INSPECTION_ALLOCATION,
    CODE_INSPECTION_INVENTORY_LIMIT, CODE_METADATA_LIMIT, CODE_NUMERIC_OVERFLOW, CODE_TENSOR_LIMIT,
};
use crate::source::CandleExpectedContentIdentity;

use super::identity::EstablishedContentIdentity;
use super::{host_memory_failure, invalid_model_failure};

mod dtype;
mod limits;
mod metadata;
mod tensor;

pub(super) use dtype::SourceTensorDType;
#[cfg(test)]
use limits::TEST_INSPECTION_ALLOCATION_FAILURES;
pub(super) use limits::{InspectionBudget, InspectionLimits, ParseFailure, try_reserve_vec};
pub(super) use tensor::{InspectedTensor, TensorShape};

use limits::reserve_bounded_slot;
use metadata::MetadataSeed;
use tensor::{BuildTensorError, HeaderKey, HeaderKeySeed, TensorInfoSeed, build_tensor};

const SHA256_BYTES: usize = 32;

#[derive(Debug)]
pub(super) struct InspectedShard {
    pub(super) file: File,
    pub(super) file_length: u64,
    pub(super) data_start: u64,
    pub(super) prefix_header_sha256: [u8; SHA256_BYTES],
    pub(super) source_expected_content: Option<CandleExpectedContentIdentity>,
    pub(super) established_content_identity: Option<EstablishedContentIdentity>,
    pub(super) tensors: Vec<InspectedTensor>,
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

#[cfg(test)]
mod tests;
