//! Transactional tensor materialization and batch-commit coordination.

use std::collections::HashMap;
use std::collections::hash_map::Entry;
use std::panic::{AssertUnwindSafe, catch_unwind};

use candle_core::{DType, Device, Tensor};
use candle_transformers::models::llama::{Config, Llama};
use domain_contracts::{
    BackendId, DeviceKind, LoadError, LoadFailureStage, LoadPlan, PreparedLoad,
    TensorFailureLocation,
};

use crate::failure::{
    CODE_DUPLICATE_TENSOR, CODE_MODEL_LOAD, CODE_MODEL_LOAD_PANIC, CODE_NUMERIC_OVERFLOW,
    CODE_TENSOR_MATERIALIZE, CODE_TENSOR_TRANSFER,
};

use super::cleanup::CandleLlamaFailedPreparation;
use super::construction::construct_llama;
use super::manifest::{InspectedShard, SourceTensorDType, TensorShape};
use super::observer::{
    LoadingSynchronization, MaterializationCheckpoint, MaterializationObserver,
    NoopMaterializationObserver,
};
use super::payload::{AlignedPayload, source_tensor};
use super::transfer_batch::{TransferBatchEndpoints, TransferBatchEntry, TransferBatchOwner};
use super::transfer_plan::TransferPlan;
use super::{
    invalid_model_failure, map_candle_load_error, unsupported_scalar, with_stage, with_tensor,
};

/// Exact source-, configuration-, device-, and plan-bound Candle preparation.
///
/// Its retained files, parsed config, selected device, and materialization
/// authority are not cloned or replaced during loading, and its plan remains
/// stable for the value's lifetime. This pre-attempt typestate is always
/// ordinary-drop-safe. A failed materialization consumes it into
/// [`CandleLlamaFailedPreparation`], which becomes the sole explicit cleanup
/// owner of every completed and in-flight tensor and any constructed model.
#[derive(Debug)]
pub struct CandleLlamaPreparedLoad {
    pub(super) backend: BackendId,
    pub(super) plan: LoadPlan,
    pub(super) config: Option<Config>,
    pub(super) execution_dtype: DType,
    pub(super) device: Option<Device>,
    pub(super) shards: Vec<InspectedShard>,
    pub(super) final_tensors: HashMap<String, Tensor>,
    pub(super) pending_source_tensor: Option<Tensor>,
    pub(super) pending_host_tensor: Option<Tensor>,
    pub(super) pending_device_tensor: Option<Tensor>,
    pub(super) transfer_plan: Option<TransferPlan>,
    pub(super) transfer_batch: Option<TransferBatchOwner>,
    pub(super) next_transfer_batch_index: usize,
    pub(super) next_transfer_entry_index: usize,
    pub(super) constructed_model: Option<Llama>,
    #[cfg(any(feature = "benchmark-observation", feature = "cuda-hardware-tests"))]
    pub(super) load_observation: Option<crate::CandleLoadObservationRecorder>,
    #[cfg(feature = "cuda-hardware-tests")]
    pub(super) hardware_load_fault: Option<CandleHardwareLoadFault>,
    pub(super) cleanup_complete: bool,
}

/// Deterministic failure point used only by the opt-in CUDA hardware suite.
#[cfg(feature = "cuda-hardware-tests")]
#[doc(hidden)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CandleHardwareLoadFault {
    /// Fail after one concrete tensor transfer produced its device endpoint.
    AfterDeviceTransfer {
        /// Accepted selected-shard ordinal to fail.
        shard_ordinal: u16,
        /// Accepted tensor-manifest ordinal within the shard to fail.
        tensor_ordinal: u32,
    },
}

#[cfg(feature = "cuda-hardware-tests")]
impl CandleHardwareLoadFault {
    const fn matches(self, location: TensorFailureLocation) -> bool {
        match self {
            Self::AfterDeviceTransfer {
                shard_ordinal,
                tensor_ordinal,
            } => {
                shard_ordinal == location.shard_ordinal && tensor_ordinal == location.tensor_ordinal
            }
        }
    }
}

#[derive(Debug)]
pub(super) struct RequiredTensorFacts {
    pub(super) shard_index: usize,
    pub(super) tensor_index: usize,
    pub(super) name: String,
    pub(super) source_dtype: SourceTensorDType,
    pub(super) shape: TensorShape,
    pub(super) source_bytes: u64,
    pub(super) location: TensorFailureLocation,
}

impl CandleLlamaPreparedLoad {
    pub(super) fn materialize(&mut self) -> Result<(), LoadError> {
        match catch_unwind(AssertUnwindSafe(|| {
            self.materialize_with_observer(&mut NoopMaterializationObserver)
        })) {
            Ok(result) => result,
            Err(_) => Err(invalid_model_failure(self.backend, CODE_MODEL_LOAD_PANIC)),
        }
    }

    fn materialize_with_observer<O: MaterializationObserver>(
        &mut self,
        observer: &mut O,
    ) -> Result<(), LoadError> {
        self.validate_materialization_start()?;
        for shard_index in 0..self.shards.len() {
            self.materialize_shard(shard_index, observer)?;
        }
        self.validate_transfer_completion()?;
        self.construct_model(observer)
    }

    fn validate_materialization_start(&self) -> Result<(), LoadError> {
        if self.cleanup_complete
            || self.constructed_model.is_some()
            || !self.final_tensors.is_empty()
            || self.pending_source_tensor.is_some()
            || self.pending_host_tensor.is_some()
            || self.pending_device_tensor.is_some()
            || self.next_transfer_batch_index != 0
            || self.next_transfer_entry_index != 0
            || self
                .transfer_batch
                .as_ref()
                .is_some_and(|batch| !batch.is_empty())
        {
            Err(invalid_model_failure(self.backend, CODE_MODEL_LOAD))
        } else {
            Ok(())
        }
    }

    pub(super) fn materialize_required_tensor<O: MaterializationObserver>(
        &mut self,
        facts: RequiredTensorFacts,
        payload: AlignedPayload,
        observer: &mut O,
    ) -> Result<(), LoadError> {
        if self.final_tensors.contains_key(facts.name.as_str()) {
            return Err(with_stage(
                invalid_model_failure(self.backend, CODE_DUPLICATE_TENSOR),
                LoadFailureStage::RetainedPlacement,
            ));
        }
        let source_dtype = facts.source_dtype.executable_dtype().ok_or_else(|| {
            with_tensor(
                unsupported_scalar(self.backend),
                LoadFailureStage::ScalarConversion,
                facts.location,
            )
        })?;
        let source_tensor = source_tensor(self.backend, &payload, source_dtype, facts.shape)
            .map_err(|error| {
                with_tensor(error, LoadFailureStage::HostMaterialization, facts.location)
            })?;
        self.pending_source_tensor = Some(source_tensor);
        observer
            .checkpoint(
                MaterializationCheckpoint::SourceOwned {
                    shard_index: facts.shard_index,
                    tensor_index: facts.tensor_index,
                },
                self.backend,
            )
            .map_err(|error| {
                with_tensor(error, LoadFailureStage::HostMaterialization, facts.location)
            })?;
        drop(payload);
        self.validate_pending_source(&facts, source_dtype)?;
        self.cast_if_required(&facts, source_dtype, observer)?;
        self.retain_on_execution_device(facts, observer)
    }

    fn validate_pending_source(
        &self,
        facts: &RequiredTensorFacts,
        source_dtype: DType,
    ) -> Result<(), LoadError> {
        let retained_source = self.pending_source_tensor.as_ref().ok_or_else(|| {
            with_tensor(
                invalid_model_failure(self.backend, CODE_TENSOR_MATERIALIZE),
                LoadFailureStage::HostMaterialization,
                facts.location,
            )
        })?;
        if retained_source.dtype() == source_dtype
            && retained_source.dims() == facts.shape.as_slice()
        {
            Ok(())
        } else {
            Err(with_tensor(
                invalid_model_failure(self.backend, CODE_TENSOR_MATERIALIZE),
                LoadFailureStage::HostMaterialization,
                facts.location,
            ))
        }
    }

    fn cast_if_required<O: MaterializationObserver>(
        &mut self,
        facts: &RequiredTensorFacts,
        source_dtype: DType,
        observer: &mut O,
    ) -> Result<(), LoadError> {
        let accelerator =
            self.plan.accepted_configuration.execution_device.kind == DeviceKind::Cuda;
        if source_dtype == self.execution_dtype {
            if accelerator {
                self.pending_host_tensor = None;
            } else {
                self.pending_host_tensor = self.pending_source_tensor.take();
            }
            observer
                .checkpoint(
                    MaterializationCheckpoint::HostOwned {
                        shard_index: facts.shard_index,
                        tensor_index: facts.tensor_index,
                    },
                    self.backend,
                )
                .map_err(|error| {
                    with_tensor(error, LoadFailureStage::HostMaterialization, facts.location)
                })?;
        } else {
            let converted = self
                .pending_source_tensor
                .as_ref()
                .ok_or_else(|| {
                    with_tensor(
                        invalid_model_failure(self.backend, CODE_TENSOR_MATERIALIZE),
                        LoadFailureStage::ScalarConversion,
                        facts.location,
                    )
                })?
                .to_dtype(self.execution_dtype)
                .map_err(|error| {
                    with_tensor(
                        map_candle_load_error(
                            self.backend,
                            &Device::Cpu,
                            &error,
                            CODE_TENSOR_MATERIALIZE,
                        ),
                        LoadFailureStage::ScalarConversion,
                        facts.location,
                    )
                })?;
            self.pending_host_tensor = Some(converted);
            observer
                .checkpoint(
                    MaterializationCheckpoint::CastOwned {
                        shard_index: facts.shard_index,
                        tensor_index: facts.tensor_index,
                    },
                    self.backend,
                )
                .map_err(|error| {
                    with_tensor(error, LoadFailureStage::ScalarConversion, facts.location)
                })?;
            if !accelerator {
                self.pending_source_tensor = None;
            }
        }
        let host = self
            .pending_host_tensor
            .as_ref()
            .or(self.pending_source_tensor.as_ref())
            .ok_or_else(|| {
                with_tensor(
                    invalid_model_failure(self.backend, CODE_TENSOR_MATERIALIZE),
                    LoadFailureStage::ScalarConversion,
                    facts.location,
                )
            })?;
        if host.dtype() != self.execution_dtype || host.dims() != facts.shape.as_slice() {
            return Err(with_tensor(
                invalid_model_failure(self.backend, CODE_TENSOR_MATERIALIZE),
                LoadFailureStage::ScalarConversion,
                facts.location,
            ));
        }
        Ok(())
    }

    fn retain_on_execution_device<O: MaterializationObserver>(
        &mut self,
        facts: RequiredTensorFacts,
        observer: &mut O,
    ) -> Result<(), LoadError> {
        match self.plan.accepted_configuration.execution_device.kind {
            DeviceKind::Cpu => self.insert_cpu_tensor(facts, observer),
            DeviceKind::Cuda => self.enqueue_transfer(facts, observer),
            _ => Err(LoadError::InvalidConfiguration),
        }
    }

    fn insert_cpu_tensor<O: MaterializationObserver>(
        &mut self,
        facts: RequiredTensorFacts,
        observer: &mut O,
    ) -> Result<(), LoadError> {
        observer
            .checkpoint(
                MaterializationCheckpoint::BeforeCpuMapInsertion {
                    shard_index: facts.shard_index,
                    tensor_index: facts.tensor_index,
                },
                self.backend,
            )
            .map_err(|error| {
                with_tensor(error, LoadFailureStage::RetainedPlacement, facts.location)
            })?;
        let tensor = self.pending_host_tensor.take().ok_or_else(|| {
            with_tensor(
                invalid_model_failure(self.backend, CODE_TENSOR_MATERIALIZE),
                LoadFailureStage::RetainedPlacement,
                facts.location,
            )
        })?;
        match self.final_tensors.entry(facts.name) {
            Entry::Vacant(entry) => {
                entry.insert(tensor);
            }
            Entry::Occupied(entry) => {
                self.pending_host_tensor = Some(tensor);
                let _ = entry;
                return Err(with_stage(
                    invalid_model_failure(self.backend, CODE_DUPLICATE_TENSOR),
                    LoadFailureStage::RetainedPlacement,
                ));
            }
        }
        observer
            .checkpoint(
                MaterializationCheckpoint::CpuMapOwned {
                    shard_index: facts.shard_index,
                    tensor_index: facts.tensor_index,
                },
                self.backend,
            )
            .map_err(|error| {
                with_tensor(error, LoadFailureStage::RetainedPlacement, facts.location)
            })
    }

    #[expect(
        clippy::too_many_lines,
        reason = "one transactional transfer operation keeps preflight, endpoint ownership, validation, and commit-boundary state auditable together"
    )]
    fn enqueue_transfer<O: MaterializationObserver>(
        &mut self,
        facts: RequiredTensorFacts,
        observer: &mut O,
    ) -> Result<(), LoadError> {
        let (batch_index, entry_index, planned_entries, expected) =
            self.current_transfer_expectation(&facts)?;
        if entry_index == 0 {
            self.transfer_batch
                .as_mut()
                .ok_or_else(|| {
                    with_tensor(
                        invalid_model_failure(self.backend, CODE_TENSOR_TRANSFER),
                        LoadFailureStage::DeviceTransfer,
                        facts.location,
                    )
                })?
                .begin(self.backend, batch_index, planned_entries)
                .map_err(|error| {
                    with_tensor(error, LoadFailureStage::DeviceTransfer, facts.location)
                })?;
            self.record_transfer_batch_started();
        }
        let device = self.device.clone().ok_or_else(|| {
            with_tensor(
                invalid_model_failure(self.backend, CODE_TENSOR_TRANSFER),
                LoadFailureStage::DeviceTransfer,
                facts.location,
            )
        })?;
        let host = self
            .pending_host_tensor
            .as_ref()
            .or(self.pending_source_tensor.as_ref())
            .ok_or_else(|| {
                with_tensor(
                    invalid_model_failure(self.backend, CODE_TENSOR_TRANSFER),
                    LoadFailureStage::DeviceTransfer,
                    facts.location,
                )
            })?;
        let next_batch_bytes = self
            .transfer_batch
            .as_ref()
            .ok_or_else(|| {
                with_tensor(
                    invalid_model_failure(self.backend, CODE_TENSOR_TRANSFER),
                    LoadFailureStage::DeviceTransfer,
                    facts.location,
                )
            })?
            .preflight_push(
                self.backend,
                expected.retained_host_bytes(),
                expected.execution_bytes(),
            )
            .map_err(|error| {
                with_tensor(error, LoadFailureStage::DeviceTransfer, facts.location)
            })?;
        let transferred = self
            .pending_host_tensor
            .as_ref()
            .unwrap_or(host)
            .to_device(&device)
            .map_err(|error| {
                with_tensor(
                    map_candle_load_error(self.backend, &device, &error, CODE_TENSOR_TRANSFER),
                    LoadFailureStage::DeviceTransfer,
                    facts.location,
                )
            })?;
        self.pending_device_tensor = Some(transferred);
        #[cfg(feature = "cuda-hardware-tests")]
        if self
            .hardware_load_fault
            .is_some_and(|fault| fault.matches(facts.location))
        {
            return Err(with_tensor(
                crate::failure::load_failure(
                    self.backend,
                    domain_contracts::BackendFailureKind::DeviceExecution,
                    CODE_TENSOR_TRANSFER,
                    LoadFailureStage::DeviceTransfer,
                ),
                LoadFailureStage::DeviceTransfer,
                facts.location,
            ));
        }
        let retained_device = self.pending_device_tensor.as_ref().ok_or_else(|| {
            with_tensor(
                invalid_model_failure(self.backend, CODE_TENSOR_TRANSFER),
                LoadFailureStage::DeviceTransfer,
                facts.location,
            )
        })?;
        if retained_device.dtype() != self.execution_dtype
            || retained_device.dims() != facts.shape.as_slice()
        {
            return Err(with_tensor(
                invalid_model_failure(self.backend, CODE_TENSOR_TRANSFER),
                LoadFailureStage::DeviceTransfer,
                facts.location,
            ));
        }
        let transfer_batch = self.transfer_batch.as_mut().ok_or_else(|| {
            with_tensor(
                invalid_model_failure(self.backend, CODE_TENSOR_TRANSFER),
                LoadFailureStage::DeviceTransfer,
                facts.location,
            )
        })?;
        let source_tensor = self.pending_source_tensor.take();
        let converted_host_tensor = self.pending_host_tensor.take();
        let device_tensor = self.pending_device_tensor.take();
        let (source_tensor, device_tensor) = match (source_tensor, device_tensor) {
            (Some(source_tensor), Some(device_tensor)) => (source_tensor, device_tensor),
            (source_tensor, device_tensor) => {
                self.pending_source_tensor = source_tensor;
                self.pending_host_tensor = converted_host_tensor;
                self.pending_device_tensor = device_tensor;
                return Err(with_tensor(
                    invalid_model_failure(self.backend, CODE_TENSOR_TRANSFER),
                    LoadFailureStage::DeviceTransfer,
                    facts.location,
                ));
            }
        };
        let entry = TransferBatchEntry::new(
            (facts.shard_index, facts.tensor_index),
            facts.name,
            facts.location,
            TransferBatchEndpoints {
                source: source_tensor,
                converted_host: converted_host_tensor,
                device: device_tensor,
            },
            expected.retained_host_bytes(),
            expected.execution_bytes(),
        );
        // Preflight occurred before the asynchronous transfer. The owner is
        // still present and its maximum Vec capacity was reserved during
        // preparation, so moving the endpoints into it performs no fallible
        // work and cannot strand them in a temporary on an error path.
        transfer_batch.push_preflighted(entry, next_batch_bytes);
        self.next_transfer_entry_index = self
            .next_transfer_entry_index
            .checked_add(1)
            .ok_or_else(|| invalid_model_failure(self.backend, CODE_NUMERIC_OVERFLOW))?;
        observer
            .checkpoint(
                MaterializationCheckpoint::TransferEnqueued {
                    batch_index,
                    entry_index,
                },
                self.backend,
            )
            .map_err(|error| {
                with_tensor(error, LoadFailureStage::DeviceTransfer, facts.location)
            })?;

        let is_complete = self.next_transfer_entry_index == planned_entries;
        let is_last_in_shard = self
            .transfer_plan
            .as_ref()
            .and_then(|plan| plan.batch(batch_index))
            .is_some_and(super::transfer_plan::TransferBatchPlan::is_last_in_shard);
        if is_complete && !is_last_in_shard {
            self.flush_transfer_batch(observer)?;
        }
        Ok(())
    }

    fn current_transfer_expectation(
        &self,
        facts: &RequiredTensorFacts,
    ) -> Result<(usize, usize, usize, super::transfer_plan::TransferEntryPlan), LoadError> {
        let batch_index = self.next_transfer_batch_index;
        let entry_index = self.next_transfer_entry_index;
        let plan = self.transfer_plan.as_ref().ok_or_else(|| {
            with_tensor(
                invalid_model_failure(self.backend, CODE_TENSOR_TRANSFER),
                LoadFailureStage::DeviceTransfer,
                facts.location,
            )
        })?;
        let batch = plan.batch(batch_index).ok_or_else(|| {
            with_tensor(
                invalid_model_failure(self.backend, CODE_TENSOR_TRANSFER),
                LoadFailureStage::DeviceTransfer,
                facts.location,
            )
        })?;
        let expected = plan.entry(batch_index, entry_index).ok_or_else(|| {
            with_tensor(
                invalid_model_failure(self.backend, CODE_TENSOR_TRANSFER),
                LoadFailureStage::DeviceTransfer,
                facts.location,
            )
        })?;
        if expected.coordinate() != (facts.shard_index, facts.tensor_index)
            || expected.source_bytes() != facts.source_bytes
            || batch.shard_index() != facts.shard_index
        {
            return Err(with_tensor(
                invalid_model_failure(self.backend, CODE_TENSOR_TRANSFER),
                LoadFailureStage::DeviceTransfer,
                facts.location,
            ));
        }
        Ok((batch_index, entry_index, batch.entry_count(), expected))
    }

    pub(super) fn flush_shard_final_batch<O: MaterializationObserver>(
        &mut self,
        shard_index: usize,
        observer: &mut O,
    ) -> Result<(), LoadError> {
        if self.plan.accepted_configuration.execution_device.kind != DeviceKind::Cuda {
            return Ok(());
        }
        let Some(batch) = self
            .transfer_plan
            .as_ref()
            .and_then(|plan| plan.batch(self.next_transfer_batch_index))
        else {
            return Ok(());
        };
        if batch.shard_index() > shard_index {
            return Ok(());
        }
        if batch.shard_index() != shard_index
            || !batch.is_last_in_shard()
            || self.next_transfer_entry_index != batch.entry_count()
        {
            return Err(invalid_model_failure(self.backend, CODE_TENSOR_TRANSFER));
        }
        self.flush_transfer_batch(observer)
    }

    #[expect(
        clippy::too_many_lines,
        reason = "one transactional batch close keeps validation, synchronization, per-entry commit, and final accounting in order"
    )]
    fn flush_transfer_batch<O: MaterializationObserver>(
        &mut self,
        observer: &mut O,
    ) -> Result<(), LoadError> {
        let batch_index = self.next_transfer_batch_index;
        let entries = self.next_transfer_entry_index;
        let planned_batch = self
            .transfer_plan
            .as_ref()
            .and_then(|plan| plan.batch(batch_index))
            .ok_or_else(|| invalid_model_failure(self.backend, CODE_TENSOR_TRANSFER))?;
        let device = self
            .device
            .as_ref()
            .ok_or_else(|| invalid_model_failure(self.backend, CODE_TENSOR_TRANSFER))?;
        let transfer_batch = self
            .transfer_batch
            .as_ref()
            .ok_or_else(|| invalid_model_failure(self.backend, CODE_TENSOR_TRANSFER))?;
        transfer_batch.validate_ready(self.backend, self.execution_dtype)?;
        if planned_batch.entry_count() != entries
            || planned_batch.retained_host_bytes() != transfer_batch.retained_host_bytes()
            || planned_batch.transferred_device_bytes() != transfer_batch.transferred_device_bytes()
        {
            return Err(invalid_model_failure(self.backend, CODE_TENSOR_TRANSFER));
        }
        observer
            .checkpoint(
                MaterializationCheckpoint::BeforeBatchSynchronization {
                    batch_index,
                    entries,
                },
                self.backend,
            )
            .map_err(|error| with_stage(error, LoadFailureStage::LoadSynchronization))?;
        self.record_loading_synchronization();
        observer
            .synchronize(
                LoadingSynchronization::TransferBatch { batch_index },
                self.backend,
                device,
            )
            .map_err(|error| with_stage(error, LoadFailureStage::LoadSynchronization))?;
        self.transfer_batch
            .as_mut()
            .ok_or_else(|| invalid_model_failure(self.backend, CODE_TENSOR_TRANSFER))?
            .mark_synchronized(self.backend)
            .map_err(|error| with_stage(error, LoadFailureStage::LoadSynchronization))?;
        observer
            .checkpoint(
                MaterializationCheckpoint::BatchSynchronized {
                    batch_index,
                    entries,
                },
                self.backend,
            )
            .map_err(|error| with_stage(error, LoadFailureStage::LoadSynchronization))?;
        observer
            .checkpoint(
                MaterializationCheckpoint::BeforeBatchCommit {
                    batch_index,
                    entries,
                },
                self.backend,
            )
            .map_err(|error| with_stage(error, LoadFailureStage::RetainedPlacement))?;
        for entry_index in 0..entries {
            let (shard_index, tensor_index, location) = self
                .transfer_batch
                .as_mut()
                .ok_or_else(|| invalid_model_failure(self.backend, CODE_TENSOR_TRANSFER))?
                .commit_next(self.backend, &mut self.final_tensors)?;
            observer
                .checkpoint(
                    MaterializationCheckpoint::BatchEntryCommitted {
                        batch_index,
                        entry_index,
                        shard_index,
                        tensor_index,
                    },
                    self.backend,
                )
                .map_err(|error| {
                    with_tensor(error, LoadFailureStage::RetainedPlacement, location)
                })?;
        }
        observer
            .checkpoint(
                MaterializationCheckpoint::BatchCommitted {
                    batch_index,
                    entries,
                },
                self.backend,
            )
            .map_err(|error| with_stage(error, LoadFailureStage::RetainedPlacement))?;
        self.transfer_batch
            .as_mut()
            .ok_or_else(|| invalid_model_failure(self.backend, CODE_TENSOR_TRANSFER))?
            .finish(self.backend)?;
        self.next_transfer_batch_index = self
            .next_transfer_batch_index
            .checked_add(1)
            .ok_or_else(|| invalid_model_failure(self.backend, CODE_NUMERIC_OVERFLOW))?;
        self.next_transfer_entry_index = 0;
        Ok(())
    }

    fn validate_transfer_completion(&self) -> Result<(), LoadError> {
        match self.plan.accepted_configuration.execution_device.kind {
            DeviceKind::Cpu => {
                if self.transfer_plan.is_none() && self.transfer_batch.is_none() {
                    Ok(())
                } else {
                    Err(invalid_model_failure(self.backend, CODE_TENSOR_TRANSFER))
                }
            }
            DeviceKind::Cuda => {
                let plan = self
                    .transfer_plan
                    .as_ref()
                    .ok_or_else(|| invalid_model_failure(self.backend, CODE_TENSOR_TRANSFER))?;
                let batch = self
                    .transfer_batch
                    .as_ref()
                    .ok_or_else(|| invalid_model_failure(self.backend, CODE_TENSOR_TRANSFER))?;
                if self.next_transfer_batch_index == plan.len()
                    && self.next_transfer_entry_index == 0
                    && batch.is_empty()
                {
                    Ok(())
                } else {
                    Err(invalid_model_failure(self.backend, CODE_TENSOR_TRANSFER))
                }
            }
            _ => Err(LoadError::InvalidConfiguration),
        }
    }

    fn construct_model<O: MaterializationObserver>(
        &mut self,
        observer: &mut O,
    ) -> Result<(), LoadError> {
        let device = self
            .device
            .clone()
            .ok_or_else(|| invalid_model_failure(self.backend, CODE_MODEL_LOAD))?;
        let config = self
            .config
            .as_ref()
            .ok_or_else(|| invalid_model_failure(self.backend, CODE_MODEL_LOAD))?;
        let loaded = construct_llama(
            self.backend,
            &self.final_tensors,
            self.execution_dtype,
            &device,
            config,
        )?;
        self.constructed_model = Some(loaded);
        observer
            .checkpoint(MaterializationCheckpoint::ModelOwned, self.backend)
            .map_err(|error| with_stage(error, LoadFailureStage::ModelConstruction))?;
        Ok(())
    }
}

impl PreparedLoad for CandleLlamaPreparedLoad {
    type Failed = CandleLlamaFailedPreparation;

    fn plan(&self) -> &LoadPlan {
        &self.plan
    }
}

#[cfg(test)]
mod tests;
