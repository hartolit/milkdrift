use milkdrift_authority::{AuthorityError, DecisionReasonCode};
use milkdrift_blueprint::{IdentityError, ModelError, MutationError};
use milkdrift_capability::ContractError;
use milkdrift_model::ModelContractError;
use milkdrift_persistence::{PersistenceError, RunSequence};
use milkdrift_runtime::RuntimeError;
use thiserror::Error;

use crate::RiskClass;

/// Bounded failure from proposal parsing, policy evaluation, or authoritative execution.
#[derive(Debug, Error)]
pub enum ControlError {
    /// A control identity or digest is malformed.
    #[error("invalid {kind}: {reason}")]
    InvalidIdentity {
        /// Identity family.
        kind: &'static str,
        /// Stable diagnostic.
        reason: String,
    },
    /// A versioned control contract is internally inconsistent.
    #[error("invalid control contract: {0}")]
    InvalidContract(String),
    /// Hostile input exceeded a lexical or decoded bound.
    #[error("control bound exceeded at {location}: {reason}")]
    Bounds {
        /// JSON-like location.
        location: String,
        /// Violated limit.
        reason: String,
    },
    /// A future or otherwise unsupported schema was supplied.
    #[error("unsupported {document} schema version {found}; supported version is {supported}")]
    UnsupportedVersion {
        /// Document family.
        document: &'static str,
        /// Supplied version.
        found: u32,
        /// Implemented version.
        supported: u32,
    },
    /// Strict JSON parsing failed.
    #[error("invalid control JSON: {0}")]
    Json(#[from] serde_json::Error),
    /// The exact base revision does not exist.
    #[error("proposal base revision was not found")]
    BaseRevisionNotFound,
    /// The supplied base workflow or digest did not match durable revision content.
    #[error("proposal base revision facts do not match durable content")]
    BaseRevisionMismatch,
    /// The proposal's optimistic run boundary is stale.
    #[error("stale run sequence: expected {expected}, actual {actual}")]
    StaleRunSequence {
        /// Proposal boundary.
        expected: RunSequence,
        /// Current boundary.
        actual: RunSequence,
    },
    /// The immutable authority evaluator denied an exact request.
    #[error("authority denied the control request: {reasons:?}")]
    AuthorizationDenied {
        /// Stable evaluator reason codes.
        reasons: Vec<DecisionReasonCode>,
    },
    /// Policy requires a separate approval command before application.
    #[error("proposal risk {risk:?} requires an explicit approval")]
    ApprovalRequired {
        /// Deterministic classifier result.
        risk: RiskClass,
    },
    /// Policy forbids this proposal at the supplied boundary.
    #[error("proposal is forbidden by deterministic control policy")]
    ForbiddenProposal,
    /// A proposal does not have the exact durable plan/decision state required by the command.
    #[error("proposal state conflict: {0}")]
    ProposalState(String),
    /// Blueprint mutation or validation failed before storage.
    #[error(transparent)]
    Blueprint(#[from] MutationError),
    /// A blueprint identity supplied to a builder was malformed.
    #[error(transparent)]
    BlueprintIdentity(#[from] IdentityError),
    /// A local blueprint node/configuration invariant failed.
    #[error(transparent)]
    BlueprintModel(#[from] ModelError),
    /// Authority-contract construction or evaluation failed.
    #[error(transparent)]
    Authority(#[from] AuthorityError),
    /// Persistence port failed.
    #[error(transparent)]
    Persistence(#[from] PersistenceError),
    /// Existing durable runtime command processing failed.
    #[error(transparent)]
    Runtime(#[from] RuntimeError),
    /// Capability contract or adapter result construction failed.
    #[error(transparent)]
    Capability(#[from] ContractError),
    /// Model structured-output contract construction failed.
    #[error(transparent)]
    Model(#[from] ModelContractError),
}
