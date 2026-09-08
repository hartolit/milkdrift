//! Decide whether an actor may perform an operation with the supplied resources.
//!
//! A trusted caller constructs an [`AuthorityRequest`] from its authenticated actor,
//! exact grant claim, resource facts, and boundary time. [`GrantSetEvaluator`] returns
//! an allowed or denied [`AuthorityDecisionSnapshot`] for the caller to enforce and
//! retain. The evaluator neither authenticates the actor nor performs the operation.
//!
//! Build grants with [`AuthorityGrantBuilder`]; use [`Selection`] for explicit wildcards
//! or exact allowlists. [`ExecutionAuthorityBasis`] carries an accepted run's grant
//! reference into later capability decisions, which still check current revocation.

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
