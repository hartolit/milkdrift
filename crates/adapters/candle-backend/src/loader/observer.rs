//! Private materialization checkpoints and bounded load evidence recording.

use candle_core::Device;
use domain_contracts::{BackendFailureKind, BackendId, LoadError};

use crate::failure::{CODE_LOAD_SYNCHRONIZE, failure};

use super::identity::ContentIdentityEstablishment;
use super::prepared::CandleLlamaPreparedLoad;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum HashedRange {
    PrefixHeader,
    IgnoredTensor,
    RequiredTensor,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum MaterializationCheckpoint {
    SourceOwned {
        shard_index: usize,
        tensor_index: usize,
    },
    HostOwned {
        shard_index: usize,
        tensor_index: usize,
    },
    CastOwned {
        shard_index: usize,
        tensor_index: usize,
    },
    TransferEnqueued {
        batch_index: usize,
        entry_index: usize,
    },
    BeforeBatchSynchronization {
        batch_index: usize,
        entries: usize,
    },
    BatchSynchronized {
        batch_index: usize,
        entries: usize,
    },
    BeforeBatchCommit {
        batch_index: usize,
        entries: usize,
    },
    BatchEntryCommitted {
        batch_index: usize,
        entry_index: usize,
        shard_index: usize,
        tensor_index: usize,
    },
    BatchCommitted {
        batch_index: usize,
        entries: usize,
    },
    BeforeCpuMapInsertion {
        shard_index: usize,
        tensor_index: usize,
    },
    CpuMapOwned {
        shard_index: usize,
        tensor_index: usize,
    },
    ModelOwned,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum LoadingSynchronization {
    TransferBatch { batch_index: usize },
}

pub(super) trait MaterializationObserver {
    fn hashed_range(&mut self, _range: HashedRange, _bytes: usize) {}

    fn checkpoint(
        &mut self,
        _checkpoint: MaterializationCheckpoint,
        _backend: BackendId,
    ) -> Result<(), LoadError> {
        Ok(())
    }

    fn whole_shard_verified(&mut self, _establishment: ContentIdentityEstablishment) {}

    fn synchronize(
        &mut self,
        boundary: LoadingSynchronization,
        backend: BackendId,
        device: &Device,
    ) -> Result<(), LoadError> {
        let _ = boundary;
        device.synchronize().map_err(|_| {
            LoadError::Backend(failure(
                backend,
                BackendFailureKind::Synchronization,
                CODE_LOAD_SYNCHRONIZE,
            ))
        })
    }
}

pub(super) struct NoopMaterializationObserver;

impl MaterializationObserver for NoopMaterializationObserver {}

impl CandleLlamaPreparedLoad {
    pub(super) fn record_verification_only_bytes(&self, bytes: usize) {
        #[cfg(any(feature = "benchmark-observation", feature = "cuda-hardware-tests"))]
        if let Some(observation) = &self.load_observation {
            observation.verification_only_bytes_read(u64::try_from(bytes).unwrap_or(u64::MAX));
        }
        #[cfg(not(any(feature = "benchmark-observation", feature = "cuda-hardware-tests")))]
        let _ = (self, bytes);
    }

    pub(super) fn record_required_bytes(&self, bytes: usize) {
        #[cfg(any(feature = "benchmark-observation", feature = "cuda-hardware-tests"))]
        if let Some(observation) = &self.load_observation {
            observation.required_and_verified_bytes_read(u64::try_from(bytes).unwrap_or(u64::MAX));
        }
        #[cfg(not(any(feature = "benchmark-observation", feature = "cuda-hardware-tests")))]
        let _ = (self, bytes);
    }

    pub(super) fn record_transfer_batch_started(&self) {
        #[cfg(any(feature = "benchmark-observation", feature = "cuda-hardware-tests"))]
        if let Some(observation) = &self.load_observation {
            observation.transfer_batches_started(1);
        }
        #[cfg(not(any(feature = "benchmark-observation", feature = "cuda-hardware-tests")))]
        let _ = self;
    }

    pub(super) fn record_loading_synchronization(&self) {
        #[cfg(any(feature = "benchmark-observation", feature = "cuda-hardware-tests"))]
        if let Some(observation) = &self.load_observation {
            observation.loading_device_synchronizations_started(1);
        }
        #[cfg(not(any(feature = "benchmark-observation", feature = "cuda-hardware-tests")))]
        let _ = self;
    }

    pub(super) fn record_materialization_started(&self) {
        #[cfg(any(feature = "benchmark-observation", feature = "cuda-hardware-tests"))]
        if let Some(observation) = &self.load_observation {
            observation.materialization_started();
        }
        #[cfg(not(any(feature = "benchmark-observation", feature = "cuda-hardware-tests")))]
        let _ = self;
    }

    pub(super) fn record_materialization_failed(&self) {
        #[cfg(any(feature = "benchmark-observation", feature = "cuda-hardware-tests"))]
        if let Some(observation) = &self.load_observation {
            observation.materialization_failed();
        }
        #[cfg(not(any(feature = "benchmark-observation", feature = "cuda-hardware-tests")))]
        let _ = self;
    }

    pub(super) fn record_materialization_succeeded(&self) {
        #[cfg(any(feature = "benchmark-observation", feature = "cuda-hardware-tests"))]
        if let Some(observation) = &self.load_observation {
            observation.materialization_succeeded();
        }
        #[cfg(not(any(feature = "benchmark-observation", feature = "cuda-hardware-tests")))]
        let _ = self;
    }
}
