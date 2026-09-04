use super::*;

#[test]
fn denied_uncontrolled_final_entry_is_valid_only_for_a_denied_decision() -> TestResult {
    let directory = TempDir::new()?;
    let store = RedbStore::open(directory.path())?;
    let owner = RunId::new("run-controller-denied-link-owner")?;
    let child = RunId::new("run-controller-denied-link-child")?;
    let origin = establish(&store, &owner, "denied-link-owner")?;
    bind_child(&store, &child, &origin, "denied-link-child")?;
    let denied = request(
        &child,
        "command-denied-link-only",
        "event-denied-link-only",
        RunSequence::ZERO,
        RunEventKind::CapabilityAdapterEntryDecisionRecorded {
            attempt: AttemptId::new("attempt-denied-link-only")?,
            authorization: decision(false, "denied-link-only")?,
            controller_admission: ControllerAdmissionOutcome::NotControlled,
        },
    )?;
    let _ = store.commit_command(&denied)?;
    assert!(!has_integrity_failure(&store)?);
    Ok(())
}

#[test]
fn allowed_uncontrolled_final_entry_is_corrupt_after_a_late_binding() -> TestResult {
    let directory = TempDir::new()?;
    let store = RedbStore::open(directory.path())?;
    let owner = RunId::new("run-controller-allowed-link-owner")?;
    let child = RunId::new("run-controller-allowed-link-child")?;
    let origin = establish(&store, &owner, "allowed-link-owner")?;
    let start = request(
        &child,
        "command-allowed-link-child-start",
        "event-allowed-link-child-start",
        RunSequence::ZERO,
        RunEventKind::RunStarted,
    )?;
    let _ = store.commit_command(&start)?;
    let allowed = request(
        &child,
        "command-allowed-link-only",
        "event-allowed-link-only",
        RunSequence::FIRST,
        RunEventKind::CapabilityAdapterEntryDecisionRecorded {
            attempt: AttemptId::new("attempt-allowed-link-only")?,
            authorization: decision(true, "allowed-link-only")?,
            controller_admission: ControllerAdmissionOutcome::NotControlled,
        },
    )?;
    let _ = store.commit_command(&allowed)?;
    bind_child(&store, &child, &origin, "allowed-link-late-bind")?;
    assert!(has_integrity_failure(&store)?);
    Ok(())
}

#[test]
fn reserved_final_entry_must_match_both_binding_and_attempt_identity() -> TestResult {
    let directory = TempDir::new()?;
    let child = RunId::new("run-controller-reserved-link-child-only")?;
    let foreign;
    {
        let store = RedbStore::open(directory.path())?;
        let owner = RunId::new("run-controller-reserved-link-owner-only")?;
        let foreign_owner = RunId::new("run-controller-reserved-link-foreign-only")?;
        let origin = establish(&store, &owner, "reserved-link-owner-only")?;
        foreign = establish(&store, &foreign_owner, "reserved-link-foreign-only")?;
        bind_child(&store, &child, &origin, "reserved-link-child-only")?;
        let state = store
            .controller_account(origin.account())?
            .ok_or("reserved-link origin account is absent")?;
        let attempt = AttemptId::new("attempt-reserved-link-only")?;
        let reservation = ControllerReservationId::for_attempt(origin.account(), &attempt)?;
        let mut candidate = state.clone();
        let outcome = candidate.admit(
            reservation.clone(),
            attempt.clone(),
            CapabilityCategory::Process,
            &InvocationAdmissionEnvelope::not_applicable(),
        )?;
        let reserved = request(
            &child,
            "command-reserved-link-only",
            "event-reserved-link-only",
            RunSequence::ZERO,
            RunEventKind::CapabilityAdapterEntryDecisionRecorded {
                attempt: attempt.clone(),
                authorization: decision(true, "reserved-link-only")?,
                controller_admission: outcome.clone(),
            },
        )?
        .with_controller_account_transaction(transaction(
            "transition-reserved-link-only",
            Some((&state, &origin)),
            vec![ControllerAccountAction::AdmitEntry {
                account: origin.account().clone(),
                reservation,
                attempt,
                category: CapabilityCategory::Process,
                envelope: InvocationAdmissionEnvelope::not_applicable(),
                expected_outcome: outcome,
            }],
        )?)?;
        let _ = store.commit_command(&reserved)?;
        assert!(!has_integrity_failure(&store)?);
    }

    let database = Database::open(directory.path().join("milkdrift.redb"))?;
    let write = database.begin_write()?;
    {
        let mut bindings = write.open_table(BINDINGS)?;
        bindings.insert(child.as_str(), foreign.account().as_str())?;
    }
    write.commit()?;
    drop(database);
    let store = RedbStore::open(directory.path())?;
    assert!(has_integrity_failure(&store)?);
    Ok(())
}

#[test]
fn reserved_final_entry_reservation_must_match_its_attempt_identity() -> TestResult {
    let directory = TempDir::new()?;
    let child = RunId::new("run-controller-reserved-attempt-link-child")?;
    let recorded;
    let origin;
    let attempt = AttemptId::new("attempt-controller-reserved-attempt-link")?;
    {
        let store = RedbStore::open(directory.path())?;
        let owner = RunId::new("run-controller-reserved-attempt-link-owner")?;
        origin = establish(&store, &owner, "reserved-attempt-link-owner")?;
        bind_child(&store, &child, &origin, "reserved-attempt-link-child")?;
        let state = store
            .controller_account(origin.account())?
            .ok_or("reserved-attempt-link origin account is absent")?;
        let reservation = ControllerReservationId::for_attempt(origin.account(), &attempt)?;
        let mut candidate = state.clone();
        let outcome = candidate.admit(
            reservation.clone(),
            attempt.clone(),
            CapabilityCategory::Process,
            &InvocationAdmissionEnvelope::not_applicable(),
        )?;
        let entry = request(
            &child,
            "command-reserved-attempt-link",
            "event-reserved-attempt-link",
            RunSequence::ZERO,
            RunEventKind::CapabilityAdapterEntryDecisionRecorded {
                attempt: attempt.clone(),
                authorization: decision(true, "reserved-attempt-link")?,
                controller_admission: outcome.clone(),
            },
        )?
        .with_controller_account_transaction(transaction(
            "transition-reserved-attempt-link",
            Some((&state, &origin)),
            vec![ControllerAccountAction::AdmitEntry {
                account: origin.account().clone(),
                reservation,
                attempt: attempt.clone(),
                category: CapabilityCategory::Process,
                envelope: InvocationAdmissionEnvelope::not_applicable(),
                expected_outcome: outcome,
            }],
        )?)?;
        recorded = entry
            .events()
            .first()
            .ok_or("reserved final-entry event is absent")?
            .clone();
        let _ = store.commit_command(&entry)?;
        assert!(!has_integrity_failure(&store)?);
    }

    let RunEventKind::CapabilityAdapterEntryDecisionRecorded { authorization, .. } =
        recorded.kind()
    else {
        return Err("recorded event is not a final-entry decision".into());
    };
    let forged_reservation = ControllerReservationId::for_attempt(
        origin.account(),
        &AttemptId::new("attempt-controller-reserved-attempt-link-forged")?,
    )?;
    let replacement = RunEventEnvelope::new(
        recorded.event_id().clone(),
        recorded.run_id().clone(),
        recorded.sequence(),
        recorded.occurred_at(),
        RunEventKind::CapabilityAdapterEntryDecisionRecorded {
            attempt,
            authorization: authorization.clone(),
            controller_admission: ControllerAdmissionOutcome::Reserved {
                account: origin.account().clone(),
                reservation: forged_reservation,
            },
        },
    )?;
    let bytes = replacement.to_canonical_json()?;
    let database = Database::open(directory.path().join("milkdrift.redb"))?;
    let write = database.begin_write()?;
    write.open_table(RUN_EVENTS)?.insert(
        stored_event_key(&child, recorded.sequence())?.as_slice(),
        bytes.as_slice(),
    )?;
    write.commit()?;
    drop(database);

    let store = RedbStore::open(directory.path())?;
    assert!(has_integrity_failure_matching(
        &store,
        "controller_accounts",
        "controller final-entry reservation disagrees with its run binding or attempt",
    )?);
    Ok(())
}
