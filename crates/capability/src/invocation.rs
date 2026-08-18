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

    /// Returns the durable artifact identity.
    #[must_use]
    pub fn identity(&self) -> &str {
        &self.identity
    }

    /// Returns the lowercase BLAKE3 content digest.
    #[must_use]
    pub fn digest(&self) -> &str {
        &self.digest
    }

    /// Returns the declared media type, when supplied.
    #[must_use]
    pub fn media_type(&self) -> Option<&str> {
        self.media_type.as_deref()
    }

    /// Returns the exact artifact byte size, when supplied.
    #[must_use]
    pub const fn size_bytes(&self) -> Option<u64> {
        self.size_bytes
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
#[derive(Clone, Debug, PartialEq, Serialize)]
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

#[derive(Deserialize)]
#[serde(rename_all = "snake_case", tag = "type", deny_unknown_fields)]
enum InvocationValueReferenceWire {
    Artifact { reference: ArtifactReference },
    WorkspaceValue { identity: String, version: String },
    Inline { value: BoundedJson },
}

impl<'de> Deserialize<'de> for InvocationValueReference {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = match InvocationValueReferenceWire::deserialize(deserializer)? {
            InvocationValueReferenceWire::Artifact { reference } => Self::Artifact { reference },
            InvocationValueReferenceWire::WorkspaceValue { identity, version } => {
                Self::WorkspaceValue { identity, version }
            }
            InvocationValueReferenceWire::Inline { value } => Self::Inline { value },
        };
        value.validate().map_err(serde::de::Error::custom)?;
        Ok(value)
    }
}

impl InvocationValueReference {
    /// Returns the artifact reference when this value points to an artifact.
    #[must_use]
    pub const fn artifact(&self) -> Option<&ArtifactReference> {
        match self {
            Self::Artifact { reference } => Some(reference),
            Self::WorkspaceValue { .. } | Self::Inline { .. } => None,
        }
    }

    /// Returns the exact workspace value identity and version when applicable.
    #[must_use]
    pub fn workspace_value(&self) -> Option<(&str, &str)> {
        match self {
            Self::WorkspaceValue { identity, version } => Some((identity, version)),
            Self::Artifact { .. } | Self::Inline { .. } => None,
        }
    }

    /// Returns the bounded inline control value when applicable.
    #[must_use]
    pub const fn inline(&self) -> Option<&BoundedJson> {
        match self {
            Self::Inline { value } => Some(value),
            Self::Artifact { .. } | Self::WorkspaceValue { .. } => None,
        }
    }

    fn validate(&self) -> Result<(), ContractError> {
        if let Self::Artifact { reference } = self {
            reference.validate()?;
        }
        if let Self::WorkspaceValue { identity, version } = self {
            if identity.is_empty()
                || identity.len() > MAX_REFERENCE
                || version.is_empty()
                || version.len() > MAX_REFERENCE
            {
                return Err(ContractError::Bounds {
                    location: "workspace_value".to_owned(),
                    reason: format!("identity and version must contain 1 to {MAX_REFERENCE} bytes"),
                });
            }
        }
        Ok(())
    }
}

/// One named invocation input reference.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct InputReference {
    name: String,
    value: InvocationValueReference,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct InputReferenceWire {
    name: String,
    value: InvocationValueReference,
}

impl<'de> Deserialize<'de> for InputReference {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let wire = InputReferenceWire::deserialize(deserializer)?;
        Self::new(wire.name, wire.value).map_err(serde::de::Error::custom)
    }
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

    /// Returns the immutable value reference supplied for this input.
    #[must_use]
    pub const fn value(&self) -> &InvocationValueReference {
        &self.value
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

    /// Exact capability selected before dispatch.
    #[must_use]
    pub const fn capability(&self) -> &CapabilityId {
        &self.capability
    }

    /// Exact operation selected before dispatch.
    #[must_use]
    pub const fn operation(&self) -> &OperationId {
        &self.operation
    }

    /// Opaque provider-profile reference selected before dispatch.
    #[must_use]
    pub const fn provider_profile(&self) -> Option<&ProviderProfileRef> {
        self.provider_profile.as_ref()
    }

    /// Caller-selected idempotency identity, when the operation accepts one.
    #[must_use]
    pub const fn idempotency_key(&self) -> Option<&IdempotencyKey> {
        self.idempotency_key.as_ref()
    }

    /// Bounded named input references in caller-defined order.
    #[must_use]
    pub fn inputs(&self) -> &[InputReference] {
        &self.inputs
    }

    /// Bounded namespaced invocation extensions.
    #[must_use]
    pub const fn extensions(&self) -> &BTreeMap<ExtensionKey, BoundedJson> {
        &self.extensions
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
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct InvocationFailure {
    /// Stable error classification.
    class: ErrorClass,
    /// Whether the adapter considers a retry potentially useful.
    retryable: bool,
    /// Bounded adapter or provider error code.
    code: String,
    /// Bounded redacted summary suitable for durable history.
    message: String,
    /// Optional retry delay supplied by the provider.
    retry_after_ms: Option<u64>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct InvocationFailureWire {
    class: ErrorClass,
    retryable: bool,
    code: String,
    message: String,
    retry_after_ms: Option<u64>,
}

impl<'de> Deserialize<'de> for InvocationFailure {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let wire = InvocationFailureWire::deserialize(deserializer)?;
        Self::new(
            wire.class,
            wire.retryable,
            wire.code,
            wire.message,
            wire.retry_after_ms,
        )
        .map_err(serde::de::Error::custom)
    }
}

impl InvocationFailure {
    /// Constructs bounded, structured failure details.
    pub fn new(
        class: ErrorClass,
        retryable: bool,
        code: impl Into<String>,
        message: impl Into<String>,
        retry_after_ms: Option<u64>,
    ) -> Result<Self, ContractError> {
        let failure = Self {
            class,
            retryable,
            code: code.into(),
            message: message.into(),
            retry_after_ms,
        };
        failure.validate()?;
        Ok(failure)
    }

    /// Returns the stable failure classification.
    #[must_use]
    pub const fn class(&self) -> ErrorClass {
        self.class
    }

    /// Returns whether retry may be useful according to the adapter.
    #[must_use]
    pub const fn retryable(&self) -> bool {
        self.retryable
    }

    /// Returns the bounded adapter or provider error code.
    #[must_use]
    pub fn code(&self) -> &str {
        &self.code
    }

    /// Returns the bounded redacted failure summary.
    #[must_use]
    pub fn message(&self) -> &str {
        &self.message
    }

    /// Returns the provider-supplied retry delay in milliseconds, when observed.
    #[must_use]
    pub const fn retry_after_ms(&self) -> Option<u64> {
        self.retry_after_ms
    }

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
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct UsageObservation {
    /// Provider-defined input units when observed.
    input_units: Option<u64>,
    /// Provider-defined output units when observed.
    output_units: Option<u64>,
    /// Total wall duration observed by the adapter.
    duration_ms: Option<u64>,
    /// Observed monetary cost in millionths when supplied.
    cost_micros: Option<u64>,
    /// Currency for `cost_micros`.
    currency: Option<String>,
    /// Namespaced bounded provider-specific observations.
    extensions: BTreeMap<ExtensionKey, BoundedJson>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct UsageObservationWire {
    input_units: Option<u64>,
    output_units: Option<u64>,
    duration_ms: Option<u64>,
    cost_micros: Option<u64>,
    currency: Option<String>,
    extensions: BTreeMap<ExtensionKey, BoundedJson>,
}

impl<'de> Deserialize<'de> for UsageObservation {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let wire = UsageObservationWire::deserialize(deserializer)?;
        Self::new(
            wire.input_units,
            wire.output_units,
            wire.duration_ms,
            wire.cost_micros,
            wire.currency,
            wire.extensions,
        )
        .map_err(serde::de::Error::custom)
    }
}

impl UsageObservation {
    /// Constructs validated adapter-reported usage measurements.
    pub fn new(
        input_units: Option<u64>,
        output_units: Option<u64>,
        duration_ms: Option<u64>,
        cost_micros: Option<u64>,
        currency: Option<String>,
        extensions: BTreeMap<ExtensionKey, BoundedJson>,
    ) -> Result<Self, ContractError> {
        let usage = Self {
            input_units,
            output_units,
            duration_ms,
            cost_micros,
            currency,
            extensions,
        };
        usage.validate()?;
        Ok(usage)
    }

    /// Returns observed input units.
    #[must_use]
    pub const fn input_units(&self) -> Option<u64> {
        self.input_units
    }

    /// Returns observed output units.
    #[must_use]
    pub const fn output_units(&self) -> Option<u64> {
        self.output_units
    }

    /// Returns observed wall duration in milliseconds.
    #[must_use]
    pub const fn duration_ms(&self) -> Option<u64> {
        self.duration_ms
    }

    /// Returns observed cost in millionths.
    #[must_use]
    pub const fn cost_micros(&self) -> Option<u64> {
        self.cost_micros
    }

    /// Returns the currency associated with observed cost.
    #[must_use]
    pub fn currency(&self) -> Option<&str> {
        self.currency.as_deref()
    }

    /// Returns bounded namespaced provider-specific observations.
    #[must_use]
    pub const fn extensions(&self) -> &BTreeMap<ExtensionKey, BoundedJson> {
        &self.extensions
    }

    fn validate(&self) -> Result<(), ContractError> {
        if self.cost_micros.is_some() != self.currency.is_some() {
            return Err(ContractError::InvalidContract(
                "usage cost and currency must be supplied together".to_owned(),
            ));
        }
        if let Some(currency) = &self.currency {
            if currency.len() != 3 || !currency.bytes().all(|byte| byte.is_ascii_uppercase()) {
                return Err(ContractError::InvalidContract(
                    "usage currency must be a three-letter uppercase ISO code".to_owned(),
                ));
            }
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
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct InvocationTerminal {
    /// Terminal status.
    status: TerminalStatus,
    /// Bounded output references for success or partial uncertain output.
    outputs: Vec<ArtifactReference>,
    /// Classified failure when status requires it.
    failure: Option<InvocationFailure>,
    /// Observed usage when the adapter supplied it.
    usage: Option<UsageObservation>,
    /// Side-effect fact observed at termination.
    side_effect: SideEffectClass,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct InvocationTerminalWire {
    status: TerminalStatus,
    outputs: Vec<ArtifactReference>,
    failure: Option<InvocationFailure>,
    usage: Option<UsageObservation>,
    side_effect: SideEffectClass,
}

impl<'de> Deserialize<'de> for InvocationTerminal {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let wire = InvocationTerminalWire::deserialize(deserializer)?;
        Self::new(
            wire.status,
            wire.outputs,
            wire.failure,
            wire.usage,
            wire.side_effect,
        )
        .map_err(serde::de::Error::custom)
    }
}

impl InvocationTerminal {
    /// Constructs a validated terminal invocation outcome.
    pub fn new(
        status: TerminalStatus,
        outputs: Vec<ArtifactReference>,
        failure: Option<InvocationFailure>,
        usage: Option<UsageObservation>,
        side_effect: SideEffectClass,
    ) -> Result<Self, ContractError> {
        let terminal = Self {
            status,
            outputs,
            failure,
            usage,
            side_effect,
        };
        terminal.validate()?;
        Ok(terminal)
    }

    /// Returns the terminal status.
    #[must_use]
    pub const fn status(&self) -> TerminalStatus {
        self.status
    }

    /// Returns bounded artifact output references.
    #[must_use]
    pub fn outputs(&self) -> &[ArtifactReference] {
        &self.outputs
    }

    /// Returns structured failure details when required by the status.
    #[must_use]
    pub const fn failure(&self) -> Option<&InvocationFailure> {
        self.failure.as_ref()
    }

    /// Returns adapter-observed usage when supplied.
    #[must_use]
    pub const fn usage(&self) -> Option<&UsageObservation> {
        self.usage.as_ref()
    }

    /// Returns the side-effect fact observed at termination.
    #[must_use]
    pub const fn side_effect(&self) -> SideEffectClass {
        self.side_effect
    }

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
        if !matches!(
            self.status,
            TerminalStatus::Success | TerminalStatus::Uncertain
        ) && !self.outputs.is_empty()
        {
            return Err(ContractError::InvalidContract(
                "outputs are permitted only for success or uncertain terminal outcomes".to_owned(),
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
#[derive(Clone, Debug, PartialEq, Serialize)]
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

#[derive(Deserialize)]
#[serde(rename_all = "snake_case", tag = "type", deny_unknown_fields)]
enum InvocationEventKindWire {
    Progress {
        message: String,
        completed_units: Option<u64>,
        total_units: Option<u64>,
    },
    Output {
        name: String,
        reference: ArtifactReference,
    },
    Terminal {
        terminal: InvocationTerminal,
    },
}

impl<'de> Deserialize<'de> for InvocationEventKind {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let kind = match InvocationEventKindWire::deserialize(deserializer)? {
            InvocationEventKindWire::Progress {
                message,
                completed_units,
                total_units,
            } => Self::Progress {
                message,
                completed_units,
                total_units,
            },
            InvocationEventKindWire::Output { name, reference } => Self::Output { name, reference },
            InvocationEventKindWire::Terminal { terminal } => Self::Terminal { terminal },
        };
        kind.validate().map_err(serde::de::Error::custom)?;
        Ok(kind)
    }
}

impl InvocationEventKind {
    /// Returns progress facts when this is a progress event.
    #[must_use]
    pub fn progress(&self) -> Option<(&str, Option<u64>, Option<u64>)> {
        match self {
            Self::Progress {
                message,
                completed_units,
                total_units,
            } => Some((message, *completed_units, *total_units)),
            Self::Output { .. } | Self::Terminal { .. } => None,
        }
    }

    /// Returns the declared output name and artifact when this is an output event.
    #[must_use]
    pub fn output(&self) -> Option<(&str, &ArtifactReference)> {
        match self {
            Self::Output { name, reference } => Some((name, reference)),
            Self::Progress { .. } | Self::Terminal { .. } => None,
        }
    }

    /// Returns the terminal report when this is a terminal event.
    #[must_use]
    pub const fn terminal(&self) -> Option<&InvocationTerminal> {
        match self {
            Self::Terminal { terminal } => Some(terminal),
            Self::Progress { .. } | Self::Output { .. } => None,
        }
    }

    fn validate(&self) -> Result<(), ContractError> {
        match self {
            Self::Progress {
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
            Self::Output { name, reference } => {
                if name.is_empty() || name.len() > MAX_INPUT_NAME {
                    return Err(ContractError::Bounds {
                        location: "event.output.name".to_owned(),
                        reason: format!("must contain 1 to {MAX_INPUT_NAME} bytes"),
                    });
                }
                reference.validate()?;
            }
            Self::Terminal { terminal } => terminal.validate()?,
        }
        Ok(())
    }
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
        kind.validate()?;
        Ok(Self {
            invocation,
            sequence,
            kind,
        })
    }

    /// Invocation identity correlated by this event.
    #[must_use]
    pub const fn invocation(&self) -> &InvocationId {
        &self.invocation
    }

    /// Executor-local event sequence, beginning at one.
    #[must_use]
    pub const fn sequence(&self) -> u64 {
        self.sequence
    }

    /// Immutable event facts.
    #[must_use]
    pub const fn kind(&self) -> &InvocationEventKind {
        &self.kind
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

    /// Invocation whose cancellation was requested.
    #[must_use]
    pub const fn invocation(&self) -> &InvocationId {
        &self.invocation
    }

    /// Monotonic runtime-supplied cancellation request sequence.
    #[must_use]
    pub const fn request_sequence(&self) -> u64 {
        self.request_sequence
    }

    /// Bounded runtime or operator reason.
    #[must_use]
    pub fn reason(&self) -> &str {
        &self.reason
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

    /// Invocation whose cancellation request was acknowledged.
    #[must_use]
    pub const fn invocation(&self) -> &InvocationId {
        &self.invocation
    }

    /// Cancellation request sequence being acknowledged.
    #[must_use]
    pub const fn request_sequence(&self) -> u64 {
        self.request_sequence
    }

    /// Whether cancellation was accepted for processing.
    #[must_use]
    pub const fn accepted(&self) -> bool {
        self.accepted
    }

    /// Whether no later externally visible side effect can occur.
    #[must_use]
    pub const fn terminal_boundary(&self) -> bool {
        self.terminal_boundary
    }

    /// Bounded rejection or uncertainty detail.
    #[must_use]
    pub fn detail(&self) -> Option<&str> {
        self.detail.as_deref()
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
