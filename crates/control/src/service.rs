use std::{
    collections::{BTreeSet, VecDeque},
    sync::Arc,
};

use milkdrift_authority::{
    AuthorityBudget, AuthorityEvaluator, AuthorityExecutionProvenance, AuthorityOperation,
    AuthorityRequest, BoundaryTimeMillis, CapabilityAuthorityScope, DecisionId,
    RequestedResourceFacts,
};
use milkdrift_blueprint::{AuthorRef, BlueprintRevision, NodeKind, ReducerStrategy, RevisionId};
use milkdrift_capability::CapabilityRequirement;
use milkdrift_persistence::{
    AuthorityDecision, CommandDisposition, CommandId, EventCursor, EventPageQuery,
    EvidenceReference, PageSize, Reason, ReconciliationDecisionId, ReconciliationId,
    ReconciliationPolicy, RepeatContinuationDecision, RevisionStore, RunSequence,
};
use milkdrift_runtime::{ExternalWorkAction, RunCommand, RunCommandDocument, RuntimeService};
use milkdrift_workspace::RunId;

use crate::{
    AttemptInspection, ControlCommand, ControlCommandDocument, ControlError, ControlResult,
    ControllerLifecycleOwner, NodeExecutionRead, OptimisticGuard, PolicyClassification,
    ProposalApplicationPolicy, ProposalProvenance, ProposalStatusRead, ProposalSubmission,
    ReconciliationStatusRead, RequestedRunAction, RevisionInspection, RiskClass, RunInspection,
    TimelinePage, WorkflowProposal, classify_proposal,
};

const MAX_PROPOSAL_AUTHORITY_REVISION_WALK: usize = 512;

/// Shared application service for authority-scoped human, service, and AI workflow control.
pub struct ControlService {
    revisions: Arc<dyn RevisionStore>,
    runtime: Arc<RuntimeService>,
    authority: Arc<dyn AuthorityEvaluator>,
    controller: Arc<ControllerLifecycleOwner>,
}

impl ControlService {
    /// Constructs a service over the same revision, runtime, and authority owners used elsewhere.
    #[must_use]
    pub fn new(
        revisions: Arc<dyn RevisionStore>,
        runtime: Arc<RuntimeService>,
        authority: Arc<dyn AuthorityEvaluator>,
    ) -> Self {
        let controller = Arc::new(ControllerLifecycleOwner::new(revisions.clone()));
        Self {
            revisions,
            runtime,
            authority,
            controller,
        }
    }

    /// Returns the one lifecycle owner installed into the deterministic runtime.
    #[must_use]
    pub fn controller_lifecycle_owner(&self) -> Arc<ControllerLifecycleOwner> {
        self.controller.clone()
    }

    /// Executes one complete versioned command through a single authoritative path.
    pub fn execute(
        &self,
        document: &ControlCommandDocument,
    ) -> Result<ControlResult, ControlError> {
        match document.command() {
            ControlCommand::InspectRun { run } => {
                let value = self.inspect_run(run, document.guard())?;
                self.authorize_simple(
                    document,
                    AuthorityOperation::InspectRun,
                    value.workflow.as_ref(),
                    Some(run),
                )?;
                Ok(ControlResult::RunInspection { value })
            }
            ControlCommand::InspectRevision { revision } => {
                let value = self
                    .revisions
                    .revision(revision)?
                    .ok_or(ControlError::BaseRevisionNotFound)?;
                match self.authorize_simple(
                    document,
                    AuthorityOperation::InspectRevision,
                    Some(value.semantic().workflow()),
                    None,
                ) {
                    Ok(()) => {}
                    Err(ControlError::AuthorizationDenied { .. }) => {
                        // Exact revision identifiers are content addresses. Normalize an
                        // out-of-scope existing revision to the same response as an absent one.
                        return Err(ControlError::BaseRevisionNotFound);
                    }
                    Err(error) => return Err(error),
                }
                Ok(ControlResult::RevisionInspection {
                    value: RevisionInspection {
                        revision: value.id().clone(),
                        workflow: value.semantic().workflow().clone(),
                        lineage_sequence: value.sequence(),
                        content_digest: value.content_digest().clone(),
                        parents: value.parents().to_vec(),
                        author: value.author().clone(),
                        reason: value.reason().to_owned(),
                        node_count: value.semantic().nodes().len(),
                        edge_count: value.semantic().edges().len(),
                    },
                })
            }
            ControlCommand::InspectTimeline { run, after, limit } => {
                let run_value = self.inspect_run(run, &OptimisticGuard::default())?;
                self.authorize_simple(
                    document,
                    AuthorityOperation::InspectTimeline,
                    run_value.workflow.as_ref(),
                    Some(run),
                )?;
                Ok(ControlResult::Timeline {
                    value: self.timeline(run, *after, *limit)?,
                })
            }
            ControlCommand::InspectController {
                run,
                controller_execution,
            } => {
                let projection = self.runtime.projection(run)?;
                self.authorize_simple(
                    document,
                    AuthorityOperation::InspectRun,
                    projection.workflow(),
                    Some(run),
                )?;
                Ok(ControlResult::ControllerStatus {
                    value: self.controller.status(
                        run,
                        &projection,
                        controller_execution,
                        document.issued_at().get(),
                    )?,
                })
            }
            ControlCommand::SubmitProposal { proposal } => {
                self.submit(document, proposal.proposal())
            }
            ControlCommand::ApproveProposal {
                run,
                proposal,
                proposal_digest,
                proposed_revision,
                decision,
            } => self.decide_proposal(
                document,
                run,
                proposal,
                proposal_digest,
                proposed_revision,
                decision,
                AuthorityDecision::Approve,
            ),
            ControlCommand::RejectProposal {
                run,
                proposal,
                proposal_digest,
                proposed_revision,
                decision,
            } => self.decide_proposal(
                document,
                run,
                proposal,
                proposal_digest,
                proposed_revision,
                decision,
                AuthorityDecision::Reject,
            ),
            ControlCommand::ApplyProposal {
                run,
                proposal,
                proposal_digest,
                proposed_revision,
            } => self.apply_proposal(document, run, proposal, proposal_digest, proposed_revision),
            ControlCommand::QueryProposal {
                run,
                proposal,
                proposed_revision,
            } => {
                let run_value = self.inspect_run(run, &OptimisticGuard::default())?;
                self.authorize_simple(
                    document,
                    AuthorityOperation::InspectProposal,
                    run_value.workflow.as_ref(),
                    Some(run),
                )?;
                Ok(ControlResult::ProposalStatus {
                    value: self.proposal_status(run, proposal, proposed_revision)?,
                })
            }
            ControlCommand::PauseRun { run } => {
                self.execute_simple_runtime(document, run, "pause", RunCommand::PauseRun)
            }
            ControlCommand::ResumeRun { run } => {
                self.execute_simple_runtime(document, run, "resume", RunCommand::ResumeRun)
            }
            ControlCommand::RequestCancellation { run } => self.execute_simple_runtime(
                document,
                run,
                "request-cancellation",
                RunCommand::RequestCancellation,
            ),
            ControlCommand::ResolveExternalWork {
                run,
                attempt,
                decision,
                action,
                remediation_node,
            } => self.execute_simple_runtime(
                document,
                run,
                "resolve-external-work",
                RunCommand::ResolveExternalWork {
                    attempt: attempt.clone(),
                    decision: decision.clone(),
                    action: *action,
                    remediation_node: remediation_node.clone(),
                },
            ),
            ControlCommand::CreateRun {
                run,
                workflow,
                revision,
                root_scope,
                workspace_budget,
                inputs,
            } => self.execute_simple_runtime(
                document,
                run,
                "create-run",
                RunCommand::CreateRun {
                    workflow: workflow.clone(),
                    revision: revision.clone(),
                    root_scope: root_scope.clone(),
                    workspace_budget: workspace_budget.clone(),
                    inputs: inputs.clone(),
                },
            ),
            ControlCommand::StartRun { run } => {
                self.execute_simple_runtime(document, run, "start-run", RunCommand::StartRun)
            }
            ControlCommand::Signal {
                run,
                signal,
                signal_type,
                correlation,
                mode,
                payload,
            } => self.execute_simple_runtime(
                document,
                run,
                "signal",
                RunCommand::DeliverSignal {
                    signal: signal.clone(),
                    signal_type: signal_type.clone(),
                    correlation: correlation.clone(),
                    mode: *mode,
                    payload: payload.clone(),
                },
            ),
            ControlCommand::ContinueController {
                run,
                controller_execution,
                decision,
            } => {
                let expected_sequence = required_sequence(document.guard())?;
                let execution = self.runtime_command(
                    document,
                    run,
                    expected_sequence,
                    "continue-controller",
                    document.reason().clone(),
                    document.evidence().to_vec(),
                    RunCommand::DecideRepeatContinuation {
                        repeat_execution: controller_execution.clone(),
                        decision: decision.clone(),
                        outcome: RepeatContinuationDecision::Approved,
                        approved_additional_iterations: Some(1),
                    },
                )?;
                ensure_accepted(&execution)?;
                let projection = self.runtime.projection(run)?;
                Ok(ControlResult::ControllerStatus {
                    value: self.controller.status(
                        run,
                        &projection,
                        controller_execution,
                        document.issued_at().get(),
                    )?,
                })
            }
        }
    }

    fn submit(
        &self,
        document: &ControlCommandDocument,
        proposal: &WorkflowProposal,
    ) -> Result<ControlResult, ControlError> {
        if proposal.proposer() != document.context().actor() {
            return Err(ControlError::InvalidContract(
                "proposal actor does not match immutable caller authority context".to_owned(),
            ));
        }
        if document.guard().expected_revision.as_ref() != Some(proposal.base_revision())
            || document.guard().expected_proposal_digest.as_ref() != Some(proposal.digest())
        {
            return Err(ControlError::InvalidContract(
                "control guard must bind the exact proposal base revision and digest".to_owned(),
            ));
        }
        if document.guard().expected_run_sequence != proposal.observed_run_sequence() {
            return Err(ControlError::InvalidContract(
                "control guard and proposal observed sequence differ".to_owned(),
            ));
        }
        let proposal_operation = if proposal.run().is_some() {
            AuthorityOperation::Propose
        } else {
            AuthorityOperation::ProposeOffline
        };
        self.authorize_simple(
            document,
            proposal_operation,
            Some(proposal.workflow()),
            proposal.run(),
        )?;

        let base = self
            .revisions
            .revision(proposal.base_revision())?
            .ok_or(ControlError::BaseRevisionNotFound)?;
        if base.semantic().workflow() != proposal.workflow()
            || base.content_digest() != proposal.base_digest()
        {
            return Err(ControlError::BaseRevisionMismatch);
        }
        let author = proposal_author(proposal)?;
        let revision_reason = proposal_revision_reason(proposal);
        let candidate = base.revise(
            proposal.base_revision(),
            proposal.mutation().clone(),
            author,
            revision_reason,
        )?;
        if proposal.run().is_none() {
            let old_policy = crate::ControllerPolicyDocument::from_controller_revision(&base)?;
            let new_policy = crate::ControllerPolicyDocument::from_controller_revision(&candidate)?;
            if old_policy.as_ref().map(|(_, value)| value.digest())
                != new_policy.as_ref().map(|(_, value)| value.digest())
            {
                return Err(ControlError::ForbiddenProposal);
            }
        }
        let candidate_preexisting = self.revisions.revision(candidate.id())?.is_some();
        let live_projection = match proposal.run() {
            Some(run) => {
                let expected = proposal.observed_run_sequence().ok_or_else(|| {
                    ControlError::InvalidContract("live proposal has no sequence".to_owned())
                })?;
                let current = self.runtime.projection(run)?;
                let projection = if current.sequence() == expected {
                    current
                } else if candidate_preexisting {
                    self.projection_at(run, expected)?
                } else {
                    return Err(ControlError::StaleRunSequence {
                        expected,
                        actual: current.sequence(),
                    });
                };
                if projection.workflow() != Some(proposal.workflow())
                    || projection.revision() != Some(proposal.base_revision())
                    || projection.revision_digest() != Some(proposal.base_digest())
                {
                    return Err(ControlError::BaseRevisionMismatch);
                }
                Some(projection)
            }
            None => None,
        };
        if let (Some(run), Some(projection)) = (proposal.run(), live_projection.as_ref()) {
            self.controller.assess_proposal(
                run,
                projection,
                proposal,
                document.issued_at().get(),
            )?;
            let controller_actor = projection.execution_authority().map(|value| value.actor());
            if controller_actor == Some(proposal.proposer()) {
                let old_policy = crate::ControllerPolicyDocument::from_controller_revision(&base)?;
                let new_policy =
                    crate::ControllerPolicyDocument::from_controller_revision(&candidate)?;
                if old_policy.as_ref().map(|(_, value)| value.digest())
                    != new_policy.as_ref().map(|(_, value)| value.digest())
                {
                    return Err(ControlError::ForbiddenProposal);
                }
            }
        }
        self.authorize_revision_delta(
            document,
            proposal_operation,
            &base,
            &candidate,
            proposal.run(),
        )?;
        let classification =
            classify_proposal(&base, &candidate, proposal, live_projection.as_ref());
        if classification.risk == RiskClass::Forbidden {
            return Err(ControlError::ForbiddenProposal);
        }
        if proposal.run().is_some() && document.evidence().len() + proposal.evidence().len() > 32 {
            return Err(ControlError::Bounds {
                location: "live_proposal.evidence".to_owned(),
                reason: "combined control and proposal evidence exceeds the runtime limit of 32"
                    .to_owned(),
            });
        }

        let _put = self.revisions.put_revision(&candidate)?;
        let reconciliation = if let Some(run) = proposal.run() {
            let policy = if classification.risk == RiskClass::Low
                && proposal.application_policy() != ProposalApplicationPolicy::RequireApproval
            {
                ReconciliationPolicy::FinishCurrentThenAdopt
            } else {
                ReconciliationPolicy::RequireAuthority
            };
            let evidence = merged_evidence(document.evidence(), proposal.evidence());
            let reason = proposal_command_reason(document, proposal, &classification)?;
            let execution = self.runtime_command(
                document,
                run,
                proposal.observed_run_sequence().ok_or_else(|| {
                    ControlError::InvalidContract("live proposal has no sequence".to_owned())
                })?,
                "propose-revision",
                reason,
                evidence,
                RunCommand::RequestRevisionAdoption {
                    reconciliation: reconciliation_id_from_parts(
                        proposal.identity(),
                        candidate.id(),
                    )?,
                    revision: candidate.id().clone(),
                    policy,
                },
            )?;
            ensure_accepted(&execution)?;
            Some(self.reconciliation_status(run, proposal, candidate.id())?)
        } else {
            None
        };

        let mut applied = false;
        let mut reconciliation = reconciliation;
        if let Some(run) = proposal.run()
            && proposal.application_policy() == ProposalApplicationPolicy::AutoApplyLowRisk
            && classification.risk == RiskClass::Low
            && self.revision_delta_is_authorized(
                document,
                AuthorityOperation::Apply,
                &base,
                &candidate,
                Some(run),
            )?
        {
            let current = self.runtime.projection(run)?;
            self.controller.assess_proposal_transition(
                run,
                &current,
                &candidate,
                milkdrift_persistence::ControllerAssessmentBoundary::ProposalApplication,
                document.issued_at().get(),
            )?;
            let plan = self
                .reconciliation_status(run, proposal, candidate.id())?
                .plan
                .ok_or_else(|| ControlError::ProposalState("proposal plan is absent".to_owned()))?;
            let execution = self.runtime_command(
                document,
                run,
                current.sequence(),
                "auto-apply-revision",
                proposal_command_reason(document, proposal, &classification)?,
                merged_evidence(document.evidence(), proposal.evidence()),
                RunCommand::ApplyReconciliation { plan },
            )?;
            ensure_accepted(&execution)?;
            applied = self.runtime.projection(run)?.revision() == Some(candidate.id());
            if applied {
                self.execute_requested_action(document, proposal, run)?;
            }
            reconciliation = Some(self.reconciliation_status(run, proposal, candidate.id())?);
        }

        Ok(ControlResult::ProposalSubmitted {
            value: ProposalSubmission {
                proposal: proposal.identity().clone(),
                proposal_digest: proposal.digest().clone(),
                proposed_revision: candidate.id().clone(),
                classification,
                reconciliation,
                applied,
            },
        })
    }

    #[allow(clippy::too_many_arguments)] // One validated control operation keeps its authority and optimistic facts explicit.
    fn decide_proposal(
        &self,
        document: &ControlCommandDocument,
        run: &RunId,
        proposal: &crate::ProposalId,
        proposal_digest: &crate::ProposalDigest,
        proposed_revision: &RevisionId,
        decision: &ReconciliationDecisionId,
        outcome: AuthorityDecision,
    ) -> Result<ControlResult, ControlError> {
        validate_proposal_guard(document.guard(), proposal_digest, proposed_revision)?;
        let candidate =
            self.validate_proposed_revision(proposal, proposal_digest, proposed_revision)?;
        let expected_sequence = required_sequence(document.guard())?;
        if outcome == AuthorityDecision::Approve {
            let projection = self.runtime.projection(run)?;
            self.controller.assess_proposal_transition(
                run,
                &projection,
                &candidate,
                milkdrift_persistence::ControllerAssessmentBoundary::ProposalApproval,
                document.issued_at().get(),
            )?;
        }
        let status = self.proposal_status(run, proposal, proposed_revision)?;
        let plan = status
            .reconciliation
            .plan
            .ok_or_else(|| ControlError::ProposalState("proposal plan is absent".to_owned()))?;
        let command = RunCommand::DecideReconciliation {
            plan,
            decision: decision.clone(),
            outcome,
        };
        let execution = self.runtime_command(
            document,
            run,
            expected_sequence,
            "decide-proposal",
            document.reason().clone(),
            document.evidence().to_vec(),
            command,
        )?;
        ensure_accepted(&execution)?;
        Ok(ControlResult::ProposalStatus {
            value: self.proposal_status(run, proposal, proposed_revision)?,
        })
    }

    fn apply_proposal(
        &self,
        document: &ControlCommandDocument,
        run: &RunId,
        proposal: &crate::ProposalId,
        proposal_digest: &crate::ProposalDigest,
        proposed_revision: &RevisionId,
    ) -> Result<ControlResult, ControlError> {
        validate_proposal_guard(document.guard(), proposal_digest, proposed_revision)?;
        let candidate =
            self.validate_proposed_revision(proposal, proposal_digest, proposed_revision)?;
        let expected_sequence = required_sequence(document.guard())?;
        let current_id = candidate.parents().first().ok_or_else(|| {
            ControlError::ProposalState("proposed revision has no base parent".to_owned())
        })?;
        let base = self
            .revisions
            .revision(current_id)?
            .ok_or(ControlError::BaseRevisionNotFound)?;
        self.authorize_revision_delta(
            document,
            AuthorityOperation::Apply,
            &base,
            &candidate,
            Some(run),
        )?;
        let projection = self.runtime.projection(run)?;
        self.controller.assess_proposal_transition(
            run,
            &projection,
            &candidate,
            milkdrift_persistence::ControllerAssessmentBoundary::ProposalApplication,
            document.issued_at().get(),
        )?;
        let status = self.proposal_status(run, proposal, proposed_revision)?;
        let plan = status
            .reconciliation
            .plan
            .ok_or_else(|| ControlError::ProposalState("proposal plan is absent".to_owned()))?;
        let execution = self.runtime_command(
            document,
            run,
            expected_sequence,
            "apply-proposal",
            document.reason().clone(),
            document.evidence().to_vec(),
            RunCommand::ApplyReconciliation { plan },
        )?;
        ensure_accepted(&execution)?;
        Ok(ControlResult::ProposalStatus {
            value: self.proposal_status(run, proposal, proposed_revision)?,
        })
    }

    fn execute_simple_runtime(
        &self,
        document: &ControlCommandDocument,
        run: &RunId,
        phase: &str,
        command: RunCommand,
    ) -> Result<ControlResult, ControlError> {
        let expected_sequence = required_sequence(document.guard())?;
        let execution = self.runtime_command(
            document,
            run,
            expected_sequence,
            phase,
            document.reason().clone(),
            document.evidence().to_vec(),
            command,
        )?;
        ensure_accepted(&execution)?;
        Ok(ControlResult::RuntimeCommand {
            resulting_sequence: execution.result().resulting_sequence(),
        })
    }

    fn execute_requested_action(
        &self,
        document: &ControlCommandDocument,
        proposal: &WorkflowProposal,
        run: &RunId,
    ) -> Result<(), ControlError> {
        let Some(action) = proposal.requested_action() else {
            return Ok(());
        };
        let command = match action {
            RequestedRunAction::Pause => RunCommand::PauseRun,
            RequestedRunAction::Resume => RunCommand::ResumeRun,
            RequestedRunAction::RequestCancellation => RunCommand::RequestCancellation,
            RequestedRunAction::RetryExternalWork { attempt } => RunCommand::ResolveExternalWork {
                attempt: attempt.clone(),
                decision: derived_resolution_decision(proposal)?,
                action: ExternalWorkAction::Retry,
                remediation_node: None,
            },
            RequestedRunAction::Signal {
                signal,
                signal_type,
                correlation,
                mode,
                payload,
            } => RunCommand::DeliverSignal {
                signal: signal.clone(),
                signal_type: signal_type.clone(),
                correlation: correlation.clone(),
                mode: *mode,
                payload: payload.clone(),
            },
        };
        let projection = self.runtime.projection(run)?;
        let execution = self.runtime_command(
            document,
            run,
            projection.sequence(),
            "proposal-run-action",
            Reason::new(format!(
                "proposal {} requested a separately authorized run action",
                proposal.identity()
            ))?,
            merged_evidence(document.evidence(), proposal.evidence()),
            command,
        )?;
        ensure_accepted(&execution)
    }

    #[allow(clippy::too_many_arguments)] // One validated control operation keeps its authority and optimistic facts explicit.
    fn runtime_command(
        &self,
        document: &ControlCommandDocument,
        run: &RunId,
        expected_sequence: RunSequence,
        phase: &str,
        reason: Reason,
        evidence: Vec<EvidenceReference>,
        command: RunCommand,
    ) -> Result<milkdrift_runtime::CommandExecution, ControlError> {
        let command = RunCommandDocument::new(
            derived_command_id(document, phase)?,
            run.clone(),
            document.context().actor().clone(),
            expected_sequence,
            document.issued_at(),
            reason,
            evidence,
            command,
        )?;
        Ok(self
            .runtime
            .handle_authorized_command(&command, document.context().authority())?)
    }

    fn projection_at(
        &self,
        run: &RunId,
        target: RunSequence,
    ) -> Result<milkdrift_runtime::RunProjection, ControlError> {
        let mut projection = milkdrift_runtime::RunProjection::new();
        if target == RunSequence::ZERO {
            return Ok(projection);
        }
        let mut cursor = None;
        loop {
            let page = self.runtime.history_page(&EventPageQuery::new(
                run.clone(),
                cursor,
                PageSize::new(256)?,
            )?)?;
            if page.observed_head < target {
                return Err(ControlError::StaleRunSequence {
                    expected: target,
                    actual: page.observed_head,
                });
            }
            for event in page.events {
                if event.sequence() > target {
                    return Ok(projection);
                }
                projection.apply(&event)?;
                if projection.sequence() == target {
                    return Ok(projection);
                }
            }
            cursor = page.next;
            if cursor.is_none() {
                return Err(ControlError::ProposalState(
                    "timeline ended before the proposal boundary".to_owned(),
                ));
            }
        }
    }

    fn inspect_run(
        &self,
        run: &RunId,
        guard: &OptimisticGuard,
    ) -> Result<RunInspection, ControlError> {
        let projection = self.runtime.projection(run)?;
        if let Some(expected) = guard.expected_run_sequence
            && expected != projection.sequence()
        {
            return Err(ControlError::StaleRunSequence {
                expected,
                actual: projection.sequence(),
            });
        }
        let mut executions = projection
            .node_executions()
            .values()
            .map(|execution| {
                let latest_attempt_id = execution.attempts().last().cloned();
                let latest_attempt = latest_attempt_id
                    .as_ref()
                    .and_then(|attempt| projection.attempts().get(attempt))
                    .map(|attempt| attempt_inspection(attempt, projection.execution_authority()));
                let side_effect = latest_attempt
                    .as_ref()
                    .and_then(|attempt| attempt.side_effect.as_ref())
                    .map(|classification| classification.side_effect());
                NodeExecutionRead {
                    execution: execution.execution().clone(),
                    node: execution.node().clone(),
                    revision: execution.revision().clone(),
                    state: execution.state().clone(),
                    attempt_count: execution.attempt_count(),
                    latest_attempt_id,
                    latest_attempt,
                    side_effect,
                    outputs: execution.outputs().to_vec(),
                }
            })
            .collect::<Vec<_>>();
        executions.extend(
            projection
                .settled_node_executions()
                .values()
                .map(|execution| {
                    let latest_attempt_id = execution.latest_attempt().cloned();
                    let latest_attempt = latest_attempt_id
                        .as_ref()
                        .and_then(|attempt| projection.attempts().get(attempt))
                        .map(|attempt| {
                            attempt_inspection(attempt, projection.execution_authority())
                        });
                    NodeExecutionRead {
                        execution: execution.execution().clone(),
                        node: execution.node().clone(),
                        revision: execution.revision().clone(),
                        state: execution.state().clone(),
                        attempt_count: execution.attempt_count(),
                        latest_attempt_id,
                        latest_attempt,
                        side_effect: Some(execution.side_effect()),
                        outputs: execution.outputs().to_vec(),
                    }
                }),
        );
        executions.sort_by(|left, right| left.execution.cmp(&right.execution));
        let reconciliation = projection
            .reconciliation()
            .current()
            .map(|request| status_from_projection(&projection, request));
        Ok(RunInspection {
            run: run.clone(),
            sequence: projection.sequence(),
            lifecycle: projection.lifecycle(),
            workflow: projection.workflow().cloned(),
            revision: projection.revision().cloned(),
            revision_digest: projection.revision_digest().cloned(),
            workspace_budget: projection.workspace_budget().cloned(),
            executions,
            reconciliation,
            input_units: projection.resource_usage().input_units(),
            output_units: projection.resource_usage().output_units(),
            duration_ms: projection.resource_usage().duration_ms(),
            artifact_bytes: projection.resource_usage().artifact_bytes(),
        })
    }

    fn timeline(
        &self,
        run: &RunId,
        after: Option<RunSequence>,
        limit: PageSize,
    ) -> Result<TimelinePage, ControlError> {
        let cursor = after.map(|next_sequence| EventCursor {
            run: run.clone(),
            next_sequence,
        });
        let page = self
            .runtime
            .history_page(&EventPageQuery::new(run.clone(), cursor, limit)?)?;
        Ok(TimelinePage {
            events: page.events,
            next_sequence: page.next.map(|value| value.next_sequence),
            observed_head: page.observed_head,
        })
    }

    fn proposal_status(
        &self,
        run: &RunId,
        proposal: &crate::ProposalId,
        proposed_revision: &RevisionId,
    ) -> Result<ProposalStatusRead, ControlError> {
        let projection = self.runtime.projection(run)?;
        let key = reconciliation_id_from_parts(proposal, proposed_revision)?;
        let request = projection
            .reconciliation()
            .requests()
            .get(&key)
            .ok_or_else(|| {
                ControlError::ProposalState("proposal reconciliation is absent".to_owned())
            })?;
        if request.to_revision() != proposed_revision {
            return Err(ControlError::ProposalState(
                "proposal reconciliation targets another revision".to_owned(),
            ));
        }
        Ok(ProposalStatusRead {
            proposal: proposal.clone(),
            proposed_revision: proposed_revision.clone(),
            reconciliation: status_from_projection(&projection, request),
        })
    }

    fn reconciliation_status(
        &self,
        run: &RunId,
        proposal: &WorkflowProposal,
        proposed_revision: &RevisionId,
    ) -> Result<ReconciliationStatusRead, ControlError> {
        Ok(self
            .proposal_status(run, proposal.identity(), proposed_revision)?
            .reconciliation)
    }

    fn validate_proposed_revision(
        &self,
        proposal: &crate::ProposalId,
        digest: &crate::ProposalDigest,
        revision: &RevisionId,
    ) -> Result<BlueprintRevision, ControlError> {
        let revision = self
            .revisions
            .revision(revision)?
            .ok_or(ControlError::BaseRevisionNotFound)?;
        let proposal_marker = format!("proposal_id={proposal}");
        let digest_marker = format!("proposal_digest={digest}");
        if !revision.reason().contains(&proposal_marker)
            || !revision.reason().contains(&digest_marker)
        {
            return Err(ControlError::ProposalState(
                "revision provenance does not match proposal identity and digest".to_owned(),
            ));
        }
        Ok(revision)
    }

    fn authorize_simple(
        &self,
        document: &ControlCommandDocument,
        operation: AuthorityOperation,
        workflow: Option<&milkdrift_blueprint::WorkflowId>,
        run: Option<&RunId>,
    ) -> Result<(), ControlError> {
        let mut resources = RequestedResourceFacts::empty();
        resources.workflow = workflow.cloned();
        resources.run = run.cloned();
        self.authorize(document, operation, resources, AuthorityBudget::default())
    }

    fn authorize_revision_delta(
        &self,
        document: &ControlCommandDocument,
        operation: AuthorityOperation,
        old: &BlueprintRevision,
        new: &BlueprintRevision,
        run: Option<&RunId>,
    ) -> Result<(), ControlError> {
        if self.revision_delta_is_authorized(document, operation, old, new, run)? {
            Ok(())
        } else {
            Err(ControlError::AuthorizationDenied {
                reasons: vec![milkdrift_authority::DecisionReasonCode::CapabilityMismatch],
                decision_digest: None,
            })
        }
    }

    fn revision_delta_is_authorized(
        &self,
        document: &ControlCommandDocument,
        operation: AuthorityOperation,
        old: &BlueprintRevision,
        new: &BlueprintRevision,
        run: Option<&RunId>,
    ) -> Result<bool, ControlError> {
        let budget = revision_budget(new);
        let mut general = RequestedResourceFacts::empty();
        general.workflow = Some(new.semantic().workflow().clone());
        general.run = run.cloned();
        if !self.evaluate_allowed(document, operation, general, budget)? {
            return Ok(false);
        }
        // A proposal is authorized against the complete old and candidate execution envelopes,
        // including capability reducers and every pinned subworkflow/repeat body. This prevents a
        // harmless-looking parent mutation from importing authority the controller does not own.
        for root in [old, new] {
            for requirement in self.revision_requirements(root)? {
                let envelope = CapabilityAuthorityScope::requirement_envelope(&requirement)?;
                let mut resources = RequestedResourceFacts::empty();
                resources.workflow = Some(new.semantic().workflow().clone());
                resources.run = run.cloned();
                resources.capability = requirement.exact_capability().cloned();
                resources.capability_operation = Some(requirement.operation().clone());
                resources.provider_profile = requirement.provider_profile_ref().cloned();
                resources.trust_zones = requirement.trust_zones().clone();
                resources.execution_trust_class = requirement.execution_trust_class();
                resources.side_effect = requirement.maximum_side_effect_class();
                resources.capability_envelope = Some(envelope);
                if !self.evaluate_allowed(document, operation, resources, budget)? {
                    return Ok(false);
                }
            }
        }
        Ok(true)
    }

    fn revision_requirements(
        &self,
        root: &BlueprintRevision,
    ) -> Result<Vec<CapabilityRequirement>, ControlError> {
        let mut requirements = Vec::new();
        let mut pending = VecDeque::new();
        let mut visited = BTreeSet::from([root.id().clone()]);
        collect_revision_requirements(root, &mut requirements, &mut pending);
        while let Some(revision_id) = pending.pop_front() {
            if !visited.insert(revision_id.clone()) {
                continue;
            }
            if visited.len() > MAX_PROPOSAL_AUTHORITY_REVISION_WALK {
                return Err(ControlError::Bounds {
                    location: "proposal.authority_revision_graph".to_owned(),
                    reason: "proposal authority revision graph exceeds 512 revisions".to_owned(),
                });
            }
            let revision = self
                .revisions
                .revision(&revision_id)?
                .ok_or(ControlError::BaseRevisionNotFound)?;
            collect_revision_requirements(&revision, &mut requirements, &mut pending);
        }
        Ok(requirements)
    }

    fn authorize(
        &self,
        document: &ControlCommandDocument,
        operation: AuthorityOperation,
        resources: RequestedResourceFacts,
        budget: AuthorityBudget,
    ) -> Result<(), ControlError> {
        let decision = self.evaluate(document, operation, resources, budget)?;
        if decision.is_allowed() {
            Ok(())
        } else {
            Err(ControlError::AuthorizationDenied {
                reasons: decision.reason_codes().to_vec(),
                decision_digest: Some(decision.digest().to_owned()),
            })
        }
    }

    fn evaluate_allowed(
        &self,
        document: &ControlCommandDocument,
        operation: AuthorityOperation,
        resources: RequestedResourceFacts,
        budget: AuthorityBudget,
    ) -> Result<bool, ControlError> {
        Ok(self
            .evaluate(document, operation, resources, budget)?
            .is_allowed())
    }

    fn evaluate(
        &self,
        document: &ControlCommandDocument,
        operation: AuthorityOperation,
        resources: RequestedResourceFacts,
        budget: AuthorityBudget,
    ) -> Result<milkdrift_authority::AuthorityDecisionSnapshot, ControlError> {
        let claim = document.context().authority();
        let mut hasher = blake3::Hasher::new();
        hasher.update(b"milkdrift.control-authority.v1\0");
        hasher.update(document.control_id().as_str().as_bytes());
        hasher.update(format!("{operation:?}{resources:?}{budget:?}").as_bytes());
        let request = AuthorityRequest {
            decision: DecisionId::new(format!("decision:{}", hasher.finalize()))?,
            actor: document.context().actor().clone(),
            grant: claim.grant().clone(),
            grant_revision: claim.grant_revision(),
            grant_digest: claim.grant_digest().clone(),
            revocation_generation: claim.revocation_generation(),
            operation,
            resources,
            budget,
            evaluated_at: BoundaryTimeMillis::new(document.issued_at().get()),
            provenance: AuthorityExecutionProvenance::default(),
        };
        Ok(self.authority.evaluate(&request)?)
    }
}

fn ensure_accepted(execution: &milkdrift_runtime::CommandExecution) -> Result<(), ControlError> {
    if execution.result().disposition() == CommandDisposition::Accepted {
        return Ok(());
    }
    if let Some(decision) = execution.result().authorization()
        && !decision.is_allowed()
    {
        return Err(ControlError::AuthorizationDenied {
            reasons: decision.reason_codes().to_vec(),
            decision_digest: Some(decision.digest().to_owned()),
        });
    }
    Err(ControlError::ProposalState(format!(
        "runtime rejected command with result {}",
        execution.result().result().value()
    )))
}

fn collect_revision_requirements(
    revision: &BlueprintRevision,
    requirements: &mut Vec<CapabilityRequirement>,
    pending: &mut VecDeque<RevisionId>,
) {
    for node in revision.semantic().nodes().values() {
        match node.kind() {
            NodeKind::Task { config } => requirements.push(config.requirement().clone()),
            NodeKind::Reducer { config } => {
                if let ReducerStrategy::Capability(operation) = config.strategy() {
                    // Reducer operation contracts are resolved at dispatch; absent a narrower
                    // blueprint envelope, treating them as unknown-effect is fail-closed.
                    requirements.push(CapabilityRequirement::new(operation.clone()));
                }
            }
            NodeKind::Subworkflow { reference } => {
                pending.push_back(reference.revision().clone());
            }
            NodeKind::Repeat { config } => pending.push_back(config.body().revision().clone()),
            NodeKind::Branch { .. }
            | NodeKind::Fork { .. }
            | NodeKind::Join { .. }
            | NodeKind::Wait { .. }
            | NodeKind::SignalWait { .. }
            | NodeKind::Terminal { .. } => {}
        }
    }
}

fn attempt_inspection(
    attempt: &milkdrift_runtime::NodeAttemptProjection,
    execution_authority: Option<&milkdrift_authority::ExecutionAuthorityBasis>,
) -> AttemptInspection {
    AttemptInspection {
        attempt: attempt.attempt().clone(),
        invocation: attempt.invocation().cloned(),
        state: attempt.state().clone(),
        capability: attempt
            .capability()
            .map(|resolution| resolution.snapshot().clone()),
        execution_authority: execution_authority.cloned(),
        resolution_authorization: attempt.resolution_authorization().cloned(),
        claim_authorization: attempt.entry_authorization().cloned(),
        entry_authorization: attempt.adapter_entry_authorization().cloned(),
        context_manifest: attempt
            .request()
            .and_then(|request| request.context_manifest())
            .cloned(),
        side_effect: attempt.side_effect().cloned(),
        outputs: attempt.outputs().to_vec(),
        progress: attempt.progress().to_vec(),
        usage: attempt.usage().cloned(),
        terminal: attempt.terminal().cloned(),
        late_terminal_evidence: attempt.late_terminal_evidence().cloned(),
        external_outcome: attempt.obligation().cloned(),
    }
}

fn status_from_projection(
    projection: &milkdrift_runtime::RunProjection,
    request: &milkdrift_runtime::ReconciliationRequestProjection,
) -> ReconciliationStatusRead {
    let plan = request
        .plan()
        .and_then(|id| projection.reconciliation().plans().get(id));
    ReconciliationStatusRead {
        revision: request.to_revision().clone(),
        plan: request.plan().cloned(),
        state: request.state(),
        approved: plan.is_some_and(|value| {
            value
                .decisions()
                .iter()
                .any(|decision| decision.outcome() == AuthorityDecision::Approve)
        }),
        applied_sequence: plan.and_then(|value| value.applied_sequence()),
        stale_sequence: plan.and_then(|value| value.stale_sequence()),
    }
}

fn validate_proposal_guard(
    guard: &OptimisticGuard,
    digest: &crate::ProposalDigest,
    revision: &RevisionId,
) -> Result<(), ControlError> {
    if guard.expected_revision.as_ref() != Some(revision)
        || guard.expected_proposal_digest.as_ref() != Some(digest)
    {
        return Err(ControlError::InvalidContract(
            "approval/application guard must bind exact proposal revision and digest".to_owned(),
        ));
    }
    Ok(())
}

fn required_sequence(guard: &OptimisticGuard) -> Result<RunSequence, ControlError> {
    guard.expected_run_sequence.ok_or_else(|| {
        ControlError::InvalidContract(
            "a mutating live command requires expected_run_sequence".to_owned(),
        )
    })
}

fn proposal_author(proposal: &WorkflowProposal) -> Result<AuthorRef, ControlError> {
    let digest = &proposal.digest().as_str()[3..35];
    Ok(AuthorRef::new(format!("proposal:{digest}"))?)
}

fn proposal_revision_reason(proposal: &WorkflowProposal) -> String {
    format!(
        "proposal_id={};proposal_digest={};proposer={};source={}",
        proposal.identity(),
        proposal.digest(),
        proposal.proposer(),
        provenance_summary(proposal.provenance())
    )
}

fn provenance_summary(provenance: &ProposalProvenance) -> String {
    match provenance {
        ProposalProvenance::Direct => "direct".to_owned(),
        ProposalProvenance::Process {
            capability,
            invocation,
        } => format!("process:{capability}:{invocation}"),
        ProposalProvenance::Model {
            capability,
            invocation,
            model_profile,
            context_manifest,
            response_artifact,
        } => format!(
            "model:{capability}:{invocation}:{model_profile}:context={}:response={}",
            context_manifest.digest(),
            response_artifact.digest()
        ),
    }
}

fn proposal_command_reason(
    document: &ControlCommandDocument,
    proposal: &WorkflowProposal,
    classification: &PolicyClassification,
) -> Result<Reason, ControlError> {
    Ok(Reason::new(format!(
        "{}; proposal_id={}; proposal_digest={}; risk={:?}; policy={}@{}; constraints={:?}",
        document.reason().as_str(),
        proposal.identity(),
        proposal.digest(),
        classification.risk,
        classification.policy,
        classification.policy_version,
        classification.constraints
    ))?)
}

fn merged_evidence(
    control: &[EvidenceReference],
    proposal: &[EvidenceReference],
) -> Vec<EvidenceReference> {
    let mut seen = BTreeSet::new();
    control
        .iter()
        .chain(proposal)
        .filter(|item| seen.insert(item.id.clone()))
        .cloned()
        .collect()
}

fn reconciliation_id_from_parts(
    proposal: &crate::ProposalId,
    proposed_revision: &RevisionId,
) -> Result<ReconciliationId, ControlError> {
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"milkdrift.proposal-reconciliation.v1\0");
    hasher.update(proposal.as_str().as_bytes());
    hasher.update(proposed_revision.as_str().as_bytes());
    Ok(ReconciliationId::new(format!(
        "proposal:{}",
        &hasher.finalize().to_hex().as_str()[..40]
    ))?)
}

fn derived_command_id(
    document: &ControlCommandDocument,
    phase: &str,
) -> Result<CommandId, ControlError> {
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"milkdrift.control-command-phase.v1\0");
    hasher.update(document.control_id().as_str().as_bytes());
    hasher.update(phase.as_bytes());
    Ok(CommandId::new(format!(
        "control:{}",
        &hasher.finalize().to_hex().as_str()[..40]
    ))?)
}

fn derived_resolution_decision(
    proposal: &WorkflowProposal,
) -> Result<ReconciliationDecisionId, ControlError> {
    Ok(ReconciliationDecisionId::new(format!(
        "proposal-action:{}",
        &proposal.digest().as_str()[3..35]
    ))?)
}

fn revision_budget(revision: &BlueprintRevision) -> AuthorityBudget {
    let mut duration_ms = None;
    let mut cost_minor = None;
    let mut invocations = 0_u64;
    for node in revision.semantic().nodes().values() {
        match node.kind() {
            NodeKind::Task { .. } => invocations = invocations.saturating_add(1),
            NodeKind::Repeat { config } => {
                invocations = invocations.saturating_add(u64::from(config.maximum_iterations()));
                duration_ms = duration_ms.max(config.budget().max_duration_ms);
                cost_minor = cost_minor.max(
                    config
                        .budget()
                        .max_cost_micros
                        .map(|value| value.saturating_add(9_999) / 10_000),
                );
            }
            _ => {}
        }
    }
    AuthorityBudget {
        cost_minor,
        duration_ms,
        invocations: (invocations > 0).then_some(invocations),
        artifact_bytes: None,
        units: None,
        concurrency: None,
    }
}
