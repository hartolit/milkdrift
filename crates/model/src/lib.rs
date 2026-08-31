//! Pure provider-neutral contracts for causal model invocations.
//!
//! This package contains no HTTP client, provider SDK, credential value, async runtime,
//! durable store, or hidden session. Provider wire mappings live in adapter packages.

mod context;
mod document;
mod task;

pub use context::{
    AuthorityFact, CONTEXT_MANIFEST_SCHEMA_VERSION_V2, ContextEvidenceReference,
    ContextInclusionReason, ContextManifest, ContextManifestDigest, ContextManifestEntry,
    ContextManifestReference, ContextOmission, ContextOmissionReason, ContextProducerFact,
    ContextSemanticKind, ContextSource, ContextTotals,
};
pub use document::{
    ContextManifestDocument, MODEL_CONTRACT_SCHEMA_VERSION_V1, ModelContractError,
    ModelResponseDocument, ModelTaskRequestDocument,
};
pub use milkdrift_capability::{CONTEXT_ITEM_INPUT_PREFIX, CONTEXT_MANIFEST_INPUT_NAME};
pub use task::{
    ContentPart, FinishReason, MAX_MODEL_OUTPUT_UNITS, Message, MessageRole, ModelResponse,
    ModelStreamEvent, ModelTaskRequest, ReasoningControl, ReasoningEffort, SessionSelection,
    StructuredOutput, ToolCall, ToolDefinition, Usage,
};

/// Reserved invocation input containing the versioned model task request.
pub const MODEL_TASK_INPUT_NAME: &str = "milkdrift.model_task";
/// Capability operation implemented by model endpoint adapters.
pub const MODEL_GENERATE_OPERATION: &str = "model.generate";
