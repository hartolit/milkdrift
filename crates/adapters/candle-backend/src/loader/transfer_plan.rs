//! Deterministic accelerator transfer partitioning shared by admission and loading.

use std::mem::size_of;

use candle_core::DType;
use domain_contracts::{BackendId, LoadError};

use crate::failure::{CODE_INSPECTION_ALLOCATION, CODE_NUMERIC_OVERFLOW};

use super::manifest::{InspectedShard, InspectedTensor};
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

    pub(super) const fn retained_host_bytes(self) -> u64 {
        self.retained_host_bytes
    }

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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct PendingTransferBatch {
    shard_index: usize,
    entry_start: usize,
    entry_count: usize,
    host_peak_bytes: u64,
    retained_host_bytes: u64,
    transferred_device_bytes: u64,
}

impl PendingTransferBatch {
    const fn new(shard_index: usize, entry_start: usize) -> Self {
        Self {
            shard_index,
            entry_start,
            entry_count: 0,
            host_peak_bytes: 0,
            retained_host_bytes: 0,
            transferred_device_bytes: 0,
        }
    }

    const fn is_empty(self) -> bool {
        self.entry_count == 0
    }

    fn should_split_before(
        self,
        backend: BackendId,
        policy: TransferPolicy,
        entry: TransferEntryPlan,
    ) -> Result<bool, LoadError> {
        if self.is_empty() {
            return Ok(false);
        }
        let projected_peak = candidate_peak(
            backend,
            self.retained_host_bytes,
            self.host_peak_bytes,
            entry,
        )?;
        Ok(self.entry_count == policy.maximum_entries
            || projected_peak > policy.preferred_host_staging_bytes)
    }

    fn push(&mut self, backend: BackendId, entry: TransferEntryPlan) -> Result<(), LoadError> {
        self.host_peak_bytes = candidate_peak(
            backend,
            self.retained_host_bytes,
            self.host_peak_bytes,
            entry,
        )?;
        self.retained_host_bytes = self
            .retained_host_bytes
            .checked_add(entry.retained_host_bytes)
            .ok_or_else(|| numeric_error(backend))?;
        self.transferred_device_bytes = self
            .transferred_device_bytes
            .checked_add(entry.execution_bytes)
            .ok_or_else(|| numeric_error(backend))?;
        self.entry_count = self
            .entry_count
            .checked_add(1)
            .ok_or_else(|| numeric_error(backend))?;
        Ok(())
    }

    const fn finish(self, last_in_shard: bool) -> TransferBatchPlan {
        TransferBatchPlan {
            shard_index: self.shard_index,
            entry_start: self.entry_start,
            entry_count: self.entry_count,
            retained_host_bytes: self.retained_host_bytes,
            transferred_device_bytes: self.transferred_device_bytes,
            last_in_shard,
        }
    }
}

struct TransferPlanBuilder {
    backend: BackendId,
    execution_dtype: DType,
    execution_width: u64,
    policy: TransferPolicy,
    plan: TransferPlan,
}

impl TransferPlanBuilder {
    fn new(
        backend: BackendId,
        execution_dtype: DType,
        policy: TransferPolicy,
    ) -> Result<Self, LoadError> {
        if policy.maximum_entries == 0 {
            return Err(numeric_error(backend));
        }
        let execution_width =
            dtype_bytes(execution_dtype).ok_or_else(|| unsupported_scalar(backend))?;
        Ok(Self {
            backend,
            execution_dtype,
            execution_width,
            policy,
            plan: TransferPlan {
                batches: Vec::new(),
                entries: Vec::new(),
                maximum_host_staging_bytes: 0,
                total_execution_bytes: 0,
                metadata_bytes: 0,
            },
        })
    }

    fn build(mut self, shards: &[InspectedShard]) -> Result<TransferPlan, LoadError> {
        for (shard_index, shard) in shards.iter().enumerate() {
            self.add_shard(shard_index, shard)?;
        }
        self.plan.metadata_bytes = self.plan.calculate_metadata_bytes(self.backend)?;
        Ok(self.plan)
    }

    fn add_shard(&mut self, shard_index: usize, shard: &InspectedShard) -> Result<(), LoadError> {
        let mut pending = PendingTransferBatch::new(shard_index, self.plan.entries.len());
        for (tensor_index, tensor) in shard.tensors.iter().enumerate() {
            if !tensor.required {
                continue;
            }
            let entry = self.entry_plan(shard_index, tensor_index, tensor)?;
            if pending.should_split_before(self.backend, self.policy, entry)? {
                self.finish_batch(pending, false)?;
                pending = PendingTransferBatch::new(shard_index, self.plan.entries.len());
            }
            pending.push(self.backend, entry)?;
            self.retain_entry(entry)?;
        }
        if !pending.is_empty() {
            self.finish_batch(pending, true)?;
        }
        Ok(())
    }

    fn entry_plan(
        &self,
        shard_index: usize,
        tensor_index: usize,
        tensor: &InspectedTensor,
    ) -> Result<TransferEntryPlan, LoadError> {
        let source_dtype = tensor
            .source_dtype
            .executable_dtype()
            .ok_or_else(|| unsupported_scalar(self.backend))?;
        let execution_bytes = tensor
            .element_count
            .checked_mul(self.execution_width)
            .ok_or_else(|| numeric_error(self.backend))?;
        let alignment = tensor
            .source_dtype
            .alignment()
            .ok_or_else(|| unsupported_scalar(self.backend))?;
        let alignment_padding = alignment
            .checked_sub(1)
            .ok_or_else(|| numeric_error(self.backend))?;
        let aligned_payload_bytes = tensor
            .source_bytes
            .checked_add(alignment_padding)
            .ok_or_else(|| numeric_error(self.backend))?;
        let retained_host_bytes = if source_dtype == self.execution_dtype {
            tensor.source_bytes
        } else {
            tensor
                .source_bytes
                .checked_add(execution_bytes)
                .ok_or_else(|| numeric_error(self.backend))?
        };
        Ok(TransferEntryPlan {
            shard_index,
            tensor_index,
            source_bytes: tensor.source_bytes,
            execution_bytes,
            aligned_payload_bytes,
            retained_host_bytes,
        })
    }

    fn retain_entry(&mut self, entry: TransferEntryPlan) -> Result<(), LoadError> {
        self.plan.total_execution_bytes = self
            .plan
            .total_execution_bytes
            .checked_add(entry.execution_bytes)
            .ok_or_else(|| numeric_error(self.backend))?;
        self.plan
            .entries
            .try_reserve(1)
            .map_err(|_| host_memory_failure(self.backend, CODE_INSPECTION_ALLOCATION))?;
        self.plan.entries.push(entry);
        Ok(())
    }

    fn finish_batch(
        &mut self,
        pending: PendingTransferBatch,
        last_in_shard: bool,
    ) -> Result<(), LoadError> {
        self.plan
            .batches
            .try_reserve(1)
            .map_err(|_| host_memory_failure(self.backend, CODE_INSPECTION_ALLOCATION))?;
        self.plan.maximum_host_staging_bytes = self
            .plan
            .maximum_host_staging_bytes
            .max(pending.host_peak_bytes);
        self.plan.batches.push(pending.finish(last_in_shard));
        Ok(())
    }
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

    fn build_with_policy(
        backend: BackendId,
        shards: &[InspectedShard],
        execution_dtype: DType,
        policy: TransferPolicy,
    ) -> Result<Self, LoadError> {
        TransferPlanBuilder::new(backend, execution_dtype, policy)?.build(shards)
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
        assert_eq!(plan.maximum_host_staging_bytes(), 43);
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
        assert!(plan.maximum_host_staging_bytes() > 8);
        Ok(())
    }

    #[test]
    fn arithmetic_overflow_is_explicit() -> Result<(), String> {
        let shards = vec![shard(vec![tensor(SourceTensorDType::F32, u64::MAX, 4)?])?];
        let error = TransferPlan::build(BackendId::new(1), &shards, DType::F32)
            .err()
            .ok_or_else(|| "overflow unexpectedly planned".to_owned())?;
        assert!(
            matches!(error, domain_contracts::LoadError::Backend(failure) if failure.failure.code == CODE_NUMERIC_OVERFLOW)
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
