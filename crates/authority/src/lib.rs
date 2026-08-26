//! Pure, deterministic authority contracts for every Milkdrift boundary.
//!
//! This crate authenticates nobody and resolves no secrets. It owns immutable grants,
//! caller-supplied boundary facts, deterministic decisions, and opaque references only.

mod document;
mod evaluator;
mod identity;
mod model;
mod secret;

pub use document::{AUTHORITY_GRANT_SCHEMA_VERSION_V1, MAX_AUTHORITY_DOCUMENT_BYTES};
pub use evaluator::{AuthorityEvaluator, GrantSetEvaluator};
pub use identity::{
    ActorRef, AuthorityError, DecisionId, GrantId, NetworkProfileRef, PolicyId, SecretRef,
};
pub use model::{
    AccessMode, AuthorityBudget, AuthorityDecisionSnapshot, AuthorityGrant, AuthorityGrantBuilder,
    AuthorityOperation, AuthorityRequest, BoundaryTimeMillis, CapabilityAuthorityScope,
    DecisionOutcome, DecisionReasonCode, FilesystemScope, NetworkScope, RequestedResourceFacts,
    ResourceScope, WorkflowRunScope,
};
pub use secret::SensitiveSecret;
