//! Pure, deterministic authority contracts for every Milkdrift boundary.
//!
//! This crate authenticates nobody and resolves no secrets. It owns immutable grants,
//! caller-supplied boundary facts, deterministic decisions, and opaque references only.

mod document;
mod evaluator;
mod identity;
mod model;
mod secret;
mod selection;

pub use evaluator::{AuthorityEvaluator, GrantSetEvaluator};
pub use identity::{
    ActorRef, AuthorityError, DecisionId, GrantDigest, GrantId, NetworkProfileRef, PolicyId,
    SecretRef,
};
pub use model::{
    AccessMode, ArtifactAuthorityScope, AuthorityBudget, AuthorityDecisionSnapshot,
    AuthorityExecutionProvenance, AuthorityGrant, AuthorityGrantBuilder, AuthorityOperation,
    AuthorityRequest, BoundaryTimeMillis, CapabilityAuthorityScope,
    CapabilityAuthorityScopeBuilder, CapabilityExecutionRequirements, DaemonAuthorityScope,
    DecisionOutcome, DecisionReasonCode, ExecutionAuthorityBasis, FilesystemScope,
    LayoutAuthorityScope, LayoutOwner, NetworkScope, PeerAuthorityScope, RequestedResourceFacts,
    ResourceScope, WorkflowRunScope, WorkspaceAuthorityScope,
};
pub use secret::SensitiveSecret;
pub use selection::{MAX_SELECTION_ITEMS, Selection};
