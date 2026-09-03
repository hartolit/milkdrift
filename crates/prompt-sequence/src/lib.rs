//! Bounded operator prompt-sequence documents compiled into ordinary Milkdrift blueprints.
//!
//! This crate is a product-facing import/template layer. It owns no scheduler,
//! process launcher, repository implementation, persistence, authority, or UI.
//! Imported prompts remain untrusted data and executable authority remains in
//! preconfigured capability and process profiles.

mod compiler;
mod document;
mod markdown;
mod remediation;

pub use compiler::{CompiledPromptSequence, StageBlueprintSummary, compile, stage_node_ids};
pub use document::{
    ApprovalPolicy, CapabilityProfileRef, DeclaredOutput, DirtyTreePolicy, FailurePolicy,
    PROMPT_SEQUENCE_SCHEMA_VERSION_V2, PromptSequence, PromptSequenceBudget,
    PromptSequenceDocument, PromptSequenceError, PromptSource, RepositoryArtifactPolicy,
    RepositoryCleanupPolicy, RepositoryIsolation, RepositoryOperation, RepositoryWorkspaceProfile,
    SessionPolicy, StageDefinition, VerificationContract,
};
pub use remediation::{RemediationProposalSpec, build_remediation_proposal};

/// Maximum accepted encoded import document size.
pub const MAX_PROMPT_SEQUENCE_DOCUMENT_BYTES: usize = 2_097_152;
/// Maximum ordered implementation stages in one import.
pub const MAX_PROMPT_SEQUENCE_STAGES: usize = 128;
/// Maximum bytes in one inline Markdown prompt.
pub const MAX_INLINE_PROMPT_BYTES: usize = 65_536;
/// Maximum aggregate bytes across inline prompts.
pub const MAX_TOTAL_INLINE_PROMPT_BYTES: usize = 1_048_576;
