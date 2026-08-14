//! Backend-independent generation admission and bounded scheduler state.

use std::collections::BTreeMap;
use std::num::{NonZeroU32, NonZeroUsize};

use domain_contracts::{
    MemoryFootprint, ModelHandle, ModelLoader, RequestId, SequenceConfiguration, SequenceId,
    TokenId, YieldReason,
};
use host_runtime::TokenOutputProducer;
use sampling::{Sampler, SamplingConfig};

use crate::{
    CleanupFailureReport, CleanupRetryState, InferenceRuntime, RequestStartReceipt, RuntimeError,
};

use self::transition::GenerationPhase;

mod admission;
mod transition;

/// One owned token stop pattern validated before generation begins.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GenerationStopSequence {
    /// Stable caller-defined stop code.
    pub code: u32,
    /// Non-empty token pattern.
    pub tokens: Box<[TokenId]>,
}

/// Minimum shared pull-accumulator capacity required by one request.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GenerationOutputCapacityPolicy {
    /// Minimum token identifiers that must fit before the consumer pulls.
    pub minimum_tokens: NonZeroUsize,
    /// Minimum token/state records that must fit before the consumer pulls.
    pub minimum_records: NonZeroUsize,
}

impl GenerationOutputCapacityPolicy {
    /// Creates an explicit output-capacity requirement.
    #[must_use]
    pub const fn new(minimum_tokens: NonZeroUsize, minimum_records: NonZeroUsize) -> Self {
        Self {
            minimum_tokens,
            minimum_records,
        }
    }
}

impl Default for GenerationOutputCapacityPolicy {
    fn default() -> Self {
        Self::new(NonZeroUsize::MIN, NonZeroUsize::MIN)
    }
}

/// Runtime-level generation request with no frontend or tokenizer state.
#[derive(Clone, Debug, PartialEq)]
pub struct GenerationRequest {
    /// Generation request identity.
    pub request_id: RequestId,
    /// Backend sequence identity.
    pub sequence_id: SequenceId,
    /// Already-tokenized direct-completion prompt.
    pub prompt_tokens: Box<[TokenId]>,
    /// Model sequence bounds used for backend allocation.
    pub sequence: SequenceConfiguration,
    /// Maximum number of sampled output tokens.
    pub maximum_generated_tokens: NonZeroU32,
    /// Immutable sampling policy.
    pub sampling: SamplingConfig,
    /// Deterministic sampler seed.
    pub seed: u64,
    /// Tokens that terminate generation after being published.
    pub eos_tokens: Box<[TokenId]>,
    /// Token suffix patterns that terminate generation after being published.
    pub stop_sequences: Box<[GenerationStopSequence]>,
    /// Minimum capacity required from the shared pull accumulator.
    pub output_capacity: GenerationOutputCapacityPolicy,
}

/// Stable generation outcome retained independently from cleanup disposition.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[expect(
    clippy::large_enum_variant,
    reason = "failed outcomes retain bounded allocation-free runtime evidence and remain Copy"
)]
pub enum GenerationOutcome {
    /// Generation reached a graceful terminal reason.
    Finished(domain_contracts::FinishReason),
    /// Generation failed in the backend, sampler, or runtime.
    Failed(RuntimeError),
}

/// State payload published beside token ranges in the pull accumulator.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GenerationOutputState {
    /// Generation yielded while retaining all request-owned state.
    Yielded(YieldReason),
    /// Generation work ended; explicit sequence cleanup is ordered after this record.
    Terminal(GenerationOutcome),
    /// Explicit sequence destruction failed and ownership remains quarantined.
    CleanupPending {
        /// Original generation outcome.
        outcome: GenerationOutcome,
        /// Primary and cleanup failure classifications.
        failure: CleanupFailureReport,
        /// Current bounded retry state.
        retry: CleanupRetryState,
    },
    /// Automatic cleanup attempts are exhausted and ownership remains retained.
    CleanupExhausted {
        /// Original generation outcome.
        outcome: GenerationOutcome,
        /// Primary and cleanup failure classifications.
        failure: CleanupFailureReport,
        /// Exhausted bounded retry state.
        retry: CleanupRetryState,
    },
    /// Sequence cleanup completed and request accounting was released.
    Released(GenerationOutcome),
}

/// Successful cold admission of a scheduled generation request.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GenerationAdmission {
    /// Backend sequence admission receipt.
    pub request: RequestStartReceipt,
}

#[expect(
    clippy::redundant_pub_crate,
    reason = "the private generation module exposes scheduler state only to the sibling \
              worker module"
)]
pub(super) struct GenerationScheduler {
    requests: BTreeMap<RequestId, GenerationTask>,
    cursor: Option<RequestId>,
}

struct GenerationTask {
    handle: ModelHandle,
    workspace_footprint: MemoryFootprint,
    prompt_tokens: Box<[TokenId]>,
    maximum_generated_tokens: usize,
    eos_tokens: Box<[TokenId]>,
    stop_sequences: Box<[GenerationStopSequence]>,
    sampler: Sampler,
    logits: Vec<f32>,
    sampling_indices: Vec<u32>,
    repetition_epochs: Vec<u32>,
    history: Vec<TokenId>,
    generated: Vec<TokenId>,
    phase: GenerationPhase,
    cancellation: Option<domain_contracts::CancellationReason>,
    pending_yield: Option<YieldReason>,
}

#[expect(
    clippy::redundant_pub_crate,
    reason = "the private generation module exposes scheduler progress only to the sibling \
              worker module"
)]
pub(super) struct SchedulerAdvance {
    pub(super) progressed: bool,
    pub(super) completed: Option<RequestId>,
    pub(super) output_poisoned: bool,
}

impl GenerationScheduler {
    pub(super) const fn new() -> Self {
        Self {
            requests: BTreeMap::new(),
            cursor: None,
        }
    }

    pub(super) fn contains(&self, request_id: RequestId) -> bool {
        self.requests.contains_key(&request_id)
    }

    pub(super) fn request_cancellation(
        &mut self,
        request_id: RequestId,
        reason: domain_contracts::CancellationReason,
    ) -> Result<(), RuntimeError> {
        let task = self
            .requests
            .get_mut(&request_id)
            .ok_or(RuntimeError::RequestNotActive(request_id))?;
        task.cancellation = Some(reason);
        Ok(())
    }

    pub(super) fn request_model_cancellation(
        &mut self,
        model_id: domain_contracts::ModelId,
        reason: domain_contracts::CancellationReason,
    ) {
        for task in self.requests.values_mut() {
            if task.handle.id == model_id {
                task.cancellation = Some(reason);
            }
        }
    }

    pub(super) fn discard_all<L: ModelLoader>(
        &mut self,
        runtime: &mut InferenceRuntime<L>,
    ) -> Result<(), RuntimeError> {
        self.cursor = None;
        let tasks = std::mem::take(&mut self.requests);
        let mut first_error = None;
        for task in tasks.into_values() {
            if let Err(error) = runtime.release_generation_workspace(task.workspace_footprint)
                && first_error.is_none()
            {
                first_error = Some(error);
            }
        }
        first_error.map_or(Ok(()), Err)
    }

    pub(super) fn admit<L: ModelLoader>(
        &mut self,
        runtime: &mut InferenceRuntime<L>,
        output: &TokenOutputProducer<GenerationOutputState>,
        handle: ModelHandle,
        request: GenerationRequest,
    ) -> Result<GenerationAdmission, RuntimeError> {
        admission::admit(self, runtime, output, handle, request)
    }

    pub(super) fn advance<L: ModelLoader>(
        &mut self,
        runtime: &mut InferenceRuntime<L>,
        output: &TokenOutputProducer<GenerationOutputState>,
    ) -> SchedulerAdvance {
        let Some(request_id) = self.next_request() else {
            return idle_advance();
        };
        self.cursor = Some(request_id);
        let result = {
            let Some(task) = self.requests.get_mut(&request_id) else {
                return idle_advance();
            };
            transition::advance_task(runtime, output, request_id, task)
        };
        if result.completed == Some(request_id) {
            self.release_completed(runtime, request_id);
        }
        result
    }

    fn release_completed<L: ModelLoader>(
        &mut self,
        runtime: &mut InferenceRuntime<L>,
        request_id: RequestId,
    ) {
        let Some(task) = self.requests.remove(&request_id) else {
            runtime.record_maintenance_error(RuntimeError::BackendContractViolation);
            return;
        };
        let workspace_footprint = task.workspace_footprint;
        drop(task);
        if let Err(error) = runtime.release_generation_workspace(workspace_footprint) {
            runtime.record_maintenance_error(error);
        }
    }

    fn next_request(&self) -> Option<RequestId> {
        next_request_id(&self.requests, self.cursor)
    }
}

fn next_request_id<T>(
    requests: &BTreeMap<RequestId, T>,
    cursor: Option<RequestId>,
) -> Option<RequestId> {
    cursor
        .and_then(|cursor| {
            requests
                .range((
                    std::ops::Bound::Excluded(cursor),
                    std::ops::Bound::Unbounded,
                ))
                .next()
                .map(|(request_id, _)| *request_id)
        })
        .or_else(|| {
            requests
                .first_key_value()
                .map(|(request_id, _)| *request_id)
        })
}

const fn idle_advance() -> SchedulerAdvance {
    SchedulerAdvance {
        progressed: false,
        completed: None,
        output_poisoned: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_robin_selection_advances_wraps_and_skips_absent_cursors() {
        let requests = [70_u64, 72, 75]
            .map(|request_id| (RequestId::new(request_id), false))
            .into_iter()
            .collect::<BTreeMap<_, _>>();

        assert_eq!(next_request_id(&requests, None), Some(RequestId::new(70)));
        assert_eq!(
            next_request_id(&requests, Some(RequestId::new(70))),
            Some(RequestId::new(72))
        );
        assert_eq!(
            next_request_id(&requests, Some(RequestId::new(71))),
            Some(RequestId::new(72))
        );
        assert_eq!(
            next_request_id(&requests, Some(RequestId::new(75))),
            Some(RequestId::new(70))
        );
        assert_eq!(next_request_id::<bool>(&BTreeMap::new(), None), None);
    }
}
