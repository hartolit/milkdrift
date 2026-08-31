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
            Command::ImportPromptSequence { document } => {
                self.prompt_sequence_command(session, request, document, true)
            }
            Command::ValidatePromptSequence { document } => {
                self.prompt_sequence_command(session, request, document, false)
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
            Command::InspectController {
                run_id,
                controller_execution,
            } => {
                let run =
                    RunId::new(run_id.clone()).map_err(|error| invalid(&error.to_string()))?;
                let result = self.execute_control_result(
                    session,
                    request,
                    request.expected_sequence,
                    None,
                    ControlCommand::InspectController {
                        run,
                        controller_execution: NodeExecutionId::new(controller_execution.clone())
                            .map_err(|error| invalid(&error.to_string()))?,
                    },
                    "inspect-controller",
                )?;
                let ControlResult::ControllerStatus { value } = result else {
                    return Err(internal());
                };
                Ok(CommandAccepted {
                    command_id: request.command_id.clone(),
                    replayed: false,
                    resulting_sequence: value.last_assessment_sequence.map(RunSequence::get),
                    result_type: "controller_status".to_owned(),
                    value: serde_json::to_value(value).map_err(|_| internal())?,
                })
            }
            Command::ContinueController {
                run_id,
                controller_execution,
                decision_id,
            } => {
                let run =
                    RunId::new(run_id.clone()).map_err(|error| invalid(&error.to_string()))?;
                let result = self.execute_control_result(
                    session,
                    request,
                    request.expected_sequence,
                    None,
                    ControlCommand::ContinueController {
                        run,
                        controller_execution: NodeExecutionId::new(controller_execution.clone())
                            .map_err(|error| invalid(&error.to_string()))?,
                        decision: RepeatDecisionId::new(decision_id.clone())
                            .map_err(|error| invalid(&error.to_string()))?,
                    },
                    "continue-controller",
                )?;
                let ControlResult::ControllerStatus { value } = result else {
                    return Err(internal());
                };
                let resulting_sequence = self
                    .store
                    .run_summary(&value.run)
                    .map_err(public_persistence)?
                    .map(|summary| summary.through_sequence.get());
                Ok(CommandAccepted {
                    command_id: request.command_id.clone(),
                    replayed: false,
                    resulting_sequence,
                    result_type: "controller_continued".to_owned(),
                    value: serde_json::to_value(value).map_err(|_| internal())?,
                })
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
                        run: run.clone(),
                        proposal,
                        proposal_digest: digest.clone(),
                        proposed_revision: revision,
                        decision: decision_id,
                    },
                    ProposalDecision::Reject => ControlCommand::RejectProposal {
                        run: run.clone(),
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
                    run: run.clone(),
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

    fn prompt_sequence_command(
        &self,
        session: &ActorSession,
        request: &CommandRequest,
        document: &Value,
        store: bool,
    ) -> Result<CommandAccepted, PublicFailure> {
        let bytes =
            serde_json::to_vec(document).map_err(|_| invalid("invalid prompt-sequence JSON"))?;
        let document = PromptSequenceDocument::from_json(&bytes)
            .map_err(|error| invalid(&bounded(&error.to_string())))?;
        let author = AuthorRef::new(session.actor.as_str().to_owned())
            .map_err(|error| invalid(&error.to_string()))?;
        let compiled = compile_prompt_sequence(&document, author)
            .map_err(|error| invalid(&bounded(&error.to_string())))?;
        let revision = compiled.revision();
        let mut resources = RequestedResourceFacts::empty();
        resources.workflow = Some(revision.semantic().workflow().clone());
        resources.revision = Some(revision.id().clone());
        let operation = if store {
            AuthorityOperation::ImportBlueprint
        } else {
            AuthorityOperation::ValidateBlueprint
        };
        let decision = self.authorize(
            session,
            operation,
            resources,
            if store {
                "command:import-prompt-sequence"
            } else {
                "command:validate-prompt-sequence"
            },
        )?;
        let replayed = if store {
            matches!(
                self.store
                    .put_revision(revision)
                    .map_err(public_persistence)?,
                milkdrift_persistence::ImmutableRevisionPut::AlreadyPresent
            )
        } else {
            false
        };
        if store {
            self.record_security_decision(&decision)?;
        }
        Ok(CommandAccepted {
            command_id: request.command_id.clone(),
            replayed,
            resulting_sequence: None,
            result_type: if store {
                "prompt_sequence_imported"
            } else {
                "prompt_sequence_valid"
            }
            .to_owned(),
            value: json!({
                "schema_version": 1,
                "sequence_id": document.sequence().id,
                "workflow_id": revision.semantic().workflow().as_str(),
                "revision_id": revision.id().as_str(),
                "semantic_digest": revision.content_digest().as_str(),
                "import_digest": compiled.import_digest(),
                "repository_profile_digest": compiled.repository_profile_digest(),
                "stages": compiled.stages(),
            }),
        })
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
        let run = match &command {
            ControlCommand::ApproveProposal { run, .. }
            | ControlCommand::RejectProposal { run, .. }
            | ControlCommand::ApplyProposal { run, .. } => run.clone(),
            _ => return Err(internal()),
        };
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
            ControlResult::ProposalStatus { .. } => self
                .store
                .run_summary(&run)
                .map_err(public_persistence)?
                .map(|summary| summary.through_sequence.get())
                .ok_or_else(not_found),
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
