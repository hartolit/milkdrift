use super::*;

#[test]
fn transition_integrity_requires_every_durable_child_binding() -> TestResult {
    let directory = TempDir::new()?;
    let owner = RunId::new("run-controller-binding-integrity-owner")?;
    let child = RunId::new("run-controller-binding-integrity-child")?;
    {
        let store = RedbStore::open(directory.path())?;
        let declaration = establish(&store, &owner, "binding-integrity")?;
        bind_child(&store, &child, &declaration, "binding-integrity-child")?;
        assert!(!has_integrity_failure(&store)?);
    }

    let database = Database::open(directory.path().join("milkdrift.redb"))?;
    let write = database.begin_write()?;
    {
        let mut bindings = write.open_table(BINDINGS)?;
        assert_eq!(
            bindings
                .remove(child.as_str())?
                .map(|value| value.value().to_owned()),
            Some(
                declaration(&owner, "binding-integrity")?
                    .account()
                    .as_str()
                    .to_owned()
            )
        );
    }
    write.commit()?;
    drop(database);

    let store = RedbStore::open(directory.path())?;
    assert!(has_integrity_failure(&store)?);
    Ok(())
}

#[test]
fn integrity_rejects_a_checksum_correct_account_totals_rewrite() -> TestResult {
    let directory = TempDir::new()?;
    let owner = RunId::new("run-controller-account-rollback-owner")?;
    let child = RunId::new("run-controller-account-rollback-child")?;
    let declaration = {
        let store = RedbStore::open(directory.path())?;
        let declaration = establish(&store, &owner, "account-rollback")?;
        bind_child(&store, &child, &declaration, "account-rollback-child")?;
        let state = store
            .controller_account(declaration.account())?
            .ok_or("controller account is absent before admission")?;
        let attempt = AttemptId::new("attempt-controller-account-rollback")?;
        let reservation = ControllerReservationId::for_attempt(declaration.account(), &attempt)?;
        let envelope = InvocationAdmissionEnvelope::new(
            AdmissionBound::Bounded(9),
            AdmissionBound::Bounded(7),
            AdmissionBound::NotApplicable,
            AdmissionBound::NotApplicable,
        );
        let mut candidate = state.clone();
        let outcome = candidate.admit(
            reservation.clone(),
            attempt.clone(),
            CapabilityCategory::Tool,
            &envelope,
        )?;
        let request = request(
            &child,
            "command-controller-account-rollback-entry",
            "event-controller-account-rollback-entry",
            RunSequence::ZERO,
            RunEventKind::CapabilityAdapterEntryDecisionRecorded {
                attempt: attempt.clone(),
                authorization: decision(true, "account-rollback-entry")?,
                controller_admission: outcome.clone(),
            },
        )?
        .with_controller_account_transaction(transaction(
            "transition-controller-account-rollback-entry",
            Some((&state, &declaration)),
            vec![ControllerAccountAction::AdmitEntry {
                account: declaration.account().clone(),
                reservation,
                attempt,
                category: CapabilityCategory::Tool,
                envelope,
                expected_outcome: outcome,
            }],
        )?)?;
        let _ = store.commit_command(&request)?;
        assert_eq!(
            store
                .controller_account(declaration.account())?
                .ok_or("controller account disappeared after admission")?
                .revision(),
            1
        );
        assert!(!has_integrity_failure(&store)?);
        declaration
    };

    let attempt = AttemptId::new("attempt-controller-account-rollback")?;
    let reservation = ControllerReservationId::for_attempt(declaration.account(), &attempt)?;
    let mut altered = ControllerAccountState::establish(declaration.clone())?;
    let altered_outcome = altered.admit(
        reservation,
        attempt,
        CapabilityCategory::Tool,
        &InvocationAdmissionEnvelope::new(
            AdmissionBound::Bounded(1),
            AdmissionBound::Bounded(1),
            AdmissionBound::NotApplicable,
            AdmissionBound::NotApplicable,
        ),
    )?;
    assert!(matches!(
        altered_outcome,
        ControllerAdmissionOutcome::Reserved { .. }
    ));
    assert_eq!(altered.revision(), 1);
    assert_eq!(altered.outstanding().input_units(), 1);
    let payload = serde_json::to_string(&altered)?;
    let bytes = encode_internal_payload("controller account", &payload)?;
    let database = Database::open(directory.path().join("milkdrift.redb"))?;
    let write = database.begin_write()?;
    write
        .open_table(ACCOUNTS)?
        .insert(declaration.account().as_str(), bytes.as_slice())?;
    write.commit()?;
    drop(database);

    let store = RedbStore::open(directory.path())?;
    assert!(has_integrity_failure(&store)?);
    Ok(())
}
