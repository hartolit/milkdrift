//! Durable installation inventory and transactional resource-use ownership.
//!
//! Local attempts remain journal-owned; incoming operations remain serving-owned. Their acceptance
//! transactions acquire these holds. Resource records retain physical identities and stop evidence
//! even when execution detail is archived. A lease or terminal workflow state is never stop proof.

mod state;
mod uses;
pub use state::{
    ApprovedSetup, InstallationRecord, ManagedChange, ManagedChangePhase, ManagedObservation,
    ManagedStep, ManagedTransition, ProtectedDeployment, ResourceReceipt,
};
pub use uses::{ManagedExecution, ManagedUse, ManagedUsePhase, QuiescenceEvidence, managed_use_id};

use crate::{PageSize, PersistenceError};
use milkdrift_authority::AuthorityDecisionSnapshot;
use milkdrift_capability::managed::{ManagedName, ManagedRequest};

/// Narrow same-store resource port. Implementations commit all guards and mutations together.
pub trait ManagedResourceStore: Send + Sync {
    /// Retain an incomplete evaluation and exact actor receipt before invoking its verifier.
    fn begin_managed_evaluation(
        &self,
        request: &ManagedRequest,
        authorization: &AuthorityDecisionSnapshot,
        evidence: &milkdrift_workspace::CandidateEvaluation,
    ) -> Result<ResourceReceipt, PersistenceError>;
    /// Append the sole terminal evaluation observation; incomplete replay never reruns a verifier.
    fn finish_managed_evaluation(
        &self,
        evidence: &milkdrift_workspace::CandidateEvaluation,
    ) -> Result<(), PersistenceError>;
    /// Read a host-produced evaluation. No public upload operation can write this journal.
    fn managed_evaluation(
        &self,
        identity: &str,
    ) -> Result<Option<milkdrift_workspace::CandidateEvaluation>, PersistenceError>;
    /// Read one bounded inventory, including unresolved holds and pending transitions.
    fn managed_installation(
        &self,
        name: &ManagedName,
    ) -> Result<Option<InstallationRecord>, PersistenceError>;
    /// Page installations in stable name order. Live and removed identities share one namespace.
    fn managed_installations(
        &self,
        after: Option<&ManagedName>,
        limit: PageSize,
    ) -> Result<Vec<InstallationRecord>, PersistenceError>;
    /// Read exact actor-scoped receipt before planning any effect.
    fn managed_receipt(
        &self,
        actor: &str,
        command: &ManagedName,
    ) -> Result<Option<ResourceReceipt>, PersistenceError>;
    /// Accept intent, close affected admission and compare active holds in one transaction.
    fn begin_managed_change(
        &self,
        request: &ManagedRequest,
        authorization: &AuthorityDecisionSnapshot,
        change: &ManagedChange,
    ) -> Result<ResourceReceipt, PersistenceError>;
    /// Guarded completion of one platform boundary, preserving intended identity on failure.
    fn advance_managed_change(
        &self,
        installation: &ManagedName,
        transition: &str,
        expected_step: u32,
        observation: ManagedObservation,
    ) -> Result<InstallationRecord, PersistenceError>;
    /// Explicitly record failure/uncertainty without publishing candidate generations.
    fn fail_managed_change(
        &self,
        installation: &ManagedName,
        transition: &str,
        expected_step: u32,
        diagnostic: &str,
    ) -> Result<(), PersistenceError>;
    /// Mark one accepted use physically entered before the platform call and bind its exact task identity.
    fn enter_managed_use(
        &self,
        use_id: &str,
        expected_claim: u64,
        physical_identity: &str,
    ) -> Result<ManagedUse, PersistenceError>;
    /// Read by stable invocation identity; no execution outcome is manufactured by this lookup.
    fn managed_use(&self, use_id: &str) -> Result<Option<ManagedUse>, PersistenceError>;
    /// Commit authorized fencing before cancelling creators or inspecting physical absence.
    fn begin_managed_resolution(
        &self,
        request: &ManagedRequest,
        authorization: &AuthorityDecisionSnapshot,
    ) -> Result<ResourceReceipt, PersistenceError>;
    /// Store adapter-verified physical stop evidence. This does not settle an invocation result.
    fn quiesce_managed_use(
        &self,
        use_id: &str,
        expected_claim: u64,
        evidence: &QuiescenceEvidence,
    ) -> Result<(), PersistenceError>;
    /// Release only proven stopped use (or accepted work whose authoritative owner proved no entry).
    fn release_managed_use(
        &self,
        use_id: &str,
        expected_claim: u64,
    ) -> Result<(), PersistenceError>;
    /// Transfer editing atomically after physical quiescence and exact accepted lineage checks.
    fn transfer_managed_editing(
        &self,
        request: &ManagedRequest,
        authorization: &AuthorityDecisionSnapshot,
    ) -> Result<ResourceReceipt, PersistenceError>;
    /// Validate inventory, receipts, bindings and execution links with admission still closed.
    fn verify_managed_integrity(&self) -> Result<(), PersistenceError>;
}
