use milkdrift_persistence::{
    ArtifactPublicationId, AtomicRunCommitRequest, AttemptUsage, ControllerAccountAction,
    ControllerAccountId, ControllerAccountState, ControllerAccountStore,
    ControllerAccountTransaction, ControllerAdmissionOutcome, ControllerArtifactChargeOutcome,
    ControllerArtifactOwner, ControllerAssessmentBoundary, ControllerReservationId, CurrencyCode,
    IntegrityDigest, PersistenceError, RunEventKind,
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
        let state = decode_account(bytes.value())?;
        validate_account_key(account, &state)?;
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
    let outcome = state.charge_artifact(reservation.as_ref(), bytes)?;
    persist_account(write, &state)?;
    if outcome == ControllerArtifactChargeOutcome::ContractViolation {
        return Ok(outcome);
    }
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
    write: &WriteTransaction,
    request: &AtomicRunCommitRequest,
    binding: Option<&ControllerAccountId>,
) -> Result<(), PersistenceError> {
    let transaction = request.controller_account_transaction();
    validate_lineage_transaction_contract(write, request, binding)?;
    let mut entry_events = Vec::new();
    for event in request.events() {
        if let RunEventKind::CapabilityAdapterEntryDecisionRecorded {
            attempt,
            controller_admission,
            authorization,
        } = event.kind()
        {
            entry_events.push((attempt, controller_admission, authorization.is_allowed()));
        }
    }
    if entry_events.len() > 1 {
        return Err(PersistenceError::InvalidDocument(
            "one atomic command cannot record multiple final-entry decisions".to_owned(),
        ));
    }

    let admission_actions = transaction.map_or_else(Vec::new, |transaction| {
        transaction
            .actions()
            .iter()
            .filter(|action| matches!(action, ControllerAccountAction::AdmitEntry { .. }))
            .collect::<Vec<_>>()
    });
    if admission_actions.len() > 1 {
        return Err(PersistenceError::InvalidDocument(
            "one final-entry event cannot authorize multiple controller admissions".to_owned(),
        ));
    }
    match entry_events.as_slice() {
        [] if admission_actions.is_empty() => {}
        [] => {
            return Err(PersistenceError::InvalidDocument(
                "controller admission action has no final-entry event".to_owned(),
            ));
        }
        [(attempt, ControllerAdmissionOutcome::NotControlled, authorized)] => {
            if !admission_actions.is_empty() {
                return Err(PersistenceError::InvalidDocument(
                    "an uncontrolled final entry cannot mutate a controller account".to_owned(),
                ));
            }
            if binding.is_some() && *authorized {
                return Err(PersistenceError::InvalidDocument(
                    "an account-bound run cannot record an allowed uncontrolled final entry"
                        .to_owned(),
                ));
            }
            let _ = attempt;
        }
        [(attempt, event_outcome, authorized)] => {
            let Some(binding) = binding else {
                return Err(PersistenceError::InvalidDocument(
                    "an unbound run cannot record controller admission".to_owned(),
                ));
            };
            if !authorized {
                return Err(PersistenceError::InvalidDocument(
                    "controller resource admission requires an allowed authority decision"
                        .to_owned(),
                ));
            }
            let Some(ControllerAccountAction::AdmitEntry {
                account,
                attempt: action_attempt,
                expected_outcome,
                ..
            }) = admission_actions.first().copied()
            else {
                return Err(PersistenceError::InvalidDocument(
                    "controller admission requires exactly one atomic account action".to_owned(),
                ));
            };
            if account != binding {
                return Err(PersistenceError::InvalidDocument(
                    "controller admission action does not use the run's account binding".to_owned(),
                ));
            }
            if action_attempt != *attempt {
                return Err(PersistenceError::InvalidDocument(
                    "controller admission action and final-entry attempt differ".to_owned(),
                ));
            }
            if expected_outcome != *event_outcome {
                return Err(PersistenceError::InvalidDocument(
                    "controller admission action and final-entry outcome differ".to_owned(),
                ));
            }
            if transaction
                .and_then(|value| value.expected_account_revision())
                .map(|(id, _)| id)
                != Some(binding)
            {
                return Err(PersistenceError::InvalidDocument(
                    "controller admission lacks its exact account revision guard".to_owned(),
                ));
            }
        }
        _ => unreachable!("entry event cardinality was checked"),
    }

    let settlement_actions = transaction.map_or_else(Vec::new, |transaction| {
        transaction
            .actions()
            .iter()
            .filter(|action| matches!(action, ControllerAccountAction::SettleTerminal { .. }))
            .collect::<Vec<_>>()
    });
    for (index, action) in settlement_actions.iter().enumerate() {
        let ControllerAccountAction::SettleTerminal {
            account,
            reservation,
            usage,
        } = action
        else {
            unreachable!("settlement actions were filtered")
        };
        let Some(binding) = binding else {
            return Err(PersistenceError::InvalidDocument(
                "an unbound run cannot settle a controller reservation".to_owned(),
            ));
        };
        if account != binding {
            return Err(PersistenceError::InvalidDocument(
                "controller settlement does not use the run's account binding".to_owned(),
            ));
        }
        let expected_usage = terminal_settlement(request, reservation, binding)?;
        if usage.as_ref() != expected_usage.as_ref() {
            return Err(PersistenceError::InvalidDocument(
                "controller settlement usage differs from its terminal event".to_owned(),
            ));
        }
        if settlement_actions[..index].iter().any(|prior| {
            matches!(
                prior,
                ControllerAccountAction::SettleTerminal {
                    reservation: prior,
                    ..
                } if prior == reservation
            )
        }) {
            return Err(PersistenceError::InvalidDocument(
                "one terminal event cannot settle a controller reservation twice".to_owned(),
            ));
        }
    }
    if let Some(account) = binding {
        let state = account_in_transaction(write, account)?.ok_or_else(|| {
            error::corruption("controller run binding has no account during atomic commit")
        })?;
        for event in request.events() {
            let (attempt, late) = match event.kind() {
                RunEventKind::NodeTerminal { attempt, .. } => (attempt, false),
                RunEventKind::LateTerminalEvidenceRecorded { attempt, .. } => (attempt, true),
                _ => continue,
            };
            let reservation = ControllerReservationId::for_attempt(account, attempt)?;
            let newly_reserved = entry_events.iter().any(|(entry_attempt, outcome, _)| {
                *entry_attempt == attempt
                    && matches!(
                        outcome,
                        ControllerAdmissionOutcome::Reserved {
                            reservation: recorded,
                            ..
                        } if recorded == &reservation
                    )
            });
            let requires_settlement =
                state.reservations().contains_key(&reservation) || newly_reserved;
            let settlement_count = settlement_actions
                .iter()
                .filter(|action| {
                    matches!(
                        action,
                        ControllerAccountAction::SettleTerminal {
                            reservation: recorded,
                            ..
                        } if recorded == &reservation
                    )
                })
                .count();
            if requires_settlement && settlement_count != 1 {
                return Err(PersistenceError::InvalidDocument(
                    "controlled terminal event lacks its exact account settlement".to_owned(),
                ));
            }
            if late && !requires_settlement {
                return Err(PersistenceError::InvalidDocument(
                    "controlled late terminal event has no outstanding reservation".to_owned(),
                ));
            }
        }
    }
    Ok(())
}

fn validate_lineage_transaction_contract(
    write: &WriteTransaction,
    request: &AtomicRunCommitRequest,
    binding: Option<&ControllerAccountId>,
) -> Result<(), PersistenceError> {
    let actions = request
        .controller_account_transaction()
        .map_or(&[][..], ControllerAccountTransaction::actions);
    let establishments = actions
        .iter()
        .filter_map(|action| match action {
            ControllerAccountAction::Establish {
                declaration,
                bind_run,
            } => Some((declaration, bind_run)),
            _ => None,
        })
        .collect::<Vec<_>>();
    if establishments.len() > 1 {
        return Err(PersistenceError::InvalidDocument(
            "one command cannot establish multiple controller accounts".to_owned(),
        ));
    }
    let assessments = request
        .events()
        .iter()
        .map(|event| match event.kind() {
            RunEventKind::ControllerAssessmentRecorded {
                boundary,
                account_declaration: Some(declaration),
                ..
            } => {
                declaration.validate()?;
                Ok(Some((*boundary, declaration)))
            }
            RunEventKind::ControllerAssessmentRecorded {
                account_declaration: None,
                ..
            } => Err(PersistenceError::InvalidDocument(
                "controller assessment omitted its account declaration".to_owned(),
            )),
            _ => Ok(None),
        })
        .collect::<Result<Vec<_>, PersistenceError>>()?
        .into_iter()
        .flatten()
        .collect::<Vec<_>>();
    if let Some(account) = binding {
        let state = account_in_transaction(write, account)?.ok_or_else(|| {
            error::corruption("controller run binding has no account during atomic commit")
        })?;
        if assessments
            .iter()
            .any(|(_, declaration)| *declaration != state.declaration())
        {
            return Err(PersistenceError::InvalidDocument(
                "controller assessment differs from its immutable account declaration".to_owned(),
            ));
        }
    }
    let activations = assessments
        .iter()
        .filter_map(|(boundary, declaration)| {
            (*boundary == ControllerAssessmentBoundary::Activation).then_some(*declaration)
        })
        .collect::<Vec<_>>();
    if activations.len() > 1 {
        return Err(PersistenceError::InvalidDocument(
            "one command cannot record multiple controller activations".to_owned(),
        ));
    }
    if binding.is_none() && !assessments.is_empty() && activations.is_empty() {
        return Err(PersistenceError::InvalidDocument(
            "non-activation controller assessment has no immutable account binding".to_owned(),
        ));
    }
    if let Some(activation) = activations.first()
        && assessments
            .iter()
            .any(|(_, declaration)| declaration != activation)
    {
        return Err(PersistenceError::InvalidDocument(
            "one command cannot attribute controller assessments to different accounts".to_owned(),
        ));
    }
    let established_account = match (activations.as_slice(), establishments.as_slice()) {
        ([declaration], [(action_declaration, bind_run)])
            if declaration == action_declaration
                && bind_run == &request.receipt().run()
                && binding.is_none_or(|account| account == declaration.account()) =>
        {
            Some(declaration.account())
        }
        ([declaration], []) if binding == Some(declaration.account()) => {
            Some(declaration.account())
        }
        ([], []) => None,
        _ => {
            return Err(PersistenceError::InvalidDocument(
                "controller establishment action and activation event differ".to_owned(),
            ));
        }
    };

    let effective_account = binding.or(established_account);
    let child_runs = request
        .events()
        .iter()
        .filter_map(|event| match event.kind() {
            RunEventKind::SubworkflowCreated { child_run, .. } => Some(child_run),
            _ => None,
        })
        .collect::<Vec<_>>();
    if child_runs
        .iter()
        .enumerate()
        .any(|(index, child)| child_runs[..index].iter().any(|prior| prior == child))
    {
        return Err(PersistenceError::InvalidDocument(
            "one command cannot create the same controller child run twice".to_owned(),
        ));
    }
    let bindings = actions
        .iter()
        .filter_map(|action| match action {
            ControllerAccountAction::BindRun { account, run } => Some((account, run)),
            _ => None,
        })
        .collect::<Vec<_>>();
    match effective_account {
        None if bindings.is_empty() => Ok(()),
        None => Err(PersistenceError::InvalidDocument(
            "an unbound parent cannot create a controller run binding".to_owned(),
        )),
        Some(account)
            if bindings.len() == child_runs.len()
                && bindings.iter().all(|(bound_account, run)| {
                    *bound_account == account
                        && child_runs.iter().filter(|child| *child == run).count() == 1
                }) =>
        {
            Ok(())
        }
        Some(_) => Err(PersistenceError::InvalidDocument(
            "controller child-run binding actions do not exactly match subworkflow creation events"
                .to_owned(),
        )),
    }
}

fn terminal_settlement(
    request: &AtomicRunCommitRequest,
    reservation: &ControllerReservationId,
    account: &ControllerAccountId,
) -> Result<Option<AttemptUsage>, PersistenceError> {
    let mut matched = None;
    for event in request.events() {
        let candidate = match event.kind() {
            RunEventKind::NodeTerminal { attempt, .. } => {
                let usage = request
                    .events()
                    .iter()
                    .filter_map(|event| match event.kind() {
                        RunEventKind::AttemptUsageRecorded {
                            attempt: usage_attempt,
                            usage,
                        } if usage_attempt == attempt => Some(usage.clone()),
                        _ => None,
                    })
                    .collect::<Vec<_>>();
                if usage.len() > 1 {
                    return Err(PersistenceError::InvalidDocument(
                        "one terminal cannot have multiple usage observations".to_owned(),
                    ));
                }
                Some((attempt, usage.into_iter().next()))
            }
            RunEventKind::LateTerminalEvidenceRecorded {
                attempt, terminal, ..
            } => Some((attempt, terminal_usage(terminal)?)),
            _ => None,
        };
        let Some((attempt, usage)) = candidate else {
            continue;
        };
        if &ControllerReservationId::for_attempt(account, attempt)? == reservation {
            if matched.is_some() {
                return Err(PersistenceError::InvalidDocument(
                    "one reservation has multiple terminal events in one command".to_owned(),
                ));
            }
            matched = Some(usage);
        }
    }
    matched.ok_or_else(|| {
        PersistenceError::InvalidDocument(
            "controller settlement has no exact terminal event".to_owned(),
        )
    })
}

fn terminal_usage(
    terminal: &milkdrift_capability::InvocationTerminal,
) -> Result<Option<AttemptUsage>, PersistenceError> {
    terminal
        .usage()
        .map(|usage| {
            let cost = match usage.cost_micros().zip(usage.currency()) {
                Some((micros, currency)) => Some(milkdrift_persistence::MonetaryUsage {
                    micros,
                    currency: CurrencyCode::new(currency.to_owned())?,
                }),
                None => None,
            };
            Ok(AttemptUsage {
                input_units: usage.input_units(),
                output_units: usage.output_units(),
                duration_ms: usage.duration_ms(),
                cost,
            })
        })
        .transpose()
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
    if !matches!(
        event.kind(),
        RunEventKind::ControllerAssessmentRecorded { .. }
            | RunEventKind::CapabilityAdapterEntryDecisionRecorded { .. }
    ) {
        return Ok(());
    }
    let bindings = read
        .open_table(CONTROLLER_RUN_BINDINGS)
        .map_err(error::redb)?;
    let binding = bindings
        .get(event.run_id().as_str())
        .map_err(error::redb)?
        .map(|value| ControllerAccountId::new(value.value().to_owned()))
        .transpose()?;
    if let RunEventKind::ControllerAssessmentRecorded {
        account_declaration,
        ..
    } = event.kind()
    {
        let declaration = account_declaration.as_ref().ok_or_else(|| {
            error::corruption("controller assessment has no immutable account declaration")
        })?;
        declaration.validate().map_err(|cause| {
            error::corruption(format!(
                "controller assessment declaration is invalid: {cause}"
            ))
        })?;
        if binding.as_ref() != Some(declaration.account()) {
            return Err(error::corruption(
                "controller assessment disagrees with its run binding",
            ));
        }
        let accounts = read.open_table(CONTROLLER_ACCOUNTS).map_err(error::redb)?;
        let bytes = accounts
            .get(declaration.account().as_str())
            .map_err(error::redb)?
            .ok_or_else(|| error::corruption("controller assessment has no durable account"))?;
        let state = decode_account(bytes.value())?;
        if state.declaration() != declaration {
            return Err(error::corruption(
                "controller assessment differs from its durable account declaration",
            ));
        }
        return Ok(());
    }
    let RunEventKind::CapabilityAdapterEntryDecisionRecorded {
        attempt,
        authorization,
        controller_admission,
    } = event.kind()
    else {
        return Ok(());
    };
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
