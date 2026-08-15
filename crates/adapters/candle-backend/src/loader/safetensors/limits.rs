use std::mem::size_of;

use domain_contracts::{BackendId, LoadError};
use serde::de;

use crate::failure::{CODE_INSPECTION_INVENTORY_LIMIT, CODE_NUMERIC_OVERFLOW};

use super::InspectedTensor;
use crate::loader::invalid_model_failure;

/// One production limit set governs all header-side growth. Values provide
/// realistic Llama headroom while keeping hostile JSON amplification bounded.
#[derive(Clone, Copy, Debug)]
pub(crate) struct InspectionLimits {
    pub(crate) shards: usize,
    pub(crate) per_shard_header_bytes: u64,
    pub(crate) aggregate_header_bytes: u64,
    pub(crate) tensors: usize,
    pub(crate) tensor_name_bytes: usize,
    pub(crate) aggregate_tensor_name_bytes: u64,
    pub(crate) rank: usize,
    pub(crate) shape_dimension_extent: u64,
    pub(crate) aggregate_shape_dimensions: usize,
    pub(crate) metadata_entries: usize,
    pub(crate) metadata_key_bytes: usize,
    pub(crate) metadata_value_bytes: usize,
    pub(crate) aggregate_metadata_string_bytes: u64,
    pub(crate) final_inventory_bytes: u64,
}

impl InspectionLimits {
    pub(crate) const PRODUCTION: Self = Self {
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

// Zero-based diagnostic ordinals are constructed only after these production
// admission bounds have accepted the selected shards and tensor inventory.
const _: () = assert!(InspectionLimits::PRODUCTION.shards <= (u16::MAX as usize) + 1);
const _: () = assert!(InspectionLimits::PRODUCTION.tensors <= (u32::MAX as usize) + 1);

#[cfg(test)]
thread_local! {
    pub(super) static TEST_INSPECTION_ALLOCATION_FAILURES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

#[derive(Clone, Copy, Debug)]
pub(crate) enum ParseFailure {
    DuplicateTensor,
    HeaderBounds,
    NumericOverflow,
    TensorLimit,
    MetadataLimit,
    InventoryLimit,
    Allocation,
}

pub(crate) struct InspectionBudget<'a> {
    pub(crate) limits: &'a InspectionLimits,
    tensor_count: usize,
    aggregate_tensor_name_bytes: u64,
    aggregate_shape_dimensions: usize,
    metadata_entries: usize,
    aggregate_metadata_string_bytes: u64,
    final_inventory_bytes: u64,
    pub(crate) parse_failure: Option<ParseFailure>,
}

impl<'a> InspectionBudget<'a> {
    pub(crate) const fn new(limits: &'a InspectionLimits) -> Self {
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

    pub(crate) fn fail<E: de::Error>(&mut self, failure: ParseFailure, message: &'static str) -> E {
        if self.parse_failure.is_none() {
            self.parse_failure = Some(failure);
        }
        E::custom(message)
    }

    pub(crate) fn begin_tensor_name<E: de::Error>(&mut self, length: usize) -> Result<(), E> {
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

    pub(crate) fn add_shape_dimension<E: de::Error>(&mut self) -> Result<(), E> {
        self.aggregate_shape_dimensions = self
            .aggregate_shape_dimensions
            .checked_add(1)
            .ok_or_else(|| self.fail(ParseFailure::TensorLimit, "shape count overflow"))?;
        if self.aggregate_shape_dimensions > self.limits.aggregate_shape_dimensions {
            return Err(self.fail(ParseFailure::TensorLimit, "shape limit exceeded"));
        }
        Ok(())
    }

    pub(crate) fn add_metadata_string<E: de::Error>(
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

    pub(crate) fn add_inventory(
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

pub(crate) fn checked_string(value: &str) -> Result<String, ()> {
    let mut retained = String::new();
    try_reserve_string(&mut retained, value.len())?;
    retained.push_str(value);
    Ok(retained)
}

pub(crate) fn reserve_bounded_slot<T>(values: &mut Vec<T>, maximum: usize) -> Result<(), ()> {
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

pub(crate) fn try_reserve_vec<T>(values: &mut Vec<T>, additional: usize) -> Result<(), ()> {
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
