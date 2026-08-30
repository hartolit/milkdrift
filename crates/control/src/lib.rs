//! Audited, authority-scoped workflow control shared by human, service, and AI callers.
//!
//! This application layer treats every proposal as untrusted data, creates prospective
//! immutable blueprint revisions, and routes live changes through the durable runtime
//! command and reconciliation path. It owns no database, network server, provider client,
//! secret resolver, UI, or alternate event writer.

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

pub use adapter::{ControlResultSink, WorkflowControlAdapter, workflow_control_descriptor};
pub use command::{
    ActorAuthorityContext, CONTROL_COMMAND_SCHEMA_VERSION_V1, ControlCommand,
    ControlCommandDocument, ControlResult, OptimisticGuard,
};
pub use controller::{
    ControllerBlueprintSpec, ControllerBound, ControllerLimits, ControllerProgress, ControllerStop,
    build_controller_blueprint,
};
pub use document::{
    ClaimedStopCondition, MAX_PROPOSAL_DOCUMENT_BYTES, PROPOSAL_SCHEMA_VERSION_V1,
    ProposalApplicationPolicy, ProposalArtifactReference, ProposalProvenance, RequestedRunAction,
    WorkflowProposal, WorkflowProposalDocument, workflow_proposal_structured_output,
};
pub use error::ControlError;
pub use identity::{ControlId, ProposalDigest, ProposalId};
pub use policy::{
    CONTROL_RISK_POLICY_ID, CONTROL_RISK_POLICY_VERSION_V1, PolicyClassification, RiskClass,
    RiskConstraint, classify_proposal,
};
pub use preset::{AuthorityPreset, GrantTemplate};
pub use read::{
    AttemptInspection, NodeExecutionRead, ProposalStatusRead, ProposalSubmission,
    ReconciliationStatusRead, RevisionInspection, RunInspection, TimelinePage,
};
pub use service::ControlService;

/// Namespaced capability operation for bounded inspection.
pub const WORKFLOW_INSPECT_OPERATION: &str = "workflow.inspect";
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
