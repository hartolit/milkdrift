use std::{
    collections::{BTreeSet, VecDeque},
    sync::Arc,
};

use milkdrift_blueprint::{BlueprintRevision, Mutation, NodeKind, PinnedSubworkflow};
use milkdrift_capability::BoundedJson;
use milkdrift_persistence::{
    ControllerAccountBlock, ControllerAccountDeclaration, ControllerAccountState,
    ControllerAssessmentBoundary, ControllerAssessmentOutcome, ControllerResourceBudget,
    CurrencyCode, NodeExecutionId, RevisionStore,
};
use milkdrift_runtime::{
    ControllerAssessment, ControllerAssessmentContext, ControllerLifecycle, RunProjection,
    RuntimeError,
};

use super::{
    ControllerBound, ControllerLimits, ControllerPolicyDocument, ControllerProgress, ControllerStop,
};
use crate::ControlError;

/// Canonical integration owner for typed controller parsing, accounting, and assessment.
///
/// The production daemon leaves this owner uninstalled until cumulative controller resources are
/// reserved at the final external-entry boundary.
pub struct ControllerLifecycleOwner {
    revisions: Arc<dyn RevisionStore>,
}

impl ControllerLifecycleOwner {
    /// Constructs the lifecycle over the same immutable revision owner used by runtime/control.
    #[must_use]
    pub fn new(revisions: Arc<dyn RevisionStore>) -> Self {
        Self { revisions }
    }

    fn account_declaration(
        document: &ControllerPolicyDocument,
        run: &milkdrift_workspace::RunId,
        execution: &NodeExecutionId,
    ) -> Result<ControllerAccountDeclaration, ControlError> {
        let limits = document.policy().limits();
        let budget = ControllerResourceBudget::new(
            limits.max_cost_micros(),
            CurrencyCode::new(document.policy().cost_currency().as_str())?,
            limits.max_input_units(),
            limits.max_output_units(),
            limits.max_artifact_bytes(),
            u64::from(limits.max_process_invocations()),
            u64::from(limits.max_model_invocations()),
        )?;
        Ok(ControllerAccountDeclaration::new(
            run.clone(),
            execution.clone(),
            document.digest().as_str(),
            budget,
        )?)
    }

    /// Derives current progress from the durable account plus lifecycle-owned projection facts.
    pub fn progress(
        &self,
        document: &ControllerPolicyDocument,
        projection: &RunProjection,
        execution: &NodeExecutionId,
        account: Option<&ControllerAccountState>,
        observed_at_ms: u64,
    ) -> Result<ControllerProgress, ControlError> {
        let usage = projection.subworkflow_usage_for_execution(execution);
        let previous_assessment = projection.controller_assessment(execution);
        let started_at = projection
            .node_executions()
            .get(execution)
            .map(|execution| execution.created_at())
            .or_else(|| previous_assessment.map(|assessment| assessment.started_at()))
            .ok_or_else(|| {
                ControlError::InvalidContract(
                    "controller progress references neither an active execution nor a durable assessment"
                        .to_owned(),
                )
            })?;
        let totals = account
            .map(ControllerAccountState::committed_totals)
            .transpose()?
            .unwrap_or_default();
        let account_block = account.and_then(ControllerAccountState::blocked).cloned();
        let (unknown_cost_observations, unknown_input_observations, unknown_output_observations) =
            match account_block.as_ref() {
                Some(ControllerAccountBlock::UnknownUsage { dimension, .. }) => (
                    u32::from(dimension == "monetary_cost"),
                    u32::from(dimension == "input_units"),
                    u32::from(dimension == "output_units"),
                ),
                Some(
                    ControllerAccountBlock::ContractViolation { .. }
                    | ControllerAccountBlock::Integrity { .. },
                )
                | None => (0, 0, 0),
            };
        let mut progress = ControllerProgress {
            invocations: checked_u32(usage.map_or(0, |value| value.completed_children()))?,
            elapsed_ms: observed_at_ms.saturating_sub(started_at.get()),
            cost_micros: totals.cost_micros(),
            input_units: totals.input_units(),
            output_units: totals.output_units(),
            artifact_bytes: totals.artifact_bytes(),
            process_invocations: checked_u32(totals.process_admissions())?,
            model_invocations: checked_u32(totals.model_admissions())?,
            failures: checked_u16(usage.map_or(0, |value| value.failed_children()))?,
            revisions: checked_u32(projection.run_actor_revision_requests())?,
            rejections: checked_u16(projection.run_actor_rejections())?,
            unknown_cost_observations,
            unknown_input_observations,
            unknown_output_observations,
            account_block,
            ..ControllerProgress::default()
        };
        if let Some(previous) = previous_assessment {
            let previous: ControllerProgress =
                serde_json::from_value(previous.progress().value().clone())?;
            progress.checkpoint_approved_invocations = previous
                .checkpoint_approved_invocations
                .min(progress.invocations);
        }
        let (repeat_depth, child_depth) = self.body_shape(document.policy().body())?;
        progress.repeat_depth = repeat_depth;
        progress.child_depth = child_depth;
        Ok(progress)
    }

    /// Builds the bounded controller read model for one exact logical occurrence.
    pub fn status(
        &self,
        run: &milkdrift_workspace::RunId,
        projection: &RunProjection,
        execution: &NodeExecutionId,
        account: Option<&ControllerAccountState>,
        observed_at_ms: u64,
    ) -> Result<crate::ControllerStatusRead, ControlError> {
        let latest = projection.controller_assessment(execution);
        let execution_view = projection.current_node_execution(execution);
        let (governing_revision, controller_node, execution_completed) =
            if let Some(view) = execution_view {
                (
                    view.revision().clone(),
                    view.node().clone(),
                    matches!(
                        view.state(),
                        milkdrift_runtime::NodeExecutionState::Terminal(_)
                            | milkdrift_runtime::NodeExecutionState::CancelledBeforeDispatch
                            | milkdrift_runtime::NodeExecutionState::RemovedProspectively(_)
                    ),
                )
            } else if let Some(assessment) = latest {
                (
                    assessment.governing_revision().clone(),
                    assessment.controller_node().clone(),
                    matches!(
                        projection.lifecycle(),
                        milkdrift_runtime::RunLifecycle::Terminal(_)
                    ),
                )
            } else {
                return Err(ControlError::InvalidContract(
                "controller status references neither a current execution nor a durable assessment"
                    .to_owned(),
            ));
            };
        let revision = self
            .revisions
            .revision(&governing_revision)?
            .ok_or(ControlError::BaseRevisionNotFound)?;
        let document = ControllerPolicyDocument::from_revision(&revision, &controller_node)?
            .ok_or_else(|| {
                ControlError::InvalidContract(
                    "requested execution is not governed by a controller policy".to_owned(),
                )
            })?;
        let progress = self.progress(&document, projection, execution, account, observed_at_ms)?;
        let current = document.policy().limits().assess(&progress);
        let (checkpoint_id, reached_bound) = match latest.map(|value| value.outcome()) {
            Some(ControllerAssessmentOutcome::HumanCheckpoint { checkpoint_id }) => {
                (Some(checkpoint_id.clone()), None)
            }
            Some(ControllerAssessmentOutcome::BoundReached { bound, .. }) => {
                (None, bound_from_name(bound))
            }
            Some(ControllerAssessmentOutcome::Continue) | None if execution_completed => {
                (None, None)
            }
            Some(ControllerAssessmentOutcome::Continue) | None => match current {
                ControllerStop::HumanCheckpoint => (
                    Some(stable_controller_identity(
                        "checkpoint",
                        document.digest().as_str(),
                        run.as_str(),
                        execution.as_str(),
                        progress.invocations,
                    )),
                    None,
                ),
                ControllerStop::BoundReached { bound } => (None, Some(bound)),
                ControllerStop::Continue => (None, None),
            },
        };
        let state = if reached_bound.is_some() {
            crate::ControllerLifecycleState::BoundReached
        } else if checkpoint_id.is_some() {
            crate::ControllerLifecycleState::AwaitingHumanCheckpoint
        } else if execution_completed {
            crate::ControllerLifecycleState::Completed
        } else {
            crate::ControllerLifecycleState::Eligible
        };
        Ok(crate::ControllerStatusRead {
            controller: document.policy().identity().clone(),
            policy_digest: document.digest().clone(),
            run: run.clone(),
            revision: revision.id().clone(),
            node: controller_node,
            execution: execution.clone(),
            state,
            progress,
            limits: document.policy().limits().clone(),
            last_assessment_sequence: latest.map(|value| value.recorded_sequence()),
            last_assessment_time: latest.map(|value| value.recorded_at().get()),
            last_assessment_id: latest.map(|value| value.assessment_id().to_owned()),
            checkpoint_id,
            reached_bound,
            cycle_eligible: state == crate::ControllerLifecycleState::Eligible,
        })
    }

    /// Assesses exact decoded proposal size before the candidate revision is persisted.
    pub fn assess_proposal(
        &self,
        run: &milkdrift_workspace::RunId,
        projection: &RunProjection,
        proposal: &crate::WorkflowProposal,
        account: Option<&ControllerAccountState>,
        observed_at_ms: u64,
    ) -> Result<(), ControlError> {
        if proposal.run() != Some(run) {
            return Err(ControlError::InvalidContract(
                "controller proposal assessment run does not match the proposal".to_owned(),
            ));
        }
        let controller_actor = projection.execution_authority().map(|value| value.actor());
        if controller_actor != Some(proposal.proposer()) {
            return Ok(());
        }
        let mut controllers = Vec::new();
        for execution in projection.node_executions().values() {
            let revision = self
                .revisions
                .revision(execution.revision())?
                .ok_or(ControlError::BaseRevisionNotFound)?;
            if ControllerPolicyDocument::from_revision(&revision, execution.node())?.is_some() {
                controllers.push(execution.execution().clone());
            }
        }
        if controllers.len() > 1 {
            return Err(ControlError::InvalidContract(
                "controller-generated proposals are unsupported when one run has multiple active controllers; use an exact controller-scoped operation"
                    .to_owned(),
            ));
        }
        let Some(execution) = controllers.first() else {
            return Ok(());
        };
        let execution_view = projection
            .node_executions()
            .get(execution)
            .ok_or_else(|| ControlError::InvalidContract("controller disappeared".to_owned()))?;
        let revision = self
            .revisions
            .revision(execution_view.revision())?
            .ok_or(ControlError::BaseRevisionNotFound)?;
        let document = ControllerPolicyDocument::from_revision(&revision, execution_view.node())?
            .ok_or_else(|| {
            ControlError::InvalidContract("controller policy disappeared".to_owned())
        })?;
        let mut progress =
            self.progress(&document, projection, execution, account, observed_at_ms)?;
        progress.mutations_in_proposal =
            u16::try_from(proposal.mutation().operations().len()).unwrap_or(u16::MAX);
        progress.nodes_in_proposal = u16::try_from(
            proposal
                .mutation()
                .operations()
                .iter()
                .filter(|mutation| {
                    matches!(
                        mutation,
                        Mutation::AddNode { .. } | Mutation::InstantiateSubworkflow { .. }
                    )
                })
                .count(),
        )
        .unwrap_or(u16::MAX);
        match document.policy().limits().assess(&progress) {
            ControllerStop::Continue => Ok(()),
            ControllerStop::HumanCheckpoint => Err(ControlError::ProposalState(
                "controller proposal requires its durable human checkpoint first".to_owned(),
            )),
            ControllerStop::BoundReached { bound } => Err(ControlError::Bounds {
                location: format!("controller.proposal.{}", bound_name(bound)),
                reason: "controller proposal exceeds its immutable cumulative policy".to_owned(),
            }),
        }
    }

    /// Reassesses cumulative controller limits before approving or applying one
    /// controller-authored prospective revision. The proposal's immutable revision
    /// reason binds its proposer, so no model-supplied counter or authority claim is
    /// consumed here.
    pub fn assess_proposal_transition(
        &self,
        run: &milkdrift_workspace::RunId,
        projection: &RunProjection,
        proposed_revision: &BlueprintRevision,
        boundary: ControllerAssessmentBoundary,
        account: Option<&ControllerAccountState>,
        observed_at_ms: u64,
    ) -> Result<(), ControlError> {
        if !matches!(
            boundary,
            ControllerAssessmentBoundary::ProposalApproval
                | ControllerAssessmentBoundary::ProposalApplication
        ) {
            return Err(ControlError::InvalidContract(
                "proposal transition assessment requires an approval/application boundary"
                    .to_owned(),
            ));
        }
        let Some(controller_actor) = projection.execution_authority().map(|value| value.actor())
        else {
            return Ok(());
        };
        if !proposed_revision
            .reason()
            .contains(&format!("proposer={controller_actor};"))
        {
            return Ok(());
        }
        let mut controllers = Vec::new();
        for execution in projection.node_executions().values() {
            let revision = self
                .revisions
                .revision(execution.revision())?
                .ok_or(ControlError::BaseRevisionNotFound)?;
            if ControllerPolicyDocument::from_revision(&revision, execution.node())?.is_some() {
                controllers.push((
                    execution.execution().clone(),
                    revision,
                    execution.node().clone(),
                ));
            }
        }
        let [(execution, revision, node)] = controllers.as_slice() else {
            return if controllers.is_empty() {
                Ok(())
            } else {
                Err(ControlError::InvalidContract(
                    "controller-authored proposal transitions are unsupported when one run has multiple active controllers"
                        .to_owned(),
                ))
            };
        };
        let document =
            ControllerPolicyDocument::from_revision(revision, node)?.ok_or_else(|| {
                ControlError::InvalidContract("controller policy disappeared".to_owned())
            })?;
        let progress = self.progress(&document, projection, execution, account, observed_at_ms)?;
        match self.outcome(&document, &progress, boundary, run, execution)? {
            ControllerAssessmentOutcome::Continue => Ok(()),
            ControllerAssessmentOutcome::HumanCheckpoint { .. } => {
                Err(ControlError::ProposalState(
                    "controller proposal transition requires its durable human checkpoint first"
                        .to_owned(),
                ))
            }
            ControllerAssessmentOutcome::BoundReached { bound, .. } => Err(ControlError::Bounds {
                location: format!("controller.proposal.{bound}"),
                reason: "controller proposal transition reached its immutable cumulative policy"
                    .to_owned(),
            }),
        }
    }

    fn body_shape(&self, body: &PinnedSubworkflow) -> Result<(u16, u16), ControlError> {
        let mut pending = VecDeque::from([(body.revision().clone(), 1_u16, 1_u16)]);
        let mut visited = BTreeSet::new();
        let mut repeat_depth = 1_u16;
        let mut child_depth = 1_u16;
        while let Some((revision_id, repeats, children)) = pending.pop_front() {
            if !visited.insert(revision_id.clone()) {
                continue;
            }
            if visited.len() > 512 {
                return Err(ControlError::Bounds {
                    location: "controller.body_revision_graph".to_owned(),
                    reason: "controller body graph exceeds 512 immutable revisions".to_owned(),
                });
            }
            let revision = self
                .revisions
                .revision(&revision_id)?
                .ok_or(ControlError::BaseRevisionNotFound)?;
            for node in revision.semantic().nodes().values() {
                match node.kind() {
                    NodeKind::Task { .. } => {}
                    NodeKind::Repeat { config } => {
                        let next = repeats.checked_add(1).ok_or_else(|| {
                            ControlError::InvalidContract(
                                "controller repeat depth overflow".to_owned(),
                            )
                        })?;
                        repeat_depth = repeat_depth.max(next);
                        pending.push_back((config.body().revision().clone(), next, children));
                    }
                    NodeKind::Subworkflow { reference } => {
                        let next = children.checked_add(1).ok_or_else(|| {
                            ControlError::InvalidContract(
                                "controller child depth overflow".to_owned(),
                            )
                        })?;
                        child_depth = child_depth.max(next);
                        pending.push_back((reference.revision().clone(), repeats, next));
                    }
                    NodeKind::Reducer { .. }
                    | NodeKind::Branch { .. }
                    | NodeKind::Fork { .. }
                    | NodeKind::Join { .. }
                    | NodeKind::Wait { .. }
                    | NodeKind::SignalWait { .. }
                    | NodeKind::Terminal { .. } => {}
                }
            }
        }
        Ok((repeat_depth, child_depth))
    }

    fn outcome(
        &self,
        document: &ControllerPolicyDocument,
        progress: &ControllerProgress,
        boundary: ControllerAssessmentBoundary,
        run: &milkdrift_workspace::RunId,
        execution: &NodeExecutionId,
    ) -> Result<ControllerAssessmentOutcome, ControlError> {
        let stop = if boundary == ControllerAssessmentBoundary::CheckpointContinuation {
            let mut continued = progress.clone();
            continued.checkpoint_approved_invocations = continued.invocations;
            document.policy().limits().assess(&continued)
        } else {
            document.policy().limits().assess(progress)
        };
        match stop {
            ControllerStop::Continue => Ok(ControllerAssessmentOutcome::Continue),
            ControllerStop::HumanCheckpoint => Ok(ControllerAssessmentOutcome::HumanCheckpoint {
                checkpoint_id: stable_controller_identity(
                    "checkpoint",
                    document.digest().as_str(),
                    run.as_str(),
                    execution.as_str(),
                    progress.invocations,
                ),
            }),
            ControllerStop::BoundReached { bound } => {
                Ok(bound_outcome(document.policy().limits(), progress, bound))
            }
        }
    }
}

impl ControllerLifecycle for ControllerLifecycleOwner {
    fn assess(
        &self,
        context: &ControllerAssessmentContext<'_>,
    ) -> Result<Option<ControllerAssessment>, RuntimeError> {
        let document = ControllerPolicyDocument::from_revision(context.revision, context.node.id())
            .map_err(|error| RuntimeError::InvalidHistory(error.to_string()))?;
        let Some(document) = document else {
            return Ok(None);
        };
        let declaration = Self::account_declaration(&document, context.run, context.execution)
            .map_err(|error| RuntimeError::InvalidHistory(error.to_string()))?;
        match context.account {
            Some(account) if account.declaration() == &declaration => {}
            Some(_) => {
                return Err(RuntimeError::InvalidHistory(
                    "a controller policy inside an account-bound run does not match the originating controller occurrence"
                        .to_owned(),
                ));
            }
            None if context.boundary != ControllerAssessmentBoundary::Activation
                || context
                    .projection
                    .controller_assessment(context.execution)
                    .is_some() =>
            {
                return Err(RuntimeError::InvalidHistory(
                    "marked controller history has no exact durable account binding".to_owned(),
                ));
            }
            None => {}
        }
        let mut progress = self
            .progress(
                &document,
                context.projection,
                context.execution,
                context.account,
                context.observed_at.get(),
            )
            .map_err(|error| RuntimeError::InvalidHistory(error.to_string()))?;
        if context.boundary == ControllerAssessmentBoundary::CheckpointContinuation {
            progress.checkpoint_approved_invocations = progress.invocations;
        }
        let outcome = self
            .outcome(
                &document,
                &progress,
                context.boundary,
                context.run,
                context.execution,
            )
            .map_err(|error| RuntimeError::InvalidHistory(error.to_string()))?;
        let assessment_id = stable_controller_assessment_identity(
            document.digest().as_str(),
            context.run.as_str(),
            context.execution.as_str(),
            context.projection.sequence().get(),
            context.boundary,
            context.next_cycle,
        );
        let cycle_id = context.next_cycle.map(|cycle| {
            stable_controller_identity(
                "cycle",
                document.digest().as_str(),
                context.run.as_str(),
                context.execution.as_str(),
                cycle,
            )
        });
        let progress = BoundedJson::new(
            serde_json::to_value(progress)
                .map_err(|error| RuntimeError::InvalidHistory(error.to_string()))?,
        )
        .map_err(|error| RuntimeError::InvalidHistory(error.to_string()))?;
        Ok(Some(ControllerAssessment {
            controller_id: document.policy().identity().as_str().to_owned(),
            policy_digest: document.digest().as_str().to_owned(),
            assessment_id,
            cycle_id,
            progress,
            account_declaration: declaration,
            outcome,
        }))
    }
}

fn checked_u32(value: u64) -> Result<u32, ControlError> {
    u32::try_from(value).map_err(|_error| {
        ControlError::InvalidContract("controller u32 progress accounting overflow".to_owned())
    })
}

fn checked_u16(value: u64) -> Result<u16, ControlError> {
    u16::try_from(value).map_err(|_error| {
        ControlError::InvalidContract("controller u16 progress accounting overflow".to_owned())
    })
}

fn bound_outcome(
    limits: &ControllerLimits,
    progress: &ControllerProgress,
    bound: ControllerBound,
) -> ControllerAssessmentOutcome {
    let (current, limit, unknown_usage) = limits.bound_fact(progress, bound);
    let bound = match progress.account_block.as_ref() {
        Some(ControllerAccountBlock::ContractViolation { dimension, .. }) => {
            format!("contract_violation.{dimension}")
        }
        Some(ControllerAccountBlock::Integrity { .. }) => "account_integrity".to_owned(),
        Some(ControllerAccountBlock::UnknownUsage { .. }) | None => bound_name(bound).to_owned(),
    };
    ControllerAssessmentOutcome::BoundReached {
        bound,
        current,
        limit,
        unknown_usage,
    }
}

const fn bound_name(bound: ControllerBound) -> &'static str {
    match bound {
        ControllerBound::Invocations => "invocations",
        ControllerBound::Revisions => "revisions",
        ControllerBound::MutationsPerProposal => "mutations_per_proposal",
        ControllerBound::NodesPerProposal => "nodes_per_proposal",
        ControllerBound::ElapsedTime => "elapsed_time",
        ControllerBound::Cost => "cost",
        ControllerBound::InputUnits => "input_units",
        ControllerBound::OutputUnits => "output_units",
        ControllerBound::ArtifactBytes => "artifact_bytes",
        ControllerBound::ProcessInvocations => "process_invocations",
        ControllerBound::ModelInvocations => "model_invocations",
        ControllerBound::Failures => "failures",
        ControllerBound::Rejections => "rejections",
        ControllerBound::RepeatDepth => "repeat_depth",
        ControllerBound::ChildDepth => "child_depth",
        ControllerBound::HumanCheckpoint => "human_checkpoint",
        ControllerBound::AccountIntegrity => "account_integrity",
    }
}

fn bound_from_name(value: &str) -> Option<ControllerBound> {
    Some(match value {
        "invocations" => ControllerBound::Invocations,
        "revisions" => ControllerBound::Revisions,
        "mutations_per_proposal" => ControllerBound::MutationsPerProposal,
        "nodes_per_proposal" => ControllerBound::NodesPerProposal,
        "elapsed_time" => ControllerBound::ElapsedTime,
        "cost" => ControllerBound::Cost,
        "input_units" => ControllerBound::InputUnits,
        "output_units" => ControllerBound::OutputUnits,
        "artifact_bytes" => ControllerBound::ArtifactBytes,
        "process_invocations" => ControllerBound::ProcessInvocations,
        "model_invocations" => ControllerBound::ModelInvocations,
        "failures" => ControllerBound::Failures,
        "rejections" => ControllerBound::Rejections,
        "repeat_depth" => ControllerBound::RepeatDepth,
        "child_depth" => ControllerBound::ChildDepth,
        "human_checkpoint" => ControllerBound::HumanCheckpoint,
        "account_integrity" => ControllerBound::AccountIntegrity,
        value if value.starts_with("contract_violation.") => {
            return bound_from_name(value.trim_start_matches("contract_violation."));
        }
        _ => return None,
    })
}

fn stable_controller_identity(
    domain: &str,
    policy_digest: &str,
    run: &str,
    execution: &str,
    number: impl Into<u64>,
) -> String {
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"milkdrift.controller-lifecycle.v1\0");
    controller_hash_field(&mut hasher, domain.as_bytes());
    controller_hash_field(&mut hasher, policy_digest.as_bytes());
    controller_hash_field(&mut hasher, run.as_bytes());
    controller_hash_field(&mut hasher, execution.as_bytes());
    hasher.update(&number.into().to_be_bytes());
    format!("controller-{}", &hasher.finalize().to_hex().as_str()[..40])
}

fn stable_controller_assessment_identity(
    policy_digest: &str,
    run: &str,
    execution: &str,
    through_sequence: u64,
    boundary: ControllerAssessmentBoundary,
    next_cycle: Option<u32>,
) -> String {
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"milkdrift.controller-assessment.v1\0");
    controller_hash_field(&mut hasher, policy_digest.as_bytes());
    controller_hash_field(&mut hasher, run.as_bytes());
    controller_hash_field(&mut hasher, execution.as_bytes());
    hasher.update(&through_sequence.to_be_bytes());
    controller_hash_field(&mut hasher, assessment_boundary_name(boundary).as_bytes());
    match next_cycle {
        Some(cycle) => {
            hasher.update(&[1]);
            hasher.update(&cycle.to_be_bytes());
        }
        None => {
            hasher.update(&[0]);
        }
    }
    format!("controller-{}", &hasher.finalize().to_hex().as_str()[..40])
}

fn controller_hash_field(hasher: &mut blake3::Hasher, bytes: &[u8]) {
    hasher.update(&u64::try_from(bytes.len()).unwrap_or(u64::MAX).to_be_bytes());
    hasher.update(bytes);
}

const fn assessment_boundary_name(boundary: ControllerAssessmentBoundary) -> &'static str {
    match boundary {
        ControllerAssessmentBoundary::Activation => "activation",
        ControllerAssessmentBoundary::CycleEntry => "cycle_entry",
        ControllerAssessmentBoundary::CheckpointContinuation => "checkpoint_continuation",
        ControllerAssessmentBoundary::ProposalAcceptance => "proposal_acceptance",
        ControllerAssessmentBoundary::ProposalApproval => "proposal_approval",
        ControllerAssessmentBoundary::ProposalApplication => "proposal_application",
    }
}
