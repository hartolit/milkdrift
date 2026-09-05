use super::*;

#[test]
fn final_entry_rejects_unlinked_and_duplicate_admission_actions() -> TestResult {
    let directory = TempDir::new()?;
    let store = RedbStore::open(directory.path())?;
    let owner = RunId::new("run-controller-admission-link-owner")?;
    let child = RunId::new("run-controller-admission-link-child")?;
    let declaration = establish(&store, &owner, "admission-link-owner")?;
    bind_child(&store, &child, &declaration, "admission-link-child")?;
    let state = store
        .controller_account(declaration.account())?
        .ok_or("controller account is absent")?;

    let attempt = AttemptId::new("attempt-controller-admission-link")?;
    let reservation = ControllerReservationId::for_attempt(declaration.account(), &attempt)?;
    let envelope = InvocationAdmissionEnvelope::not_applicable();
    let mut candidate = state.clone();
    let outcome = candidate.admit(
        reservation.clone(),
        attempt.clone(),
        CapabilityCategory::Tool,
        &envelope,
    )?;
    let action = ControllerAccountAction::AdmitEntry {
        account: declaration.account().clone(),
        reservation: reservation.clone(),
        attempt: attempt.clone(),
        category: CapabilityCategory::Tool,
        envelope: envelope.clone(),
        expected_outcome: outcome.clone(),
    };

    let unlinked = request(
        &child,
        "command-controller-admission-unlinked",
        "event-controller-admission-unlinked",
        RunSequence::ZERO,
        RunEventKind::RunStarted,
    )?
    .with_controller_account_transaction(transaction(
        "transition-controller-admission-unlinked",
        Some((&state, &declaration)),
        vec![action.clone()],
    )?)?;
    assert!(matches!(
        store.commit_command(&unlinked),
        Err(PersistenceError::InvalidDocument(_))
    ));
    assert_eq!(
        store
            .controller_account(declaration.account())?
            .ok_or("controller account disappeared")?,
        state
    );

    let duplicate = request(
        &child,
        "command-controller-admission-duplicate",
        "event-controller-admission-duplicate",
        RunSequence::ZERO,
        RunEventKind::CapabilityAdapterEntryDecisionRecorded {
            attempt: attempt.clone(),
            authorization: decision(true, "admission-duplicate")?,
            controller_admission: outcome.clone(),
        },
    )?
    .with_controller_account_transaction(transaction(
        "transition-controller-admission-duplicate",
        Some((&state, &declaration)),
        vec![action.clone(), action.clone()],
    )?)?;
    assert!(matches!(
        store.commit_command(&duplicate),
        Err(PersistenceError::InvalidDocument(_))
    ));

    let unterminated_reservation = request_many_with_workspace(
        &child,
        "command-controller-admission-unterminated-reservation",
        "event-controller-admission-unterminated-reservation",
        RunSequence::ZERO,
        vec![
            RunEventKind::CapabilityAdapterEntryDecisionRecorded {
                attempt: attempt.clone(),
                authorization: decision(true, "admission-unterminated-reservation")?,
                controller_admission: outcome.clone(),
            },
            RunEventKind::NodeTerminal {
                execution: NodeExecutionId::new("execution-admission-unterminated-reservation")?,
                attempt: attempt.clone(),
                report_sequence: 1,
                outcome: NodeOutcome::Succeeded,
                error_class: None,
                detail: None,
            },
        ],
        Vec::new(),
    )?
    .with_controller_account_transaction(transaction(
        "transition-controller-admission-unterminated-reservation",
        Some((&state, &declaration)),
        vec![action.clone()],
    )?)?;
    assert!(matches!(
        store.commit_command(&unterminated_reservation),
        Err(PersistenceError::InvalidDocument(_))
    ));

    let late_without_reservation = request(
        &child,
        "command-controller-admission-late-without-reservation",
        "event-controller-admission-late-without-reservation",
        RunSequence::ZERO,
        RunEventKind::LateTerminalEvidenceRecorded {
            attempt: AttemptId::new("attempt-controller-admission-late-without-reservation")?,
            worker: WorkerId::new("worker-controller-admission-late-without-reservation")?,
            report_sequence: 1,
            terminal: InvocationTerminal::new(
                TerminalStatus::Success,
                Vec::new(),
                None,
                None,
                SideEffectClass::None,
            )?,
        },
    )?;
    assert!(matches!(
        store.commit_command(&late_without_reservation),
        Err(PersistenceError::InvalidDocument(_))
    ));

    let forged_event = request(
        &child,
        "command-controller-admission-forged-event",
        "event-controller-admission-forged-event",
        RunSequence::ZERO,
        RunEventKind::CapabilityAdapterEntryDecisionRecorded {
            attempt: attempt.clone(),
            authorization: decision(true, "admission-forged-event")?,
            controller_admission: ControllerAdmissionOutcome::Denied {
                account: declaration.account().clone(),
                reason: milkdrift_persistence::ControllerAdmissionDenial::Limit {
                    dimension: "process_admissions".to_owned(),
                },
            },
        },
    )?
    .with_controller_account_transaction(transaction(
        "transition-controller-admission-forged-event",
        Some((&state, &declaration)),
        vec![action],
    )?)?;
    assert!(matches!(
        store.commit_command(&forged_event),
        Err(PersistenceError::InvalidDocument(_))
    ));

    let action_attempt = AttemptId::new("attempt-controller-admission-other-action")?;
    let action_reservation =
        ControllerReservationId::for_attempt(declaration.account(), &action_attempt)?;
    let mut action_candidate = state.clone();
    let action_outcome = action_candidate.admit(
        action_reservation.clone(),
        action_attempt.clone(),
        CapabilityCategory::Tool,
        &envelope,
    )?;
    let mismatched_attempt = request(
        &child,
        "command-controller-admission-mismatched-attempt",
        "event-controller-admission-mismatched-attempt",
        RunSequence::ZERO,
        RunEventKind::CapabilityAdapterEntryDecisionRecorded {
            attempt,
            authorization: decision(true, "admission-mismatched-attempt")?,
            controller_admission: action_outcome.clone(),
        },
    )?
    .with_controller_account_transaction(transaction(
        "transition-controller-admission-mismatched-attempt",
        Some((&state, &declaration)),
        vec![ControllerAccountAction::AdmitEntry {
            account: declaration.account().clone(),
            reservation: action_reservation,
            attempt: action_attempt,
            category: CapabilityCategory::Tool,
            envelope,
            expected_outcome: action_outcome,
        }],
    )?)?;
    assert!(matches!(
        store.commit_command(&mismatched_attempt),
        Err(PersistenceError::InvalidDocument(_))
    ));

    assert_eq!(
        store
            .controller_account(declaration.account())?
            .ok_or("controller account disappeared")?,
        state
    );
    Ok(())
}

#[test]
fn final_entry_integrity_distinguishes_denied_uncontrolled_and_reserved_links() -> TestResult {
    let directory = TempDir::new()?;
    let owner = RunId::new("run-controller-event-link-owner")?;
    let allowed_child = RunId::new("run-controller-event-link-allowed-child")?;
    let denied_child = RunId::new("run-controller-event-link-denied-child")?;
    let reserved_child = RunId::new("run-controller-event-link-reserved-child")?;
    let foreign_owner = RunId::new("run-controller-event-link-foreign-owner")?;
    let (origin, foreign) = {
        let store = RedbStore::open(directory.path())?;
        let origin = establish(&store, &owner, "event-link-owner")?;
        let foreign = establish(&store, &foreign_owner, "event-link-foreign")?;

        let allowed_start = request(
            &allowed_child,
            "command-allowed-child-start",
            "event-allowed-child-start",
            RunSequence::ZERO,
            RunEventKind::RunStarted,
        )?;
        let _ = store.commit_command(&allowed_start)?;
        let allowed = request(
            &allowed_child,
            "command-allowed-uncontrolled",
            "event-allowed-uncontrolled",
            RunSequence::FIRST,
            RunEventKind::CapabilityAdapterEntryDecisionRecorded {
                attempt: AttemptId::new("attempt-allowed-uncontrolled")?,
                authorization: decision(true, "allowed-uncontrolled")?,
                controller_admission: ControllerAdmissionOutcome::NotControlled,
            },
        )?;
        let _ = store.commit_command(&allowed)?;
        bind_child(&store, &denied_child, &origin, "denied-child")?;
        let denied = request(
            &denied_child,
            "command-denied-uncontrolled",
            "event-denied-uncontrolled",
            RunSequence::ZERO,
            RunEventKind::CapabilityAdapterEntryDecisionRecorded {
                attempt: AttemptId::new("attempt-denied-uncontrolled")?,
                authorization: decision(false, "denied-uncontrolled")?,
                controller_admission: ControllerAdmissionOutcome::NotControlled,
            },
        )?;
        let _ = store.commit_command(&denied)?;

        bind_child(&store, &reserved_child, &origin, "reserved-child")?;
        let state = store
            .controller_account(origin.account())?
            .ok_or("originating controller account is absent")?;
        let attempt = AttemptId::new("attempt-reserved-link")?;
        let reservation = ControllerReservationId::for_attempt(origin.account(), &attempt)?;
        let reservation_envelope = InvocationAdmissionEnvelope::new(
            AdmissionBound::Bounded(8),
            AdmissionBound::NotApplicable,
            AdmissionBound::NotApplicable,
            AdmissionBound::NotApplicable,
        );
        let mut candidate = state.clone();
        let outcome = candidate.admit(
            reservation.clone(),
            attempt.clone(),
            CapabilityCategory::Process,
            &reservation_envelope,
        )?;
        let reserved = request(
            &reserved_child,
            "command-reserved-link",
            "event-reserved-link",
            RunSequence::ZERO,
            RunEventKind::CapabilityAdapterEntryDecisionRecorded {
                attempt: attempt.clone(),
                authorization: decision(true, "reserved-link")?,
                controller_admission: outcome.clone(),
            },
        )?
        .with_controller_account_transaction(transaction(
            "transition-reserved-link",
            Some((&state, &origin)),
            vec![ControllerAccountAction::AdmitEntry {
                account: origin.account().clone(),
                reservation: reservation.clone(),
                attempt: attempt.clone(),
                category: CapabilityCategory::Process,
                envelope: reservation_envelope.clone(),
                expected_outcome: outcome,
            }],
        )?)?;
        let _ = store.commit_command(&reserved)?;
        let missing_settlement = request(
            &reserved_child,
            "command-reserved-link-missing-settlement",
            "event-reserved-link-missing-settlement",
            RunSequence::FIRST,
            RunEventKind::NodeTerminal {
                execution: NodeExecutionId::new("execution-reserved-link")?,
                attempt: attempt.clone(),
                report_sequence: 1,
                outcome: NodeOutcome::Succeeded,
                error_class: None,
                detail: None,
            },
        )?;
        assert!(matches!(
            store.commit_command(&missing_settlement),
            Err(PersistenceError::InvalidDocument(_))
        ));
        let missing_late_settlement = request(
            &reserved_child,
            "command-reserved-link-missing-late-settlement",
            "event-reserved-link-missing-late-settlement",
            RunSequence::FIRST,
            RunEventKind::LateTerminalEvidenceRecorded {
                attempt: attempt.clone(),
                worker: WorkerId::new("worker-reserved-link-late")?,
                report_sequence: 2,
                terminal: InvocationTerminal::new(
                    TerminalStatus::Success,
                    Vec::new(),
                    None,
                    None,
                    SideEffectClass::None,
                )?,
            },
        )?;
        assert!(matches!(
            store.commit_command(&missing_late_settlement),
            Err(PersistenceError::InvalidDocument(_))
        ));
        let admitted = store
            .controller_account(origin.account())?
            .ok_or("originating controller account disappeared before settlement")?;
        let usage = AttemptUsage {
            input_units: Some(3),
            output_units: None,
            duration_ms: Some(4),
            cost: None,
        };
        let cross_attempt_usage = request_many_with_workspace(
            &reserved_child,
            "command-reserved-link-cross-attempt-usage",
            "event-reserved-link-cross-attempt-usage",
            RunSequence::FIRST,
            vec![
                RunEventKind::AttemptUsageRecorded {
                    attempt: AttemptId::new("attempt-reserved-link-other-usage")?,
                    usage: usage.clone(),
                },
                RunEventKind::NodeTerminal {
                    execution: NodeExecutionId::new("execution-reserved-link-cross-usage")?,
                    attempt: attempt.clone(),
                    report_sequence: 3,
                    outcome: NodeOutcome::Succeeded,
                    error_class: None,
                    detail: None,
                },
            ],
            Vec::new(),
        )?
        .with_controller_account_transaction(transaction(
            "transition-reserved-link-cross-attempt-usage",
            Some((&admitted, &origin)),
            vec![ControllerAccountAction::SettleTerminal {
                account: origin.account().clone(),
                reservation: reservation.clone(),
                usage: Some(usage.clone()),
            }],
        )?)?;
        assert!(matches!(
            store.commit_command(&cross_attempt_usage),
            Err(PersistenceError::InvalidDocument(_))
        ));
        let terminal = request_many_with_workspace(
            &reserved_child,
            "command-reserved-link-terminal",
            "event-reserved-link-terminal",
            RunSequence::FIRST,
            vec![
                RunEventKind::AttemptUsageRecorded {
                    attempt: attempt.clone(),
                    usage: usage.clone(),
                },
                RunEventKind::NodeTerminal {
                    execution: NodeExecutionId::new("execution-reserved-link")?,
                    attempt: attempt.clone(),
                    report_sequence: 3,
                    outcome: NodeOutcome::Succeeded,
                    error_class: None,
                    detail: None,
                },
            ],
            Vec::new(),
        )?
        .with_controller_account_transaction(transaction(
            "transition-reserved-link-terminal",
            Some((&admitted, &origin)),
            vec![ControllerAccountAction::SettleTerminal {
                account: origin.account().clone(),
                reservation: reservation.clone(),
                usage: Some(usage),
            }],
        )?)?;
        let _ = store.commit_command(&terminal)?;
        let settled = store
            .controller_account(origin.account())?
            .ok_or("originating controller account disappeared after settlement")?;
        assert_eq!(settled.settled().input_units(), 3);
        assert!(!settled.reservations().contains_key(&reservation));

        let late_attempt = AttemptId::new("attempt-reserved-link-late-positive")?;
        let late_reservation =
            ControllerReservationId::for_attempt(origin.account(), &late_attempt)?;
        let mut late_candidate = settled.clone();
        let late_outcome = late_candidate.admit(
            late_reservation.clone(),
            late_attempt.clone(),
            CapabilityCategory::Tool,
            &reservation_envelope,
        )?;
        let late_entry = request(
            &reserved_child,
            "command-reserved-link-late-entry",
            "event-reserved-link-late-entry",
            RunSequence::new(3),
            RunEventKind::CapabilityAdapterEntryDecisionRecorded {
                attempt: late_attempt.clone(),
                authorization: decision(true, "reserved-link-late-entry")?,
                controller_admission: late_outcome.clone(),
            },
        )?
        .with_controller_account_transaction(transaction(
            "transition-reserved-link-late-entry",
            Some((&settled, &origin)),
            vec![ControllerAccountAction::AdmitEntry {
                account: origin.account().clone(),
                reservation: late_reservation.clone(),
                attempt: late_attempt.clone(),
                category: CapabilityCategory::Tool,
                envelope: reservation_envelope,
                expected_outcome: late_outcome,
            }],
        )?)?;
        let _ = store.commit_command(&late_entry)?;
        let late_admitted = store
            .controller_account(origin.account())?
            .ok_or("originating controller account disappeared before late settlement")?;
        let late_usage = AttemptUsage {
            input_units: Some(2),
            output_units: None,
            duration_ms: Some(5),
            cost: None,
        };
        let late_terminal = request(
            &reserved_child,
            "command-reserved-link-late-terminal",
            "event-reserved-link-late-terminal",
            RunSequence::new(4),
            RunEventKind::LateTerminalEvidenceRecorded {
                attempt: late_attempt,
                worker: WorkerId::new("worker-reserved-link-late-positive")?,
                report_sequence: 4,
                terminal: InvocationTerminal::new(
                    TerminalStatus::Success,
                    Vec::new(),
                    None,
                    Some(UsageObservation::new(
                        Some(2),
                        None,
                        Some(5),
                        None,
                        None,
                        std::collections::BTreeMap::new(),
                    )?),
                    SideEffectClass::None,
                )?,
            },
        )?
        .with_controller_account_transaction(transaction(
            "transition-reserved-link-late-terminal",
            Some((&late_admitted, &origin)),
            vec![ControllerAccountAction::SettleTerminal {
                account: origin.account().clone(),
                reservation: late_reservation.clone(),
                usage: Some(late_usage),
            }],
        )?)?;
        let _ = store.commit_command(&late_terminal)?;
        let late_settled = store
            .controller_account(origin.account())?
            .ok_or("originating controller account disappeared after late settlement")?;
        assert_eq!(late_settled.settled().input_units(), 5);
        assert!(!late_settled.reservations().contains_key(&late_reservation));
        bind_child(&store, &allowed_child, &origin, "late-bind-allowed-child")?;
        assert!(has_integrity_failure(&store)?);
        (origin, foreign)
    };

    let database = Database::open(directory.path().join("milkdrift.redb"))?;
    let write = database.begin_write()?;
    {
        let mut bindings = write.open_table(BINDINGS)?;
        bindings.insert(reserved_child.as_str(), foreign.account().as_str())?;
    }
    write.commit()?;
    drop(database);

    let store = RedbStore::open(directory.path())?;
    assert_eq!(
        store.controller_account_binding(&denied_child)?.as_ref(),
        Some(origin.account())
    );
    assert!(has_integrity_failure(&store)?);
    Ok(())
}
