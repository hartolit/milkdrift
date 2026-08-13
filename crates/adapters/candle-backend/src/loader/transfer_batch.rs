//! Failure-safe ownership and commit for one accelerator transfer batch.

use std::collections::HashMap;
use std::mem::size_of;

use candle_core::{DType, Tensor};
use domain_contracts::{BackendId, LoadError};

use crate::failure::{
    CODE_DUPLICATE_TENSOR, CODE_NUMERIC_OVERFLOW, CODE_TENSOR_MAP_ALLOCATION, CODE_TENSOR_TRANSFER,
};

use super::{host_memory_failure, invalid_model_failure};

/// Every host and device endpoint whose asynchronous transfer is not yet
/// transactionally committed.
///
/// The source tensor remains owned even when a converted host tensor exists.
/// This makes the actual retained graph match the transfer plan and leaves all
/// endpoints reachable if synchronization or a later commit checkpoint fails.
#[derive(Debug)]
pub(super) struct TransferBatchEntry {
    shard_index: usize,
    tensor_index: usize,
    name: Option<String>,
    source_tensor: Tensor,
    converted_host_tensor: Option<Tensor>,
    device_tensor: Tensor,
    retained_host_bytes: u64,
    device_bytes: u64,
    committed: bool,
}

impl TransferBatchEntry {
    #[expect(
        clippy::too_many_arguments,
        reason = "the constructor makes every independently owned endpoint and exact byte fact explicit"
    )]
    pub(super) fn new(
        shard_index: usize,
        tensor_index: usize,
        name: String,
        source_tensor: Tensor,
        converted_host_tensor: Option<Tensor>,
        device_tensor: Tensor,
        retained_host_bytes: u64,
        device_bytes: u64,
    ) -> Self {
        Self {
            shard_index,
            tensor_index,
            name: Some(name),
            source_tensor,
            converted_host_tensor,
            device_tensor,
            retained_host_bytes,
            device_bytes,
            committed: false,
        }
    }

    pub(super) const fn coordinate(&self) -> (usize, usize) {
        (self.shard_index, self.tensor_index)
    }

    pub(super) fn host_execution_tensor(&self) -> &Tensor {
        self.converted_host_tensor
            .as_ref()
            .unwrap_or(&self.source_tensor)
    }

    pub(super) fn device_tensor(&self) -> &Tensor {
        &self.device_tensor
    }

    pub(super) const fn retained_host_bytes(&self) -> u64 {
        self.retained_host_bytes
    }

    pub(super) const fn device_bytes(&self) -> u64 {
        self.device_bytes
    }

    pub(super) const fn committed(&self) -> bool {
        self.committed
    }
}

/// The sole owner of all in-flight accelerator entries for one planned batch.
#[derive(Debug)]
pub(super) struct TransferBatchOwner {
    entries: Vec<TransferBatchEntry>,
    maximum_entries: usize,
    active_batch_index: Option<usize>,
    planned_entries: usize,
    retained_host_bytes: u64,
    transferred_device_bytes: u64,
    synchronized: bool,
    committed_entries: usize,
}

impl TransferBatchOwner {
    pub(super) fn allocate(backend: BackendId, maximum_entries: usize) -> Result<Self, LoadError> {
        let mut entries = Vec::new();
        entries
            .try_reserve_exact(maximum_entries)
            .map_err(|_| host_memory_failure(backend, CODE_TENSOR_MAP_ALLOCATION))?;
        Ok(Self {
            entries,
            maximum_entries,
            active_batch_index: None,
            planned_entries: 0,
            retained_host_bytes: 0,
            transferred_device_bytes: 0,
            synchronized: false,
            committed_entries: 0,
        })
    }

    pub(super) fn metadata_bytes(&self, backend: BackendId) -> Result<u64, LoadError> {
        u64::try_from(self.entries.capacity())
            .ok()
            .and_then(|capacity| {
                u64::try_from(size_of::<TransferBatchEntry>())
                    .ok()
                    .and_then(|entry| capacity.checked_mul(entry))
            })
            .ok_or_else(|| numeric_error(backend))
    }

    pub(super) fn begin(
        &mut self,
        backend: BackendId,
        batch_index: usize,
        planned_entries: usize,
    ) -> Result<(), LoadError> {
        if !self.entries.is_empty()
            || self.active_batch_index.is_some()
            || planned_entries == 0
            || planned_entries > self.maximum_entries
        {
            return Err(invalid_model_failure(backend, CODE_TENSOR_TRANSFER));
        }
        self.active_batch_index = Some(batch_index);
        self.planned_entries = planned_entries;
        self.retained_host_bytes = 0;
        self.transferred_device_bytes = 0;
        self.synchronized = false;
        self.committed_entries = 0;
        Ok(())
    }

    pub(super) fn preflight_push(
        &self,
        backend: BackendId,
        retained_host_bytes: u64,
        device_bytes: u64,
    ) -> Result<(u64, u64), LoadError> {
        if self.active_batch_index.is_none()
            || self.synchronized
            || self.entries.len() >= self.planned_entries
            || self.entries.len() >= self.maximum_entries
        {
            return Err(invalid_model_failure(backend, CODE_TENSOR_TRANSFER));
        }
        let next_retained_host_bytes = self
            .retained_host_bytes
            .checked_add(retained_host_bytes)
            .ok_or_else(|| numeric_error(backend))?;
        let next_transferred_device_bytes = self
            .transferred_device_bytes
            .checked_add(device_bytes)
            .ok_or_else(|| numeric_error(backend))?;
        Ok((next_retained_host_bytes, next_transferred_device_bytes))
    }

    /// Moves a preflighted entry into storage whose complete maximum capacity
    /// was reserved before materialization. No fallible work remains after the
    /// asynchronous transfer endpoints enter this owner.
    pub(super) fn push_preflighted(&mut self, entry: TransferBatchEntry, next_bytes: (u64, u64)) {
        debug_assert!(self.active_batch_index.is_some());
        debug_assert!(!self.synchronized);
        debug_assert!(self.entries.len() < self.planned_entries);
        debug_assert!(self.entries.len() < self.maximum_entries);
        debug_assert_eq!(
            self.retained_host_bytes
                .checked_add(entry.retained_host_bytes()),
            Some(next_bytes.0)
        );
        debug_assert_eq!(
            self.transferred_device_bytes
                .checked_add(entry.device_bytes()),
            Some(next_bytes.1)
        );
        self.retained_host_bytes = next_bytes.0;
        self.transferred_device_bytes = next_bytes.1;
        self.entries.push(entry);
    }

    pub(super) fn validate_ready(
        &self,
        backend: BackendId,
        execution_dtype: DType,
    ) -> Result<(), LoadError> {
        if self.active_batch_index.is_none()
            || self.entries.len() != self.planned_entries
            || self.entries.is_empty()
            || self.synchronized
            || self.committed_entries != 0
        {
            return Err(invalid_model_failure(backend, CODE_TENSOR_TRANSFER));
        }
        for entry in &self.entries {
            let host = entry.host_execution_tensor();
            let device = entry.device_tensor();
            if host.dtype() != execution_dtype
                || device.dtype() != execution_dtype
                || host.dims() != device.dims()
            {
                return Err(invalid_model_failure(backend, CODE_TENSOR_TRANSFER));
            }
        }
        Ok(())
    }

    pub(super) fn mark_synchronized(&mut self, backend: BackendId) -> Result<(), LoadError> {
        if self.synchronized || self.entries.len() != self.planned_entries {
            return Err(invalid_model_failure(backend, CODE_TENSOR_TRANSFER));
        }
        self.synchronized = true;
        Ok(())
    }

    /// Inserts one shallow device handle while retaining the original and all
    /// host endpoints in this owner until the complete batch commits.
    pub(super) fn commit_next(
        &mut self,
        backend: BackendId,
        final_tensors: &mut HashMap<String, Tensor>,
    ) -> Result<(usize, usize), LoadError> {
        if !self.synchronized || self.committed_entries >= self.entries.len() {
            return Err(invalid_model_failure(backend, CODE_TENSOR_TRANSFER));
        }
        let entry = self
            .entries
            .get_mut(self.committed_entries)
            .ok_or_else(|| invalid_model_failure(backend, CODE_TENSOR_TRANSFER))?;
        if entry.committed {
            return Err(invalid_model_failure(backend, CODE_TENSOR_TRANSFER));
        }
        let name = entry
            .name
            .as_ref()
            .ok_or_else(|| invalid_model_failure(backend, CODE_TENSOR_TRANSFER))?;
        if final_tensors.contains_key(name.as_str()) {
            return Err(invalid_model_failure(backend, CODE_DUPLICATE_TENSOR));
        }
        let name = entry
            .name
            .take()
            .ok_or_else(|| invalid_model_failure(backend, CODE_TENSOR_TRANSFER))?;
        final_tensors.insert(name, entry.device_tensor.clone());
        entry.committed = true;
        self.committed_entries = self
            .committed_entries
            .checked_add(1)
            .ok_or_else(|| numeric_error(backend))?;
        Ok(entry.coordinate())
    }

    pub(super) fn finish(&mut self, backend: BackendId) -> Result<(), LoadError> {
        if !self.synchronized
            || self.committed_entries != self.entries.len()
            || self.entries.iter().any(|entry| !entry.committed())
        {
            return Err(invalid_model_failure(backend, CODE_TENSOR_TRANSFER));
        }
        self.entries.clear();
        self.active_batch_index = None;
        self.planned_entries = 0;
        self.retained_host_bytes = 0;
        self.transferred_device_bytes = 0;
        self.synchronized = false;
        self.committed_entries = 0;
        Ok(())
    }

    #[cfg(test)]
    pub(super) const fn active_batch_index(&self) -> Option<usize> {
        self.active_batch_index
    }

    #[cfg(test)]
    pub(super) fn len(&self) -> usize {
        self.entries.len()
    }

    pub(super) fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    #[cfg(test)]
    pub(super) const fn retained_host_bytes(&self) -> u64 {
        self.retained_host_bytes
    }

    #[cfg(test)]
    pub(super) const fn transferred_device_bytes(&self) -> u64 {
        self.transferred_device_bytes
    }

    #[cfg(test)]
    pub(super) const fn synchronized(&self) -> bool {
        self.synchronized
    }

    #[cfg(test)]
    pub(super) const fn committed_entries(&self) -> usize {
        self.committed_entries
    }

    pub(super) fn clear(&mut self) {
        self.entries.clear();
        self.active_batch_index = None;
        self.planned_entries = 0;
        self.retained_host_bytes = 0;
        self.transferred_device_bytes = 0;
        self.synchronized = false;
        self.committed_entries = 0;
    }
}

const fn numeric_error(backend: BackendId) -> LoadError {
    invalid_model_failure(backend, CODE_NUMERIC_OVERFLOW)
}
