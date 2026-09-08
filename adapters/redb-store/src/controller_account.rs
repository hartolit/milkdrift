//! Commit controller resource changes with the events or artifacts that justify them.
//!
//! Recompute journal transitions against the guarded account and validate their entry,
//! terminal, and child-lineage links before writing. Artifact publication uses the same
//! account owner within its metadata transaction. Immutable predecessor revisions retain
//! evidence for replay and integrity inspection after active reservations settle.

mod validation;
use validation::validate_event_transaction_contract;
pub(crate) use validation::{terminal_settlement_from_events, validate_event_link};

use milkdrift_persistence::{
    ArtifactPublicationId, AtomicRunCommitRequest, CommandId, ControllerAccountAction,
    ControllerAccountId, ControllerAccountState, ControllerAccountStore,
    ControllerAccountTransaction, ControllerAdmissionOutcome, ControllerArtifactChargeOutcome,
    ControllerArtifactOwner, ControllerReservationId, PersistenceError,
};
use milkdrift_workspace::RunId;
use redb::{ReadableTable as _, WriteTransaction};
use serde::{Deserialize, Serialize};

use crate::{
    RedbStore, error, json, schema::CONTROLLER_ACCOUNT_REVISIONS, schema::CONTROLLER_ACCOUNTS,
    schema::CONTROLLER_ARTIFACT_CHARGES, schema::CONTROLLER_RUN_BINDINGS,
    schema::CONTROLLER_TRANSITIONS,
};

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ControllerArtifactCharge {
    pub(crate) account: ControllerAccountId,
    pub(crate) run: RunId,
    pub(crate) reservation: Option<ControllerReservationId>,
    pub(crate) bytes: u64,
    pub(crate) outcome: ControllerArtifactMutationOutcome,
    pub(crate) account_revision: Option<u64>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ControllerArtifactMutationOutcome {
    Charged,
    ContractViolation,
}

impl From<ControllerArtifactChargeOutcome> for ControllerArtifactMutationOutcome {
    fn from(value: ControllerArtifactChargeOutcome) -> Self {
        match value {
            ControllerArtifactChargeOutcome::Charged => Self::Charged,
            ControllerArtifactChargeOutcome::ContractViolation => Self::ContractViolation,
        }
    }
}

const CONTROLLER_TRANSITION_RECORD_SCHEMA_VERSION: u32 = 2;
const CONTROLLER_ACCOUNT_REVISION_RECORD_SCHEMA_VERSION: u32 = 1;

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case", tag = "type", deny_unknown_fields)]
pub(crate) enum ControllerAccountRevisionSource {
    Transition {
        transition: milkdrift_persistence::ControllerTransitionId,
        action_index: u32,
    },
    ArtifactPublication {
        publication: ArtifactPublicationId,
    },
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ControllerAccountRevisionRecord {
    schema_version: u32,
    pub(crate) account: ControllerAccountId,
    pub(crate) revision: u64,
    pub(crate) state: ControllerAccountState,
    pub(crate) source: ControllerAccountRevisionSource,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ControllerTransitionAccountRevision {
    pub(crate) account: ControllerAccountId,
    pub(crate) revision: u64,
    pub(crate) action_index: u32,
}

/// Self-validating transition evidence retained beside the atomic command receipt.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ControllerTransitionRecord {
    schema_version: u32,
    pub(crate) run: RunId,
    pub(crate) command: CommandId,
    pub(crate) transaction: ControllerAccountTransaction,
    pub(crate) account_revisions: Vec<ControllerTransitionAccountRevision>,
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
        let state = decode_account(bytes.value())?;
        validate_account_key(account, &state)?;
        let revisions = read
            .open_table(CONTROLLER_ACCOUNT_REVISIONS)
            .map_err(error::redb)?;
        validate_account_revision_head(&revisions, &state)?;
        Ok(Some(state))
    }
}

pub(crate) fn charge_artifact_publication(
    write: &WriteTransaction,
    publication: &ArtifactPublicationId,
    run: &RunId,
    owner: &ControllerArtifactOwner,
    bytes: u64,
) -> Result<ControllerArtifactChargeOutcome, PersistenceError> {
    let binding = binding_in_transaction(write, run)?;
    let (account, reservation) = match owner {
        ControllerArtifactOwner::RunBinding => {
            let Some(account) = binding else {
                return Ok(ControllerArtifactChargeOutcome::Charged);
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
    {
        let charges = write
            .open_table(CONTROLLER_ARTIFACT_CHARGES)
            .map_err(error::redb)?;
        if let Some(existing) = charges.get(publication.as_str()).map_err(error::redb)? {
            let existing: ControllerArtifactCharge =
                json::decode(existing.value(), "controller artifact charge")?;
            return if existing.account == account
                && &existing.run == run
                && existing.reservation == reservation
                && existing.bytes == bytes
            {
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
    let prior_revision = state.revision();
    let outcome = state.charge_artifact(reservation.as_ref(), bytes)?;
    let account_revision = (state.revision() != prior_revision).then_some(state.revision());
    if account_revision.is_some() {
        persist_account_revision(
            write,
            &state,
            ControllerAccountRevisionSource::ArtifactPublication {
                publication: publication.clone(),
            },
        )?;
    }
    let charge = ControllerArtifactCharge {
        account,
        run: run.clone(),
        reservation,
        bytes,
        outcome: outcome.into(),
        account_revision,
    };
    let bytes = json::encode(&charge, "controller artifact charge")?;
    let mut charges = write
        .open_table(CONTROLLER_ARTIFACT_CHARGES)
        .map_err(error::redb)?;
    charges
        .insert(publication.as_str(), bytes.as_slice())
        .map_err(error::redb)?;
    Ok(outcome)
}

pub(crate) fn apply_controller_transaction(
    write: &WriteTransaction,
    request: &AtomicRunCommitRequest,
) -> Result<(), PersistenceError> {
    let transaction = request.controller_account_transaction();
    let existing_binding = binding_in_transaction(write, request.receipt().run())?;
    validate_event_transaction_contract(write, request, existing_binding.as_ref())?;
    let Some(transaction) = transaction else {
        return Ok(());
    };
    transaction.validate()?;
    {
        let transitions = write
            .open_table(CONTROLLER_TRANSITIONS)
            .map_err(error::redb)?;
        if let Some(stored) = transitions
            .get(transaction.transition().as_str())
            .map_err(error::redb)?
        {
            let stored = decode_transition_record(stored.value())?;
            if stored.transaction.fingerprint() == transaction.fingerprint() {
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

    let mut account_revisions = Vec::new();
    for (action_index, action) in transaction.actions().iter().enumerate() {
        let action_index = u32::try_from(action_index).map_err(|_| {
            PersistenceError::InvalidDocument(
                "controller transaction action index exceeds u32".to_owned(),
            )
        })?;
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
                    None => {
                        let state = ControllerAccountState::establish(declaration.clone())?;
                        persist_account_revision(
                            write,
                            &state,
                            ControllerAccountRevisionSource::Transition {
                                transition: transaction.transition().clone(),
                                action_index,
                            },
                        )?;
                        account_revisions.push(ControllerTransitionAccountRevision {
                            account: declaration.account().clone(),
                            revision: state.revision(),
                            action_index,
                        });
                    }
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
                    persist_account_revision(
                        write,
                        &state,
                        ControllerAccountRevisionSource::Transition {
                            transition: transaction.transition().clone(),
                            action_index,
                        },
                    )?;
                    account_revisions.push(ControllerTransitionAccountRevision {
                        account: account.clone(),
                        revision: state.revision(),
                        action_index,
                    });
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
                persist_account_revision(
                    write,
                    &state,
                    ControllerAccountRevisionSource::Transition {
                        transition: transaction.transition().clone(),
                        action_index,
                    },
                )?;
                account_revisions.push(ControllerTransitionAccountRevision {
                    account: account.clone(),
                    revision: state.revision(),
                    action_index,
                });
            }
        }
    }

    let transition_record = ControllerTransitionRecord {
        schema_version: CONTROLLER_TRANSITION_RECORD_SCHEMA_VERSION,
        run: request.receipt().run().clone(),
        command: request.receipt().command().clone(),
        transaction: transaction.clone(),
        account_revisions,
    };
    let mut transitions = write
        .open_table(CONTROLLER_TRANSITIONS)
        .map_err(error::redb)?;
    let transition_bytes = encode_transition_record(&transition_record)?;
    transitions
        .insert(
            transaction.transition().as_str(),
            transition_bytes.as_slice(),
        )
        .map_err(error::redb)?;
    Ok(())
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
    let state = decode_account(bytes.value())?;
    validate_account_key(account, &state)?;
    let revisions = write
        .open_table(CONTROLLER_ACCOUNT_REVISIONS)
        .map_err(error::redb)?;
    validate_account_revision_head(&revisions, &state)?;
    Ok(Some(state))
}

fn validate_account_key(
    account: &ControllerAccountId,
    state: &ControllerAccountState,
) -> Result<(), PersistenceError> {
    if state.declaration().account() != account {
        return Err(error::corruption(
            "controller account key disagrees with its declaration",
        ));
    }
    Ok(())
}

pub(crate) fn validate_account_revision_head(
    revisions: &impl redb::ReadableTable<&'static str, &'static [u8]>,
    state: &ControllerAccountState,
) -> Result<(), PersistenceError> {
    let account = state.declaration().account();
    let key = controller_account_revision_key(account, state.revision());
    let bytes = revisions
        .get(key.as_str())
        .map_err(error::redb)?
        .ok_or_else(|| error::corruption("controller account has no exact revision evidence"))?;
    let revision = decode_account_revision_record(bytes.value())?;
    if revision.account != *account
        || revision.revision != state.revision()
        || revision.state != *state
    {
        return Err(error::corruption(
            "controller account differs from its exact revision evidence",
        ));
    }
    if let Some(next) = state.revision().checked_add(1) {
        let next_key = controller_account_revision_key(account, next);
        if revisions
            .get(next_key.as_str())
            .map_err(error::redb)?
            .is_some()
        {
            return Err(error::corruption(
                "controller account state predates its durable revision evidence",
            ));
        }
    }
    Ok(())
}

fn persist_account(
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

fn persist_account_revision(
    write: &WriteTransaction,
    state: &ControllerAccountState,
    source: ControllerAccountRevisionSource,
) -> Result<(), PersistenceError> {
    state.validate()?;
    let record = ControllerAccountRevisionRecord {
        schema_version: CONTROLLER_ACCOUNT_REVISION_RECORD_SCHEMA_VERSION,
        account: state.declaration().account().clone(),
        revision: state.revision(),
        state: state.clone(),
        source,
    };
    let key = controller_account_revision_key(&record.account, record.revision);
    let bytes = json::encode(&record, "controller account revision record")?;
    let mut revisions = write
        .open_table(CONTROLLER_ACCOUNT_REVISIONS)
        .map_err(error::redb)?;
    if revisions.get(key.as_str()).map_err(error::redb)?.is_some() {
        return Err(error::corruption(
            "controller account revision already exists before its account mutation",
        ));
    }
    revisions
        .insert(key.as_str(), bytes.as_slice())
        .map_err(error::redb)?;
    drop(revisions);
    persist_account(write, state)
}

pub(crate) fn controller_account_revision_key(
    account: &ControllerAccountId,
    revision: u64,
) -> String {
    format!("{}:{revision:016x}", account.as_str())
}

pub(crate) fn parse_controller_account_revision_key(
    key: &str,
) -> Result<(ControllerAccountId, u64), PersistenceError> {
    let (account, revision) = key.rsplit_once(':').ok_or_else(|| {
        error::corruption("controller account revision key has no revision suffix")
    })?;
    let account = ControllerAccountId::new(account)?;
    let revision = u64::from_str_radix(revision, 16)
        .map_err(|_| error::corruption("controller account revision key is malformed"))?;
    Ok((account, revision))
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

pub(crate) fn decode_account_revision_record(
    bytes: &[u8],
) -> Result<ControllerAccountRevisionRecord, PersistenceError> {
    let record: ControllerAccountRevisionRecord =
        json::decode(bytes, "controller account revision record")?;
    if record.schema_version != CONTROLLER_ACCOUNT_REVISION_RECORD_SCHEMA_VERSION {
        return Err(PersistenceError::UnsupportedVersion {
            document: "controller_account_revision_record",
            found: record.schema_version,
            supported: CONTROLLER_ACCOUNT_REVISION_RECORD_SCHEMA_VERSION,
        });
    }
    record.state.validate().map_err(|cause| {
        error::corruption(format!(
            "stored controller account revision failed validation: {cause}"
        ))
    })?;
    Ok(record)
}

fn encode_transition_record(
    record: &ControllerTransitionRecord,
) -> Result<Vec<u8>, PersistenceError> {
    json::encode(record, "controller transition record")
}

pub(crate) fn decode_transition_record(
    bytes: &[u8],
) -> Result<ControllerTransitionRecord, PersistenceError> {
    let record: ControllerTransitionRecord = json::decode(bytes, "controller transition record")?;
    if record.schema_version != CONTROLLER_TRANSITION_RECORD_SCHEMA_VERSION {
        return Err(PersistenceError::UnsupportedVersion {
            document: "controller_transition_record",
            found: record.schema_version,
            supported: CONTROLLER_TRANSITION_RECORD_SCHEMA_VERSION,
        });
    }
    record.transaction.validate().map_err(|cause| {
        error::corruption(format!(
            "stored controller transition failed validation: {cause}"
        ))
    })?;
    Ok(record)
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
