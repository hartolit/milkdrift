//! Inspect work and change its future through one authorized control service.
//!
//! Submit a [`ControlCommandDocument`] to [`ControlService::execute`]. Simple operations
//! become runtime commands. A [`WorkflowProposalDocument`] first passes validation,
//! authority, and risk classification before creating an immutable prospective revision;
//! runtime reconciliation governs its application to a live run.
//!
//! [`WorkflowControlAdapter`] exposes the same service as a hosted capability.
//! [`ControllerLifecycleOwner`] assesses ordinary bounded controller repeats using durable
//! accounts and history. It is available for library integration; the production daemon
//! leaves it uninstalled pending external-evidence qualification.

mod acceptance;
mod adapter;
mod command;
mod controller;
mod document;
mod error;
mod identity;
mod policy;
mod preset;
mod read;
mod service;

pub use acceptance::{
    ACCEPTED_RESULT_OUTPUT, AcceptanceModelDiagnostics, AcceptanceReason, CodingResult,
    MAX_ACCEPTANCE_INPUT_BYTES, RESULT_ACCEPTANCE_INPUT, RESULT_ACCEPTANCE_OUTPUT,
    RESULT_ACCEPTANCE_SCHEMA_VERSION, ResultAcceptance, ResultAcceptanceContract,
    ResultRequirement, VerifiedCheckpoint, result_acceptance_gate, result_acceptance_task,
};
pub use adapter::{
    ControlArtifactAccess, MAX_CONTROL_RESULT_BYTES, WorkflowControlAdapter,
    workflow_control_descriptor,
};
pub use command::{
    ActorAuthorityContext, CONTROL_COMMAND_SCHEMA_VERSION_V1, ControlCommand,
    ControlCommandDocument, ControlResult, OptimisticGuard,
};
pub use controller::{
    CONTROLLER_POLICY_SCHEMA_VERSION_V1, ControllerBlueprintSpec, ControllerBound,
    ControllerLifecycleOwner, ControllerLimits, ControllerOperationRequirements, ControllerPolicy,
    ControllerPolicyDocument, ControllerProgress, ControllerStop, ControllerStopBehavior,
    ControllerWrapperBinding, UnknownUsagePolicy, build_controller_blueprint,
};
pub use document::{
    ClaimedStopCondition, MAX_PROPOSAL_DOCUMENT_BYTES, PROPOSAL_SCHEMA_VERSION_V1,
    ProposalApplicationPolicy, ProposalProvenance, RequestedRunAction, WorkflowProposal,
    WorkflowProposalDocument, workflow_proposal_structured_output,
};
pub use error::ControlError;
pub use identity::{ControlId, ControllerId, ControllerPolicyDigest, ProposalDigest, ProposalId};
pub use policy::{
    CONTROL_RISK_POLICY_ID, CONTROL_RISK_POLICY_VERSION_V1, PolicyClassification, RiskClass,
    RiskConstraint, classify_proposal,
};
pub use preset::{AuthorityPreset, GrantTemplate};
pub use read::{
    AttemptInspection, ControllerLifecycleState, ControllerStatusRead, NodeExecutionRead,
    ProposalStatusRead, ProposalSubmission, ReconciliationStatusRead, RevisionInspection,
    RunInspection, TimelinePage,
};
pub use service::ControlService;

/// Namespaced capability operation for bounded inspection.
pub const WORKFLOW_INSPECT_OPERATION: &str = "workflow.inspect";
/// Evaluates purpose-specific immutable output evidence without invoking an external validator.
pub const WORKFLOW_ACCEPT_RESULT_OPERATION: &str = "workflow.accept_result";
/// Namespaced capability operation for untrusted proposal submission.
pub const WORKFLOW_PROPOSE_OPERATION: &str = "workflow.propose_revision";
/// Namespaced capability operation for pausing a run.
pub const WORKFLOW_PAUSE_OPERATION: &str = "workflow.pause";
/// Namespaced capability operation for resuming a run.
pub const WORKFLOW_RESUME_OPERATION: &str = "workflow.resume";
/// Namespaced capability operation for applying an exact proposal.
pub const WORKFLOW_APPLY_OPERATION: &str = "workflow.apply_proposal";
/// Namespaced capability operation for retrying retained external work.
pub const WORKFLOW_RETRY_OPERATION: &str = "workflow.retry";
/// Namespaced capability operation for delivering a signal.
pub const WORKFLOW_SIGNAL_OPERATION: &str = "workflow.signal";
/// Namespaced capability operation for controller lifecycle inspection.
pub const CONTROLLER_INSPECT_OPERATION: &str = "controller.inspect";
/// Namespaced capability operation for authorized checkpoint continuation.
pub const CONTROLLER_CONTINUE_OPERATION: &str = "controller.continue";
