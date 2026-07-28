//! Owned commands and bounded-channel event receipts.

use std::vec::Vec;

use domain_contracts::{
    CancellationReason, DecodeOutcome, DeviceId, DeviceKind, FinishReason, GenerationUsage,
    MemoryFootprint, ModelDescriptor, ModelHandle, ModelId, ModelLifecycleState, PrefillOutcome,
    RequestId, SequenceConfiguration, SequenceId, TokenId, UnloadPolicy,
};

use crate::{CleanupRetryState, GenerationAdmission, GenerationRequest, RuntimeError};

/// Caller-assigned command correlation value.
#[repr(transparent)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CommandTicket(u64);

impl CommandTicket {
    /// Creates a command ticket.
    #[must_use]
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    /// Returns the stable numeric ticket.
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }
}

/// Successful model admission and load receipt.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LoadReceipt {
    /// Runtime-assigned generation-safe model handle.
    pub handle: ModelHandle,
    /// Backend-inspected model description.
    pub descriptor: ModelDescriptor,
    /// Footprint reserved by the runtime registry.
    pub reserved_footprint: MemoryFootprint,
}

/// Successful request-owned sequence creation receipt.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RequestStartReceipt {
    /// Active request identity.
    pub request_id: RequestId,
    /// Created backend sequence identity.
    pub sequence_id: SequenceId,
    /// Required reusable logits elements for incremental decode.
    pub logits_capacity: usize,
    /// Sequence-specific footprint reserved by admission control.
    pub reserved_footprint: MemoryFootprint,
}

/// Successful or graceful prompt-prefill receipt.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PrefillReceipt {
    /// Checked backend outcome.
    pub outcome: PrefillOutcome,
    /// Usage accumulated by this request after the operation.
    pub usage: GenerationUsage,
}

/// Successful or graceful incremental-decode receipt.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DecodeReceipt {
    /// Checked backend outcome.
    pub outcome: DecodeOutcome,
    /// Usage accumulated by this request after the operation.
    pub usage: GenerationUsage,
}

/// Result of an unload request.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UnloadReceipt {
    /// Handle addressed by the unload operation.
    pub handle: ModelHandle,
    /// Resulting lifecycle disposition.
    pub status: UnloadStatus,
    /// Requests cancelled at an already safe runtime boundary.
    pub cancelled_requests: u32,
}

/// Model disposition after applying an unload policy.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnloadStatus {
    /// The exact generation had already completed unloading.
    AlreadyAbsent,
    /// Active work may continue until the mandatory hard timeout.
    Draining,
    /// Backend resources were synchronized and removed from the registry.
    Unloaded,
}

/// Aggregate runtime state without owned model or backend references.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RuntimeSnapshot {
    /// Number of currently resident model instances.
    pub loaded_models: u32,
    /// Number of active request-owned sequences.
    pub active_requests: u32,
    /// Aggregate reserved resident footprint, including quarantined resources.
    pub reserved_footprint: MemoryFootprint,
    /// Generation tasks retaining host workspaces until terminal output is released.
    pub generation_workspaces: u32,
    /// Host workspace bytes retained by active or terminal generation tasks.
    pub reserved_generation_workspace: MemoryFootprint,
    /// Loaded models retained only for pending cleanup.
    pub pending_cleanup_models: u32,
    /// Sequences retained only for pending cleanup.
    pub pending_cleanup_sequences: u32,
    /// Pending model cleanups whose automatic retry budget is exhausted.
    pub exhausted_cleanup_models: u32,
    /// Pending sequence cleanups whose automatic retry budget is exhausted.
    pub exhausted_cleanup_sequences: u32,
    /// Most recent cleanup retry state observed by maintenance.
    pub last_cleanup: Option<CleanupRetryState>,
    /// Most recent unexpected maintenance invariant failure.
    pub maintenance_error: Option<RuntimeError>,
    /// Whether shutdown rejects new work.
    pub shutting_down: bool,
}

/// Snapshot of one resident model generation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ModelSnapshot {
    /// Generation-safe handle.
    pub handle: ModelHandle,
    /// Current deterministic lifecycle state.
    pub lifecycle: ModelLifecycleState,
    /// Inspected backend description.
    pub descriptor: ModelDescriptor,
    /// Model and active-sequence bytes currently reserved under this slot.
    pub reserved_footprint: MemoryFootprint,
    /// Number of normally active request-owned sequences in the slot.
    pub active_requests: u32,
    /// Number of quarantined sequences awaiting explicit destruction.
    pub pending_cleanup_sequences: u32,
    /// Number of sequence cleanups whose automatic retry budget is exhausted.
    pub exhausted_cleanup_sequences: u32,
    /// Whether cleanup failure prevents new request admission.
    pub degraded: bool,
}

/// Successful runtime shutdown receipt.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ShutdownReceipt {
    /// Model instances unloaded during shutdown.
    pub unloaded_models: u32,
    /// Requests cancelled during shutdown.
    pub cancelled_requests: u32,
}

/// Owned command accepted by the single-owner runtime worker.
pub enum RuntimeCommand<S> {
    /// Inspect, admit, and synchronously load one model source.
    LoadModel {
        /// Correlation ticket.
        ticket: CommandTicket,
        /// Logical model identity.
        model_id: ModelId,
        /// Backend-specific owned source descriptor.
        source: S,
        /// Backend-visible device identity.
        device: DeviceId,
        /// Device category.
        device_kind: DeviceKind,
    },
    /// Allocate one request-owned backend sequence.
    StartRequest {
        /// Correlation ticket.
        ticket: CommandTicket,
        /// Exact resident model generation.
        handle: ModelHandle,
        /// Generation request identity.
        request_id: RequestId,
        /// Backend sequence identity.
        sequence_id: SequenceId,
        /// Cold-path sequence bounds.
        configuration: SequenceConfiguration,
    },
    /// Admit a complete generation request for worker-owned scheduling.
    Generate {
        /// Correlation ticket.
        ticket: CommandTicket,
        /// Exact resident model generation.
        handle: ModelHandle,
        /// Token-level runtime request and preallocated policy bounds.
        request: GenerationRequest,
    },
    /// Execute checked prompt prefill using caller-owned reusable logits storage.
    Prefill {
        /// Correlation ticket.
        ticket: CommandTicket,
        /// Active request identity.
        request_id: RequestId,
        /// Owned prompt tokens transferred into the worker.
        tokens: Box<[TokenId]>,
        /// Whether final-position logits are required.
        emit_logits: bool,
        /// Reusable caller-owned logits allocation returned in the event.
        logits: Vec<f32>,
    },
    /// Execute one checked incremental decode step.
    Decode {
        /// Correlation ticket.
        ticket: CommandTicket,
        /// Active request identity.
        request_id: RequestId,
        /// Token appended to sequence state.
        token: TokenId,
        /// Reusable caller-owned logits allocation returned in the event.
        logits: Vec<f32>,
    },
    /// Complete and destroy one request-owned sequence at a safe boundary.
    CompleteRequest {
        /// Correlation ticket.
        ticket: CommandTicket,
        /// Active request identity.
        request_id: RequestId,
        /// Engine-level completion reason.
        reason: FinishReason,
    },
    /// Cancel and destroy one request-owned sequence at a safe boundary.
    CancelRequest {
        /// Correlation ticket.
        ticket: CommandTicket,
        /// Active request identity.
        request_id: RequestId,
        /// Stable cancellation reason.
        reason: CancellationReason,
    },
    /// Apply an explicit unload policy to one exact model generation.
    UnloadModel {
        /// Correlation ticket.
        ticket: CommandTicket,
        /// Exact resident or most recently unloaded generation.
        handle: ModelHandle,
        /// Rejection, immediate cancellation, or bounded draining policy.
        policy: UnloadPolicy,
    },
    /// Read aggregate and per-model state.
    Snapshot {
        /// Correlation ticket.
        ticket: CommandTicket,
    },
    /// Cancel all requests, unload all models, and stop the worker.
    Shutdown {
        /// Correlation ticket.
        ticket: CommandTicket,
    },
}

impl<S> RuntimeCommand<S> {
    /// Returns the caller-assigned correlation ticket.
    #[must_use]
    pub const fn ticket(&self) -> CommandTicket {
        match self {
            Self::LoadModel { ticket, .. }
            | Self::StartRequest { ticket, .. }
            | Self::Generate { ticket, .. }
            | Self::Prefill { ticket, .. }
            | Self::Decode { ticket, .. }
            | Self::CompleteRequest { ticket, .. }
            | Self::CancelRequest { ticket, .. }
            | Self::UnloadModel { ticket, .. }
            | Self::Snapshot { ticket }
            | Self::Shutdown { ticket } => *ticket,
        }
    }
}

/// Event returned through the bounded worker event queue.
pub enum RuntimeEvent {
    /// Completion of a model-load command.
    ModelLoaded {
        /// Correlation ticket.
        ticket: CommandTicket,
        /// Load result.
        result: Result<LoadReceipt, RuntimeError>,
    },
    /// Completion of request-sequence creation.
    RequestStarted {
        /// Correlation ticket.
        ticket: CommandTicket,
        /// Start result.
        result: Result<RequestStartReceipt, RuntimeError>,
    },
    /// Completion of generation admission; token steps continue inside the worker.
    GenerationAdmitted {
        /// Correlation ticket.
        ticket: CommandTicket,
        /// Request admission result.
        result: Result<GenerationAdmission, RuntimeError>,
    },
    /// A cancellation request was recorded for a scheduled generation.
    GenerationCancellationRequested {
        /// Correlation ticket.
        ticket: CommandTicket,
        /// Request identity.
        request_id: RequestId,
        /// Control-plane result; terminal cleanup is published through token output.
        result: Result<(), RuntimeError>,
    },
    /// Completion of checked prompt prefill.
    PrefillCompleted {
        /// Correlation ticket.
        ticket: CommandTicket,
        /// Active request identity.
        request_id: RequestId,
        /// Prefill result.
        result: Result<PrefillReceipt, RuntimeError>,
        /// Reusable logits allocation returned to the caller.
        logits: Vec<f32>,
    },
    /// Completion of one checked decode step.
    DecodeCompleted {
        /// Correlation ticket.
        ticket: CommandTicket,
        /// Active request identity.
        request_id: RequestId,
        /// Decode result.
        result: Result<DecodeReceipt, RuntimeError>,
        /// Reusable logits allocation returned to the caller.
        logits: Vec<f32>,
    },
    /// Completion or cancellation of one active request.
    RequestFinished {
        /// Correlation ticket.
        ticket: CommandTicket,
        /// Request identity.
        request_id: RequestId,
        /// Completion result.
        result: Result<FinishReason, RuntimeError>,
    },
    /// Completion of an unload-policy command.
    ModelUnload {
        /// Correlation ticket.
        ticket: CommandTicket,
        /// Unload result.
        result: Result<UnloadReceipt, RuntimeError>,
    },
    /// Snapshot response.
    Snapshot {
        /// Correlation ticket.
        ticket: CommandTicket,
        /// Aggregate state.
        runtime: RuntimeSnapshot,
        /// Per-model state allocated at this cold inspection boundary.
        models: Vec<ModelSnapshot>,
    },
    /// Worker shutdown response.
    Shutdown {
        /// Correlation ticket.
        ticket: CommandTicket,
        /// Shutdown result.
        result: Result<ShutdownReceipt, RuntimeError>,
    },
}

impl RuntimeEvent {
    /// Returns the command correlation ticket.
    #[must_use]
    pub const fn ticket(&self) -> CommandTicket {
        match self {
            Self::ModelLoaded { ticket, .. }
            | Self::RequestStarted { ticket, .. }
            | Self::GenerationAdmitted { ticket, .. }
            | Self::GenerationCancellationRequested { ticket, .. }
            | Self::PrefillCompleted { ticket, .. }
            | Self::DecodeCompleted { ticket, .. }
            | Self::RequestFinished { ticket, .. }
            | Self::ModelUnload { ticket, .. }
            | Self::Snapshot { ticket, .. }
            | Self::Shutdown { ticket, .. } => *ticket,
        }
    }
}
