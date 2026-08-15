//! Stable frontend-facing application failures.

use std::error::Error;
use std::fmt::{self, Debug, Display, Formatter};

use domain_contracts::{BackendLoadFailure, RequestId};

use crate::{
    ApplicationActivity, ApplicationDevice, ApplicationDeviceUnavailableReason, GenerationPhase,
};

/// Infrastructure or adapter category associated with one failure.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ApplicationFailureKind {
    /// Hugging Face infrastructure or artifact resolution failed.
    Hub,
    /// Immutable artifact resolution failed without exposing a vendor diagnostic.
    ArtifactResolution,
    /// Model configuration or a declaration field was malformed.
    MalformedArtifactConfiguration,
    /// A present configuration scalar declaration was unsupported.
    UnsupportedArtifactDeclaration,
    /// Modern and legacy configuration declarations contradicted one another.
    ConflictingArtifactDeclaration,
    /// Tokenizer loading, encoding, or streaming decode failed.
    Tokenizer,
    /// Persistent state could not be read or written.
    Storage,
    /// Backend model-source construction failed before lower admission.
    ModelSource,
    /// Immutable artifacts or their format-neutral layout are unsupported.
    UnsupportedArtifact,
    /// Application or lower-runtime memory admission rejected a model load.
    MemoryAdmission,
    /// Model preparation or materialization failed after resolution.
    ModelLoad,
    /// Model resources remain owned or cleanup coordination could not prove release.
    RetainedCleanup,
    /// A successful lower load receipt contradicted stable application compatibility facts.
    IncompatibleReceipt,
    /// Inference runtime rejected or failed an operation outside model loading.
    Inference,
    /// A host worker or bounded output accumulator failed.
    Worker,
}

/// Owned failure that can cross any frontend boundary without vendor types.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ApplicationFailure {
    /// Stable failure category.
    pub kind: ApplicationFailureKind,
    /// Human-readable cold-path diagnostic.
    pub message: String,
    /// Portable bounded backend-load provenance, when this is a lower load failure.
    ///
    /// This is independent of presentation text and never contains tensor names,
    /// paths, vendor errors, or adapter-owned types.
    pub load_diagnostic: Option<BackendLoadFailure>,
}

impl ApplicationFailure {
    /// Creates a normalized failure from one displayable source.
    #[must_use]
    pub fn new(kind: ApplicationFailureKind, source: impl Display) -> Self {
        Self {
            kind,
            message: source.to_string(),
            load_diagnostic: None,
        }
    }

    /// Creates a normalized failure from one debug-only stable domain error.
    #[must_use]
    pub fn from_debug(kind: ApplicationFailureKind, context: &str, source: impl Debug) -> Self {
        Self {
            kind,
            message: format!("{context}: {source:?}"),
            load_diagnostic: None,
        }
    }

    /// Attaches one structured lower backend-load diagnostic.
    #[must_use]
    pub fn with_load_diagnostic(mut self, diagnostic: Option<BackendLoadFailure>) -> Self {
        self.load_diagnostic = diagnostic;
        self
    }

    /// Returns structured lower backend-load provenance, when available.
    #[must_use]
    pub const fn load_diagnostic(&self) -> Option<BackendLoadFailure> {
        self.load_diagnostic
    }
}

impl Display for ApplicationFailure {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}", self.message)
    }
}

impl Error for ApplicationFailure {}

/// Host worker involved in a bounded shutdown failure.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ApplicationWorker {
    /// Inference runtime worker.
    Inference,
    /// Hugging Face resolver worker.
    Hub,
}

/// Invalid runtime-configuration field.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ApplicationConfigurationField {
    /// Maximum active-request count.
    MaximumRequests,
    /// Inference command queue capacity.
    CommandCapacity,
    /// Inference event queue capacity.
    EventCapacity,
    /// Hub command and event queue capacity.
    HubChannelCapacity,
    /// E0 token output capacity.
    TokenOutputCapacity,
    /// E0 token/state record capacity.
    TokenOutputRecordCapacity,
    /// E1 decoded text byte capacity.
    TextOutputByteCapacity,
    /// E1 decoded text/state record capacity.
    TextOutputRecordCapacity,
    /// Combined E1 pending token/state capacity overflowed `usize`.
    PendingGenerationOutputCapacity,
    /// Inference worker poll interval.
    RuntimePoll,
    /// Hub worker poll interval.
    HubWorkerPoll,
    /// Hub event send timeout.
    HubEventSendTimeout,
    /// Hub shutdown command timeout.
    HubCommandShutdownTimeout,
    /// Inference shutdown timeout.
    RuntimeShutdownTimeout,
    /// Inference shutdown-event poll interval.
    RuntimeShutdownEventPoll,
    /// Inference join timeout.
    RuntimeJoinTimeout,
    /// Inference join poll interval.
    RuntimeJoinPoll,
    /// Hub join timeout.
    HubShutdownTimeout,
    /// Hub join poll interval.
    HubShutdownPoll,
    /// Persisted or default repository revision.
    DefaultRevision,
    /// Persisted or default drain timeout.
    DrainTimeout,
}

/// Application-level generation setting rejected before E0 admission.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GenerationSettingsField {
    /// Maximum generated-token count must be non-zero.
    MaximumNewTokens,
    /// Temperature must be finite and positive.
    Temperature,
    /// Top-p must be finite and in `(0, 1]`.
    TopP,
    /// Min-p must be finite and in `[0, 1]`.
    MinP,
    /// Repetition penalty must be finite and positive.
    RepetitionPenalty,
    /// Explicit EOS token identifiers must belong to the loaded vocabulary.
    EndOfSequenceToken,
    /// Textual stop sequences must be non-empty and encode to at least one token.
    StopSequence,
}

/// Immediate command, configuration, or shutdown failure.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ApplicationError {
    /// Static or persisted configuration is invalid.
    InvalidConfiguration(ApplicationConfigurationField),
    /// Application-level generation settings are invalid.
    InvalidGenerationSettings(GenerationSettingsField),
    /// Another stateful operation must complete first.
    Busy(ApplicationActivity),
    /// A loaded model must be unloaded before resolving another selection.
    ModelAlreadyLoaded,
    /// No immutable model artifacts have been resolved.
    NoResolvedModel,
    /// No model generation is currently loaded.
    NoLoadedModel,
    /// No compatible tokenizer is retained for direct completion.
    NoTokenizer,
    /// A generation request is already active or awaiting output release.
    GenerationAlreadyActive(RequestId),
    /// The addressed generation request is not the active request.
    GenerationNotActive(RequestId),
    /// The active request has already crossed the cancellable lifecycle boundary.
    GenerationNotCancellable {
        /// Active request identity.
        request_id: RequestId,
        /// Current non-cancellable phase.
        phase: GenerationPhase,
    },
    /// Encoded direct-completion prompt is empty.
    EmptyPrompt,
    /// A submitted conversation message is empty after trimming.
    EmptyConversationMessage,
    /// Semantic message content contains a reserved renderer control marker.
    ReservedChatMarker,
    /// The resolved model/tokenizer pair has no verified chat profile.
    UnsupportedChatCompatibility,
    /// A system instruction may only be installed before conversation history exists.
    SystemInstructionRequiresEmptyConversation,
    /// No prior assistant attempt is available for regeneration.
    NoRegenerableResponse,
    /// Stable conversation or attempt identity space was exhausted.
    ConversationIdentityExhausted,
    /// Pinned semantic content cannot fit within the available input budget.
    PinnedBudgetExceeded {
        /// Lower bound on required input positions.
        required_at_least: u64,
        /// Available input positions after output reservation.
        available: u64,
    },
    /// Exact-token correction failed to strictly reduce the selected set.
    UnchangedContextCorrection,
    /// Encoded prompt exceeds the model's prefill limit.
    PromptTooLong {
        /// Encoded prompt tokens required.
        required: usize,
        /// Maximum prompt tokens accepted in one E0 prefill.
        available: usize,
    },
    /// Prompt plus configured continuation exceeds the model context window.
    ContextCapacityExceeded {
        /// Total token positions required.
        required: u64,
        /// Maximum token positions available.
        available: u64,
    },
    /// Visible artifact selection changed after immutable artifact resolution.
    SelectionChanged,
    /// Device selection is locked by active or retained runtime ownership.
    DeviceSelectionLocked,
    /// The requested device is not part of the bounded application catalogue.
    DeviceNotInCatalogue(ApplicationDevice),
    /// The explicitly selected device failed its latest bounded availability probe.
    SelectedDeviceUnavailable {
        /// Device that remains selected; no fallback was attempted.
        device: ApplicationDevice,
        /// Stable reason the selected device cannot currently load.
        reason: ApplicationDeviceUnavailableReason,
    },
    /// The startup-fixed accelerator budget is not valid for the selected device's latest facts.
    SelectedDeviceMemoryBudgetUnavailable {
        /// Device that remains selected; no fallback was attempted.
        device: ApplicationDevice,
        /// Accelerator bytes fixed into E0 at process startup.
        budget_bytes: u64,
        /// Latest reported physical capacity, or `None` when discovery omitted it.
        total_memory_bytes: Option<u64>,
    },
    /// No retained model cleanup is currently owned by E1.
    NoRetainedModelCleanup,
    /// Retained cleanup exists, but lower policy or worker state makes E1 retry invalid.
    ModelCleanupNotRetryable,
    /// Correlation ticket space was exhausted.
    TicketExhausted,
    /// Bounded Hub command queue has no capacity.
    HubBusy,
    /// Hub worker is disconnected.
    HubDisconnected,
    /// Bounded inference command queue has no capacity.
    RuntimeBusy,
    /// Inference worker is disconnected.
    RuntimeDisconnected,
    /// Worker did not stop before its deterministic deadline.
    ShutdownTimeout(ApplicationWorker),
    /// Adapter or worker operation failed.
    Failure(ApplicationFailure),
}

impl Display for ApplicationError {
    #[expect(
        clippy::too_many_lines,
        reason = "sole production exception: one exhaustive Display match keeps every stable ApplicationError mapping auditable in one place"
    )]
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidConfiguration(field) => {
                write!(formatter, "invalid application configuration: {field:?}")
            }
            Self::InvalidGenerationSettings(field) => {
                write!(formatter, "invalid generation setting: {field:?}")
            }
            Self::Busy(activity) => {
                write!(
                    formatter,
                    "application operation is already active: {activity:?}"
                )
            }
            Self::ModelAlreadyLoaded => {
                formatter.write_str("unload the resident model before resolving another model")
            }
            Self::NoResolvedModel => formatter.write_str("no model artifacts have been resolved"),
            Self::NoLoadedModel => formatter.write_str("no model generation is loaded"),
            Self::NoTokenizer => formatter.write_str("no tokenizer is available for generation"),
            Self::GenerationAlreadyActive(request_id) => write!(
                formatter,
                "generation request {} is still active or awaiting output release",
                request_id.get()
            ),
            Self::GenerationNotActive(request_id) => write!(
                formatter,
                "generation request {} is not the active request",
                request_id.get()
            ),
            Self::GenerationNotCancellable { request_id, phase } => write!(
                formatter,
                "generation request {} is no longer cancellable in phase {phase:?}",
                request_id.get()
            ),
            Self::EmptyPrompt => {
                formatter.write_str("direct-completion prompt encoded to no tokens")
            }
            Self::EmptyConversationMessage => {
                formatter.write_str("conversation message is empty")
            }
            Self::ReservedChatMarker => formatter.write_str(
                "conversation message contains a marker reserved by the active chat profile",
            ),
            Self::UnsupportedChatCompatibility => formatter.write_str(
                "the resolved model and tokenizer have no verified chat prompt/termination profile; direct completion remains available",
            ),
            Self::SystemInstructionRequiresEmptyConversation => formatter.write_str(
                "a system instruction can only be set before conversation history exists",
            ),
            Self::NoRegenerableResponse => {
                formatter.write_str("no assistant response attempt is available for regeneration")
            }
            Self::ConversationIdentityExhausted => {
                formatter.write_str("conversation identity space is exhausted")
            }
            Self::PinnedBudgetExceeded {
                required_at_least,
                available,
            } => write!(
                formatter,
                "pinned conversation content requires at least {required_at_least} input tokens but only {available} are available",
            ),
            Self::UnchangedContextCorrection => formatter.write_str(
                "exact-token context correction did not reduce the selected non-pinned set",
            ),
            Self::PromptTooLong {
                required,
                available,
            } => write!(
                formatter,
                "encoded prompt requires {required} tokens but prefill accepts {available}"
            ),
            Self::ContextCapacityExceeded {
                required,
                available,
            } => write!(
                formatter,
                "generation requires {required} token positions but model context provides \
                 {available}"
            ),
            Self::SelectionChanged => formatter.write_str(
                "the complete model selection changed after resolution; resolve the current \
                 selection again",
            ),
            Self::DeviceSelectionLocked => formatter.write_str(
                "device selection cannot change while model or generation ownership is active",
            ),
            Self::DeviceNotInCatalogue(device) => {
                write!(formatter, "device {device:?} is not in the application catalogue")
            }
            Self::SelectedDeviceUnavailable { device, reason } => write!(
                formatter,
                "selected device {device:?} is unavailable ({reason:?}); CPU fallback was not attempted",
            ),
            Self::SelectedDeviceMemoryBudgetUnavailable {
                device,
                budget_bytes,
                total_memory_bytes,
            } => write!(
                formatter,
                "selected device {device:?} cannot load under the startup accelerator budget ({budget_bytes} bytes, latest physical total {total_memory_bytes:?}); restart after device discovery changes; CPU fallback was not attempted",
            ),
            Self::NoRetainedModelCleanup => {
                formatter.write_str("no retained model cleanup is available")
            }
            Self::ModelCleanupNotRetryable => formatter.write_str(
                "retained model cleanup cannot be retried under its current lower or worker state",
            ),
            Self::TicketExhausted => formatter.write_str("command ticket space is exhausted"),
            Self::HubBusy => formatter.write_str("Hub resolver queue is full"),
            Self::HubDisconnected => formatter.write_str("Hub resolver is disconnected"),
            Self::RuntimeBusy => formatter.write_str("inference runtime queue is full"),
            Self::RuntimeDisconnected => formatter.write_str("inference runtime is disconnected"),
            Self::ShutdownTimeout(worker) => {
                write!(
                    formatter,
                    "{worker:?} worker did not stop before its deadline"
                )
            }
            Self::Failure(failure) => Display::fmt(failure, formatter),
        }
    }
}

impl Error for ApplicationError {}

impl From<ApplicationFailure> for ApplicationError {
    fn from(value: ApplicationFailure) -> Self {
        Self::Failure(value)
    }
}
