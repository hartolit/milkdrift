//! Durable controller declaration, resource vectors and exact state model.
mod identity;
mod transaction;
mod transition;
mod validation;

use identity::framed_digest;
pub use identity::{ControllerAccountId, ControllerReservationId, ControllerTransitionId};
pub use transaction::{ControllerAccountAction, ControllerAccountTransaction};

use std::collections::BTreeMap;

use milkdrift_capability::CapabilityCategory;
use milkdrift_workspace::RunId;
use serde::{Deserialize, Serialize};

use crate::{
    AttemptId, CurrencyCode, IntegrityDigest, NodeExecutionId, PersistenceError,
    document::canonical_json_bytes,
};

const MAX_CONTROLLER_ACCOUNT_ACTIONS: usize = 256;

/// Immutable resource ceilings owned by one controller account.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ControllerResourceBudget {
    cost_micros: u64,
    currency: CurrencyCode,
    input_units: u64,
    output_units: u64,
    artifact_bytes: u64,
    process_admissions: u64,
    model_admissions: u64,
}

impl ControllerResourceBudget {
    /// Constructs nonzero immutable ceilings for every ledger-owned dimension.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        cost_micros: u64,
        currency: CurrencyCode,
        input_units: u64,
        output_units: u64,
        artifact_bytes: u64,
        process_admissions: u64,
        model_admissions: u64,
    ) -> Result<Self, PersistenceError> {
        if [
            cost_micros,
            input_units,
            output_units,
            artifact_bytes,
            process_admissions,
            model_admissions,
        ]
        .contains(&0)
        {
            return Err(PersistenceError::InvalidDocument(
                "controller resource ceilings must be nonzero".to_owned(),
            ));
        }
        Ok(Self {
            cost_micros,
            currency,
            input_units,
            output_units,
            artifact_bytes,
            process_admissions,
            model_admissions,
        })
    }

    /// Maximum monetary use in millionths of the declared currency.
    #[must_use]
    pub const fn cost_micros(&self) -> u64 {
        self.cost_micros
    }
    /// Exact currency shared by every monetary reservation.
    #[must_use]
    pub const fn currency(&self) -> &CurrencyCode {
        &self.currency
    }
    /// Maximum provider-defined input units.
    #[must_use]
    pub const fn input_units(&self) -> u64 {
        self.input_units
    }
    /// Maximum provider-defined output units.
    #[must_use]
    pub const fn output_units(&self) -> u64 {
        self.output_units
    }
    /// Maximum logical artifact bytes.
    #[must_use]
    pub const fn artifact_bytes(&self) -> u64 {
        self.artifact_bytes
    }
    /// Maximum admitted process-category entries.
    #[must_use]
    pub const fn process_admissions(&self) -> u64 {
        self.process_admissions
    }
    /// Maximum admitted model-category entries.
    #[must_use]
    pub const fn model_admissions(&self) -> u64 {
        self.model_admissions
    }
}

/// Stable immutable declaration for one logical controller occurrence.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ControllerAccountDeclaration {
    account: ControllerAccountId,
    controller_run: RunId,
    controller_execution: NodeExecutionId,
    policy_digest: String,
    budget: ControllerResourceBudget,
    declaration_digest: IntegrityDigest,
}

#[derive(Serialize)]
struct DeclarationDigestInput<'a> {
    domain: &'static str,
    account: &'a ControllerAccountId,
    controller_run: &'a RunId,
    controller_execution: &'a NodeExecutionId,
    policy_digest: &'a str,
    budget: &'a ControllerResourceBudget,
}

impl ControllerAccountDeclaration {
    /// Derives a stable account identity and declaration digest from immutable policy facts.
    pub fn new(
        controller_run: RunId,
        controller_execution: NodeExecutionId,
        policy_digest: impl Into<String>,
        budget: ControllerResourceBudget,
    ) -> Result<Self, PersistenceError> {
        let policy_digest = policy_digest.into();
        if policy_digest.len() < 4 || policy_digest.len() > 192 || !policy_digest.is_ascii() {
            return Err(PersistenceError::InvalidDocument(
                "controller policy digest is malformed".to_owned(),
            ));
        }
        let identity = framed_digest(
            b"milkdrift.controller-account.identity.v1\0",
            &[
                controller_run.as_str(),
                controller_execution.as_str(),
                &policy_digest,
            ],
        );
        let account = ControllerAccountId::new(format!("controller-account:{identity}"))?;
        let declaration_digest = declaration_digest(
            &account,
            &controller_run,
            &controller_execution,
            &policy_digest,
            &budget,
        )?;
        Ok(Self {
            account,
            controller_run,
            controller_execution,
            policy_digest,
            budget,
            declaration_digest,
        })
    }

    /// Revalidates an untrusted stored declaration.
    pub fn validate(&self) -> Result<(), PersistenceError> {
        let rebuilt = Self::new(
            self.controller_run.clone(),
            self.controller_execution.clone(),
            self.policy_digest.clone(),
            self.budget.clone(),
        )?;
        if &rebuilt != self {
            return Err(PersistenceError::InvalidDocument(
                "controller account declaration is not canonical".to_owned(),
            ));
        }
        Ok(())
    }

    /// Stable account identity derived from the declaration facts.
    #[must_use]
    pub const fn account(&self) -> &ControllerAccountId {
        &self.account
    }
    /// Run containing the controller occurrence.
    #[must_use]
    pub const fn controller_run(&self) -> &RunId {
        &self.controller_run
    }
    /// Exact controller node execution that owns the occurrence.
    #[must_use]
    pub const fn controller_execution(&self) -> &NodeExecutionId {
        &self.controller_execution
    }
    /// Immutable validated controller-policy digest.
    #[must_use]
    pub fn policy_digest(&self) -> &str {
        &self.policy_digest
    }
    /// Immutable resource ceilings for the occurrence.
    #[must_use]
    pub const fn budget(&self) -> &ControllerResourceBudget {
        &self.budget
    }
    /// Canonical digest of all declaration facts.
    #[must_use]
    pub const fn declaration_digest(&self) -> &IntegrityDigest {
        &self.declaration_digest
    }
}

fn declaration_digest(
    account: &ControllerAccountId,
    controller_run: &RunId,
    controller_execution: &NodeExecutionId,
    policy_digest: &str,
    budget: &ControllerResourceBudget,
) -> Result<IntegrityDigest, PersistenceError> {
    Ok(IntegrityDigest::hash(&canonical_json_bytes(
        &DeclarationDigestInput {
            domain: "milkdrift.controller-account.declaration.v1",
            account,
            controller_run,
            controller_execution,
            policy_digest,
            budget,
        },
        65_536,
    )?))
}

/// Settled or outstanding resource vector.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ControllerResourceTotals {
    cost_micros: u64,
    input_units: u64,
    output_units: u64,
    artifact_bytes: u64,
    process_admissions: u64,
    model_admissions: u64,
}

impl ControllerResourceTotals {
    /// Monetary component in millionths of the declared currency.
    #[must_use]
    pub const fn cost_micros(self) -> u64 {
        self.cost_micros
    }
    /// Provider-defined input-unit component.
    #[must_use]
    pub const fn input_units(self) -> u64 {
        self.input_units
    }
    /// Provider-defined output-unit component.
    #[must_use]
    pub const fn output_units(self) -> u64 {
        self.output_units
    }
    /// Logical artifact-byte component.
    #[must_use]
    pub const fn artifact_bytes(self) -> u64 {
        self.artifact_bytes
    }
    /// Conservatively settled process-entry component.
    #[must_use]
    pub const fn process_admissions(self) -> u64 {
        self.process_admissions
    }
    /// Conservatively settled model-entry component.
    #[must_use]
    pub const fn model_admissions(self) -> u64 {
        self.model_admissions
    }

    /// Conservative committed use: settled facts plus unresolved remainders.
    fn checked_add(self, other: Self) -> Result<Self, PersistenceError> {
        Ok(Self {
            cost_micros: checked_add(self.cost_micros, other.cost_micros)?,
            input_units: checked_add(self.input_units, other.input_units)?,
            output_units: checked_add(self.output_units, other.output_units)?,
            artifact_bytes: checked_add(self.artifact_bytes, other.artifact_bytes)?,
            process_admissions: checked_add(self.process_admissions, other.process_admissions)?,
            model_admissions: checked_add(self.model_admissions, other.model_admissions)?,
        })
    }
}

/// Fail-closed condition on a controller account.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case", tag = "type", deny_unknown_fields)]
pub enum ControllerAccountBlock {
    /// A bounded terminal dimension lacked authoritative usage.
    UnknownUsage {
        /// Resource dimension whose observation was absent.
        dimension: String,
        /// Reservation retaining the unresolved obligation.
        reservation: ControllerReservationId,
    },
    /// An adapter observation exceeded its enforceable envelope.
    ContractViolation {
        /// Resource dimension whose contract was violated.
        dimension: String,
        /// Reservation whose envelope was exceeded.
        reservation: ControllerReservationId,
        /// Authoritative observed use.
        observed: u64,
        /// Enforceable maximum admitted before entry.
        reserved: u64,
    },
    /// Stored/controller history is not sufficient for safe admission.
    Integrity {
        /// Stable fail-closed explanation.
        reason: String,
    },
}

/// Durable result of attempting to charge one logical artifact publication.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ControllerArtifactChargeOutcome {
    /// The exact logical bytes were charged once.
    Charged,
    /// The publication exceeded its final-entry reservation and the account was blocked.
    ContractViolation,
}

/// One exact outstanding final-entry obligation.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ControllerReservation {
    reservation: ControllerReservationId,
    attempt: AttemptId,
    category: CapabilityCategory,
    input: ControllerReservationDimension,
    output: ControllerReservationDimension,
    artifact: ControllerReservationDimension,
    cost: ControllerReservationDimension,
}

/// Exact lifecycle of one dimension inside an accepted reservation.
///
/// `NotApplicable` must remain distinguishable from an already-settled bound: the former is an
/// enforceable assertion that positive use is impossible, while the latter deliberately ignores
/// repeated terminal evidence for an obligation that was already settled.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(
    rename_all = "snake_case",
    tag = "state",
    content = "remaining",
    deny_unknown_fields
)]
enum ControllerReservationDimension {
    NotApplicable,
    Outstanding(u64),
    Settled,
}

impl ControllerReservationDimension {
    const fn from_admitted_value(value: Option<u64>) -> Self {
        match value {
            Some(value) => Self::Outstanding(value),
            None => Self::NotApplicable,
        }
    }

    const fn remaining(self) -> Option<u64> {
        match self {
            Self::Outstanding(remaining) => Some(remaining),
            Self::NotApplicable | Self::Settled => None,
        }
    }
}

impl ControllerReservation {
    /// Stable reservation identity.
    #[must_use]
    pub const fn reservation(&self) -> &ControllerReservationId {
        &self.reservation
    }
    /// Exact runtime attempt that owns the obligation.
    #[must_use]
    pub const fn attempt(&self) -> &AttemptId {
        &self.attempt
    }
    /// Frozen capability category charged at admission.
    #[must_use]
    pub const fn category(&self) -> &CapabilityCategory {
        &self.category
    }
    /// Artifact allowance not yet published or released.
    #[must_use]
    pub const fn artifact_remaining(&self) -> Option<u64> {
        self.artifact.remaining()
    }
}

/// Exact current durable account state.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ControllerAccountState {
    declaration: ControllerAccountDeclaration,
    revision: u64,
    revision_digest: IntegrityDigest,
    settled: ControllerResourceTotals,
    outstanding: ControllerResourceTotals,
    reservations: BTreeMap<ControllerReservationId, ControllerReservation>,
    blocked: Option<ControllerAccountBlock>,
}

#[derive(Serialize)]
struct StateDigestInput<'a> {
    domain: &'static str,
    declaration: &'a ControllerAccountDeclaration,
    revision: u64,
    settled: ControllerResourceTotals,
    outstanding: ControllerResourceTotals,
    reservations: &'a BTreeMap<ControllerReservationId, ControllerReservation>,
    blocked: &'a Option<ControllerAccountBlock>,
}

impl ControllerAccountState {
    /// Creates the exact genesis state for one declaration.
    pub fn establish(declaration: ControllerAccountDeclaration) -> Result<Self, PersistenceError> {
        declaration.validate()?;
        let mut state = Self {
            declaration,
            revision: 0,
            revision_digest: IntegrityDigest::hash(b"uninitialized"),
            settled: ControllerResourceTotals::default(),
            outstanding: ControllerResourceTotals::default(),
            reservations: BTreeMap::new(),
            blocked: None,
        };
        state.revision_digest = state.calculate_digest()?;
        state.validate()?;
        Ok(state)
    }

    /// Immutable declaration that owns this state.
    #[must_use]
    pub const fn declaration(&self) -> &ControllerAccountDeclaration {
        &self.declaration
    }
    /// Monotonic state revision.
    #[must_use]
    pub const fn revision(&self) -> u64 {
        self.revision
    }
    /// Digest used for optimistic comparison against this exact revision.
    #[must_use]
    pub const fn revision_digest(&self) -> &IntegrityDigest {
        &self.revision_digest
    }
    /// Authoritatively settled use.
    #[must_use]
    pub const fn settled(&self) -> ControllerResourceTotals {
        self.settled
    }
    /// Sum of every unresolved reservation remainder.
    #[must_use]
    pub const fn outstanding(&self) -> ControllerResourceTotals {
        self.outstanding
    }
    /// Permanent fail-closed condition, when present.
    #[must_use]
    pub const fn blocked(&self) -> Option<&ControllerAccountBlock> {
        self.blocked.as_ref()
    }
    /// Exact current reservations keyed by stable identity.
    #[must_use]
    pub const fn reservations(&self) -> &BTreeMap<ControllerReservationId, ControllerReservation> {
        &self.reservations
    }

    /// Conservative totals consumed by controller lifecycle assessment.
    pub fn committed_totals(&self) -> Result<ControllerResourceTotals, PersistenceError> {
        self.settled.checked_add(self.outstanding)
    }

    fn calculate_digest(&self) -> Result<IntegrityDigest, PersistenceError> {
        Ok(IntegrityDigest::hash(&canonical_json_bytes(
            &StateDigestInput {
                domain: "milkdrift.controller-account.state.v2",
                declaration: &self.declaration,
                revision: self.revision,
                settled: self.settled,
                outstanding: self.outstanding,
                reservations: &self.reservations,
                blocked: &self.blocked,
            },
            1_048_576,
        )?))
    }

    fn advance(&mut self) -> Result<(), PersistenceError> {
        self.revision = self.revision.checked_add(1).ok_or_else(|| {
            PersistenceError::InvalidDocument("controller account revision overflow".to_owned())
        })?;
        self.revision_digest = self.calculate_digest()?;
        self.validate()
    }
}

/// Stable reason a controlled final entry was refused.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case", tag = "type", deny_unknown_fields)]
pub enum ControllerAdmissionDenial {
    /// A required dimension had no enforceable pre-entry maximum.
    Unknown {
        /// Resource dimension lacking an enforceable bound.
        dimension: String,
    },
    /// Cost envelope and controller budget use different exact currencies.
    CurrencyMismatch,
    /// Candidate obligation would exceed its immutable ceiling.
    Limit {
        /// Resource dimension whose ceiling would be exceeded.
        dimension: String,
    },
    /// Candidate arithmetic could not be represented without wrapping.
    Overflow {
        /// Resource dimension whose candidate overflowed.
        dimension: String,
    },
    /// A prior unknown or contract violation permanently closed admission.
    Blocked,
}

/// Controller portion of the sole final adapter-entry fact.
#[derive(Clone, Debug, Default, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case", tag = "type", deny_unknown_fields)]
pub enum ControllerAdmissionOutcome {
    /// The run has no controller-account binding.
    #[default]
    NotControlled,
    /// Admission atomically committed an exact reservation.
    Reserved {
        /// Owning controller account.
        account: ControllerAccountId,
        /// Stable reservation identity.
        reservation: ControllerReservationId,
    },
    /// Admission was refused without adapter entry.
    Denied {
        /// Owning controller account.
        account: ControllerAccountId,
        /// Exact refusal reason.
        reason: ControllerAdmissionDenial,
    },
}

impl ControllerAdmissionOutcome {
    /// Returns the committed reservation for an accepted controlled entry.
    #[must_use]
    pub const fn reservation(&self) -> Option<&ControllerReservationId> {
        match self {
            Self::Reserved { reservation, .. } => Some(reservation),
            Self::NotControlled | Self::Denied { .. } => None,
        }
    }
}

/// Artifact-account owner selected explicitly by every publication producer.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(
    rename_all = "snake_case",
    tag = "type",
    content = "reservation",
    deny_unknown_fields
)]
pub enum ControllerArtifactOwner {
    /// Resolve the run's immutable binding; ordinary unbound runs remain unchanged.
    RunBinding,
    /// Consume the exact reservation committed at final adapter entry.
    InvocationReservation(ControllerReservationId),
}

/// Narrow durable read port; mutations occur only inside journal/artifact transactions.
pub trait ControllerAccountStore: Send + Sync {
    /// Resolves the immutable optional account binding for a run.
    fn controller_account_binding(
        &self,
        run: &RunId,
    ) -> Result<Option<ControllerAccountId>, PersistenceError>;
    /// Loads and validates one exact current account state.
    fn controller_account(
        &self,
        account: &ControllerAccountId,
    ) -> Result<Option<ControllerAccountState>, PersistenceError>;
}

fn checked_add(left: u64, right: u64) -> Result<u64, PersistenceError> {
    left.checked_add(right).ok_or_else(|| {
        PersistenceError::InvalidDocument("controller resource arithmetic overflow".to_owned())
    })
}

#[cfg(test)]
mod tests;
