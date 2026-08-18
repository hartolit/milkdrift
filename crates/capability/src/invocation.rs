use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::{
    BoundedJson, CapabilityId, ContractError, ExtensionKey, IdempotencyKey, InvocationId,
    OperationId, ProviderProfileRef, SideEffectClass, bounded::validate_extensions,
};

const MAX_INPUTS: usize = 256;
const MAX_INPUT_NAME: usize = 128;
const MAX_REFERENCE: usize = 256;
const MAX_EVENT_TEXT: usize = 4_096;

/// Reference to an immutable artifact rather than its unbounded bytes.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ArtifactReference {
    identity: String,
    digest: String,
    media_type: Option<String>,
    size_bytes: Option<u64>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ArtifactReferenceWire {
    identity: String,
    digest: String,
    media_type: Option<String>,
    size_bytes: Option<u64>,
}

impl<'de> Deserialize<'de> for ArtifactReference {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let wire = ArtifactReferenceWire::deserialize(deserializer)?;
        Self::new(wire.identity, wire.digest, wire.media_type, wire.size_bytes)
            .map_err(serde::de::Error::custom)
    }
}

impl ArtifactReference {
    /// Constructs a bounded artifact reference with a lowercase BLAKE3 hex digest.
    pub fn new(
        identity: impl Into<String>,
        digest: impl Into<String>,
        media_type: Option<String>,
        size_bytes: Option<u64>,
    ) -> Result<Self, ContractError> {
        let identity = identity.into();
        let digest = digest.into();
        if identity.is_empty() || identity.len() > MAX_REFERENCE {
            return Err(ContractError::Bounds {
                location: "artifact.identity".to_owned(),
                reason: format!("must contain 1 to {MAX_REFERENCE} bytes"),
            });
        }
        if digest.len() != 64
            || !digest
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            return Err(ContractError::InvalidContract(
                "artifact digest must be 64 lowercase hexadecimal characters".to_owned(),
            ));
        }
        if media_type
            .as_ref()
            .is_some_and(|value| value.is_empty() || value.len() > 128)
        {
            return Err(ContractError::Bounds {
                location: "artifact.media_type".to_owned(),
                reason: "must contain 1 to 128 bytes when supplied".to_owned(),
            });
        }
        Ok(Self {
            identity,
            digest,
            media_type,
            size_bytes,
        })
    }

    fn validate(&self) -> Result<(), ContractError> {
        Self::new(
            self.identity.clone(),
            self.digest.clone(),
            self.media_type.clone(),
            self.size_bytes,
        )
        .map(|_| ())
    }
}

/// Provider-neutral reference used as an invocation input.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case", tag = "type", deny_unknown_fields)]
pub enum InvocationValueReference {
    /// Immutable artifact reference.
    Artifact {
        /// Artifact identity, digest, and optional media facts.
        reference: ArtifactReference,
    },
    /// Bounded value held in a durable workspace.
    WorkspaceValue {
        /// Workspace value identity.
        identity: String,
        /// Exact durable value version.
        version: String,
    },
    /// Small inline control value; never an unbounded artifact.
    Inline {
        /// Bounded structured control value.
        value: BoundedJson,
    },
}

impl InvocationValueReference {
    fn validate(&self) -> Result<(), ContractError> {
        if let Self::Artifact { reference } = self {
            reference.validate()?;
        }
        if let Self::WorkspaceValue { identity, version } = self
            && (identity.is_empty()
                || identity.len() > MAX_REFERENCE
                || version.is_empty()
                || version.len() > MAX_REFERENCE)
        {
            return Err(ContractError::Bounds {
                location: "workspace_value".to_owned(),
                reason: format!("identity and version must contain 1 to {MAX_REFERENCE} bytes"),
            });
        }
        Ok(())
    }
}

/// One named invocation input reference.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct InputReference {
    name: String,
    value: InvocationValueReference,
}

impl InputReference {
    /// Constructs a bounded named input.
    pub fn new(
        name: impl Into<String>,
        value: InvocationValueReference,
    ) -> Result<Self, ContractError> {
        let name = name.into();
        if name.is_empty()
            || name.len() > MAX_INPUT_NAME
            || !name
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.'))
        {
            return Err(ContractError::Bounds {
                location: "input.name".to_owned(),
                reason: format!("must contain 1 to {MAX_INPUT_NAME} safe ASCII bytes"),
            });
        }
        value.validate()?;
        Ok(Self { name, value })
    }

    /// Returns the input name.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    fn validate(&self) -> Result<(), ContractError> {
        if self.name.is_empty()
            || self.name.len() > MAX_INPUT_NAME
            || !self
                .name
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.'))
        {
            return Err(ContractError::Bounds {
                location: "input.name".to_owned(),
                reason: format!("must contain 1 to {MAX_INPUT_NAME} safe ASCII bytes"),
            });
        }
        self.value.validate()
    }
}

/// Immutable request delivered to an executor adapter.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct InvocationRequest {
    invocation: InvocationId,
    capability: CapabilityId,
    operation: OperationId,
    provider_profile: Option<ProviderProfileRef>,
    idempotency_key: Option<IdempotencyKey>,
    inputs: Vec<InputReference>,
    extensions: BTreeMap<ExtensionKey, BoundedJson>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct InvocationRequestWire {
    invocation: InvocationId,
    capability: CapabilityId,
    operation: OperationId,
    provider_profile: Option<ProviderProfileRef>,
    idempotency_key: Option<IdempotencyKey>,
    inputs: Vec<InputReference>,
    extensions: BTreeMap<ExtensionKey, BoundedJson>,
}

impl<'de> Deserialize<'de> for InvocationRequest {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let wire = InvocationRequestWire::deserialize(deserializer)?;
        Self::new(
            wire.invocation,
            wire.capability,
            wire.operation,
            wire.provider_profile,
            wire.idempotency_key,
            wire.inputs,
            wire.extensions,
        )
        .map_err(serde::de::Error::custom)
    }
}

impl InvocationRequest {
    /// Constructs a completely validated invocation request.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        invocation: InvocationId,
        capability: CapabilityId,
        operation: OperationId,
        provider_profile: Option<ProviderProfileRef>,
        idempotency_key: Option<IdempotencyKey>,
        inputs: Vec<InputReference>,
        extensions: BTreeMap<ExtensionKey, BoundedJson>,
    ) -> Result<Self, ContractError> {
        if inputs.len() > MAX_INPUTS {
            return Err(ContractError::Bounds {
                location: "invocation.inputs".to_owned(),
                reason: format!("at most {MAX_INPUTS} inputs are allowed"),
            });
        }
        let mut names = std::collections::BTreeSet::new();
        for input in &inputs {
            input.validate()?;
            if !names.insert(input.name()) {
                return Err(ContractError::InvalidContract(format!(
                    "duplicate input name '{}'",
                    input.name()
                )));
            }
        }
        validate_extensions(&extensions)?;
        Ok(Self {
            invocation,
            capability,
            operation,
            provider_profile,
            idempotency_key,
            inputs,
            extensions,
        })
    }

    /// Invocation identity used to correlate every event.
    #[must_use]
    pub const fn invocation(&self) -> &InvocationId {
        &self.invocation
    }
}

/// Stable failure classification; adapters report it but never choose workflow state.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorClass {
    /// Authentication or credential-reference failure.
    Authentication,
    /// Authorization policy rejected the request.
    Authorization,
    /// Request was invalid for the selected operation.
    InvalidRequest,
    /// Provider quota or rate limit was reached.
    RateLimit,
    /// Transport failed before a certain outcome was observed.
    Transport,
    /// Provider returned an internal or service failure.
    Provider,
    /// Local adapter contract failed.
    Adapter,
    /// Requested capability or feature was unavailable.
    Unsupported,
    /// Classification could not be established honestly.
    Unknown,
}

/// Structured failure reported by an adapter.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct InvocationFailure {
    /// Stable error classification.
    pub class: ErrorClass,
    /// Whether the adapter considers a retry potentially useful.
    pub retryable: bool,
    /// Bounded adapter or provider error code.
    pub code: String,
    /// Bounded redacted summary suitable for durable history.
    pub message: String,
    /// Optional retry delay supplied by the provider.
    pub retry_after_ms: Option<u64>,
}

impl InvocationFailure {
    fn validate(&self) -> Result<(), ContractError> {
        if self.code.is_empty() || self.code.len() > 128 || self.message.len() > MAX_EVENT_TEXT {
            return Err(ContractError::Bounds {
                location: "invocation.failure".to_owned(),
                reason: format!("code must be 1..=128 bytes and message at most {MAX_EVENT_TEXT}"),
            });
        }
        Ok(())
    }
}

/// Adapter-reported resource and usage measurements.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct UsageObservation {
    /// Provider-defined input units when observed.
    pub input_units: Option<u64>,
    /// Provider-defined output units when observed.
    pub output_units: Option<u64>,
    /// Total wall duration observed by the adapter.
    pub duration_ms: Option<u64>,
    /// Observed monetary cost in millionths when supplied.
    pub cost_micros: Option<u64>,
    /// Currency for `cost_micros`.
    pub currency: Option<String>,
    /// Namespaced bounded provider-specific observations.
    pub extensions: BTreeMap<ExtensionKey, BoundedJson>,
}

impl UsageObservation {
    fn validate(&self) -> Result<(), ContractError> {
        if self.cost_micros.is_some() != self.currency.is_some() {
            return Err(ContractError::InvalidContract(
                "usage cost and currency must be supplied together".to_owned(),
            ));
        }
        validate_extensions(&self.extensions)
    }
}

/// Terminal outcome status, including uncertainty after possible side effects.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TerminalStatus {
    /// Output was produced successfully.
    Success,
    /// Invocation failed with a classified error.
    Failure,
    /// Executor confirmed cancellation.
    Cancelled,
    /// Executor rejected before admission.
    Rejected,
    /// Outcome or side effects could not be determined.
    Uncertain,
}

/// Terminal invocation report.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct InvocationTerminal {
    /// Terminal status.
    pub status: TerminalStatus,
    /// Bounded output references for success or partial uncertain output.
    pub outputs: Vec<ArtifactReference>,
    /// Classified failure when status requires it.
    pub failure: Option<InvocationFailure>,
    /// Observed usage when the adapter supplied it.
    pub usage: Option<UsageObservation>,
    /// Side-effect fact observed at termination.
    pub side_effect: SideEffectClass,
}

impl InvocationTerminal {
    fn validate(&self) -> Result<(), ContractError> {
        if self.outputs.len() > MAX_INPUTS {
            return Err(ContractError::Bounds {
                location: "terminal.outputs".to_owned(),
                reason: format!("at most {MAX_INPUTS} output references are allowed"),
            });
        }
        for output in &self.outputs {
            output.validate()?;
        }
        let requires_failure = matches!(
            self.status,
            TerminalStatus::Failure | TerminalStatus::Rejected | TerminalStatus::Uncertain
        );
        if requires_failure != self.failure.is_some() {
            return Err(ContractError::InvalidContract(
                "failure details must be present exactly for failure, rejection, or uncertainty"
                    .to_owned(),
            ));
        }
        if let Some(failure) = &self.failure {
            failure.validate()?;
        }
        if let Some(usage) = &self.usage {
            usage.validate()?;
        }
        Ok(())
    }
}

/// Bounded progress, output-reference, and terminal event variants.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case", tag = "type", deny_unknown_fields)]
pub enum InvocationEventKind {
    /// Human-readable bounded progress observation.
    Progress {
        /// Bounded redacted progress text.
        message: String,
        /// Completed provider-defined units.
        completed_units: Option<u64>,
        /// Total provider-defined units when known.
        total_units: Option<u64>,
    },
    /// An output artifact became durably referenceable.
    Output {
        /// Declared output name.
        name: String,
        /// Durable artifact reference.
        reference: ArtifactReference,
    },
    /// Exactly one terminal report.
    Terminal {
        /// Terminal status and observations.
        terminal: InvocationTerminal,
    },
}

/// Sequenced adapter event for one invocation.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct InvocationEvent {
    invocation: InvocationId,
    sequence: u64,
    kind: InvocationEventKind,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct InvocationEventWire {
    invocation: InvocationId,
    sequence: u64,
    kind: InvocationEventKind,
}

impl<'de> Deserialize<'de> for InvocationEvent {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let wire = InvocationEventWire::deserialize(deserializer)?;
        Self::new(wire.invocation, wire.sequence, wire.kind).map_err(serde::de::Error::custom)
    }
}

impl InvocationEvent {
    /// Constructs a validated event; sequence starts at one.
    pub fn new(
        invocation: InvocationId,
        sequence: u64,
        kind: InvocationEventKind,
    ) -> Result<Self, ContractError> {
        if sequence == 0 {
            return Err(ContractError::InvalidContract(
                "event sequence must start at one".to_owned(),
            ));
        }
        match &kind {
            InvocationEventKind::Progress {
                message,
                completed_units,
                total_units,
            } => {
                if message.len() > MAX_EVENT_TEXT {
                    return Err(ContractError::Bounds {
                        location: "event.progress.message".to_owned(),
                        reason: format!("must not exceed {MAX_EVENT_TEXT} bytes"),
                    });
                }
                if completed_units
                    .zip(*total_units)
                    .is_some_and(|(done, total)| done > total)
                {
                    return Err(ContractError::InvalidContract(
                        "completed progress units cannot exceed total units".to_owned(),
                    ));
                }
            }
            InvocationEventKind::Output { name, reference } => {
                if name.is_empty() || name.len() > MAX_INPUT_NAME {
                    return Err(ContractError::Bounds {
                        location: "event.output.name".to_owned(),
                        reason: format!("must contain 1 to {MAX_INPUT_NAME} bytes"),
                    });
                }
                reference.validate()?;
            }
            InvocationEventKind::Terminal { terminal } => terminal.validate()?,
        }
        Ok(Self {
            invocation,
            sequence,
            kind,
        })
    }
}

/// Request to cancel one invocation without assuming it will succeed.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CancellationRequest {
    /// Invocation to cancel.
    invocation: InvocationId,
    /// Monotonic request identity supplied by the runtime.
    request_sequence: u64,
    /// Bounded operator/runtime reason.
    reason: String,
}

impl CancellationRequest {
    /// Constructs a validated cancellation request.
    pub fn new(
        invocation: InvocationId,
        request_sequence: u64,
        reason: impl Into<String>,
    ) -> Result<Self, ContractError> {
        let request = Self {
            invocation,
            request_sequence,
            reason: reason.into(),
        };
        request.validate()?;
        Ok(request)
    }

    pub(crate) fn validate(&self) -> Result<(), ContractError> {
        if self.request_sequence == 0 || self.reason.is_empty() || self.reason.len() > 512 {
            return Err(ContractError::InvalidContract(
                "cancellation sequence must be nonzero and reason must contain 1..=512 bytes"
                    .to_owned(),
            ));
        }
        Ok(())
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CancellationRequestWire {
    invocation: InvocationId,
    request_sequence: u64,
    reason: String,
}

impl<'de> Deserialize<'de> for CancellationRequest {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let wire = CancellationRequestWire::deserialize(deserializer)?;
        Self::new(wire.invocation, wire.request_sequence, wire.reason)
            .map_err(serde::de::Error::custom)
    }
}

/// Executor acknowledgement of a cancellation request.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CancellationAcknowledgement {
    /// Invocation being cancelled.
    invocation: InvocationId,
    /// Request sequence being acknowledged.
    request_sequence: u64,
    /// Whether cancellation was accepted for processing.
    accepted: bool,
    /// Whether the executor can guarantee no later externally visible side effect.
    terminal_boundary: bool,
    /// Bounded explanation when rejected or uncertain.
    detail: Option<String>,
}

impl CancellationAcknowledgement {
    /// Constructs a validated cancellation acknowledgement.
    pub fn new(
        invocation: InvocationId,
        request_sequence: u64,
        accepted: bool,
        terminal_boundary: bool,
        detail: Option<String>,
    ) -> Result<Self, ContractError> {
        let acknowledgement = Self {
            invocation,
            request_sequence,
            accepted,
            terminal_boundary,
            detail,
        };
        acknowledgement.validate()?;
        Ok(acknowledgement)
    }

    pub(crate) fn validate(&self) -> Result<(), ContractError> {
        if self.request_sequence == 0
            || self
                .detail
                .as_ref()
                .is_some_and(|detail| detail.len() > 512)
            || (!self.accepted && self.terminal_boundary)
        {
            return Err(ContractError::InvalidContract(
                "invalid cancellation acknowledgement semantics".to_owned(),
            ));
        }
        Ok(())
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CancellationAcknowledgementWire {
    invocation: InvocationId,
    request_sequence: u64,
    accepted: bool,
    terminal_boundary: bool,
    detail: Option<String>,
}

impl<'de> Deserialize<'de> for CancellationAcknowledgement {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let wire = CancellationAcknowledgementWire::deserialize(deserializer)?;
        Self::new(
            wire.invocation,
            wire.request_sequence,
            wire.accepted,
            wire.terminal_boundary,
            wire.detail,
        )
        .map_err(serde::de::Error::custom)
    }
}
