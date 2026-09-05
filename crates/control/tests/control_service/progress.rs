use super::*;

#[test]
fn installed_runtime_assesses_and_stops_a_controller_at_exact_cycle_bound() -> TestResult {
    let directory = TempDir::new()?;
    let store = Arc::new(RedbStore::open(directory.path().join("controller.redb"))?);
    let run = RunId::new("run-controller-lifecycle")?;
    let actor = ActorRef::new("controller:bounded")?;
    let grant_id = GrantId::new("grant:controller-bounded")?;
    let (runtime, service, context) = services(
        store.clone(),
        &actor,
        &run,
        &grant_id,
        "controller-lifecycle",
    )?;

    let body_terminal = Node::new(
        NodeId::new("cycle-complete")?,
        NodeKind::Terminal {
            outcome: TerminalOutcome::Success,
        },
    )?;
    let body = BlueprintRevision::genesis(
        WorkflowId::new("controller-cycle-body")?,
        MutationBatch::new(vec![Mutation::AddNode {
            node: body_terminal,
        }])?,
        AuthorRef::new("human:controller-test")?,
        "controller cycle body",
    )?;
    store.put_revision(&body)?;
    let limits = ControllerLimits::new(
        2, 2, 8, 4, 60_000, 1_000_000, 10_000, 10_000, 1_000_000, 2, 2, 2, 2, 2, 2, None,
    )?;
    let wrapper = build_controller_blueprint(ControllerBlueprintSpec {
        workflow: WorkflowId::new("controller-wrapper")?,
        body: PinnedSubworkflow::new(
            body.semantic().workflow().clone(),
            body.id().clone(),
            WorkflowInterface::new([], [])?,
        ),
        continue_condition: Condition::Constant { value: true },
        limits,
        author: AuthorRef::new("human:controller-test")?,
    })?;
    store.put_revision(&wrapper)?;
    create_and_start(&service, &runtime, &context, &run, &wrapper)?;

    for _ in 0..64 {
        runtime_tick(&runtime)?;
        if runtime.projection(&run)?.lifecycle().is_completed() {
            break;
        }
    }
    let projection = runtime.projection(&run)?;
    assert_eq!(
        projection.lifecycle(),
        RunLifecycle::Terminal(RunOutcome::Failed)
    );
    let history = runtime.history(&run)?;
    assert_eq!(
        history
            .iter()
            .filter(|event| matches!(event.kind(), RunEventKind::RepeatIterationCreated { .. }))
            .count(),
        2
    );
    assert!(history.iter().any(|event| {
        matches!(
            event.kind(),
            RunEventKind::ControllerAssessmentRecorded {
                outcome: ControllerAssessmentOutcome::BoundReached { bound, .. },
                ..
            } if bound == "invocations"
        )
    }));
    let controller_execution = projection
        .controller_assessments()
        .keys()
        .next()
        .cloned()
        .ok_or("bound assessment is absent after terminal compaction")?;
    let status = service.execute(&command(
        "inspect-controller-after-bound",
        &context,
        OptimisticGuard::default(),
        ControlCommand::InspectController {
            run: run.clone(),
            controller_execution,
        },
    )?)?;
    assert!(matches!(
        status,
        ControlResult::ControllerStatus { value }
            if value.state == milkdrift_control::ControllerLifecycleState::BoundReached
                && value.reached_bound == Some(milkdrift_control::ControllerBound::Invocations)
                && value.progress.invocations == 2
                && !value.cycle_eligible
    ));
    Ok(())
}

#[test]
fn controller_progress_preserves_every_durable_counter_and_reassesses_matching_proposals()
-> TestResult {
    let directory = TempDir::new()?;
    let store = Arc::new(RedbStore::open(
        directory.path().join("controller-progress.redb"),
    )?);
    let run = RunId::new("run-controller-progress")?;
    let actor = ActorRef::new("controller:progress")?;
    let grant_id = GrantId::new("grant:controller-progress")?;
    let (runtime, service, context) = services(
        store.clone(),
        &actor,
        &run,
        &grant_id,
        "controller-progress",
    )?;
    let body = base_revision("controller-progress-body")?;
    store.put_revision(&body)?;
    let wrapper = build_controller_blueprint(ControllerBlueprintSpec {
        workflow: WorkflowId::new("controller-progress-wrapper")?,
        body: PinnedSubworkflow::new(
            body.semantic().workflow().clone(),
            body.id().clone(),
            WorkflowInterface::new([], [])?,
        ),
        continue_condition: Condition::Constant { value: true },
        limits: ControllerLimits::new(
            2, 64, 8, 4, 60_000, 1_000_000, 10_000, 10_000, 1_000_000, 64, 64, 64, 64, 2, 2, None,
        )?,
        author: AuthorRef::new("human:controller-progress-test")?,
    })?;
    store.put_revision(&wrapper)?;
    create_and_start(&service, &runtime, &context, &run, &wrapper)?;

    let projection = runtime.projection(&run)?;
    let controller_execution = projection
        .node_executions()
        .values()
        .find(|execution| execution.node().as_str() == "controller-repeat")
        .map(|execution| execution.execution().clone())
        .ok_or("active controller execution is absent")?;
    let mut value = serde_json::to_value(&projection)?;
    value["subworkflow_usage_by_execution"] = serde_json::json!([[
        serde_json::to_value(&controller_execution)?,
        {
            "completed_children": 3,
            "failed_children": 2,
            "cost_micros": [["USD", 700]],
            "overflowed": false,
            "input_units": 11,
            "output_units": 13,
            "artifact_bytes": 17,
            "process_invocations": 19,
            "model_invocations": 23,
            "unknown_input_usage": 29,
            "unknown_output_usage": 31,
            "unknown_cost_usage": 37
        }
    ]]);
    value["run_actor_revision_requests"] = serde_json::json!(41);
    value["run_actor_rejections"] = serde_json::json!(43);
    let projection: milkdrift_runtime::RunProjection = serde_json::from_value(value)?;
    let document =
        ControllerPolicyDocument::from_revision(&wrapper, &NodeId::new("controller-repeat")?)?
            .ok_or("controller policy document is absent")?;
    let limits = document.policy().limits();
    let mut account = ControllerAccountState::establish(ControllerAccountDeclaration::new(
        run.clone(),
        controller_execution.clone(),
        document.digest().as_str(),
        ControllerResourceBudget::new(
            limits.max_cost_micros(),
            CurrencyCode::new(document.policy().cost_currency().as_str())?,
            limits.max_input_units(),
            limits.max_output_units(),
            limits.max_artifact_bytes(),
            u64::from(limits.max_process_invocations()),
            u64::from(limits.max_model_invocations()),
        )?,
    )?)?;
    let attempt = milkdrift_persistence::AttemptId::new("attempt-controller-progress-account")?;
    let reservation =
        ControllerReservationId::for_attempt(account.declaration().account(), &attempt)?;
    assert!(matches!(
        account.admit(
            reservation.clone(),
            attempt,
            CapabilityCategory::Process,
            &InvocationAdmissionEnvelope::new(
                AdmissionBound::Bounded(11),
                AdmissionBound::Bounded(13),
                AdmissionBound::Bounded(17),
                AdmissionBound::Bounded(AdmissionMonetaryBound::new(700, "USD")?),
            ),
        )?,
        ControllerAdmissionOutcome::Reserved { .. }
    ));
    account.charge_artifact(Some(&reservation), 17)?;
    let model_attempt = milkdrift_persistence::AttemptId::new("attempt-controller-progress-model")?;
    let model_reservation =
        ControllerReservationId::for_attempt(account.declaration().account(), &model_attempt)?;
    assert!(matches!(
        account.admit(
            model_reservation,
            model_attempt,
            CapabilityCategory::Model,
            &InvocationAdmissionEnvelope::not_applicable(),
        )?,
        ControllerAdmissionOutcome::Reserved { .. }
    ));
    account.settle_terminal(&reservation, None)?;
    let progress = service.controller_lifecycle_owner().progress(
        &document,
        &projection,
        &controller_execution,
        Some(&account),
        NOW + 47,
    )?;
    assert_eq!(progress.invocations, 3);
    assert_eq!(progress.elapsed_ms, 47);
    assert_eq!(progress.cost_micros, 700);
    assert_eq!(progress.input_units, 11);
    assert_eq!(progress.output_units, 13);
    assert_eq!(progress.artifact_bytes, 17);
    assert_eq!(progress.process_invocations, 1);
    assert_eq!(progress.model_invocations, 1);
    assert_eq!(progress.failures, 2);
    assert_eq!(progress.revisions, 41);
    assert_eq!(progress.rejections, 43);
    assert_eq!(progress.unknown_input_observations, 1);
    assert_eq!(progress.unknown_output_observations, 0);
    assert_eq!(progress.unknown_cost_observations, 0);
    assert!(matches!(
        progress.account_block.as_ref(),
        Some(milkdrift_persistence::ControllerAccountBlock::UnknownUsage {
            dimension,
            reservation: blocked_reservation,
        }) if dimension == "input_units" && blocked_reservation == &reservation
    ));

    for (suffix, output_bound, cost_bound, expected) in [
        (
            "output",
            AdmissionBound::Bounded(13),
            AdmissionBound::NotApplicable,
            [0, 1, 0],
        ),
        (
            "cost",
            AdmissionBound::NotApplicable,
            AdmissionBound::Bounded(AdmissionMonetaryBound::new(700, "USD")?),
            [0, 0, 1],
        ),
    ] {
        let mut missing_usage = ControllerAccountState::establish(account.declaration().clone())?;
        let attempt =
            milkdrift_persistence::AttemptId::new(format!("attempt-controller-progress-{suffix}"))?;
        let reservation =
            ControllerReservationId::for_attempt(missing_usage.declaration().account(), &attempt)?;
        let _ = missing_usage.admit(
            reservation.clone(),
            attempt,
            CapabilityCategory::Model,
            &InvocationAdmissionEnvelope::new(
                AdmissionBound::NotApplicable,
                output_bound,
                AdmissionBound::NotApplicable,
                cost_bound,
            ),
        )?;
        missing_usage.settle_terminal(&reservation, None)?;
        let progress = service.controller_lifecycle_owner().progress(
            &document,
            &projection,
            &controller_execution,
            Some(&missing_usage),
            NOW + 47,
        )?;
        assert_eq!(
            [
                progress.unknown_input_observations,
                progress.unknown_output_observations,
                progress.unknown_cost_observations
            ],
            expected,
            "{suffix}"
        );
    }

    let controller_node = wrapper
        .semantic()
        .nodes()
        .get(&NodeId::new("controller-repeat")?)
        .ok_or("controller repeat node is absent")?;
    let assessment_context = |account| ControllerAssessmentContext {
        run: &run,
        revision: &wrapper,
        node: controller_node,
        execution: &controller_execution,
        projection: &projection,
        account,
        observed_at: TimestampMillis::new(NOW + 47),
        boundary: ControllerAssessmentBoundary::CycleEntry,
        next_cycle: Some(2),
    };
    let assessment = service
        .controller_lifecycle_owner()
        .assess(&assessment_context(Some(&account)))?
        .ok_or("controller account block did not produce an assessment")?;
    assert!(matches!(
        assessment.outcome,
        ControllerAssessmentOutcome::BoundReached {
            ref bound,
            current: None,
            unknown_usage: true,
            ..
        } if bound == "input_units"
    ));

    let mut contract_account = ControllerAccountState::establish(account.declaration().clone())?;
    let contract_attempt =
        milkdrift_persistence::AttemptId::new("attempt-controller-progress-contract")?;
    let contract_reservation = ControllerReservationId::for_attempt(
        contract_account.declaration().account(),
        &contract_attempt,
    )?;
    let _ = contract_account.admit(
        contract_reservation.clone(),
        contract_attempt,
        CapabilityCategory::Process,
        &InvocationAdmissionEnvelope::new(
            AdmissionBound::Bounded(5),
            AdmissionBound::NotApplicable,
            AdmissionBound::NotApplicable,
            AdmissionBound::NotApplicable,
        ),
    )?;
    contract_account.settle_terminal(
        &contract_reservation,
        Some(&AttemptUsage {
            input_units: Some(6),
            output_units: None,
            duration_ms: None,
            cost: None,
        }),
    )?;
    let contract_assessment = service
        .controller_lifecycle_owner()
        .assess(&assessment_context(Some(&contract_account)))?
        .ok_or("controller contract violation did not produce an assessment")?;
    assert!(matches!(
        contract_assessment.outcome,
        ControllerAssessmentOutcome::BoundReached {
            ref bound,
            current: Some(6),
            limit: 5,
            unknown_usage: false,
        } if bound == "contract_violation.input_units"
    ));

    let mut integrity_account = ControllerAccountState::establish(account.declaration().clone())?;
    let integrity_attempt =
        milkdrift_persistence::AttemptId::new("attempt-controller-progress-integrity")?;
    let integrity_reservation = ControllerReservationId::for_attempt(
        integrity_account.declaration().account(),
        &integrity_attempt,
    )?;
    let _ = integrity_account.admit(
        integrity_reservation.clone(),
        integrity_attempt,
        CapabilityCategory::Model,
        &InvocationAdmissionEnvelope::new(
            AdmissionBound::NotApplicable,
            AdmissionBound::NotApplicable,
            AdmissionBound::NotApplicable,
            AdmissionBound::Bounded(AdmissionMonetaryBound::new(5, "USD")?),
        ),
    )?;
    integrity_account.settle_terminal(
        &integrity_reservation,
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
    let integrity_assessment = service
        .controller_lifecycle_owner()
        .assess(&assessment_context(Some(&integrity_account)))?
        .ok_or("controller integrity block did not produce an assessment")?;
    assert!(matches!(
        integrity_assessment.outcome,
        ControllerAssessmentOutcome::BoundReached {
            ref bound,
            current: None,
            limit: 1,
            unknown_usage: false,
        } if bound == "account_integrity"
    ));
    let foreign = ControllerAccountState::establish(ControllerAccountDeclaration::new(
        run.clone(),
        milkdrift_persistence::NodeExecutionId::new("foreign-controller-execution")?,
        document.digest().as_str(),
        account.declaration().budget().clone(),
    )?)?;
    assert!(matches!(
        service
            .controller_lifecycle_owner()
            .assess(&assessment_context(Some(&foreign))),
        Err(RuntimeError::InvalidHistory(reason))
            if reason.contains("does not match the originating controller occurrence")
    ));
    assert!(matches!(
        service
            .controller_lifecycle_owner()
            .assess(&assessment_context(None)),
        Err(RuntimeError::InvalidHistory(reason))
            if reason.contains("no exact durable account binding")
    ));
    let activation_without_account = ControllerAssessmentContext {
        boundary: ControllerAssessmentBoundary::Activation,
        next_cycle: Some(1),
        ..assessment_context(None)
    };
    assert!(matches!(
        service
            .controller_lifecycle_owner()
            .assess(&activation_without_account),
        Err(RuntimeError::InvalidHistory(reason))
            if reason.contains("no exact durable account binding")
    ));

    let proposed = wrapper.revise(
        wrapper.id(),
        MutationBatch::new(vec![Mutation::SetMetadata {
            metadata: BlueprintMetadata::new(
                "controller-progress-proposal",
                "proposal transition cumulative-bound fixture",
                BTreeSet::from(["controller-progress".to_owned()]),
                BTreeMap::new(),
            )?,
        }])?,
        AuthorRef::new("controller:progress")?,
        format!("proposer={actor};source=controller-progress-test"),
    )?;
    assert!(matches!(
        service.controller_lifecycle_owner().assess_proposal_transition(
            &run,
            &projection,
            &proposed,
            ControllerAssessmentBoundary::ProposalApproval,
            Some(&account),
            NOW + 47,
        ),
        Err(ControlError::Bounds { location, .. })
            if location == "controller.proposal.input_units"
    ));
    Ok(())
}
