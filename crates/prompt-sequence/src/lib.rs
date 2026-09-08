//! Turn ordered implementation prompts into an inspectable workflow definition.
//!
//! Read JSON or the Markdown envelope with [`PromptSequenceDocument::from_bytes`], then
//! [`compile`] it into ordinary coding, verification, branch, review-wait, and terminal
//! nodes. [`CompiledPromptSequence`] exposes the immutable revision and its stage mapping.
//! The daemon admits that revision through the ordinary control path before starting work.
//!
//! Schema v2 uses configured trusted process capabilities. Prompts and repository policies
//! become task inputs; configured processes carry them out under the run's authority.
//! Compilation performs no process launch, repository operation, or storage write.
//!
//! This example compiles the repository's maintained import without starting its named
//! capabilities. Use [`stage_node_ids`] to associate later executions with an imported
//! stage, and [`build_remediation_proposal`] to propose a prospective repair.
//!
//! ```
//! use milkdrift_blueprint::AuthorRef;
//! use milkdrift_prompt_sequence::{PromptSequenceDocument, compile, stage_node_ids};
//!
//! let bytes = include_bytes!(concat!(
//!     env!("CARGO_MANIFEST_DIR"), "/../../examples/headless-dogfood-sequence.md",
//! ));
//! let document = PromptSequenceDocument::from_bytes(bytes)?;
//! let compiled = compile(&document, AuthorRef::new("human:author")?)?;
//! let revision_bytes = compiled.to_canonical_json()?;
//! let stage = &compiled.stages()[0];
//! let nodes = stage_node_ids(&revision_bytes, &stage.stage_id)?;
//! assert!(nodes.contains(&stage.coding_node));
//! assert!(nodes.contains(&stage.verification_node));
//! assert_eq!(compiled.revision().sequence(), 1);
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```

mod compiler;
mod document;
mod markdown;
mod remediation;

pub use compiler::{CompiledPromptSequence, StageBlueprintSummary, compile, stage_node_ids};
pub use document::{
    ApprovalPolicy, CapabilityProfileRef, DeclaredOutput, DirtyTreePolicy, FailurePolicy,
    PromptSequence, PromptSequenceBudget, PromptSequenceDocument, PromptSequenceError,
    PromptSource, RepositoryArtifactPolicy, RepositoryCleanupPolicy, RepositoryIsolation,
    RepositoryOperation, RepositoryWorkspaceProfile, SessionPolicy, StageDefinition,
    VerificationContract,
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
