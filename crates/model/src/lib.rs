//! Describe an external model request and preserve its response and selected context.
//!
//! Authors construct [`ModelTaskRequest`] from ordered [`Message`]s and explicit
//! generation choices. [`ModelTaskRequestDocument`] provides the bounded JSON form
//! supplied under [`MODEL_TASK_INPUT_NAME`]. The model-provider adapter uses the resolved
//! endpoint profile to check supported features and map the request to that endpoint's
//! protocol. A locally valid request can still ask for an unsupported endpoint feature
//! or token allowance; [`MAX_MODEL_OUTPUT_UNITS`] explains that boundary.
//!
//! Separately, the runtime uses a blueprint task's context policy to construct a
//! [`ContextManifest`]. It records what evidence an attempt received, including omitted
//! sources and byte/provenance facts, and survives retries without selecting newer
//! history. This contract also serves process tasks. [`ModelResponse`] preserves final
//! text, structured output, tool-call data, finish reason, and reported usage; a returned
//! tool call is data, not an instruction this crate executes.
//!
//! This package contains no HTTP client, provider SDK, credential value, async runtime,
//! durable store, or hidden session. Provider wire mappings live in adapter packages.
//!
//! A request can be constructed and round-tripped without a provider or credentials:
//!
//! ```
//! use std::collections::BTreeMap;
//! use milkdrift_model::{
//!     ContentPart, Message, MessageRole, ModelContractError, ModelTaskRequest,
//!     ModelTaskRequestDocument, SessionSelection,
//! };
//!
//! let request = ModelTaskRequest::new(
//!     vec![Message::new(
//!         MessageRole::User,
//!         vec![ContentPart::Text { text: "Summarize the supplied evidence.".to_owned() }],
//!         None,
//!     )?],
//!     Vec::new(), None, SessionSelection::Fresh, None, 512, false, BTreeMap::new(),
//! )?;
//! let document = ModelTaskRequestDocument::new(request);
//! let bytes = document.to_canonical_json()?;
//! let decoded = ModelTaskRequestDocument::from_json(&bytes)?;
//! assert_eq!(decoded, document);
//! assert_eq!(decoded.body().maximum_output_units(), 512);
//! # Ok::<(), ModelContractError>(())
//! ```

mod context;
mod document;
mod task;

pub use context::{
    AuthorityFact, ContextEvidenceReference, ContextInclusionReason, ContextManifest,
    ContextManifestDigest, ContextManifestEntry, ContextManifestReference, ContextOmission,
    ContextOmissionReason, ContextProducerFact, ContextSemanticKind, ContextSource, ContextTotals,
};
pub use document::{
    ContextManifestDocument, ModelContractError, ModelResponseDocument, ModelTaskRequestDocument,
};
pub use task::{
    ContentPart, FinishReason, MAX_MODEL_OUTPUT_UNITS, Message, MessageRole, ModelResponse,
    ModelStreamEvent, ModelTaskRequest, ReasoningControl, ReasoningEffort, SessionSelection,
    StructuredOutput, ToolCall, ToolDefinition, Usage,
};

/// Reserved invocation input containing the versioned model task request.
pub const MODEL_TASK_INPUT_NAME: &str = "milkdrift.model_task";
/// Capability operation implemented by model endpoint adapters.
pub const MODEL_GENERATE_OPERATION: &str = "model.generate";
