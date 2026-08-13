//! Retryable sole ownership and cleanup after partial Candle materialization.

use domain_contracts::{BackendFailureKind, FailedLoadOwner, LoadPlan, SynchronizationError};

use crate::failure::{CODE_PARTIAL_LOAD_SYNCHRONIZE, failure};

use super::prepared::CandleLlamaPreparedLoad;

#[cfg(test)]
thread_local! {
    pub(super) static TEST_CLEANUP_SYNCHRONIZATION_FAILURES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// Sole cleanup owner after a Candle Llama materialization failure.
///
/// Only a consumed materialization attempt can produce this type. Cleanup is
/// retryable and idempotent, and a failed cleanup retains the complete owner.
#[must_use = "a failed Candle load retains native resources requiring explicit cleanup"]
#[derive(Debug)]
pub struct CandleLlamaFailedPreparation {
    plan: LoadPlan,
    pub(super) prepared: Option<CandleLlamaPreparedLoad>,
}

impl CandleLlamaPreparedLoad {
    pub(super) fn into_failed(self) -> CandleLlamaFailedPreparation {
        CandleLlamaFailedPreparation {
            plan: self.plan,
            prepared: Some(self),
        }
    }

    fn cleanup_failed_materialization(&mut self) -> Result<(), SynchronizationError> {
        if self.cleanup_complete {
            return Ok(());
        }
        #[cfg(test)]
        if TEST_CLEANUP_SYNCHRONIZATION_FAILURES.with(|remaining| {
            let value = remaining.get();
            if value == 0 {
                false
            } else {
                remaining.set(value - 1);
                true
            }
        }) {
            return Err(SynchronizationError::Backend(failure(
                self.backend,
                BackendFailureKind::Synchronization,
                CODE_PARTIAL_LOAD_SYNCHRONIZE,
            )));
        }
        if let Some(device) = &self.device {
            device.synchronize().map_err(|_| {
                SynchronizationError::Backend(failure(
                    self.backend,
                    BackendFailureKind::Synchronization,
                    CODE_PARTIAL_LOAD_SYNCHRONIZE,
                ))
            })?;
        }

        self.constructed_model = None;
        self.final_tensors.clear();
        self.pending_device_tensor = None;
        self.pending_host_tensor = None;
        self.pending_source_tensor = None;
        self.shards.clear();
        self.config = None;
        self.device = None;
        self.cleanup_complete = true;
        Ok(())
    }
}

impl FailedLoadOwner for CandleLlamaFailedPreparation {
    fn plan(&self) -> &LoadPlan {
        &self.plan
    }

    fn cleanup(&mut self) -> Result<(), SynchronizationError> {
        let Some(prepared) = self.prepared.as_mut() else {
            return Ok(());
        };
        prepared.cleanup_failed_materialization()?;
        self.prepared = None;
        Ok(())
    }
}
