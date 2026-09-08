use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use milkdrift_capability::{ArtifactReference, BoundedJson, ExtensionKey};

use crate::ModelContractError;

const MAX_MESSAGES: usize = 256;
const MAX_PARTS: usize = 1_024;
const MAX_TEXT_BYTES: usize = 1_048_576;
const MAX_TOOLS: usize = 64;
const MAX_TOOL_CALLS: usize = 128;

/// Maximum generated-output units per request; endpoint support may impose a lower limit.
pub const MAX_MODEL_OUTPUT_UNITS: u64 = 4_000_000;

/// Provider-neutral message role. Adapters must reject roles they cannot map.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MessageRole {
    /// Highest-priority system instruction.
    System,
    /// Developer instruction where independently supported.
    Developer,
    /// User message.
    User,
    /// Prior assistant response selected explicitly.
    Assistant,
    /// Result corresponding to an exact tool-call identity.
    ToolResult,
}

/// Text or an immutable content reference in a message.
///
/// References keep large content out of the request document; the adapter materializes
/// supported parts. The current mappings support configured images but refuse generic
/// artifact/file parts. Representable contract variants do not imply endpoint support.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case", tag = "type", deny_unknown_fields)]
pub enum ContentPart {
    /// UTF-8 text.
    Text {
        /// Bounded UTF-8 content.
        text: String,
    },
    /// Generic immutable artifact reference.
    Artifact {
        /// Exact immutable artifact.
        reference: ArtifactReference,
    },
    /// Image reference with exact media facts in the artifact reference.
    Image {
        /// Exact immutable image artifact.
        reference: ArtifactReference,
    },
    /// File/document reference.
    File {
        /// Exact immutable file artifact.
        reference: ArtifactReference,
    },
}

/// One role-labelled message whose parts retain their supplied order.
///
/// Use `ToolResult` with the exact returned call ID when supplying tool results; all
/// other roles must omit that ID. Role labels describe the conversation, not an actor's
/// authority to execute tools or change a workflow.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Message {
    role: MessageRole,
    parts: Vec<ContentPart>,
    tool_call_id: Option<String>,
}

impl Message {
    /// Constructs and validates one message.
    ///
    /// Supply 1..=1,024 parts, with at most 1,048,576 UTF-8 bytes per text part. Tool-result
    /// messages require a nonempty safe call ID; supplying it for another role is refused.
    /// The whole request imposes an additional aggregate text limit.
    pub fn new(
        role: MessageRole,
        parts: Vec<ContentPart>,
        tool_call_id: Option<String>,
    ) -> Result<Self, ModelContractError> {
        let message = Self {
            role,
            parts,
            tool_call_id,
        };
        message.validate()?;
        Ok(message)
    }

    /// Message role.
    #[must_use]
    pub const fn role(&self) -> MessageRole {
        self.role
    }

    /// Ordered message parts.
    #[must_use]
    pub fn parts(&self) -> &[ContentPart] {
        &self.parts
    }

    /// Exact tool-call identity for a tool result.
    #[must_use]
    pub fn tool_call_id(&self) -> Option<&str> {
        self.tool_call_id.as_deref()
    }

    fn validate(&self) -> Result<(), ModelContractError> {
        if self.parts.is_empty() || self.parts.len() > MAX_PARTS {
            return Err(ModelContractError::Invalid(format!(
                "a message must contain 1..={MAX_PARTS} parts"
            )));
        }
        let is_tool = self.role == MessageRole::ToolResult;
        if is_tool != self.tool_call_id.is_some() {
            return Err(ModelContractError::Invalid(
                "tool-result role requires exactly one tool_call_id".to_owned(),
            ));
        }
        if self
            .tool_call_id
            .as_ref()
            .is_some_and(|value| !safe_name(value, 128))
        {
            return Err(ModelContractError::Invalid(
                "tool_call_id is not a bounded safe identity".to_owned(),
            ));
        }
        for part in &self.parts {
            if let ContentPart::Text { text } = part
                && text.len() > MAX_TEXT_BYTES
            {
                return Err(ModelContractError::Bounds {
                    location: "message.parts.text".to_owned(),
                    reason: format!("text exceeds {MAX_TEXT_BYTES} bytes"),
                });
            }
        }
        Ok(())
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MessageWire {
    role: MessageRole,
    parts: Vec<ContentPart>,
    tool_call_id: Option<String>,
}

milkdrift_contracts::deserialize_via!(Message, MessageWire, |wire| Self::new(
    wire.role,
    wire.parts,
    wire.tool_call_id
));

/// Describes a callable shape that a model may request in its response.
///
/// This is schema data for the endpoint, not a capability registration. A returned
/// [`ToolCall`] must go through the normal authorized execution path if a caller chooses
/// to act on it. The input schema is bounded JSON; this constructor does not interpret it.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ToolDefinition {
    name: String,
    description: String,
    input_schema: BoundedJson,
}

impl ToolDefinition {
    /// Constructs a bounded tool definition.
    pub fn new(
        name: impl Into<String>,
        description: impl Into<String>,
        input_schema: BoundedJson,
    ) -> Result<Self, ModelContractError> {
        let value = Self {
            name: name.into(),
            description: description.into(),
            input_schema,
        };
        value.validate()?;
        Ok(value)
    }

    /// Tool name.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }
    /// Tool description.
    #[must_use]
    pub fn description(&self) -> &str {
        &self.description
    }
    /// Bounded input schema.
    #[must_use]
    pub const fn input_schema(&self) -> &BoundedJson {
        &self.input_schema
    }

    fn validate(&self) -> Result<(), ModelContractError> {
        if !safe_name(&self.name, 128) || self.description.len() > 4_096 {
            return Err(ModelContractError::Invalid(
                "tool name or description exceeds its bound".to_owned(),
            ));
        }
        Ok(())
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ToolDefinitionWire {
    name: String,
    description: String,
    input_schema: BoundedJson,
}

milkdrift_contracts::deserialize_via!(ToolDefinition, ToolDefinitionWire, |wire| Self::new(
    wire.name,
    wire.description,
    wire.input_schema
));

/// Asks the endpoint to produce a named JSON result shape.
///
/// `strict` requests schema enforcement from a supporting mapping. Construction checks
/// the name and retains bounded schema data; it does not validate a schema language or
/// check a later response against it.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct StructuredOutput {
    name: String,
    schema: BoundedJson,
    strict: bool,
}

impl StructuredOutput {
    /// Constructs a named bounded output schema.
    pub fn new(
        name: impl Into<String>,
        schema: BoundedJson,
        strict: bool,
    ) -> Result<Self, ModelContractError> {
        let name = name.into();
        if !safe_name(&name, 128) {
            return Err(ModelContractError::Invalid(
                "invalid structured-output name".to_owned(),
            ));
        }
        Ok(Self {
            name,
            schema,
            strict,
        })
    }
    /// Output name.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }
    /// Output JSON schema.
    #[must_use]
    pub const fn schema(&self) -> &BoundedJson {
        &self.schema
    }
    /// Whether exact schema enforcement is required.
    #[must_use]
    pub const fn strict(&self) -> bool {
        self.strict
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct StructuredOutputWire {
    name: String,
    schema: BoundedJson,
    strict: bool,
}

milkdrift_contracts::deserialize_via!(StructuredOutput, StructuredOutputWire, |wire| Self::new(
    wire.name,
    wire.schema,
    wire.strict
));

/// Selects the model request's conversation state explicitly.
///
/// Current endpoint mappings accept only `Fresh`. The continuation variants preserve
/// intent in the contract but are refused before HTTP until a mapping supports them.
/// This request field is separate from the blueprint context-session declaration;
/// runtime currently does not compare the two.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case", tag = "type", deny_unknown_fields)]
pub enum SessionSelection {
    /// Complete selected context is sent for this invocation.
    Fresh,
    /// Exact Milkdrift artifacts define the continuation.
    ExplicitContinuation {
        /// Prior frozen context manifest.
        manifest: ArtifactReference,
        /// Prior canonical model response.
        response: ArtifactReference,
    },
    /// Explicit provider-managed opaque session identity.
    ProviderManaged {
        /// Explicit opaque provider session identity.
        session_id: String,
    },
}

/// Requested reasoning effort, subject to the endpoint profile and protocol mapping.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ReasoningEffort {
    /// Low reasoning effort.
    Low,
    /// Medium reasoning effort.
    Medium,
    /// High reasoning effort.
    High,
}

/// Optional typed reasoning controls; adapters reject unsupported settings.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ReasoningControl {
    /// Abstract effort request.
    pub effort: Option<ReasoningEffort>,
    /// Optional provider-neutral reasoning-unit cap.
    pub maximum_units: Option<u64>,
}

/// Describes the messages and generation choices for one external model request.
///
/// Construct with [`Self::new`], then encode through [`crate::ModelTaskRequestDocument`]
/// for the reserved [`crate::MODEL_TASK_INPUT_NAME`] invocation input. The resolved
/// endpoint profile supplies the model identity. Its adapter checks whether it can map
/// the requested roles, parts, tools, session, reasoning, and streaming choices before
/// contacting the endpoint. Shared validation here does not prove endpoint support.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ModelTaskRequest {
    messages: Vec<Message>,
    tools: Vec<ToolDefinition>,
    structured_output: Option<StructuredOutput>,
    session: SessionSelection,
    reasoning: Option<ReasoningControl>,
    maximum_output_units: u64,
    streaming: bool,
    extensions: BTreeMap<ExtensionKey, BoundedJson>,
}

impl ModelTaskRequest {
    /// Validates messages and generation choices without contacting a model endpoint.
    ///
    /// `maximum_output_units` is an inclusive output allowance in
    /// `1..=MAX_MODEL_OUTPUT_UNITS`, sent as `max_tokens` by the current mappings. It
    /// neither limits input size nor reserves that many bytes. Set `session` explicitly;
    /// both current endpoint mappings support only [`SessionSelection::Fresh`].
    ///
    /// # Errors
    ///
    /// Supply 1..=256 messages, at most 64 distinct tool names, and at most 64 extensions.
    /// Invalid counts, duplicate tools, a zero reasoning cap, or an output allowance outside
    /// `1..=MAX_MODEL_OUTPUT_UNITS` return [`ModelContractError::Invalid`]. Aggregate text
    /// beyond 1,048,576 UTF-8 bytes returns [`ModelContractError::Bounds`].
    ///
    /// A valid allowance can still exceed an endpoint's own limit; the adapter does not
    /// discover numeric token ceilings. Encoded HTTP request/response limits are separate.
    /// See the [crate example](crate) for request construction and document encoding.
    #[allow(clippy::too_many_arguments)] // Messages, tools, output format, session, and generation bounds are validated as one executable model request.
    pub fn new(
        messages: Vec<Message>,
        tools: Vec<ToolDefinition>,
        structured_output: Option<StructuredOutput>,
        session: SessionSelection,
        reasoning: Option<ReasoningControl>,
        maximum_output_units: u64,
        streaming: bool,
        extensions: BTreeMap<ExtensionKey, BoundedJson>,
    ) -> Result<Self, ModelContractError> {
        let value = Self {
            messages,
            tools,
            structured_output,
            session,
            reasoning,
            maximum_output_units,
            streaming,
            extensions,
        };
        value.validate()?;
        Ok(value)
    }
    /// Ordered input messages.
    #[must_use]
    pub fn messages(&self) -> &[Message] {
        &self.messages
    }
    /// Available tool definitions.
    #[must_use]
    pub fn tools(&self) -> &[ToolDefinition] {
        &self.tools
    }
    /// Optional structured-output contract.
    #[must_use]
    pub const fn structured_output(&self) -> Option<&StructuredOutput> {
        self.structured_output.as_ref()
    }
    /// Explicit session behavior.
    #[must_use]
    pub const fn session(&self) -> &SessionSelection {
        &self.session
    }
    /// Optional reasoning controls.
    #[must_use]
    pub const fn reasoning(&self) -> Option<ReasoningControl> {
        self.reasoning
    }
    /// Requested generated-output allowance, sent as `max_tokens` by current adapters.
    /// See [`Self::new`] for its bounds and distinction from byte limits and usage.
    #[must_use]
    pub const fn maximum_output_units(&self) -> u64 {
        self.maximum_output_units
    }
    /// Whether bounded fragments were requested.
    #[must_use]
    pub const fn streaming(&self) -> bool {
        self.streaming
    }
    /// Namespaced explicit provider options.
    #[must_use]
    pub const fn extensions(&self) -> &BTreeMap<ExtensionKey, BoundedJson> {
        &self.extensions
    }

    fn validate(&self) -> Result<(), ModelContractError> {
        if self.messages.is_empty()
            || self.messages.len() > MAX_MESSAGES
            || self.tools.len() > MAX_TOOLS
            || self.maximum_output_units == 0
            || self.maximum_output_units > MAX_MODEL_OUTPUT_UNITS
            || self.extensions.len() > 64
        {
            return Err(ModelContractError::Invalid(
                "model request exceeds count/unit bounds".to_owned(),
            ));
        }
        let mut total_text = 0usize;
        for message in &self.messages {
            message.validate()?;
            for part in message.parts() {
                if let ContentPart::Text { text } = part {
                    total_text = total_text.checked_add(text.len()).ok_or_else(|| {
                        ModelContractError::Invalid("model text accounting overflow".to_owned())
                    })?;
                }
            }
        }
        if total_text > MAX_TEXT_BYTES {
            return Err(ModelContractError::Bounds {
                location: "request.messages".to_owned(),
                reason: format!("aggregate text exceeds {MAX_TEXT_BYTES} bytes"),
            });
        }
        let mut names = BTreeSet::new();
        for tool in &self.tools {
            tool.validate()?;
            if !names.insert(tool.name()) {
                return Err(ModelContractError::Invalid(
                    "duplicate tool name".to_owned(),
                ));
            }
        }
        if let SessionSelection::ProviderManaged { session_id } = &self.session
            && !safe_name(session_id, 512)
        {
            return Err(ModelContractError::Invalid(
                "invalid provider session identity".to_owned(),
            ));
        }
        if self
            .reasoning
            .is_some_and(|control| control.maximum_units == Some(0))
        {
            return Err(ModelContractError::Invalid(
                "reasoning maximum_units must be nonzero".to_owned(),
            ));
        }
        Ok(())
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ModelTaskRequestWire {
    messages: Vec<Message>,
    tools: Vec<ToolDefinition>,
    structured_output: Option<StructuredOutput>,
    session: SessionSelection,
    reasoning: Option<ReasoningControl>,
    maximum_output_units: u64,
    streaming: bool,
    extensions: BTreeMap<ExtensionKey, BoundedJson>,
}

milkdrift_contracts::deserialize_via!(ModelTaskRequest, ModelTaskRequestWire, |w| Self::new(
    w.messages,
    w.tools,
    w.structured_output,
    w.session,
    w.reasoning,
    w.maximum_output_units,
    w.streaming,
    w.extensions,
));

/// A model's proposed tool invocation, preserved as response data.
///
/// `id` correlates a later [`MessageRole::ToolResult`]; `name` and `arguments` describe
/// the proposed call. This constructor checks identity spelling, not whether a tool
/// exists, its argument schema matches, or execution is authorized.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ToolCall {
    id: String,
    name: String,
    arguments: BoundedJson,
}

impl ToolCall {
    /// Constructs a bounded returned tool call.
    pub fn new(
        id: impl Into<String>,
        name: impl Into<String>,
        arguments: BoundedJson,
    ) -> Result<Self, ModelContractError> {
        let value = Self {
            id: id.into(),
            name: name.into(),
            arguments,
        };
        if !safe_name(&value.id, 128) || !safe_name(&value.name, 128) {
            return Err(ModelContractError::Invalid(
                "invalid returned tool call identity".to_owned(),
            ));
        }
        Ok(value)
    }
    /// Provider call identity.
    #[must_use]
    pub fn id(&self) -> &str {
        &self.id
    }
    /// Declared tool name.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }
    /// Structured arguments.
    #[must_use]
    pub const fn arguments(&self) -> &BoundedJson {
        &self.arguments
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ToolCallWire {
    id: String,
    name: String,
    arguments: BoundedJson,
}

milkdrift_contracts::deserialize_via!(ToolCall, ToolCallWire, |wire| Self::new(
    wire.id,
    wire.name,
    wire.arguments
));

/// Stable provider-neutral stop classification.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FinishReason {
    /// Provider reached a natural stop.
    Stop,
    /// Configured output bound was reached.
    Length,
    /// Provider requested one or more tools.
    ToolCalls,
    /// Provider content policy stopped output.
    ContentFilter,
    /// Invocation was cancelled.
    Cancelled,
    /// Provider reported an error stop.
    Error,
    /// Provider reason had no exact neutral mapping.
    Unknown,
}

/// Provider-observed usage and optional cost evidence.
///
/// `None` means unreported, not zero. Units follow the endpoint's accounting; the core
/// does not infer tokenization or prices. A [`ModelResponse`] requires cost and currency
/// together, with a three-letter uppercase currency spelling.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Usage {
    /// Input units.
    pub input_units: Option<u64>,
    /// Output units.
    pub output_units: Option<u64>,
    /// Cache-read units.
    pub cached_input_units: Option<u64>,
    /// Observed cost in millionths.
    pub cost_micros: Option<u64>,
    /// ISO currency for cost.
    pub currency: Option<String>,
}

/// The endpoint's complete reported result, retained for inspection and later work.
///
/// Check [`Self::finish_reason`] alongside text, structured output, and tool calls.
/// Empty text is valid, including a length-limited result, so a successful document
/// round trip is not proof of a useful answer. A tool call remains data. Adapters publish
/// the response and output artifacts before reporting success; incomplete streams do
/// not become a complete response merely because text fragments arrived.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ModelResponse {
    text: String,
    structured: Option<BoundedJson>,
    tool_calls: Vec<ToolCall>,
    finish_reason: FinishReason,
    usage: Usage,
    provider_metadata: BTreeMap<ExtensionKey, BoundedJson>,
}

impl ModelResponse {
    /// Constructs a bounded canonical model response.
    ///
    /// Refuses text above 1,048,576 bytes, more than 128 tool calls or 64 metadata entries,
    /// and inconsistent cost/currency facts. It does not infer missing usage or validate
    /// structured output against the original request's schema.
    pub fn new(
        text: String,
        structured: Option<BoundedJson>,
        tool_calls: Vec<ToolCall>,
        finish_reason: FinishReason,
        usage: Usage,
        provider_metadata: BTreeMap<ExtensionKey, BoundedJson>,
    ) -> Result<Self, ModelContractError> {
        if text.len() > MAX_TEXT_BYTES
            || tool_calls.len() > MAX_TOOL_CALLS
            || provider_metadata.len() > 64
            || usage.cost_micros.is_some() != usage.currency.is_some()
        {
            return Err(ModelContractError::Invalid(
                "model response exceeds bounds or has invalid cost facts".to_owned(),
            ));
        }
        if let Some(currency) = &usage.currency
            && (currency.len() != 3 || !currency.bytes().all(|b| b.is_ascii_uppercase()))
        {
            return Err(ModelContractError::Invalid(
                "usage currency must be three uppercase ASCII letters".to_owned(),
            ));
        }
        Ok(Self {
            text,
            structured,
            tool_calls,
            finish_reason,
            usage,
            provider_metadata,
        })
    }
    /// Final text.
    #[must_use]
    pub fn text(&self) -> &str {
        &self.text
    }
    /// Parsed structured result.
    #[must_use]
    pub const fn structured(&self) -> Option<&BoundedJson> {
        self.structured.as_ref()
    }
    /// Returned tool calls; never auto-executed.
    #[must_use]
    pub fn tool_calls(&self) -> &[ToolCall] {
        &self.tool_calls
    }
    /// Stop classification.
    #[must_use]
    pub const fn finish_reason(&self) -> FinishReason {
        self.finish_reason
    }
    /// Provider usage evidence.
    #[must_use]
    pub const fn usage(&self) -> &Usage {
        &self.usage
    }
    /// Bounded raw provider observations.
    #[must_use]
    pub const fn provider_metadata(&self) -> &BTreeMap<ExtensionKey, BoundedJson> {
        &self.provider_metadata
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ModelResponseWire {
    text: String,
    structured: Option<BoundedJson>,
    tool_calls: Vec<ToolCall>,
    finish_reason: FinishReason,
    usage: Usage,
    provider_metadata: BTreeMap<ExtensionKey, BoundedJson>,
}

milkdrift_contracts::deserialize_via!(ModelResponse, ModelResponseWire, |w| Self::new(
    w.text,
    w.structured,
    w.tool_calls,
    w.finish_reason,
    w.usage,
    w.provider_metadata,
));

/// Progress toward a model result, or the complete result when one is available.
///
/// Text fragments alone do not establish completion; callers need terminal evidence.
/// Producers and enclosing documents impose fragment/progress bounds: these enum fields
/// are directly constructible and their strings have no independent constructor check.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case", tag = "type", deny_unknown_fields)]
pub enum ModelStreamEvent {
    /// Verified UTF-8 text fragment.
    Text {
        /// Verified UTF-8 fragment.
        fragment: String,
    },
    /// Provider progress without prompt/response content.
    Progress {
        /// Bounded progress summary.
        message: String,
        /// Completed provider units.
        completed_units: Option<u64>,
        /// Total provider units if known.
        total_units: Option<u64>,
    },
    /// Final canonical response.
    Complete {
        /// Complete canonical response.
        response: ModelResponse,
    },
}

fn safe_name(value: &str, maximum: usize) -> bool {
    !value.is_empty()
        && value.len() <= maximum
        && value.is_ascii()
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.' | b':' | b'/'))
}
