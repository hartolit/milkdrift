//! Authorized external command application through the runtime, control, layout, and revision owners.

use super::*;

impl Owner {
    pub(super) fn execute_new_command(
        &mut self,
        session: &ActorSession,
        request: &CommandRequest,
    ) -> Result<CommandAccepted, PublicFailure> {
        match &request.command {
            Command::ImportBlueprint { document } => {
                let bytes =
                    serde_json::to_vec(document).map_err(|_| invalid("invalid blueprint JSON"))?;
                let (_document, revision) = BlueprintRevisionDocument::from_json(&bytes)
                    .map_err(|error| invalid(&bounded(&error.to_string())))?;
                let mut resources = RequestedResourceFacts::empty();
                resources.workflow = Some(revision.semantic().workflow().clone());
                resources.revision = Some(revision.id().clone());
                let decision = self.authorize(
                    session,
                    AuthorityOperation::ImportBlueprint,
                    resources,
                    "command:import-blueprint",
                )?;
                let outcome = self
                    .store
                    .put_revision(&revision)
                    .map_err(public_persistence)?;
                self.record_security_decision(&decision)?;
                Ok(CommandAccepted {
                    command_id: request.command_id.clone(),
                    replayed: matches!(
                        outcome,
                        milkdrift_persistence::ImmutableRevisionPut::AlreadyPresent
                    ),
                    resulting_sequence: None,
                    result_type: "blueprint_imported".to_owned(),
                    value: json!({
                        "revision_id": revision.id().as_str(),
                        "workflow_id": revision.semantic().workflow().as_str(),
                        "semantic_digest": revision.content_digest().as_str(),
                    }),
                })
            }
            Command::ValidateBlueprint { document } => {
                let bytes =
                    serde_json::to_vec(document).map_err(|_| invalid("invalid blueprint JSON"))?;
                let (_document, revision) = BlueprintRevisionDocument::from_json(&bytes)
                    .map_err(|error| invalid(&bounded(&error.to_string())))?;
                let mut resources = RequestedResourceFacts::empty();
                resources.workflow = Some(revision.semantic().workflow().clone());
                resources.revision = Some(revision.id().clone());
                self.authorize(
                    session,
                    AuthorityOperation::ValidateBlueprint,
                    resources,
                    "command:validate-blueprint",
                )?;
                Ok(CommandAccepted {
                    command_id: request.command_id.clone(),
                    replayed: false,
                    resulting_sequence: None,
                    result_type: "blueprint_valid".to_owned(),
                    value: json!({"revision_id": revision.id().as_str(), "semantic_digest": revision.content_digest().as_str()}),
                })
            }
            Command::StartRun {
                run_id,
                workflow_id,
                revision_id,
            } => {
                let run =
                    RunId::new(run_id.clone()).map_err(|error| invalid(&error.to_string()))?;
                let workflow = WorkflowId::new(workflow_id.clone())
                    .map_err(|error| invalid(&error.to_string()))?;
                let revision = parse_revision_id(revision_id)?;
                let root_scope = WorkspaceScope::run_root(
                    run.clone(),
                    ScopeId::new("root").map_err(|error| invalid(&error.to_string()))?,
                );
                let create = ControlCommand::CreateRun {
                    run: run.clone(),
                    workflow,
                    revision,
                    root_scope,
                    workspace_budget: default_workspace_budget()
                        .map_err(|error| invalid(&error.to_string()))?,
                    inputs: Vec::new(),
                };
                let create_sequence =
                    self.execute_control(session, request, Some(0), create, "create")?;
                let start = ControlCommand::StartRun { run };
                let sequence =
                    self.execute_control(session, request, Some(create_sequence), start, "start")?;
                accepted_sequence(request, sequence, "run_started")
            }
            Command::PauseRun { run_id } => {
                self.simple_run_command(session, request, run_id, "pause", |run| {
                    ControlCommand::PauseRun { run }
                })
            }
            Command::ResumeRun { run_id } => {
                self.simple_run_command(session, request, run_id, "resume", |run| {
                    ControlCommand::ResumeRun { run }
                })
            }
            Command::CancelRun { run_id } => {
                self.simple_run_command(session, request, run_id, "cancel", |run| {
                    ControlCommand::RequestCancellation { run }
                })
            }
            Command::SignalRun {
                run_id,
                signal_id,
                signal_type,
                correlation,
                broadcast,
                payload,
            } => {
                let run =
                    RunId::new(run_id.clone()).map_err(|error| invalid(&error.to_string()))?;
                let command = ControlCommand::Signal {
                    run,
                    signal: SignalId::new(signal_id.clone())
                        .map_err(|error| invalid(&error.to_string()))?,
                    signal_type: SignalTypeId::new(signal_type.clone())
                        .map_err(|error| invalid(&error.to_string()))?,
                    correlation: correlation
                        .as_ref()
                        .map(|value| CorrelationKey::new(value.clone()))
                        .transpose()
                        .map_err(|error| invalid(&error.to_string()))?,
                    mode: if *broadcast {
                        SignalDeliveryMode::Broadcast
                    } else {
                        SignalDeliveryMode::OneShot
                    },
                    payload: milkdrift_capability::BoundedJson::new(payload.clone())
                        .map_err(|error| invalid(&error.to_string()))?,
                };
                let sequence = self.execute_control(
                    session,
                    request,
                    request.expected_sequence,
                    command,
                    "signal",
                )?;
                accepted_sequence(request, sequence, "signal_delivered")
            }
            Command::ResolveWork {
                run_id,
                attempt_id,
                decision_id,
                action,
                remediation_node,
            } => {
                let run =
                    RunId::new(run_id.clone()).map_err(|error| invalid(&error.to_string()))?;
                let command = ControlCommand::ResolveExternalWork {
                    run,
                    attempt: AttemptId::new(attempt_id.clone())
                        .map_err(|error| invalid(&error.to_string()))?,
                    decision: ReconciliationDecisionId::new(decision_id.clone())
                        .map_err(|error| invalid(&error.to_string()))?,
                    action: map_resolve(*action),
                    remediation_node: remediation_node
                        .as_ref()
                        .map(|value| milkdrift_blueprint::NodeId::new(value.clone()))
                        .transpose()
                        .map_err(|error| invalid(&error.to_string()))?,
                };
                let sequence = self.execute_control(
                    session,
                    request,
                    request.expected_sequence,
                    command,
                    "resolve",
                )?;
                accepted_sequence(request, sequence, "external_work_resolved")
            }
            Command::SubmitProposal { document } => {
                let bytes =
                    serde_json::to_vec(document).map_err(|_| invalid("invalid proposal JSON"))?;
                let proposal = WorkflowProposalDocument::from_json(&bytes)
                    .map_err(|error| invalid(&bounded(&error.to_string())))?;
                let digest = proposal.proposal().digest().clone();
                let command = ControlCommand::SubmitProposal { proposal };
                let value = self.execute_control_result(
                    session,
                    request,
                    request.expected_sequence,
                    Some(digest),
                    command,
                    "proposal",
                )?;
                match value {
                    ControlResult::ProposalSubmitted { value } => Ok(CommandAccepted {
                        command_id: request.command_id.clone(),
                        replayed: false,
                        resulting_sequence: value
                            .reconciliation
                            .as_ref()
                            .and_then(|item| item.applied_sequence)
                            .map(|sequence| sequence.get()),
                        result_type: "proposal_submitted".to_owned(),
                        value: serde_json::to_value(value).map_err(|_| internal())?,
                    }),
                    _ => Err(internal()),
                }
            }
            Command::DecideProposal {
                run_id,
                proposal_id,
                proposal_digest,
                proposed_revision,
                decision_id,
                decision,
            } => {
                let run =
                    RunId::new(run_id.clone()).map_err(|error| invalid(&error.to_string()))?;
                let proposal = ProposalId::new(proposal_id.clone())
                    .map_err(|error| invalid(&error.to_string()))?;
                let digest: ProposalDigest =
                    serde_json::from_value(Value::String(proposal_digest.clone()))
                        .map_err(|error| invalid(&error.to_string()))?;
                let revision = parse_revision_id(proposed_revision)?;
                let decision_id = ReconciliationDecisionId::new(decision_id.clone())
                    .map_err(|error| invalid(&error.to_string()))?;
                let command = match decision {
                    ProposalDecision::Approve => ControlCommand::ApproveProposal {
                        run,
                        proposal,
                        proposal_digest: digest.clone(),
                        proposed_revision: revision,
                        decision: decision_id,
                    },
                    ProposalDecision::Reject => ControlCommand::RejectProposal {
                        run,
                        proposal,
                        proposal_digest: digest.clone(),
                        proposed_revision: revision,
                        decision: decision_id,
                    },
                };
                let sequence = self.execute_control_guarded(
                    session,
                    request,
                    request.expected_sequence,
                    Some(digest),
                    command,
                    "decision",
                )?;
                accepted_sequence(request, sequence, "proposal_decided")
            }
            Command::ApplyProposal {
                run_id,
                proposal_id,
                proposal_digest,
                proposed_revision,
            } => {
                let run =
                    RunId::new(run_id.clone()).map_err(|error| invalid(&error.to_string()))?;
                let digest: ProposalDigest =
                    serde_json::from_value(Value::String(proposal_digest.clone()))
                        .map_err(|error| invalid(&error.to_string()))?;
                let command = ControlCommand::ApplyProposal {
                    run,
                    proposal: ProposalId::new(proposal_id.clone())
                        .map_err(|error| invalid(&error.to_string()))?,
                    proposal_digest: digest.clone(),
                    proposed_revision: parse_revision_id(proposed_revision)?,
                };
                let sequence = self.execute_control_guarded(
                    session,
                    request,
                    request.expected_sequence,
                    Some(digest),
                    command,
                    "apply",
                )?;
                accepted_sequence(request, sequence, "proposal_applied")
            }
            Command::PutLayout { layout } => layouts::execute(self, session, request, layout),
        }
    }

    fn simple_run_command<F>(
        &self,
        session: &ActorSession,
        request: &CommandRequest,
        run_id: &str,
        suffix: &str,
        build: F,
    ) -> Result<CommandAccepted, PublicFailure>
    where
        F: FnOnce(RunId) -> ControlCommand,
    {
        let run = RunId::new(run_id.to_owned()).map_err(|error| invalid(&error.to_string()))?;
        let sequence = self.execute_control(
            session,
            request,
            request.expected_sequence,
            build(run),
            suffix,
        )?;
        accepted_sequence(request, sequence, &format!("run_{suffix}d"))
    }

    fn execute_control(
        &self,
        session: &ActorSession,
        request: &CommandRequest,
        expected_sequence: Option<u64>,
        command: ControlCommand,
        suffix: &str,
    ) -> Result<u64, PublicFailure> {
        let result = self.execute_control_result(
            session,
            request,
            expected_sequence,
            None,
            command,
            suffix,
        )?;
        match result {
            ControlResult::RuntimeCommand { resulting_sequence } => Ok(resulting_sequence.get()),
            _ => Err(internal()),
        }
    }

    fn execute_control_guarded(
        &self,
        session: &ActorSession,
        request: &CommandRequest,
        expected_sequence: Option<u64>,
        proposal_digest: Option<ProposalDigest>,
        command: ControlCommand,
        suffix: &str,
    ) -> Result<u64, PublicFailure> {
        let result = self.execute_control_result(
            session,
            request,
            expected_sequence,
            proposal_digest,
            command,
            suffix,
        )?;
        match result {
            ControlResult::RuntimeCommand { resulting_sequence } => Ok(resulting_sequence.get()),
            _ => Err(internal()),
        }
    }

    fn execute_control_result(
        &self,
        session: &ActorSession,
        request: &CommandRequest,
        expected_sequence: Option<u64>,
        proposal_digest: Option<ProposalDigest>,
        command: ControlCommand,
        suffix: &str,
    ) -> Result<ControlResult, PublicFailure> {
        let document = ControlCommandDocument::new(
            internal_control_id(session, request, suffix)?,
            session.context.clone(),
            TimestampMillis::new(unix_millis()),
            OptimisticGuard {
                expected_run_sequence: expected_sequence.map(RunSequence::new),
                expected_revision: request
                    .expected_revision
                    .as_deref()
                    .map(parse_revision_id)
                    .transpose()?,
                expected_proposal_digest: proposal_digest,
            },
            Reason::new(request.reason.clone()).map_err(public_persistence)?,
            evidence(request)?,
            command,
        )
        .map_err(public_control)?;
        self.control.execute(&document).map_err(public_control)
    }
}
