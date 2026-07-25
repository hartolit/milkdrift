//! Stable frontend-facing application failures.

use std::error::Error;
use std::fmt::{self, Debug, Display, Formatter};

use domain_contracts::RequestId;

use crate::ApplicationActivity;

/// Infrastructure or adapter category associated with one failure.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ApplicationFailureKind {
    /// Hugging Face artifact resolution failed.
    Hub,
    /// Tokenizer loading, encoding, or streaming decode failed.
    Tokenizer,
    /// Persistent state could not be read or written.
    Storage,
    /// Backend model-source construction failed.
    ModelSource,
    /// Inference runtime rejected or failed an operation.
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
}

impl ApplicationFailure {
    /// Creates a normalized failure from one displayable source.
    #[must_use]
    pub fn new(kind: ApplicationFailureKind, source: impl Display) -> Self {
        Self {
            kind,
            message: source.to_string(),
        }
    }

    /// Creates a normalized failure from one debug-only stable domain error.
    #[must_use]
    pub fn from_debug(kind: ApplicationFailureKind, context: &str, source: impl Debug) -> Self {
        Self {
            kind,
            message: format!("{context}: {source:?}"),
        }
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
    /// Encoded prompt is empty.
    EmptyPrompt,
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
    /// The resolved configuration does not declare a supported scalar type.
    UnknownScalarType,
    /// Visible selection changed after immutable artifact resolution.
    SelectionChanged,
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
            Self::EmptyPrompt => {
                formatter.write_str("direct-completion prompt encoded to no tokens")
            }
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
            Self::UnknownScalarType => formatter.write_str(
                "model configuration does not declare a supported floating-point scalar type",
            ),
            Self::SelectionChanged => formatter.write_str(
                "repository or revision changed after resolution; resolve the current selection \
                 again",
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
