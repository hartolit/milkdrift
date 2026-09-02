use milkdrift_persistence::{
    ArtifactPublicationId, AtomicRunCommitRequest, ControllerAccountAction, ControllerAccountId,
    ControllerAccountState, ControllerAccountStore, ControllerAdmissionOutcome,
    ControllerArtifactOwner, ControllerReservationId, IntegrityDigest, PersistenceError,
    RunEventKind,
};
use milkdrift_workspace::RunId;
use redb::{ReadableTable as _, WriteTransaction};
use serde::{Deserialize, Serialize};

use crate::schema::{
    CONTROLLER_ACCOUNTS, CONTROLLER_ARTIFACT_CHARGES, CONTROLLER_RUN_BINDINGS,
    CONTROLLER_TRANSITIONS,
};
use crate::{RedbStore, error, json};

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ControllerArtifactCharge {
    pub(crate) account: ControllerAccountId,
    pub(crate) reservation: Option<ControllerReservationId>,
    pub(crate) bytes: u64,
}

impl ControllerAccountStore for RedbStore {
    fn controller_account_binding(
        &self,
        run: &RunId,
    ) -> Result<Option<ControllerAccountId>, PersistenceError> {
        let read = self.database().begin_read().map_err(error::redb)?;
        let bindings = read
            .open_table(CONTROLLER_RUN_BINDINGS)
            .map_err(error::redb)?;
        bindings
            .get(run.as_str())
            .map_err(error::redb)?
            .map(|value| ControllerAccountId::new(value.value().to_owned()))
            .transpose()
    }

    fn controller_account(
        &self,
        account: &ControllerAccountId,
    ) -> Result<Option<ControllerAccountState>, PersistenceError> {
        let read = self.database().begin_read().map_err(error::redb)?;
        let accounts = read.open_table(CONTROLLER_ACCOUNTS).map_err(error::redb)?;
        let Some(bytes) = accounts.get(account.as_str()).map_err(error::redb)? else {
            return Ok(None);
        };
        decode_account(bytes.value()).map(Some)
    }
}

pub(crate) fn charge_artifact_publication(
    write: &WriteTransaction,
    publication: &ArtifactPublicationId,
    run: &RunId,
    owner: &ControllerArtifactOwner,
    bytes: u64,
) -> Result<(), PersistenceError> {
    let binding = binding_in_transaction(write, run)?;
    let (account, reservation) = match owner {
        ControllerArtifactOwner::RunBinding => {
            let Some(account) = binding else {
                return Ok(());
            };
            (account, None)
        }
        ControllerArtifactOwner::InvocationReservation(reservation) => {
            let account = binding.ok_or_else(|| {
                PersistenceError::InvalidDocument(
                    "invocation artifact reservation belongs to an unbound run".to_owned(),
                )
            })?;
            (account, Some(reservation.clone()))
        }
    };
    let charge = ControllerArtifactCharge {
        account: account.clone(),
        reservation: reservation.clone(),
        bytes,
    };
    {
        let charges = write
            .open_table(CONTROLLER_ARTIFACT_CHARGES)
            .map_err(error::redb)?;
        if let Some(existing) = charges.get(publication.as_str()).map_err(error::redb)? {
            let existing: ControllerArtifactCharge =
                json::decode(existing.value(), "controller artifact charge")?;
            return if existing == charge {
                Err(error::corruption(
                    "controller artifact charge exists before publication is committed",
                ))
            } else {
                Err(PersistenceError::ImmutableConflict {
                    entity: "controller artifact charge",
                    identity: publication.as_str().to_owned(),
                })
            };
        }
    }
    let mut state =
        account_in_transaction(write, &account)?.ok_or_else(|| PersistenceError::NotFound {
            entity: "controller account",
            identity: account.as_str().to_owned(),
        })?;
    state.charge_artifact(reservation.as_ref(), bytes)?;
    persist_account(write, &state)?;
    let bytes = json::encode(&charge, "controller artifact charge")?;
    let mut charges = write
        .open_table(CONTROLLER_ARTIFACT_CHARGES)
        .map_err(error::redb)?;
    charges
        .insert(publication.as_str(), bytes.as_slice())
        .map_err(error::redb)?;
    Ok(())
}

pub(crate) fn apply_controller_transaction(
    write: &WriteTransaction,
    request: &AtomicRunCommitRequest,
) -> Result<(), PersistenceError> {
    let transaction = request.controller_account_transaction();
    let existing_binding = binding_in_transaction(write, request.receipt().run())?;
    validate_event_transaction_contract(request, existing_binding.as_ref())?;
    let Some(transaction) = transaction else {
        return Ok(());
    };

    {
        let transitions = write
            .open_table(CONTROLLER_TRANSITIONS)
            .map_err(error::redb)?;
        if let Some(stored) = transitions
            .get(transaction.transition().as_str())
            .map_err(error::redb)?
        {
            let stored = std::str::from_utf8(stored.value())
                .map_err(|_| error::corruption("controller transition fingerprint is not UTF-8"))?;
            let stored = IntegrityDigest::new(stored.to_owned())?;
            if &stored == transaction.fingerprint() {
                return Err(error::corruption(
                    "controller transition exists without its atomic command receipt",
                ));
            }
            return Err(PersistenceError::ImmutableConflict {
                entity: "controller transition",
                identity: transaction.transition().as_str().to_owned(),
            });
        }
    }

    if let Some((account, expected)) = transaction.expected_account_revision() {
        let state =
            account_in_transaction(write, account)?.ok_or_else(|| PersistenceError::NotFound {
                entity: "controller account",
                identity: account.as_str().to_owned(),
            })?;
        if state.revision_digest() != expected {
            return Err(PersistenceError::ControllerAccountRevisionConflict {
                account: account.clone(),
                expected: expected.clone(),
                actual: state.revision_digest().clone(),
            });
        }
    }

    for action in transaction.actions() {
        match action {
            ControllerAccountAction::Establish {
                declaration,
                bind_run,
            } => {
                declaration.validate()?;
                match account_in_transaction(write, declaration.account())? {
                    Some(existing) if existing.declaration() == declaration => {}
                    Some(_) => {
                        return Err(PersistenceError::ImmutableConflict {
                            entity: "controller account",
                            identity: declaration.account().as_str().to_owned(),
                        });
                    }
                    None => persist_account(
                        write,
                        &ControllerAccountState::establish(declaration.clone())?,
                    )?,
                }
                bind_run_to_account(write, bind_run, declaration.account())?;
            }
            ControllerAccountAction::BindRun { account, run } => {
                if account_in_transaction(write, account)?.is_none() {
                    return Err(PersistenceError::NotFound {
                        entity: "controller account",
                        identity: account.as_str().to_owned(),
                    });
                }
                bind_run_to_account(write, run, account)?;
            }
            ControllerAccountAction::AdmitEntry {
                account,
                reservation,
                attempt,
                category,
                envelope,
                expected_outcome,
            } => {
                if binding_in_transaction(write, request.receipt().run())?.as_ref() != Some(account)
                {
                    return Err(PersistenceError::InvalidDocument(
                        "controller final entry does not use the run's immutable account binding"
                            .to_owned(),
                    ));
                }
                let mut state = account_in_transaction(write, account)?.ok_or_else(|| {
                    PersistenceError::NotFound {
                        entity: "controller account",
                        identity: account.as_str().to_owned(),
                    }
                })?;
                let actual = state.admit(
                    reservation.clone(),
                    attempt.clone(),
                    category.clone(),
                    envelope,
                )?;
                if &actual != expected_outcome {
                    return Err(PersistenceError::InvalidDocument(
                        "planned controller admission differs from authoritative account state"
                            .to_owned(),
                    ));
                }
                if matches!(actual, ControllerAdmissionOutcome::Reserved { .. }) {
                    persist_account(write, &state)?;
                }
            }
            ControllerAccountAction::SettleTerminal {
                account,
                reservation,
                usage,
            } => {
                let mut state = account_in_transaction(write, account)?.ok_or_else(|| {
                    PersistenceError::NotFound {
                        entity: "controller account",
                        identity: account.as_str().to_owned(),
                    }
                })?;
                state.settle_terminal(reservation, usage.as_ref())?;
                persist_account(write, &state)?;
            }
        }
    }

    let mut transitions = write
        .open_table(CONTROLLER_TRANSITIONS)
        .map_err(error::redb)?;
    transitions
        .insert(
            transaction.transition().as_str(),
            transaction.fingerprint().as_str().as_bytes(),
        )
        .map_err(error::redb)?;
    Ok(())
}

fn validate_event_transaction_contract(
    request: &AtomicRunCommitRequest,
    binding: Option<&ControllerAccountId>,
) -> Result<(), PersistenceError> {
    let mut event_admission = None;
    let mut event_authorized = None;
    for event in request.events() {
        if let RunEventKind::CapabilityAdapterEntryDecisionRecorded {
            controller_admission,
            authorization,
            ..
        } = event.kind()
        {
            event_admission = Some(controller_admission);
            event_authorized = Some(authorization.is_allowed());
        }
    }
    match (
        binding,
        event_admission,
        request.controller_account_transaction(),
    ) {
        (None, Some(ControllerAdmissionOutcome::NotControlled), _) | (_, None, _) => Ok(()),
        (
            Some(_),
            Some(
                ControllerAdmissionOutcome::Reserved { .. }
                | ControllerAdmissionOutcome::Denied { .. },
            ),
            Some(transaction),
        ) => {
            let planned = transaction
                .actions()
                .iter()
                .find_map(|action| match action {
                    ControllerAccountAction::AdmitEntry {
                        expected_outcome, ..
                    } => Some(expected_outcome),
                    _ => None,
                });
            if planned == event_admission {
                Ok(())
            } else {
                Err(PersistenceError::InvalidDocument(
                    "controller account admission action and final-entry event differ".to_owned(),
                ))
            }
        }
        (Some(_), Some(ControllerAdmissionOutcome::NotControlled), _)
            if event_authorized == Some(false) =>
        {
            Ok(())
        }
        (Some(_), Some(ControllerAdmissionOutcome::NotControlled), _) => {
            Err(PersistenceError::InvalidDocument(
                "an account-bound run cannot record an uncontrolled final entry".to_owned(),
            ))
        }
        (
            Some(_),
            Some(
                ControllerAdmissionOutcome::Reserved { .. }
                | ControllerAdmissionOutcome::Denied { .. },
            ),
            None,
        ) => Err(PersistenceError::InvalidDocument(
            "controller admission requires an atomic account transaction".to_owned(),
        )),
        (
            None,
            Some(
                ControllerAdmissionOutcome::Reserved { .. }
                | ControllerAdmissionOutcome::Denied { .. },
            ),
            _,
        ) => Err(PersistenceError::InvalidDocument(
            "an unbound run cannot record controller admission".to_owned(),
        )),
    }
}

pub(crate) fn binding_in_transaction(
    write: &WriteTransaction,
    run: &RunId,
) -> Result<Option<ControllerAccountId>, PersistenceError> {
    let bindings = write
        .open_table(CONTROLLER_RUN_BINDINGS)
        .map_err(error::redb)?;
    bindings
        .get(run.as_str())
        .map_err(error::redb)?
        .map(|value| ControllerAccountId::new(value.value().to_owned()))
        .transpose()
}

pub(crate) fn account_in_transaction(
    write: &WriteTransaction,
    account: &ControllerAccountId,
) -> Result<Option<ControllerAccountState>, PersistenceError> {
    let accounts = write.open_table(CONTROLLER_ACCOUNTS).map_err(error::redb)?;
    let Some(bytes) = accounts.get(account.as_str()).map_err(error::redb)? else {
        return Ok(None);
    };
    decode_account(bytes.value()).map(Some)
}

pub(crate) fn persist_account(
    write: &WriteTransaction,
    state: &ControllerAccountState,
) -> Result<(), PersistenceError> {
    state.validate()?;
    let bytes = json::encode(state, "controller account")?;
    let mut accounts = write.open_table(CONTROLLER_ACCOUNTS).map_err(error::redb)?;
    accounts
        .insert(state.declaration().account().as_str(), bytes.as_slice())
        .map_err(error::redb)?;
    Ok(())
}

pub(crate) fn decode_account(bytes: &[u8]) -> Result<ControllerAccountState, PersistenceError> {
    let state: ControllerAccountState = json::decode(bytes, "controller account")?;
    state.validate().map_err(|cause| {
        error::corruption(format!(
            "stored controller account failed validation: {cause}"
        ))
    })?;
    Ok(state)
}

pub(crate) fn decode_artifact_charge(
    bytes: &[u8],
) -> Result<ControllerArtifactCharge, PersistenceError> {
    json::decode(bytes, "controller artifact charge")
}

pub(crate) fn validate_event_link(
    read: &redb::ReadTransaction,
    event: &milkdrift_persistence::RunEventEnvelope,
) -> Result<(), PersistenceError> {
    let RunEventKind::CapabilityAdapterEntryDecisionRecorded {
        attempt,
        authorization,
        controller_admission,
    } = event.kind()
    else {
        return Ok(());
    };
    let bindings = read
        .open_table(CONTROLLER_RUN_BINDINGS)
        .map_err(error::redb)?;
    let binding = bindings
        .get(event.run_id().as_str())
        .map_err(error::redb)?
        .map(|value| ControllerAccountId::new(value.value().to_owned()))
        .transpose()?;
    match (binding, controller_admission) {
        (None, ControllerAdmissionOutcome::NotControlled) => Ok(()),
        (Some(_), ControllerAdmissionOutcome::NotControlled) if !authorization.is_allowed() => {
            Ok(())
        }
        (
            Some(account),
            ControllerAdmissionOutcome::Reserved {
                account: recorded,
                reservation,
            },
        ) => {
            if &account != recorded
                || reservation != &ControllerReservationId::for_attempt(recorded, attempt)?
            {
                return Err(error::corruption(
                    "controller final-entry reservation disagrees with its run binding or attempt",
                ));
            }
            require_account_in_read(read, recorded)
        }
        (
            Some(account),
            ControllerAdmissionOutcome::Denied {
                account: recorded, ..
            },
        ) => {
            if &account != recorded {
                return Err(error::corruption(
                    "controller final-entry denial disagrees with its run binding",
                ));
            }
            require_account_in_read(read, recorded)
        }
        (Some(_), ControllerAdmissionOutcome::NotControlled) => Err(error::corruption(
            "allowed final entry on a controller-bound run was recorded as uncontrolled",
        )),
        (
            None,
            ControllerAdmissionOutcome::Reserved { .. } | ControllerAdmissionOutcome::Denied { .. },
        ) => Err(error::corruption(
            "controller final-entry fact has no immutable run binding",
        )),
    }
}

fn require_account_in_read(
    read: &redb::ReadTransaction,
    account: &ControllerAccountId,
) -> Result<(), PersistenceError> {
    let accounts = read.open_table(CONTROLLER_ACCOUNTS).map_err(error::redb)?;
    let bytes = accounts
        .get(account.as_str())
        .map_err(error::redb)?
        .ok_or_else(|| error::corruption("controller run binding has no account"))?;
    let state = decode_account(bytes.value())?;
    if state.declaration().account() != account {
        return Err(error::corruption(
            "controller account key disagrees with its declaration",
        ));
    }
    Ok(())
}

fn bind_run_to_account(
    write: &WriteTransaction,
    run: &RunId,
    account: &ControllerAccountId,
) -> Result<(), PersistenceError> {
    let mut bindings = write
        .open_table(CONTROLLER_RUN_BINDINGS)
        .map_err(error::redb)?;
    if let Some(existing) = bindings.get(run.as_str()).map_err(error::redb)? {
        if existing.value() != account.as_str() {
            return Err(PersistenceError::ImmutableConflict {
                entity: "controller run binding",
                identity: run.as_str().to_owned(),
            });
        }
        return Ok(());
    }
    bindings
        .insert(run.as_str(), account.as_str())
        .map_err(error::redb)?;
    Ok(())
}
