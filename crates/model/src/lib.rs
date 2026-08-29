//! Pure provider-neutral contracts for causal model invocations.
//!
//! This package contains no HTTP client, provider SDK, credential value, async runtime,
//! durable store, or hidden session. Provider wire mappings live in adapter packages.

mod context;
mod document;
mod task;

pub use context::{
    AuthorityFact, CONTEXT_MANIFEST_SCHEMA_VERSION_V1, ContextEvidenceReference,
    ContextInclusionReason, ContextManifest, ContextManifestDigest, ContextManifestEntry,
    ContextManifestReference, ContextOmission, ContextOmissionReason, ContextSemanticKind,
    ContextSource, ContextTotals,
};
pub use document::{
    ContextManifestDocument, MODEL_CONTRACT_SCHEMA_VERSION_V1, ModelContractError,
    ModelResponseDocument, ModelTaskRequestDocument,
};
pub use task::{
    ContentPart, FinishReason, MAX_MODEL_OUTPUT_UNITS, Message, MessageRole, ModelResponse,
    ModelStreamEvent, ModelTaskRequest, ReasoningControl, ReasoningEffort, SessionSelection,
    StructuredOutput, ToolCall, ToolDefinition, Usage,
};

/// Reserved invocation input containing the versioned model task request.
pub const MODEL_TASK_INPUT_NAME: &str = "milkdrift.model_task";
/// Reserved invocation input containing the exact persisted context manifest artifact.
pub const CONTEXT_MANIFEST_INPUT_NAME: &str = "milkdrift.context_manifest";
/// Capability operation implemented by model endpoint adapters.
pub const MODEL_GENERATE_OPERATION: &str = "model.generate";
