use milkdrift_persistence::{
    ArtifactPublicationId, ControllerAccountAction, ControllerAccountId, ControllerArtifactOwner,
    ControllerAssessmentBoundary, ControllerTransitionId, RunEventEnvelope, RunEventKind,
};
use milkdrift_workspace::RunId;

use super::super::{
    COMMAND_RESULTS, CONTROLLER_ACCOUNTS, CONTROLLER_ARTIFACT_CHARGES, CONTROLLER_RUN_BINDINGS,
    CONTROLLER_TRANSITIONS, PersistenceError, RUN_EVENTS, RUN_HEADS, codec, error,
};
use super::{ScanContext, phase};

pub(super) fn scan(context: &mut ScanContext<'_, '_>) -> Result<(), PersistenceError> {
    let read = context.read;
    let accounts = read.open_table(CONTROLLER_ACCOUNTS).map_err(error::redb)?;
    let bindings = read
        .open_table(CONTROLLER_RUN_BINDINGS)
        .map_err(error::redb)?;
    let transitions = read
        .open_table(CONTROLLER_TRANSITIONS)
        .map_err(error::redb)?;
    let commands = read.open_table(COMMAND_RESULTS).map_err(error::redb)?;
    let events = read.open_table(RUN_EVENTS).map_err(error::redb)?;
    let heads = read.open_table(RUN_HEADS).map_err(error::redb)?;
    let charges = read
        .open_table(CONTROLLER_ARTIFACT_CHARGES)
        .map_err(error::redb)?;

    context.string_bytes(
        phase::CONTROLLER_ACCOUNTS,
        &accounts,
        "controller_accounts",
        |key, bytes| {
            let key = ControllerAccountId::new(key)?;
            let state = crate::controller_account::decode_account(bytes)?;
            if state.declaration().account() != &key {
                return Err(error::corruption(
                    "controller account key disagrees with its declaration",
                ));
            }
            let binding = bindings
                .get(state.declaration().controller_run().as_str())
                .map_err(error::redb)?
                .ok_or_else(|| {
                    error::corruption("controller account has no originating run binding")
                })?;
            if binding.value() != key.as_str() {
                return Err(error::corruption(
                    "controller account originating run is bound elsewhere",
                ));
            }
            Ok(())
        },
    )?;
    context.string_string(
        phase::CONTROLLER_RUN_BINDINGS,
        &bindings,
        "controller_run_bindings",
        |key, value| {
            let _run = RunId::new(key).map_err(|cause| {
                error::corruption(format!("invalid controller-bound run identity: {cause}"))
            })?;
            let account = ControllerAccountId::new(value)?;
            let bytes = accounts
                .get(account.as_str())
                .map_err(error::redb)?
                .ok_or_else(|| error::corruption("controller run binding has no account"))?;
            let state = crate::controller_account::decode_account(bytes.value())?;
            if state.declaration().account() != &account {
                return Err(error::corruption(
                    "controller run binding points to a mismatched declaration",
                ));
            }
            Ok(())
        },
    )?;
    context.string_bytes(
        phase::CONTROLLER_TRANSITIONS,
        &transitions,
        "controller_transitions",
        |key, bytes| {
            let transition = ControllerTransitionId::new(key)?;
            let record = crate::controller_account::decode_transition_record(bytes)?;
            if record.transaction.transition() != &transition {
                return Err(error::corruption(
                    "controller transition key disagrees with its transaction",
                ));
            }
            let command_key = codec::pair(record.run.as_str(), record.command.as_str())?;
            let command_bytes = commands
                .get(command_key.as_slice())
                .map_err(error::redb)?
                .ok_or_else(|| {
                    error::corruption("controller transition has no atomic command receipt")
                })?
                .value()
                .to_vec();
            let command = crate::journal::decode_command_record(&command_bytes)?;
            if command.run != record.run || command.command != record.command {
                return Err(error::corruption(
                    "controller transition command link disagrees with its receipt",
                ));
            }
            if command.controller_transition.as_ref() != Some(record.transaction.transition()) {
                return Err(error::corruption(
                    "controller transition is not referenced by its atomic command receipt",
                ));
            }
            let head = crate::journal::validated_run_head(&heads, &events, &record.run)?;
            crate::journal::validate_command_record_history(&command, head, &events)?;
            let command_events = command_events(&events, &command)?;
            validate_transition_event_links(&record.transaction, &record.run, &command_events)
        },
    )?;
    context.string_bytes(
        phase::CONTROLLER_ARTIFACT_CHARGES,
        &charges,
        "controller_artifact_charges",
        |key, bytes| {
            let publication = ArtifactPublicationId::new(key)?;
            let charge = crate::controller_account::decode_artifact_charge(bytes)?;
            if accounts
                .get(charge.account.as_str())
                .map_err(error::redb)?
                .is_none()
            {
                return Err(error::corruption(
                    "controller artifact charge has no account",
                ));
            }
            let owner = charge.reservation.clone().map_or(
                ControllerArtifactOwner::RunBinding,
                ControllerArtifactOwner::InvocationReservation,
            );
            let run = crate::artifact::validate_controller_charge_publication(
                read,
                &publication,
                &owner,
                charge.bytes,
            )?;
            let binding = bindings
                .get(run.as_str())
                .map_err(error::redb)?
                .ok_or_else(|| {
                    error::corruption("controller artifact charge run has no binding")
                })?;
            if binding.value() != charge.account.as_str() {
                return Err(error::corruption(
                    "controller artifact charge disagrees with the publication run binding",
                ));
            }
            Ok(())
        },
    )
}

fn command_events(
    events: &impl redb::ReadableTable<&'static [u8], &'static [u8]>,
    command: &crate::journal::OwnedCommandRecord,
) -> Result<Vec<RunEventEnvelope>, PersistenceError> {
    let mut sequence = command.expected_sequence;
    command
        .result
        .event_ids()
        .iter()
        .map(|expected| {
            sequence = sequence.next()?;
            let key = codec::run_sequence(command.run.as_str(), sequence)?;
            let bytes = events
                .get(key.as_slice())
                .map_err(error::redb)?
                .ok_or_else(|| {
                    error::corruption("controller transition command event is absent")
                })?;
            let event = milkdrift_persistence::RunEventEnvelope::from_json(bytes.value())?;
            if event.event_id() != expected
                || event.run_id() != &command.run
                || event.sequence() != sequence
            {
                return Err(error::corruption(
                    "controller transition command event range changed",
                ));
            }
            Ok(event)
        })
        .collect()
}

fn validate_transition_event_links(
    transaction: &milkdrift_persistence::ControllerAccountTransaction,
    run: &RunId,
    events: &[RunEventEnvelope],
) -> Result<(), PersistenceError> {
    let corrupt = |message: &str| error::corruption(message);
    let mut admissions = Vec::new();
    let mut establishments = Vec::new();
    let mut bindings = Vec::new();
    let mut settlements = Vec::new();
    for action in transaction.actions() {
        match action {
            ControllerAccountAction::Establish {
                declaration,
                bind_run,
            } => establishments.push((declaration, bind_run)),
            ControllerAccountAction::BindRun { account, run } => bindings.push((account, run)),
            ControllerAccountAction::AdmitEntry { .. } => admissions.push(action),
            ControllerAccountAction::SettleTerminal { .. } => settlements.push(action),
        }
    }

    for (declaration, bind_run) in establishments {
        if bind_run != run
            || events
                .iter()
                .filter(|event| {
                    matches!(
                        event.kind(),
                        RunEventKind::ControllerAssessmentRecorded {
                            boundary: ControllerAssessmentBoundary::Activation,
                            account_declaration: Some(recorded),
                            ..
                        } if recorded == declaration
                    )
                })
                .count()
                != 1
        {
            return Err(corrupt(
                "controller establishment no longer matches its activation event",
            ));
        }
    }

    let child_runs = events
        .iter()
        .filter_map(|event| match event.kind() {
            RunEventKind::SubworkflowCreated { child_run, .. } => Some(child_run),
            _ => None,
        })
        .collect::<Vec<_>>();
    if bindings.len() != child_runs.len()
        || bindings.iter().any(|(_, bound_run)| {
            child_runs
                .iter()
                .filter(|child_run| **child_run == *bound_run)
                .count()
                != 1
        })
    {
        return Err(corrupt(
            "controller transition child bindings no longer match its command events",
        ));
    }

    let controlled_entries = events
        .iter()
        .filter(|event| {
            matches!(
                event.kind(),
                RunEventKind::CapabilityAdapterEntryDecisionRecorded {
                    controller_admission,
                    ..
                } if !matches!(
                    controller_admission,
                    milkdrift_persistence::ControllerAdmissionOutcome::NotControlled
                )
            )
        })
        .collect::<Vec<_>>();
    if admissions.len() != controlled_entries.len() {
        return Err(corrupt(
            "controller transition admissions no longer match final-entry events",
        ));
    }
    for action in admissions {
        let ControllerAccountAction::AdmitEntry {
            account,
            reservation,
            attempt,
            expected_outcome,
            ..
        } = action
        else {
            unreachable!("admission actions were filtered")
        };
        let outcome_owns_action = match expected_outcome {
            milkdrift_persistence::ControllerAdmissionOutcome::Reserved {
                account: outcome_account,
                reservation: outcome_reservation,
            } => outcome_account == account && outcome_reservation == reservation,
            milkdrift_persistence::ControllerAdmissionOutcome::Denied {
                account: outcome_account,
                ..
            } => outcome_account == account,
            milkdrift_persistence::ControllerAdmissionOutcome::NotControlled => false,
        };
        let matches_event = controlled_entries
            .iter()
            .filter(|event| {
                matches!(
                    event.kind(),
                    RunEventKind::CapabilityAdapterEntryDecisionRecorded {
                        attempt: recorded_attempt,
                        controller_admission,
                        ..
                    } if recorded_attempt == attempt && controller_admission == expected_outcome
                )
            })
            .count()
            == 1;
        if !outcome_owns_action || !matches_event {
            return Err(corrupt(
                "controller transition admission differs from final-entry evidence",
            ));
        }
    }

    for action in settlements {
        let ControllerAccountAction::SettleTerminal {
            account,
            reservation,
            usage,
        } = action
        else {
            unreachable!("settlement actions were filtered")
        };
        let recorded = crate::controller_account::terminal_settlement_from_events(
            events,
            reservation,
            account,
        )
        .map_err(|cause| {
            error::corruption(format!(
                "controller transition settlement lost terminal evidence: {cause}"
            ))
        })?;
        if recorded.as_ref() != usage.as_ref() {
            return Err(corrupt(
                "controller transition settlement differs from terminal evidence",
            ));
        }
    }
    Ok(())
}
