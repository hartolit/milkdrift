use std::collections::BTreeMap;

use milkdrift_capability::{AdmissionBound, CapabilityCategory, InvocationAdmissionEnvelope};
use milkdrift_workspace::RunId;
use serde::{Deserialize, Serialize};

use crate::{
    AttemptId, CurrencyCode, IntegrityDigest, NodeExecutionId, PersistenceError,
    document::canonical_json_bytes,
};

const MAX_CONTROLLER_ACCOUNT_ACTIONS: usize = 256;

macro_rules! account_identity {
    ($(#[$meta:meta])* $name:ident) => {
        milkdrift_contracts::validated_string_type! {
            $(#[$meta])*
            pub struct $name;
            error = PersistenceError;
            validate = validate_account_identity;
        }
    };
}

fn validate_account_identity(value: &str, kind: &'static str) -> Result<(), PersistenceError> {
    if value.is_empty()
        || value.len() > 192
        || !value.is_ascii()
        || !value.as_bytes()[0].is_ascii_alphanumeric()
        || !value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':' | b'/')
        })
    {
        return Err(PersistenceError::InvalidIdentity {
            kind,
            reason: "must contain 1..=192 safe ASCII identity bytes".to_owned(),
        });
    }
    Ok(())
}

account_identity!(/// One immutable continuous-controller resource account.
    ControllerAccountId);
account_identity!(/// One exact final-entry reservation.
    ControllerReservationId);
account_identity!(/// One idempotent durable account transition.
    ControllerTransitionId);

impl ControllerReservationId {
    /// Derives the one stable reservation identity for an account-bound attempt.
    pub fn for_attempt(
        account: &ControllerAccountId,
        attempt: &AttemptId,
    ) -> Result<Self, PersistenceError> {
        Self::new(format!(
            "controller-reservation:{}",
            framed_digest(
                b"milkdrift.controller-reservation.v1\0",
                &[account.as_str(), attempt.as_str()],
            )
        ))
    }
}

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
    pub fn checked_add(self, other: Self) -> Result<Self, PersistenceError> {
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

/// Permanent fail-closed condition on a controller account.
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

/// One exact outstanding final-entry obligation.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ControllerReservation {
    reservation: ControllerReservationId,
    attempt: AttemptId,
    category: CapabilityCategory,
    input_remaining: Option<u64>,
    output_remaining: Option<u64>,
    artifact_remaining: Option<u64>,
    cost_remaining: Option<u64>,
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
        self.artifact_remaining
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

    /// Validates all redundant totals and the exact revision digest.
    pub fn validate(&self) -> Result<(), PersistenceError> {
        self.declaration.validate()?;
        let mut summed = ControllerResourceTotals::default();
        for (identity, reservation) in &self.reservations {
            if identity != reservation.reservation() {
                return Err(PersistenceError::InvalidDocument(
                    "controller reservation map identity mismatch".to_owned(),
                ));
            }
            summed.input_units =
                checked_add(summed.input_units, reservation.input_remaining.unwrap_or(0))?;
            summed.output_units = checked_add(
                summed.output_units,
                reservation.output_remaining.unwrap_or(0),
            )?;
            summed.artifact_bytes = checked_add(
                summed.artifact_bytes,
                reservation.artifact_remaining.unwrap_or(0),
            )?;
            summed.cost_micros =
                checked_add(summed.cost_micros, reservation.cost_remaining.unwrap_or(0))?;
        }
        if summed.cost_micros != self.outstanding.cost_micros
            || summed.input_units != self.outstanding.input_units
            || summed.output_units != self.outstanding.output_units
            || summed.artifact_bytes != self.outstanding.artifact_bytes
            || self.outstanding.process_admissions != 0
            || self.outstanding.model_admissions != 0
        {
            return Err(PersistenceError::InvalidDocument(
                "controller outstanding totals disagree with exact reservations".to_owned(),
            ));
        }
        let committed = self.committed_totals()?;
        let budget = self.declaration.budget();
        if committed.cost_micros > budget.cost_micros
            || committed.input_units > budget.input_units
            || committed.output_units > budget.output_units
            || committed.artifact_bytes > budget.artifact_bytes
            || committed.process_admissions > budget.process_admissions
            || committed.model_admissions > budget.model_admissions
        {
            return Err(PersistenceError::InvalidDocument(
                "controller committed use exceeds its immutable budget".to_owned(),
            ));
        }
        if self.calculate_digest()? != self.revision_digest {
            return Err(PersistenceError::InvalidDocument(
                "controller account revision digest mismatch".to_owned(),
            ));
        }
        Ok(())
    }

    fn calculate_digest(&self) -> Result<IntegrityDigest, PersistenceError> {
        Ok(IntegrityDigest::hash(&canonical_json_bytes(
            &StateDigestInput {
                domain: "milkdrift.controller-account.state.v1",
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

    /// Applies one validated entry operation and returns the independently computed outcome.
    pub fn admit(
        &mut self,
        reservation: ControllerReservationId,
        attempt: AttemptId,
        category: CapabilityCategory,
        envelope: &InvocationAdmissionEnvelope,
    ) -> Result<ControllerAdmissionOutcome, PersistenceError> {
        if self.blocked.is_some() {
            return Ok(ControllerAdmissionOutcome::Denied {
                account: self.declaration.account.clone(),
                reason: ControllerAdmissionDenial::Blocked,
            });
        }
        if self.reservations.contains_key(&reservation) {
            return Err(PersistenceError::ImmutableConflict {
                entity: "controller reservation",
                identity: reservation.as_str().to_owned(),
            });
        }
        for (dimension, bound) in [
            ("input_units", envelope.input_units()),
            ("output_units", envelope.output_units()),
            ("artifact_bytes", envelope.artifact_bytes()),
        ] {
            if bound.is_unknown() {
                return Ok(ControllerAdmissionOutcome::Denied {
                    account: self.declaration.account.clone(),
                    reason: ControllerAdmissionDenial::Unknown {
                        dimension: dimension.to_owned(),
                    },
                });
            }
        }
        let input = admission_value(envelope.input_units());
        let output = admission_value(envelope.output_units());
        let artifact = admission_value(envelope.artifact_bytes());
        let cost = match envelope.monetary_cost() {
            AdmissionBound::Bounded(value) => {
                if value.currency() != self.declaration.budget.currency.as_str() {
                    return Ok(ControllerAdmissionOutcome::Denied {
                        account: self.declaration.account.clone(),
                        reason: ControllerAdmissionDenial::CurrencyMismatch,
                    });
                }
                Some(value.maximum_micros())
            }
            AdmissionBound::NotApplicable => None,
            AdmissionBound::Unknown => {
                return Ok(ControllerAdmissionOutcome::Denied {
                    account: self.declaration.account.clone(),
                    reason: ControllerAdmissionDenial::Unknown {
                        dimension: "monetary_cost".to_owned(),
                    },
                });
            }
        };
        for (dimension, value, settled, outstanding, limit) in [
            (
                "input_units",
                input,
                self.settled.input_units,
                self.outstanding.input_units,
                self.declaration.budget.input_units,
            ),
            (
                "output_units",
                output,
                self.settled.output_units,
                self.outstanding.output_units,
                self.declaration.budget.output_units,
            ),
            (
                "artifact_bytes",
                artifact,
                self.settled.artifact_bytes,
                self.outstanding.artifact_bytes,
                self.declaration.budget.artifact_bytes,
            ),
            (
                "monetary_cost",
                cost,
                self.settled.cost_micros,
                self.outstanding.cost_micros,
                self.declaration.budget.cost_micros,
            ),
        ] {
            let Some(value) = value else {
                continue;
            };
            let Some(candidate) = settled
                .checked_add(outstanding)
                .and_then(|committed| committed.checked_add(value))
            else {
                return Ok(ControllerAdmissionOutcome::Denied {
                    account: self.declaration.account.clone(),
                    reason: ControllerAdmissionDenial::Overflow {
                        dimension: dimension.to_owned(),
                    },
                });
            };
            let exceeds = candidate > limit;
            if exceeds {
                return Ok(ControllerAdmissionOutcome::Denied {
                    account: self.declaration.account.clone(),
                    reason: ControllerAdmissionDenial::Limit {
                        dimension: dimension.to_owned(),
                    },
                });
            }
        }
        let (process, model) = match category {
            CapabilityCategory::Process => (1, 0),
            CapabilityCategory::Model => (0, 1),
            CapabilityCategory::Tool
            | CapabilityCategory::Human
            | CapabilityCategory::Peer
            | CapabilityCategory::Custom(_) => (0, 0),
        };
        let Some(process_candidate) = self.settled.process_admissions.checked_add(process) else {
            return Ok(ControllerAdmissionOutcome::Denied {
                account: self.declaration.account.clone(),
                reason: ControllerAdmissionDenial::Overflow {
                    dimension: "process_admissions".to_owned(),
                },
            });
        };
        if process_candidate > self.declaration.budget.process_admissions {
            return Ok(ControllerAdmissionOutcome::Denied {
                account: self.declaration.account.clone(),
                reason: ControllerAdmissionDenial::Limit {
                    dimension: "process_admissions".to_owned(),
                },
            });
        }
        let Some(model_candidate) = self.settled.model_admissions.checked_add(model) else {
            return Ok(ControllerAdmissionOutcome::Denied {
                account: self.declaration.account.clone(),
                reason: ControllerAdmissionDenial::Overflow {
                    dimension: "model_admissions".to_owned(),
                },
            });
        };
        if model_candidate > self.declaration.budget.model_admissions {
            return Ok(ControllerAdmissionOutcome::Denied {
                account: self.declaration.account.clone(),
                reason: ControllerAdmissionDenial::Limit {
                    dimension: "model_admissions".to_owned(),
                },
            });
        }
        self.outstanding.input_units =
            checked_add(self.outstanding.input_units, input.unwrap_or(0))?;
        self.outstanding.output_units =
            checked_add(self.outstanding.output_units, output.unwrap_or(0))?;
        self.outstanding.artifact_bytes =
            checked_add(self.outstanding.artifact_bytes, artifact.unwrap_or(0))?;
        self.outstanding.cost_micros =
            checked_add(self.outstanding.cost_micros, cost.unwrap_or(0))?;
        self.settled.process_admissions = checked_add(self.settled.process_admissions, process)?;
        self.settled.model_admissions = checked_add(self.settled.model_admissions, model)?;
        self.reservations.insert(
            reservation.clone(),
            ControllerReservation {
                reservation: reservation.clone(),
                attempt,
                category,
                input_remaining: input,
                output_remaining: output,
                artifact_remaining: artifact,
                cost_remaining: cost,
            },
        );
        self.advance()?;
        Ok(ControllerAdmissionOutcome::Reserved {
            account: self.declaration.account.clone(),
            reservation,
        })
    }

    /// Settles exact terminal usage. `None` retains a bounded remainder and blocks admission.
    pub fn settle_terminal(
        &mut self,
        reservation: &ControllerReservationId,
        usage: Option<&crate::AttemptUsage>,
    ) -> Result<(), PersistenceError> {
        let mut record =
            self.reservations
                .remove(reservation)
                .ok_or_else(|| PersistenceError::NotFound {
                    entity: "controller reservation",
                    identity: reservation.as_str().to_owned(),
                })?;
        settle_dimension(
            "input_units",
            reservation,
            &mut record.input_remaining,
            usage.and_then(|value| value.input_units),
            &mut self.outstanding.input_units,
            &mut self.settled.input_units,
            &mut self.blocked,
        )?;
        settle_dimension(
            "output_units",
            reservation,
            &mut record.output_remaining,
            usage.and_then(|value| value.output_units),
            &mut self.outstanding.output_units,
            &mut self.settled.output_units,
            &mut self.blocked,
        )?;
        let observed_cost = usage
            .and_then(|value| value.cost.as_ref())
            .and_then(|cost| {
                (cost.currency == self.declaration.budget.currency).then_some(cost.micros)
            });
        if usage
            .and_then(|value| value.cost.as_ref())
            .is_some_and(|cost| cost.currency != self.declaration.budget.currency)
            && self.blocked.is_none()
        {
            self.blocked = Some(ControllerAccountBlock::Integrity {
                reason: "terminal cost currency differs from the admitted controller currency"
                    .to_owned(),
            });
        }
        settle_dimension(
            "monetary_cost",
            reservation,
            &mut record.cost_remaining,
            observed_cost,
            &mut self.outstanding.cost_micros,
            &mut self.settled.cost_micros,
            &mut self.blocked,
        )?;
        // Artifact bytes settle only at publication. A known terminal proves that no later
        // adapter publication for this synchronous invocation can begin.
        if let Some(remaining) = record.artifact_remaining.take() {
            self.outstanding.artifact_bytes = self
                .outstanding
                .artifact_bytes
                .checked_sub(remaining)
                .ok_or_else(|| {
                    PersistenceError::InvalidDocument(
                        "controller artifact remainder underflow".to_owned(),
                    )
                })?;
        }
        if record.input_remaining.is_some()
            || record.output_remaining.is_some()
            || record.cost_remaining.is_some()
        {
            self.reservations.insert(reservation.clone(), record);
        }
        self.advance()
    }

    /// Charges first logical artifact publication against a reservation or directly to the account.
    pub fn charge_artifact(
        &mut self,
        reservation: Option<&ControllerReservationId>,
        bytes: u64,
    ) -> Result<(), PersistenceError> {
        if reservation.is_none() && self.blocked.is_some() {
            return Err(PersistenceError::Bounds {
                location: "controller.artifact_budget",
                reason: "controller account is durably blocked".to_owned(),
            });
        }
        if let Some(reservation) = reservation {
            let record = self.reservations.get_mut(reservation).ok_or_else(|| {
                PersistenceError::NotFound {
                    entity: "controller reservation",
                    identity: reservation.as_str().to_owned(),
                }
            })?;
            let remaining = record.artifact_remaining.ok_or_else(|| {
                PersistenceError::InvalidDocument(
                    "controller invocation asserted artifacts not applicable".to_owned(),
                )
            })?;
            if bytes > remaining {
                return Err(PersistenceError::Bounds {
                    location: "controller.artifact_reservation",
                    reason: "logical artifact bytes exceed the exact reservation remainder"
                        .to_owned(),
                });
            }
            record.artifact_remaining = Some(remaining - bytes);
            self.outstanding.artifact_bytes = self
                .outstanding
                .artifact_bytes
                .checked_sub(bytes)
                .ok_or_else(|| {
                    PersistenceError::InvalidDocument(
                        "controller artifact outstanding underflow".to_owned(),
                    )
                })?;
        } else {
            let committed = self.committed_totals()?;
            if checked_add(committed.artifact_bytes, bytes)?
                > self.declaration.budget.artifact_bytes
            {
                return Err(PersistenceError::Bounds {
                    location: "controller.artifact_budget",
                    reason: "logical artifact bytes exceed the controller account remainder"
                        .to_owned(),
                });
            }
        }
        self.settled.artifact_bytes = checked_add(self.settled.artifact_bytes, bytes)?;
        self.advance()
    }
}

fn admission_value(bound: &AdmissionBound<u64>) -> Option<u64> {
    match bound {
        AdmissionBound::Bounded(value) => Some(*value),
        AdmissionBound::NotApplicable | AdmissionBound::Unknown => None,
    }
}

fn settle_dimension(
    dimension: &str,
    reservation: &ControllerReservationId,
    remaining: &mut Option<u64>,
    observed: Option<u64>,
    outstanding: &mut u64,
    settled: &mut u64,
    blocked: &mut Option<ControllerAccountBlock>,
) -> Result<(), PersistenceError> {
    let Some(reserved) = *remaining else {
        return Ok(());
    };
    let Some(observed) = observed else {
        if blocked.is_none() {
            *blocked = Some(ControllerAccountBlock::UnknownUsage {
                dimension: dimension.to_owned(),
                reservation: reservation.clone(),
            });
        }
        return Ok(());
    };
    *outstanding = outstanding.checked_sub(reserved).ok_or_else(|| {
        PersistenceError::InvalidDocument("controller outstanding settlement underflow".to_owned())
    })?;
    let charged = observed.min(reserved);
    *settled = checked_add(*settled, charged)?;
    *remaining = None;
    if observed > reserved && blocked.is_none() {
        *blocked = Some(ControllerAccountBlock::ContractViolation {
            dimension: dimension.to_owned(),
            reservation: reservation.clone(),
            observed,
            reserved,
        });
    }
    Ok(())
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

/// One closed account mutation included in an atomic runtime commit.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case", tag = "type", deny_unknown_fields)]
pub enum ControllerAccountAction {
    /// Creates one immutable account and binds its controller run.
    Establish {
        /// Canonical immutable declaration.
        declaration: ControllerAccountDeclaration,
        /// Controller run receiving the initial binding.
        bind_run: RunId,
    },
    /// Immutably inherits an existing account into a descendant run.
    BindRun {
        /// Existing originating account.
        account: ControllerAccountId,
        /// Descendant run to bind.
        run: RunId,
    },
    /// Applies the independently planned final-entry outcome.
    AdmitEntry {
        /// Bound account used for admission.
        account: ControllerAccountId,
        /// Stable reservation identity for the attempt.
        reservation: ControllerReservationId,
        /// Exact admitted attempt.
        attempt: AttemptId,
        /// Frozen capability category charged at admission.
        category: CapabilityCategory,
        /// Exact-generation request-specific envelope.
        envelope: InvocationAdmissionEnvelope,
        /// Outcome computed from the guarded prior state.
        expected_outcome: ControllerAdmissionOutcome,
    },
    /// Settles or conservatively retains an accepted reservation at terminal evidence.
    SettleTerminal {
        /// Account owning the reservation.
        account: ControllerAccountId,
        /// Reservation associated with the terminal attempt.
        reservation: ControllerReservationId,
        /// Authoritative bounded usage, or absence when usage is unknown.
        usage: Option<crate::AttemptUsage>,
    },
}

/// Validated idempotent account transition attached to one journal transaction.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ControllerAccountTransaction {
    transition: ControllerTransitionId,
    fingerprint: IntegrityDigest,
    expected_account_revision: Option<(ControllerAccountId, IntegrityDigest)>,
    actions: Vec<ControllerAccountAction>,
}

impl ControllerAccountTransaction {
    /// Constructs a bounded transition and its exact replay fingerprint.
    pub fn new(
        transition: ControllerTransitionId,
        expected_account_revision: Option<(ControllerAccountId, IntegrityDigest)>,
        actions: Vec<ControllerAccountAction>,
    ) -> Result<Self, PersistenceError> {
        if actions.is_empty() || actions.len() > MAX_CONTROLLER_ACCOUNT_ACTIONS {
            return Err(PersistenceError::Bounds {
                location: "controller.account_actions",
                reason: format!("must contain 1..={MAX_CONTROLLER_ACCOUNT_ACTIONS} actions"),
            });
        }
        let mut guarded_account = None;
        for account in actions.iter().filter_map(|action| match action {
            ControllerAccountAction::AdmitEntry { account, .. }
            | ControllerAccountAction::SettleTerminal { account, .. } => Some(account),
            ControllerAccountAction::Establish { .. } | ControllerAccountAction::BindRun { .. } => {
                None
            }
        }) {
            if guarded_account
                .as_ref()
                .is_some_and(|guarded| guarded != account)
            {
                return Err(PersistenceError::InvalidDocument(
                    "one controller transaction cannot guard multiple accounts".to_owned(),
                ));
            }
            guarded_account = Some(account.clone());
        }
        match (guarded_account.as_ref(), expected_account_revision.as_ref()) {
            (Some(account), Some((expected, _))) if account == expected => {}
            (Some(_), _) => {
                return Err(PersistenceError::InvalidDocument(
                    "controller admission and settlement require the exact account revision guard"
                        .to_owned(),
                ));
            }
            (None, Some(_)) => {
                return Err(PersistenceError::InvalidDocument(
                    "controller establishment and inheritance cannot carry an unrelated account revision guard"
                        .to_owned(),
                ));
            }
            (None, None) => {}
        }
        let fingerprint = IntegrityDigest::hash(&canonical_json_bytes(
            &(
                "milkdrift.controller-account.transition.v1",
                &expected_account_revision,
                &actions,
            ),
            1_048_576,
        )?);
        Ok(Self {
            transition,
            fingerprint,
            expected_account_revision,
            actions,
        })
    }
    /// Stable idempotency identity.
    #[must_use]
    pub const fn transition(&self) -> &ControllerTransitionId {
        &self.transition
    }
    /// Exact transition-content fingerprint.
    #[must_use]
    pub const fn fingerprint(&self) -> &IntegrityDigest {
        &self.fingerprint
    }
    /// Optional optimistic guard for a previously read account revision.
    #[must_use]
    pub const fn expected_account_revision(
        &self,
    ) -> Option<&(ControllerAccountId, IntegrityDigest)> {
        self.expected_account_revision.as_ref()
    }
    /// Closed ordered state operations in this atomic transition.
    #[must_use]
    pub fn actions(&self) -> &[ControllerAccountAction] {
        &self.actions
    }
}

/// Artifact-account owner selected explicitly by every publication producer.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case", tag = "type", deny_unknown_fields)]
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

fn framed_digest(domain: &[u8], values: &[&str]) -> String {
    let mut hasher = blake3::Hasher::new();
    hasher.update(domain);
    for value in values {
        hasher.update(&(value.len() as u64).to_be_bytes());
        hasher.update(value.as_bytes());
    }
    hasher.finalize().to_hex().to_string()
}

#[cfg(test)]
mod tests {
    use milkdrift_capability::{AdmissionMonetaryBound, CapabilityCategory};

    use super::*;
    use crate::{AttemptUsage, MonetaryUsage};

    type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

    fn account(process: u64, model: u64) -> TestResult<ControllerAccountState> {
        let budget =
            ControllerResourceBudget::new(8, CurrencyCode::new("USD")?, 8, 8, 8, process, model)?;
        Ok(ControllerAccountState::establish(
            ControllerAccountDeclaration::new(
                RunId::new("run-controller-account-test")?,
                NodeExecutionId::new("execution-controller-account-test")?,
                "policy:test-controller-account",
                budget,
            )?,
        )?)
    }

    fn reservation(
        state: &ControllerAccountState,
        suffix: &str,
    ) -> TestResult<(ControllerReservationId, AttemptId)> {
        let attempt = AttemptId::new(format!("attempt-{suffix}"))?;
        let reservation =
            ControllerReservationId::for_attempt(state.declaration().account(), &attempt)?;
        Ok((reservation, attempt))
    }

    fn bounded_envelope(maximum: u64) -> TestResult<InvocationAdmissionEnvelope> {
        Ok(InvocationAdmissionEnvelope::new(
            AdmissionBound::Bounded(maximum),
            AdmissionBound::Bounded(maximum),
            AdmissionBound::Bounded(maximum),
            AdmissionBound::Bounded(AdmissionMonetaryBound::new(maximum, "USD")?),
        ))
    }

    fn single_dimension_envelope(
        dimension: &str,
        maximum: u64,
    ) -> TestResult<InvocationAdmissionEnvelope> {
        let mut input = AdmissionBound::NotApplicable;
        let mut output = AdmissionBound::NotApplicable;
        let mut artifact = AdmissionBound::NotApplicable;
        let mut cost = AdmissionBound::NotApplicable;
        match dimension {
            "input_units" => input = AdmissionBound::Bounded(maximum),
            "output_units" => output = AdmissionBound::Bounded(maximum),
            "artifact_bytes" => artifact = AdmissionBound::Bounded(maximum),
            "monetary_cost" => {
                cost = AdmissionBound::Bounded(AdmissionMonetaryBound::new(maximum, "USD")?)
            }
            _ => return Err(format!("unsupported test dimension {dimension}").into()),
        }
        Ok(InvocationAdmissionEnvelope::new(
            input, output, artifact, cost,
        ))
    }

    #[test]
    fn exact_process_ceiling_accepts_n_and_denies_n_plus_one() -> TestResult {
        let mut state = account(2, 2)?;
        for suffix in ["one", "two"] {
            let (reservation, attempt) = reservation(&state, suffix)?;
            assert!(matches!(
                state.admit(
                    reservation,
                    attempt,
                    CapabilityCategory::Process,
                    &InvocationAdmissionEnvelope::not_applicable(),
                )?,
                ControllerAdmissionOutcome::Reserved { .. }
            ));
        }
        let revision = state.revision();
        let (reservation, attempt) = reservation(&state, "three")?;
        assert!(matches!(
            state.admit(
                reservation,
                attempt,
                CapabilityCategory::Process,
                &InvocationAdmissionEnvelope::not_applicable(),
            )?,
            ControllerAdmissionOutcome::Denied {
                reason: ControllerAdmissionDenial::Limit { dimension },
                ..
            } if dimension == "process_admissions"
        ));
        assert_eq!(state.revision(), revision);
        assert_eq!(state.committed_totals()?.process_admissions(), 2);
        Ok(())
    }

    #[test]
    fn every_resource_ceiling_accepts_exact_equality_and_denies_one_more() -> TestResult {
        for dimension in [
            "input_units",
            "output_units",
            "artifact_bytes",
            "monetary_cost",
        ] {
            let mut state = account(2, 2)?;
            let (exact, exact_attempt) = reservation(&state, &format!("{dimension}-exact"))?;
            assert!(matches!(
                state.admit(
                    exact,
                    exact_attempt,
                    CapabilityCategory::Tool,
                    &single_dimension_envelope(dimension, 8)?,
                )?,
                ControllerAdmissionOutcome::Reserved { .. }
            ));
            let (over, over_attempt) = reservation(&state, &format!("{dimension}-over"))?;
            assert!(matches!(
                state.admit(
                    over,
                    over_attempt,
                    CapabilityCategory::Tool,
                    &single_dimension_envelope(dimension, 1)?,
                )?,
                ControllerAdmissionOutcome::Denied {
                    reason: ControllerAdmissionDenial::Limit {
                        dimension: denied,
                    },
                    ..
                } if denied == dimension
            ));
        }

        for (category, dimension) in [
            (CapabilityCategory::Process, "process_admissions"),
            (CapabilityCategory::Model, "model_admissions"),
        ] {
            let mut state = account(1, 1)?;
            let (exact, exact_attempt) = reservation(&state, &format!("{dimension}-exact"))?;
            assert!(matches!(
                state.admit(
                    exact,
                    exact_attempt,
                    category.clone(),
                    &InvocationAdmissionEnvelope::not_applicable(),
                )?,
                ControllerAdmissionOutcome::Reserved { .. }
            ));
            let (over, over_attempt) = reservation(&state, &format!("{dimension}-over"))?;
            assert!(matches!(
                state.admit(
                    over,
                    over_attempt,
                    category,
                    &InvocationAdmissionEnvelope::not_applicable(),
                )?,
                ControllerAdmissionOutcome::Denied {
                    reason: ControllerAdmissionDenial::Limit {
                        dimension: denied,
                    },
                    ..
                } if denied == dimension
            ));
        }
        Ok(())
    }

    #[test]
    fn unknown_currency_and_overflow_are_distinct_fail_closed_denials() -> TestResult {
        let mut state = account(2, 2)?;
        for (dimension, envelope) in [
            (
                "input_units",
                InvocationAdmissionEnvelope::new(
                    AdmissionBound::Unknown,
                    AdmissionBound::NotApplicable,
                    AdmissionBound::NotApplicable,
                    AdmissionBound::NotApplicable,
                ),
            ),
            (
                "output_units",
                InvocationAdmissionEnvelope::new(
                    AdmissionBound::NotApplicable,
                    AdmissionBound::Unknown,
                    AdmissionBound::NotApplicable,
                    AdmissionBound::NotApplicable,
                ),
            ),
            (
                "artifact_bytes",
                InvocationAdmissionEnvelope::new(
                    AdmissionBound::NotApplicable,
                    AdmissionBound::NotApplicable,
                    AdmissionBound::Unknown,
                    AdmissionBound::NotApplicable,
                ),
            ),
            (
                "monetary_cost",
                InvocationAdmissionEnvelope::new(
                    AdmissionBound::NotApplicable,
                    AdmissionBound::NotApplicable,
                    AdmissionBound::NotApplicable,
                    AdmissionBound::Unknown,
                ),
            ),
        ] {
            let (reservation, attempt) = reservation(&state, &format!("unknown-{dimension}"))?;
            assert!(matches!(
                state.admit(reservation, attempt, CapabilityCategory::Tool, &envelope)?,
                ControllerAdmissionOutcome::Denied {
                    reason: ControllerAdmissionDenial::Unknown {
                        dimension: denied,
                    },
                    ..
                } if denied == dimension
            ));
        }
        let (currency_reservation, currency_attempt) = reservation(&state, "currency")?;
        let currency = InvocationAdmissionEnvelope::new(
            AdmissionBound::NotApplicable,
            AdmissionBound::NotApplicable,
            AdmissionBound::NotApplicable,
            AdmissionBound::Bounded(AdmissionMonetaryBound::new(1, "EUR")?),
        );
        assert!(matches!(
            state.admit(
                currency_reservation,
                currency_attempt,
                CapabilityCategory::Tool,
                &currency,
            )?,
            ControllerAdmissionOutcome::Denied {
                reason: ControllerAdmissionDenial::CurrencyMismatch,
                ..
            }
        ));

        let budget =
            ControllerResourceBudget::new(1, CurrencyCode::new("USD")?, u64::MAX, 1, 1, 1, 1)?;
        let mut overflow = ControllerAccountState::establish(ControllerAccountDeclaration::new(
            RunId::new("run-controller-overflow")?,
            NodeExecutionId::new("execution-controller-overflow")?,
            "policy:controller-overflow",
            budget,
        )?)?;
        let (exact, exact_attempt) = reservation(&overflow, "overflow-exact")?;
        assert!(matches!(
            overflow.admit(
                exact,
                exact_attempt,
                CapabilityCategory::Tool,
                &single_dimension_envelope("input_units", u64::MAX)?,
            )?,
            ControllerAdmissionOutcome::Reserved { .. }
        ));
        let (over, over_attempt) = reservation(&overflow, "overflow-over")?;
        assert!(matches!(
            overflow.admit(
                over,
                over_attempt,
                CapabilityCategory::Tool,
                &single_dimension_envelope("input_units", 1)?,
            )?,
            ControllerAdmissionOutcome::Denied {
                reason: ControllerAdmissionDenial::Overflow { dimension },
                ..
            } if dimension == "input_units"
        ));
        Ok(())
    }

    #[test]
    fn uncertainty_retains_remainder_blocks_retry_and_roundtrips() -> TestResult {
        let mut state = account(4, 4)?;
        let (first, first_attempt) = reservation(&state, "uncertain-first")?;
        state.admit(
            first.clone(),
            first_attempt,
            CapabilityCategory::Tool,
            &bounded_envelope(4)?,
        )?;
        let (second, second_attempt) = reservation(&state, "uncertain-retry")?;
        state.admit(
            second,
            second_attempt,
            CapabilityCategory::Tool,
            &bounded_envelope(4)?,
        )?;
        let (third, third_attempt) = reservation(&state, "uncertain-over")?;
        assert!(matches!(
            state.admit(
                third,
                third_attempt,
                CapabilityCategory::Tool,
                &bounded_envelope(1)?,
            )?,
            ControllerAdmissionOutcome::Denied {
                reason: ControllerAdmissionDenial::Limit { .. },
                ..
            }
        ));
        state.settle_terminal(&first, None)?;
        assert!(matches!(
            state.blocked(),
            Some(ControllerAccountBlock::UnknownUsage { .. })
        ));
        assert_eq!(state.committed_totals()?.input_units(), 8);
        let stored = serde_json::to_vec(&state)?;
        let reopened: ControllerAccountState = serde_json::from_slice(&stored)?;
        reopened.validate()?;
        assert_eq!(reopened, state);
        Ok(())
    }

    #[test]
    fn missing_usage_blocks_and_late_evidence_settles_the_original_reservation_once() -> TestResult
    {
        let mut state = account(2, 2)?;
        let (reservation, attempt) = reservation(&state, "late-usage")?;
        state.admit(
            reservation.clone(),
            attempt,
            CapabilityCategory::Tool,
            &single_dimension_envelope("input_units", 8)?,
        )?;
        state.settle_terminal(&reservation, None)?;
        assert!(matches!(
            state.blocked(),
            Some(ControllerAccountBlock::UnknownUsage { dimension, .. })
                if dimension == "input_units"
        ));
        assert_eq!(state.outstanding().input_units(), 8);

        state.settle_terminal(
            &reservation,
            Some(&AttemptUsage {
                input_units: Some(3),
                output_units: None,
                duration_ms: None,
                cost: None,
            }),
        )?;
        assert_eq!(state.outstanding().input_units(), 0);
        assert_eq!(state.settled().input_units(), 3);
        assert!(!state.reservations().contains_key(&reservation));
        assert!(matches!(
            state.settle_terminal(&reservation, None),
            Err(PersistenceError::NotFound { .. })
        ));
        state.validate()?;
        Ok(())
    }

    #[test]
    fn terminal_cost_currency_and_partial_dimension_retention_are_exact() -> TestResult {
        let cost_envelope = InvocationAdmissionEnvelope::new(
            AdmissionBound::NotApplicable,
            AdmissionBound::NotApplicable,
            AdmissionBound::NotApplicable,
            AdmissionBound::Bounded(AdmissionMonetaryBound::new(4, "USD")?),
        );
        let mut matching = account(4, 4)?;
        let (matching_reservation, matching_attempt) = reservation(&matching, "matching-cost")?;
        matching.admit(
            matching_reservation.clone(),
            matching_attempt,
            CapabilityCategory::Tool,
            &cost_envelope,
        )?;
        matching.settle_terminal(
            &matching_reservation,
            Some(&AttemptUsage {
                input_units: None,
                output_units: None,
                duration_ms: None,
                cost: Some(MonetaryUsage {
                    micros: 3,
                    currency: CurrencyCode::new("USD")?,
                }),
            }),
        )?;
        assert_eq!(matching.settled().cost_micros(), 3);
        assert_eq!(matching.outstanding().cost_micros(), 0);
        assert!(matching.blocked().is_none());
        assert!(!matching.reservations().contains_key(&matching_reservation));

        let mut mismatching = account(4, 4)?;
        let (mismatching_reservation, mismatching_attempt) =
            reservation(&mismatching, "mismatching-cost")?;
        mismatching.admit(
            mismatching_reservation.clone(),
            mismatching_attempt,
            CapabilityCategory::Tool,
            &cost_envelope,
        )?;
        mismatching.settle_terminal(
            &mismatching_reservation,
            Some(&AttemptUsage {
                input_units: None,
                output_units: None,
                duration_ms: None,
                cost: Some(MonetaryUsage {
                    micros: 3,
                    currency: CurrencyCode::new("EUR")?,
                }),
            }),
        )?;
        assert!(matches!(
            mismatching.blocked(),
            Some(ControllerAccountBlock::Integrity { reason })
                if reason.contains("currency differs")
        ));
        assert_eq!(mismatching.settled().cost_micros(), 0);
        assert_eq!(mismatching.outstanding().cost_micros(), 4);
        assert!(
            mismatching
                .reservations()
                .contains_key(&mismatching_reservation)
        );

        let mut partial = account(4, 4)?;
        let (partial_reservation, partial_attempt) = reservation(&partial, "partial-usage")?;
        partial.admit(
            partial_reservation.clone(),
            partial_attempt,
            CapabilityCategory::Tool,
            &InvocationAdmissionEnvelope::new(
                AdmissionBound::Bounded(4),
                AdmissionBound::Bounded(4),
                AdmissionBound::NotApplicable,
                AdmissionBound::NotApplicable,
            ),
        )?;
        partial.settle_terminal(
            &partial_reservation,
            Some(&AttemptUsage {
                input_units: Some(2),
                output_units: None,
                duration_ms: None,
                cost: None,
            }),
        )?;
        assert_eq!(partial.settled().input_units(), 2);
        assert_eq!(partial.outstanding().input_units(), 0);
        assert_eq!(partial.outstanding().output_units(), 4);
        assert!(partial.reservations().contains_key(&partial_reservation));
        partial.validate()?;
        Ok(())
    }

    #[test]
    fn account_mutating_transactions_require_the_exact_revision_guard() -> TestResult {
        let state = account(4, 4)?;
        let (reservation, attempt) = reservation(&state, "transaction-guard")?;
        let envelope = InvocationAdmissionEnvelope::not_applicable();
        let mut candidate = state.clone();
        let outcome = candidate.admit(
            reservation.clone(),
            attempt.clone(),
            CapabilityCategory::Process,
            &envelope,
        )?;
        let action = ControllerAccountAction::AdmitEntry {
            account: state.declaration().account().clone(),
            reservation,
            attempt,
            category: CapabilityCategory::Process,
            envelope,
            expected_outcome: outcome,
        };
        assert!(matches!(
            ControllerAccountTransaction::new(
                ControllerTransitionId::new("transition-controller-unguarded")?,
                None,
                vec![action.clone()],
            ),
            Err(PersistenceError::InvalidDocument(reason))
                if reason.contains("require the exact account revision guard")
        ));
        assert!(matches!(
            ControllerAccountTransaction::new(
                ControllerTransitionId::new("transition-controller-wrong-guard")?,
                Some((
                    ControllerAccountId::new("controller-account:foreign")?,
                    state.revision_digest().clone(),
                )),
                vec![action.clone()],
            ),
            Err(PersistenceError::InvalidDocument(reason))
                if reason.contains("require the exact account revision guard")
        ));
        let guarded = ControllerAccountTransaction::new(
            ControllerTransitionId::new("transition-controller-guarded")?,
            Some((
                state.declaration().account().clone(),
                state.revision_digest().clone(),
            )),
            vec![action],
        )?;
        assert_eq!(
            guarded
                .expected_account_revision()
                .map(|(account, _)| account),
            Some(state.declaration().account())
        );
        Ok(())
    }

    #[test]
    fn over_contract_usage_records_actual_evidence_and_blocks() -> TestResult {
        let mut state = account(4, 4)?;
        let (reservation, attempt) = reservation(&state, "contract")?;
        state.admit(
            reservation.clone(),
            attempt,
            CapabilityCategory::Tool,
            &bounded_envelope(4)?,
        )?;
        state.settle_terminal(
            &reservation,
            Some(&AttemptUsage {
                input_units: Some(5),
                output_units: Some(4),
                duration_ms: None,
                cost: Some(MonetaryUsage {
                    micros: 4,
                    currency: CurrencyCode::new("USD")?,
                }),
            }),
        )?;
        assert!(matches!(
            state.blocked(),
            Some(ControllerAccountBlock::ContractViolation {
                dimension,
                observed: 5,
                reserved: 4,
                ..
            }) if dimension == "input_units"
        ));
        assert_eq!(state.settled().input_units(), 4);
        state.validate()?;
        Ok(())
    }

    #[test]
    fn artifact_reservation_accepts_exact_boundary_and_refuses_one_more() -> TestResult {
        let mut state = account(4, 4)?;
        let (reservation, attempt) = reservation(&state, "artifact")?;
        state.admit(
            reservation.clone(),
            attempt,
            CapabilityCategory::Tool,
            &bounded_envelope(8)?,
        )?;
        state.charge_artifact(Some(&reservation), 8)?;
        assert_eq!(state.settled().artifact_bytes(), 8);
        assert!(matches!(
            state.charge_artifact(Some(&reservation), 1),
            Err(PersistenceError::Bounds {
                location: "controller.artifact_reservation",
                ..
            })
        ));
        state.validate()?;
        Ok(())
    }
}
