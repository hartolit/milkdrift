//! Deterministic accelerator transfer partitioning shared by admission and loading.

use std::mem::size_of;

use candle_core::DType;
use domain_contracts::{BackendId, LoadError};

use crate::failure::{CODE_INSPECTION_ALLOCATION, CODE_NUMERIC_OVERFLOW};

use super::manifest::InspectedShard;
use super::{host_memory_failure, invalid_model_failure, unsupported_scalar};

/// Preferred maximum live tensor staging for one accelerator batch.
pub(super) const PREFERRED_BATCH_HOST_STAGING_BYTES: u64 = 256 * 1024 * 1024;
/// Hard bound on entries and their owner metadata in one accelerator batch.
pub(super) const MAXIMUM_BATCH_ENTRIES: usize = 64;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct TransferPolicy {
    preferred_host_staging_bytes: u64,
    maximum_entries: usize,
}

impl TransferPolicy {
    const PRODUCTION: Self = Self {
        preferred_host_staging_bytes: PREFERRED_BATCH_HOST_STAGING_BYTES,
        maximum_entries: MAXIMUM_BATCH_ENTRIES,
    };
}

/// One required tensor's immutable byte and manifest coordinates.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct TransferEntryPlan {
    shard_index: usize,
    tensor_index: usize,
    source_bytes: u64,
    execution_bytes: u64,
    aligned_payload_bytes: u64,
    retained_host_bytes: u64,
}

impl TransferEntryPlan {
    pub(super) const fn coordinate(self) -> (usize, usize) {
        (self.shard_index, self.tensor_index)
    }

    pub(super) const fn source_bytes(self) -> u64 {
        self.source_bytes
    }

    pub(super) const fn execution_bytes(self) -> u64 {
        self.execution_bytes
    }

    pub(super) const fn retained_host_bytes(self) -> u64 {
        self.retained_host_bytes
    }

    pub(super) const fn requires_cast(self) -> bool {
        self.retained_host_bytes != self.source_bytes
    }
}

/// One bounded batch range in the plan's flat entry inventory.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct TransferBatchPlan {
    shard_index: usize,
    entry_start: usize,
    entry_count: usize,
    host_peak_bytes: u64,
    retained_host_bytes: u64,
    transferred_device_bytes: u64,
    last_in_shard: bool,
}

impl TransferBatchPlan {
    pub(super) const fn shard_index(self) -> usize {
        self.shard_index
    }

    pub(super) const fn entry_count(self) -> usize {
        self.entry_count
    }

    #[cfg(test)]
    pub(super) const fn host_peak_bytes(self) -> u64 {
        self.host_peak_bytes
    }

    #[cfg(test)]
    pub(super) const fn retained_host_bytes(self) -> u64 {
        self.retained_host_bytes
    }

    #[cfg(test)]
    pub(super) const fn transferred_device_bytes(self) -> u64 {
        self.transferred_device_bytes
    }

    pub(super) const fn is_last_in_shard(self) -> bool {
        self.last_in_shard
    }
}

/// Immutable batch partition retained by a prepared accelerator load.
#[derive(Debug)]
pub(super) struct TransferPlan {
    batches: Vec<TransferBatchPlan>,
    entries: Vec<TransferEntryPlan>,
    maximum_host_staging_bytes: u64,
    total_execution_bytes: u64,
    metadata_bytes: u64,
}

impl TransferPlan {
    pub(super) fn build(
        backend: BackendId,
        shards: &[InspectedShard],
        execution_dtype: DType,
    ) -> Result<Self, LoadError> {
        Self::build_with_policy(backend, shards, execution_dtype, TransferPolicy::PRODUCTION)
    }

    #[cfg(test)]
    pub(super) fn build_with_test_limits(
        backend: BackendId,
        shards: &[InspectedShard],
        execution_dtype: DType,
        preferred_host_staging_bytes: u64,
        maximum_entries: usize,
    ) -> Result<Self, LoadError> {
        Self::build_with_policy(
            backend,
            shards,
            execution_dtype,
            TransferPolicy {
                preferred_host_staging_bytes,
                maximum_entries,
            },
        )
    }

    #[expect(
        clippy::too_many_lines,
        reason = "one sequential pass owns the deterministic partition state and checked arithmetic"
    )]
    fn build_with_policy(
        backend: BackendId,
        shards: &[InspectedShard],
        execution_dtype: DType,
        policy: TransferPolicy,
    ) -> Result<Self, LoadError> {
        if policy.maximum_entries == 0 {
            return Err(numeric_error(backend));
        }
        let execution_width =
            dtype_bytes(execution_dtype).ok_or_else(|| unsupported_scalar(backend))?;
        let mut plan = Self {
            batches: Vec::new(),
            entries: Vec::new(),
            maximum_host_staging_bytes: 0,
            total_execution_bytes: 0,
            metadata_bytes: 0,
        };

        for (shard_index, shard) in shards.iter().enumerate() {
            let mut batch_start = plan.entries.len();
            let mut batch_count = 0_usize;
            let mut retained_host_bytes = 0_u64;
            let mut transferred_device_bytes = 0_u64;
            let mut host_peak_bytes = 0_u64;

            for (tensor_index, tensor) in shard.tensors.iter().enumerate() {
                if !tensor.required {
                    continue;
                }
                let source_dtype = tensor
                    .source_dtype
                    .executable_dtype()
                    .ok_or_else(|| unsupported_scalar(backend))?;
                let execution_bytes = tensor
                    .element_count
                    .checked_mul(execution_width)
                    .ok_or_else(|| numeric_error(backend))?;
                let aligned_payload_bytes = tensor
                    .source_bytes
                    .checked_add(
                        tensor
                            .source_dtype
                            .alignment()
                            .ok_or_else(|| unsupported_scalar(backend))?
                            .checked_sub(1)
                            .ok_or_else(|| numeric_error(backend))?,
                    )
                    .ok_or_else(|| numeric_error(backend))?;
                let retained_for_entry = if source_dtype == execution_dtype {
                    tensor.source_bytes
                } else {
                    tensor
                        .source_bytes
                        .checked_add(execution_bytes)
                        .ok_or_else(|| numeric_error(backend))?
                };
                let entry = TransferEntryPlan {
                    shard_index,
                    tensor_index,
                    source_bytes: tensor.source_bytes,
                    execution_bytes,
                    aligned_payload_bytes,
                    retained_host_bytes: retained_for_entry,
                };
                let projected_peak =
                    candidate_peak(backend, retained_host_bytes, host_peak_bytes, entry)?;
                if batch_count > 0
                    && (batch_count == policy.maximum_entries
                        || projected_peak > policy.preferred_host_staging_bytes)
                {
                    plan.finish_batch(
                        backend,
                        shard_index,
                        batch_start,
                        batch_count,
                        host_peak_bytes,
                        retained_host_bytes,
                        transferred_device_bytes,
                        false,
                    )?;
                    batch_start = plan.entries.len();
                    batch_count = 0;
                    retained_host_bytes = 0;
                    transferred_device_bytes = 0;
                    host_peak_bytes = 0;
                }

                let entry_peak =
                    candidate_peak(backend, retained_host_bytes, host_peak_bytes, entry)?;
                retained_host_bytes = retained_host_bytes
                    .checked_add(entry.retained_host_bytes)
                    .ok_or_else(|| numeric_error(backend))?;
                transferred_device_bytes = transferred_device_bytes
                    .checked_add(entry.execution_bytes)
                    .ok_or_else(|| numeric_error(backend))?;
                host_peak_bytes = entry_peak;
                batch_count = batch_count
                    .checked_add(1)
                    .ok_or_else(|| numeric_error(backend))?;
                plan.total_execution_bytes = plan
                    .total_execution_bytes
                    .checked_add(entry.execution_bytes)
                    .ok_or_else(|| numeric_error(backend))?;
                plan.entries
                    .try_reserve(1)
                    .map_err(|_| host_memory_failure(backend, CODE_INSPECTION_ALLOCATION))?;
                plan.entries.push(entry);
            }

            if batch_count > 0 {
                plan.finish_batch(
                    backend,
                    shard_index,
                    batch_start,
                    batch_count,
                    host_peak_bytes,
                    retained_host_bytes,
                    transferred_device_bytes,
                    true,
                )?;
            }
        }
        plan.metadata_bytes = plan.calculate_metadata_bytes(backend)?;
        Ok(plan)
    }

    #[allow(clippy::too_many_arguments)]
    fn finish_batch(
        &mut self,
        backend: BackendId,
        shard_index: usize,
        entry_start: usize,
        entry_count: usize,
        host_peak_bytes: u64,
        retained_host_bytes: u64,
        transferred_device_bytes: u64,
        last_in_shard: bool,
    ) -> Result<(), LoadError> {
        self.batches
            .try_reserve(1)
            .map_err(|_| host_memory_failure(backend, CODE_INSPECTION_ALLOCATION))?;
        self.batches.push(TransferBatchPlan {
            shard_index,
            entry_start,
            entry_count,
            host_peak_bytes,
            retained_host_bytes,
            transferred_device_bytes,
            last_in_shard,
        });
        self.maximum_host_staging_bytes = self.maximum_host_staging_bytes.max(host_peak_bytes);
        Ok(())
    }

    fn calculate_metadata_bytes(&self, backend: BackendId) -> Result<u64, LoadError> {
        capacity_bytes::<TransferBatchPlan>(backend, self.batches.capacity())?
            .checked_add(capacity_bytes::<TransferEntryPlan>(
                backend,
                self.entries.capacity(),
            )?)
            .ok_or_else(|| numeric_error(backend))
    }

    pub(super) fn batch(&self, index: usize) -> Option<TransferBatchPlan> {
        self.batches.get(index).copied()
    }

    pub(super) fn entry(
        &self,
        batch_index: usize,
        entry_index: usize,
    ) -> Option<TransferEntryPlan> {
        let batch = self.batch(batch_index)?;
        if entry_index >= batch.entry_count {
            return None;
        }
        self.entries
            .get(batch.entry_start.checked_add(entry_index)?)
            .copied()
    }

    #[cfg(test)]
    pub(super) fn batches(&self) -> &[TransferBatchPlan] {
        self.batches.as_slice()
    }

    pub(super) fn len(&self) -> usize {
        self.batches.len()
    }

    pub(super) const fn maximum_host_staging_bytes(&self) -> u64 {
        self.maximum_host_staging_bytes
    }

    pub(super) const fn total_execution_bytes(&self) -> u64 {
        self.total_execution_bytes
    }

    pub(super) const fn metadata_bytes(&self) -> u64 {
        self.metadata_bytes
    }
}

fn candidate_peak(
    backend: BackendId,
    retained_prefix: u64,
    existing_peak: u64,
    entry: TransferEntryPlan,
) -> Result<u64, LoadError> {
    let raw_peak = retained_prefix
        .checked_add(entry.aligned_payload_bytes)
        .and_then(|bytes| bytes.checked_add(entry.source_bytes))
        .ok_or_else(|| numeric_error(backend))?;
    let retained_peak = retained_prefix
        .checked_add(entry.retained_host_bytes)
        .ok_or_else(|| numeric_error(backend))?;
    let cast_peak = if entry.requires_cast() {
        retained_prefix
            .checked_add(entry.source_bytes)
            .and_then(|bytes| bytes.checked_add(entry.execution_bytes))
            .ok_or_else(|| numeric_error(backend))?
    } else {
        0
    };
    Ok(existing_peak
        .max(raw_peak)
        .max(cast_peak)
        .max(retained_peak))
}

fn capacity_bytes<T>(backend: BackendId, capacity: usize) -> Result<u64, LoadError> {
    u64::try_from(capacity)
        .ok()
        .and_then(|count| {
            u64::try_from(size_of::<T>())
                .ok()
                .and_then(|width| count.checked_mul(width))
        })
        .ok_or_else(|| numeric_error(backend))
}

const fn dtype_bytes(dtype: DType) -> Option<u64> {
    match dtype {
        DType::F32 => Some(4),
        DType::F16 | DType::BF16 => Some(2),
        _ => None,
    }
}

const fn numeric_error(backend: BackendId) -> LoadError {
    invalid_model_failure(backend, CODE_NUMERIC_OVERFLOW)
}

#[cfg(test)]
mod tests {
    use std::fs::File;

    use candle_core::DType;
    use domain_contracts::BackendId;

    use super::{TransferBatchPlan, TransferPlan, TransferPolicy};
    use crate::failure::CODE_NUMERIC_OVERFLOW;
    use crate::loader::manifest::{
        InspectedShard, InspectedTensor, SourceTensorDType, TensorShape,
    };

    #[test]
    fn mixed_entries_share_a_batch_and_use_exact_live_peak() -> Result<(), String> {
        let shards = vec![shard(vec![
            tensor(SourceTensorDType::F16, 4, 8)?,
            tensor(SourceTensorDType::F32, 4, 16)?,
        ])?];
        let plan = TransferPlan::build_with_policy(
            BackendId::new(1),
            &shards,
            DType::F16,
            TransferPolicy {
                preferred_host_staging_bytes: 1_000,
                maximum_entries: 64,
            },
        )
        .map_err(|error| format!("plan: {error:?}"))?;
        assert_eq!(plan.len(), 1);
        let batch = plan.batch(0).ok_or_else(|| "missing batch".to_owned())?;
        // First entry raw peak: 9 + 8. Second raw peak: 8 + 19 + 16.
        assert_eq!(batch.host_peak_bytes(), 43);
        assert_eq!(batch.retained_host_bytes(), 32);
        assert_eq!(batch.transferred_device_bytes(), 16);
        assert!(batch.is_last_in_shard());
        Ok(())
    }

    #[test]
    fn equality_fits_while_count_and_byte_boundaries_split() -> Result<(), String> {
        let shards = vec![shard(vec![
            tensor(SourceTensorDType::F32, 1, 4)?,
            tensor(SourceTensorDType::F32, 1, 4)?,
            tensor(SourceTensorDType::F32, 1, 4)?,
        ])?];
        let byte_plan = TransferPlan::build_with_policy(
            BackendId::new(1),
            &shards,
            DType::F32,
            TransferPolicy {
                preferred_host_staging_bytes: 15,
                maximum_entries: 64,
            },
        )
        .map_err(|error| format!("byte plan: {error:?}"))?;
        assert_eq!(
            byte_plan
                .batches()
                .iter()
                .map(|batch| batch.entry_count())
                .collect::<Vec<_>>(),
            vec![2, 1]
        );

        let count_plan = TransferPlan::build_with_policy(
            BackendId::new(1),
            &shards,
            DType::F32,
            TransferPolicy {
                preferred_host_staging_bytes: u64::MAX,
                maximum_entries: 2,
            },
        )
        .map_err(|error| format!("count plan: {error:?}"))?;
        assert_eq!(
            count_plan
                .batches()
                .iter()
                .map(|batch| batch.entry_count())
                .collect::<Vec<_>>(),
            vec![2, 1]
        );
        Ok(())
    }

    #[test]
    fn oversized_tensor_is_a_singleton_and_shards_never_mix() -> Result<(), String> {
        let shards = vec![
            shard(vec![tensor(SourceTensorDType::F32, 8, 32)?])?,
            shard(vec![tensor(SourceTensorDType::F32, 1, 4)?])?,
        ];
        let plan = TransferPlan::build_with_policy(
            BackendId::new(1),
            &shards,
            DType::F32,
            TransferPolicy {
                preferred_host_staging_bytes: 8,
                maximum_entries: 64,
            },
        )
        .map_err(|error| format!("plan: {error:?}"))?;
        assert_eq!(plan.len(), 2);
        assert_eq!(plan.batch(0).map(TransferBatchPlan::shard_index), Some(0));
        assert_eq!(plan.batch(1).map(TransferBatchPlan::shard_index), Some(1));
        assert!(
            plan.batch(0)
                .is_some_and(|batch| batch.host_peak_bytes() > 8)
        );
        Ok(())
    }

    #[test]
    fn arithmetic_overflow_is_explicit() -> Result<(), String> {
        let shards = vec![shard(vec![tensor(SourceTensorDType::F32, u64::MAX, 4)?])?];
        let error = TransferPlan::build(BackendId::new(1), &shards, DType::F32)
            .err()
            .ok_or_else(|| "overflow unexpectedly planned".to_owned())?;
        assert!(
            matches!(error, domain_contracts::LoadError::Backend(failure) if failure.code == CODE_NUMERIC_OVERFLOW)
        );
        Ok(())
    }

    fn tensor(
        dtype: SourceTensorDType,
        element_count: u64,
        source_bytes: u64,
    ) -> Result<InspectedTensor, String> {
        Ok(InspectedTensor {
            name: format!("tensor-{element_count}-{source_bytes}"),
            source_dtype: dtype,
            shape: TensorShape::from_slice(&[1]).ok_or_else(|| "shape".to_owned())?,
            data_start: 0,
            source_bytes,
            element_count,
            required: true,
        })
    }

    fn shard(tensors: Vec<InspectedTensor>) -> Result<InspectedShard, String> {
        let file = File::open(std::env::current_exe().map_err(|error| error.to_string())?)
            .map_err(|error| error.to_string())?;
        Ok(InspectedShard {
            file,
            file_length: 0,
            data_start: 0,
            prefix_header_sha256: [0; 32],
            source_expected_content: None,
            established_content_identity: None,
            tensors,
        })
    }
}
