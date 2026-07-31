//! Private concrete composition for the local Candle E0 worker.

use candle_backend::{CandleLlamaLoader, CandleLlamaSource};
use domain_contracts::BackendId;
use host_runtime::{OutputPullError, TokenOutputBatch};
use inference_runtime::{
    GenerationOutputState, HostedRuntime, HostedRuntimeConfiguration, RuntimeCommand, RuntimeEvent,
    RuntimeLimits, RuntimeReceiveError, RuntimeThread, start_hosted_runtime,
};

use crate::{ApplicationError, ApplicationFailure, ApplicationFailureKind};

pub const CANDLE_BACKEND_ID: BackendId = BackendId::new(1);

/// Private owner of the concrete, monomorphized Candle E0 endpoint.
pub struct LocalInference {
    runtime: HostedRuntime<CandleLlamaSource>,
    thread: Option<RuntimeThread>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LocalSubmitError {
    Full,
    Disconnected,
}

impl LocalInference {
    pub(crate) fn start(
        limits: RuntimeLimits,
        hosted: HostedRuntimeConfiguration,
    ) -> Result<Self, ApplicationError> {
        let (runtime, thread) =
            start_hosted_runtime(CandleLlamaLoader::new(CANDLE_BACKEND_ID), limits, hosted)
                .map_err(worker_start_failure)?;
        Ok(Self {
            runtime,
            thread: Some(thread),
        })
    }

    pub(crate) fn submit(
        &self,
        command: RuntimeCommand<CandleLlamaSource>,
    ) -> Result<(), LocalSubmitError> {
        match self.runtime.try_submit(command) {
            Ok(()) => Ok(()),
            Err(inference_runtime::RuntimeSubmitError::Full(_)) => Err(LocalSubmitError::Full),
            Err(inference_runtime::RuntimeSubmitError::Disconnected(_)) => {
                Err(LocalSubmitError::Disconnected)
            }
        }
    }

    pub(crate) fn try_receive(&self) -> Result<RuntimeEvent, RuntimeReceiveError> {
        self.runtime.try_receive()
    }

    #[cfg(test)]
    pub(crate) fn receive_timeout(
        &self,
        timeout: std::time::Duration,
    ) -> Result<RuntimeEvent, RuntimeReceiveError> {
        self.runtime.receive_timeout(timeout)
    }

    pub(crate) fn pull_token_output<R, F>(&self, consume: F) -> Result<R, OutputPullError>
    where
        F: for<'batch> FnOnce(TokenOutputBatch<'batch, GenerationOutputState>) -> R,
    {
        self.runtime.pull_token_output(consume)
    }

    pub(crate) const fn runtime(&self) -> &HostedRuntime<CandleLlamaSource> {
        &self.runtime
    }

    pub(crate) const fn take_thread(&mut self) -> Option<RuntimeThread> {
        self.thread.take()
    }

    #[cfg(test)]
    pub(crate) const fn thread_is_present(&self) -> bool {
        self.thread.is_some()
    }
}

fn worker_start_failure(error: inference_runtime::HostedRuntimeStartError) -> ApplicationError {
    ApplicationFailure::new(ApplicationFailureKind::Worker, error).into()
}
