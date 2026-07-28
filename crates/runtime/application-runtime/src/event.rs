//! Structured application events consumed by Slint, Tauri, CLI, or other frontends.

use domain_contracts::{ModelHandle, RequestId};

use crate::{ApplicationFailure, GenerationTerminal, LoadedModel, ResolvedModel};

/// Frontend-neutral result of polling the application orchestrator.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ApplicationEvent {
    /// Immutable model artifacts and tokenizer were resolved successfully.
    ModelResolved {
        /// Validated immutable model summary.
        model: ResolvedModel,
        /// Non-fatal catalogue persistence failure, when persistence failed.
        persistence_warning: Option<ApplicationFailure>,
    },
    /// Artifact resolution or tokenizer validation failed.
    ModelResolutionFailed {
        /// Normalized failure.
        failure: ApplicationFailure,
    },
    /// A model generation was loaded successfully.
    ModelLoaded {
        /// Loaded model summary.
        model: LoadedModel,
    },
    /// Loading failed before a safe resident model became available.
    ModelLoadFailed {
        /// Normalized failure.
        failure: ApplicationFailure,
    },
    /// Tokenizer and model metadata were incompatible and unload was requested.
    ModelCompatibilityFailed {
        /// Normalized compatibility diagnostic.
        failure: ApplicationFailure,
    },
    /// E0 admitted the complete direct-completion request.
    GenerationStarted {
        /// Active request identity.
        request_id: RequestId,
    },
    /// E0 accepted a cancellation request for the active generation.
    GenerationCancellationRequested {
        /// Request being cancelled.
        request_id: RequestId,
    },
    /// E0 rejected a cancellation request before terminal release.
    GenerationCancellationFailed {
        /// Request addressed by the cancellation command.
        request_id: RequestId,
        /// Normalized failure.
        failure: ApplicationFailure,
    },
    /// Sequence cleanup failed and ownership remains retained by E0.
    GenerationCleanupPending {
        /// Request whose sequence remains retained.
        request_id: RequestId,
        /// Whether the automatic cleanup attempt budget is exhausted.
        exhausted: bool,
        /// Normalized cleanup diagnostic.
        failure: ApplicationFailure,
    },
    /// Generation reached terminal release or failed admission.
    GenerationFinished {
        /// Final frontend-neutral request summary.
        terminal: GenerationTerminal,
    },
    /// Active work is draining before deterministic unload.
    ModelDraining {
        /// Generation being drained.
        handle: ModelHandle,
    },
    /// Model resources are no longer resident.
    ModelUnloaded {
        /// Generation released or confirmed absent.
        handle: ModelHandle,
        /// Requests force-cancelled at safe boundaries.
        cancelled_requests: u32,
    },
    /// Unloading failed and the frontend may retry.
    ModelUnloadFailed {
        /// Normalized failure.
        failure: ApplicationFailure,
    },
    /// Hub worker disconnected and cannot accept further resolution requests.
    HubDisconnected,
    /// Inference worker disconnected and cannot accept further model operations.
    RuntimeDisconnected,
}
