//! Closed atomic action vocabulary and exact canonical idempotency request.
use super::{
    ControllerAccountDeclaration, ControllerAccountId, ControllerReservationId,
    ControllerTransitionId, MAX_CONTROLLER_ACCOUNT_ACTIONS,
};
use crate::{
    AttemptId, ControllerAdmissionOutcome, IntegrityDigest, PersistenceError,
    document::canonical_json_bytes,
};
use milkdrift_capability::{CapabilityCategory, InvocationAdmissionEnvelope};
use milkdrift_workspace::RunId;
use serde::{Deserialize, Serialize};

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
        for action in &actions {
            if let ControllerAccountAction::Establish {
                declaration,
                bind_run,
            } = action
                && declaration.controller_run() != bind_run
            {
                return Err(PersistenceError::InvalidDocument(
                    "controller establishment must bind its declared originating run".to_owned(),
                ));
            }
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
    /// Recomputes every invariant and the canonical content fingerprint of an untrusted record.
    pub fn validate(&self) -> Result<(), PersistenceError> {
        let rebuilt = Self::new(
            self.transition.clone(),
            self.expected_account_revision.clone(),
            self.actions.clone(),
        )?;
        if &rebuilt != self {
            return Err(PersistenceError::InvalidDocument(
                "controller account transaction is not canonical".to_owned(),
            ));
        }
        Ok(())
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
