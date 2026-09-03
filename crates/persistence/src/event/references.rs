use milkdrift_capability::TerminalStatus;
use milkdrift_workspace::{
    ArtifactId, ArtifactReference, CausalReference, ContentDigest as ArtifactContentDigest,
    MediaType, RunId, WorkspaceScope, WorkspaceValueReference,
};

use crate::{EvidenceReference, PersistenceError, bounded::MAX_EVIDENCE_REFERENCES};

use super::{
    MAX_RECONCILIATION_PLAN_ITEMS, MAX_REFERENCES_PER_EVENT,
    MAX_REPEAT_CONTINUATION_ADDITIONAL_ITERATIONS, MAX_REPEAT_EFFECTIVE_ITERATIONS,
    kind::RunEventKind,
    model::{
        AuthorityDecision, ControllerAssessmentOutcome, JoinRule, NodeOutcome,
        ReconciliationClassification, RepeatContinuationCause, RepeatContinuationDecision,
        RunOutcome,
    },
};

impl RunEventKind {
    /// Derives every complete content-addressed artifact reference retained by this fact.
    ///
    /// The atomic journal uses this as the sole event-side ownership source. Executor
    /// requests may carry provider-neutral artifact references, but durable history
    /// requires their media type and exact size so verification and workspace accounting
    /// cannot be bypassed by a direct blueprint artifact binding.
    pub fn required_artifacts(&self) -> Result<Vec<ArtifactReference>, PersistenceError> {
        match self {
            Self::RunTerminal { artifacts, .. } => Ok(artifacts.clone()),
            Self::NodeScheduled { request, .. } => {
                let mut references = request
                    .inputs()
                    .iter()
                    .filter_map(|input| input.value().artifact())
                    .map(workspace_artifact_reference)
                    .collect::<Result<Vec<_>, _>>()?;
                if let Some(manifest) = request.context_manifest() {
                    references.push(workspace_artifact_reference(manifest)?);
                }
                Ok(references)
            }
            Self::NodeOutputPublished {
                artifact: Some(reference),
                ..
            } => Ok(vec![reference.clone()]),
            Self::DeterministicOutputPublished {
                artifact: Some(reference),
                ..
            } => Ok(vec![reference.clone()]),
            Self::ArtifactPublished { metadata } => {
                let mut references = vec![metadata.reference().clone()];
                for causal in std::iter::once(metadata.provenance().producer())
                    .chain(metadata.provenance().causes())
                {
                    if let CausalReference::Artifact { reference } = causal {
                        references.push(reference.clone());
                    }
                }
                Ok(references)
            }
            _ => Ok(Vec::new()),
        }
    }

    pub(crate) fn validate_for_run(&self, run: &RunId) -> Result<(), PersistenceError> {
        if matches!(
            self,
            Self::NodeProgressRecorded {
                report_sequence: 0,
                ..
            } | Self::NodeOutputPublished {
                report_sequence: 0,
                ..
            } | Self::NodeTerminal {
                report_sequence: 0,
                ..
            } | Self::ExternalOutcomeUncertain {
                report_sequence: 0,
                ..
            } | Self::LateTerminalEvidenceRecorded {
                report_sequence: 0,
                ..
            }
        ) {
            return Err(PersistenceError::InvalidDocument(
                "executor report sequences are one-based".to_owned(),
            ));
        }
        let check_references = |location: &'static str, count: usize| {
            if count > MAX_REFERENCES_PER_EVENT {
                Err(PersistenceError::Bounds {
                    location,
                    reason: format!("at most {MAX_REFERENCES_PER_EVENT} references are allowed"),
                })
            } else {
                Ok(())
            }
        };
        let check_evidence = |evidence: &[EvidenceReference]| {
            if evidence.len() > MAX_EVIDENCE_REFERENCES {
                Err(PersistenceError::Bounds {
                    location: "event.evidence",
                    reason: format!("at most {MAX_EVIDENCE_REFERENCES} references are allowed"),
                })
            } else if evidence
                .iter()
                .map(|item| &item.id)
                .collect::<std::collections::BTreeSet<_>>()
                .len()
                != evidence.len()
            {
                Err(PersistenceError::InvalidDocument(
                    "event evidence identities must be distinct".to_owned(),
                ))
            } else {
                Ok(())
            }
        };

        match self {
            Self::RunCreated { inputs, .. } | Self::SubworkflowCreated { inputs, .. } => {
                check_references("event.inputs", inputs.len())?;
                if inputs
                    .iter()
                    .collect::<std::collections::BTreeSet<_>>()
                    .len()
                    != inputs.len()
                {
                    return Err(PersistenceError::InvalidDocument(
                        "event input references must be distinct".to_owned(),
                    ));
                }
            }
            Self::RunTerminal {
                outputs, artifacts, ..
            } => {
                check_references("event.outputs", outputs.len())?;
                check_references("event.artifacts", artifacts.len())?;
                if outputs
                    .iter()
                    .collect::<std::collections::BTreeSet<_>>()
                    .len()
                    != outputs.len()
                    || artifacts
                        .iter()
                        .collect::<std::collections::BTreeSet<_>>()
                        .len()
                        != artifacts.len()
                {
                    return Err(PersistenceError::InvalidDocument(
                        "terminal output and artifact references must be distinct".to_owned(),
                    ));
                }
            }
            Self::RunTerminationRequested { outcome, .. } if *outcome != RunOutcome::Failed => {
                return Err(PersistenceError::InvalidDocument(
                    "internal run termination currently supports only an explicit failed outcome"
                        .to_owned(),
                ));
            }
            Self::SignalBroadcastScanAdvanced {
                through_execution: None,
                complete: false,
                ..
            } => {
                return Err(PersistenceError::InvalidDocument(
                    "an incomplete broadcast scan must advance through one wait execution"
                        .to_owned(),
                ));
            }
            Self::RecoveryDecisionRecorded {
                outcome: AuthorityDecision::ResolveSucceeded | AuthorityDecision::ResolveFailed,
                evidence,
                ..
            } if evidence.is_empty() => {
                return Err(PersistenceError::InvalidDocument(
                    "terminal external-work resolution requires at least one evidence reference"
                        .to_owned(),
                ));
            }
            Self::RunPaused { evidence, .. }
            | Self::RunResumed { evidence, .. }
            | Self::RunCancellationRequested { evidence, .. }
            | Self::ExternalOutcomeUncertain { evidence, .. }
            | Self::ReconciliationDecisionRecorded { evidence, .. }
            | Self::RecoveryDecisionRecorded { evidence, .. } => check_evidence(evidence)?,
            Self::RepeatContinuationRequested {
                initial_iteration_limit,
                effective_iteration_limit,
                cause,
                ..
            } => {
                let limits_valid = *initial_iteration_limit > 0
                    && *initial_iteration_limit <= *effective_iteration_limit
                    && *effective_iteration_limit <= MAX_REPEAT_EFFECTIVE_ITERATIONS;
                let cause_valid = match cause {
                    RepeatContinuationCause::IterationLimit => true,
                    RepeatContinuationCause::DurationBudget {
                        maximum_ms,
                        observed_ms,
                    } => *maximum_ms > 0 && observed_ms >= maximum_ms,
                    RepeatContinuationCause::CostBudget {
                        maximum_micros,
                        observed_micros,
                        ..
                    } => *maximum_micros > 0 && observed_micros >= maximum_micros,
                    RepeatContinuationCause::ControllerCheckpoint {
                        checkpoint_id,
                        completed_cycles,
                    } => {
                        !checkpoint_id.is_empty()
                            && checkpoint_id.len() <= 192
                            && *completed_cycles > 0
                    }
                };
                if !limits_valid || !cause_valid {
                    return Err(PersistenceError::InvalidDocument(format!(
                        "repeat continuation request requires limits within 1..={MAX_REPEAT_EFFECTIVE_ITERATIONS} and a truthfully exhausted typed cause"
                    )));
                }
            }
            Self::RepeatContinuationDecided {
                outcome,
                approved_additional_iterations,
                evidence,
                ..
            } => {
                check_evidence(evidence)?;
                let valid = match (outcome, approved_additional_iterations) {
                    (RepeatContinuationDecision::Approved, Some(additional)) => {
                        (1..=MAX_REPEAT_CONTINUATION_ADDITIONAL_ITERATIONS).contains(additional)
                    }
                    (RepeatContinuationDecision::Rejected, None) => true,
                    (RepeatContinuationDecision::Approved, None)
                    | (RepeatContinuationDecision::Rejected, Some(_)) => false,
                };
                if !valid {
                    return Err(PersistenceError::InvalidDocument(format!(
                        "repeat approval requires 1..={MAX_REPEAT_CONTINUATION_ADDITIONAL_ITERATIONS} additional iterations and rejection forbids them"
                    )));
                }
            }
            Self::ControllerAssessmentRecorded {
                controller_id,
                policy_digest,
                assessment_id,
                cycle_id,
                through_sequence: _,
                outcome,
                ..
            } => {
                let bounded_identity = |value: &str| !value.is_empty() && value.len() <= 192;
                let outcome_valid = match outcome {
                    ControllerAssessmentOutcome::Continue => true,
                    ControllerAssessmentOutcome::HumanCheckpoint { checkpoint_id } => {
                        bounded_identity(checkpoint_id)
                    }
                    ControllerAssessmentOutcome::BoundReached {
                        bound,
                        current,
                        limit,
                        unknown_usage,
                    } => {
                        bounded_identity(bound)
                            && *limit > 0
                            && if bound == "account_integrity" {
                                current.is_none() && !unknown_usage
                            } else {
                                *unknown_usage == current.is_none()
                            }
                    }
                };
                if !bounded_identity(controller_id)
                    || !bounded_identity(policy_digest)
                    || !bounded_identity(assessment_id)
                    || cycle_id
                        .as_deref()
                        .is_some_and(|value| !bounded_identity(value))
                    || !outcome_valid
                {
                    return Err(PersistenceError::InvalidDocument(
                        "controller assessment identities, sequence, or outcome are invalid"
                            .to_owned(),
                    ));
                }
            }
            Self::NodeScheduled {
                invocation,
                idempotency_key,
                request,
                ..
            } if request.invocation() != invocation
                || request.idempotency_key() != idempotency_key.as_ref() =>
            {
                return Err(PersistenceError::InvalidDocument(
                    "scheduled invocation/idempotency facts contradict the persisted request"
                        .to_owned(),
                ));
            }
            Self::CapabilityResolved {
                requirement,
                snapshot,
                ..
            } => {
                requirement
                    .validate()
                    .map_err(|error| PersistenceError::InvalidDocument(error.to_string()))?;
                if requirement.operation() != snapshot.operation()
                    || requirement
                        .exact_capability()
                        .is_some_and(|identity| identity != snapshot.capability())
                    || requirement
                        .provider_profile_ref()
                        .is_some_and(|profile| Some(profile) != snapshot.provider_profile())
                {
                    return Err(PersistenceError::InvalidDocument(
                        "resolved capability snapshot contradicts the recorded requirement"
                            .to_owned(),
                    ));
                }
            }
            Self::CapabilityResolutionDecisionRecorded {
                attempt,
                snapshot,
                authorization,
                ..
            } if !authorization.is_allowed()
                || authorization.request().resources.capability.as_ref()
                    != Some(snapshot.capability())
                || authorization
                    .request()
                    .resources
                    .capability_operation
                    .as_ref()
                    != Some(snapshot.operation())
                || authorization.request().resources.provider_profile.as_ref()
                    != snapshot.provider_profile()
                || authorization.request().provenance.descriptor_revision
                    != Some(snapshot.descriptor_revision())
                || authorization.request().provenance.attempt.as_deref()
                    != Some(attempt.as_str()) =>
            {
                return Err(PersistenceError::InvalidDocument(
                    "capability authorization does not bind the resolved generation".to_owned(),
                ));
            }
            Self::NodeProgressRecorded {
                completed_units,
                total_units: Some(total),
                ..
            } if completed_units.is_some_and(|completed| completed > *total) => {
                return Err(PersistenceError::InvalidDocument(
                    "completed progress units exceed total units".to_owned(),
                ));
            }
            Self::LateTerminalEvidenceRecorded { terminal, .. }
                if terminal.status() == TerminalStatus::Uncertain =>
            {
                return Err(PersistenceError::InvalidDocument(
                    "late terminal evidence must add a known terminal observation".to_owned(),
                ));
            }
            Self::NodeRetryScheduled {
                attempt_number: 0, ..
            }
            | Self::RepeatIterationCreated {
                iteration_number: 0,
                ..
            } => {
                return Err(PersistenceError::InvalidDocument(
                    "attempt and iteration numbers are one-based".to_owned(),
                ));
            }
            Self::NodeTerminal {
                outcome,
                error_class,
                ..
            }
            | Self::DeterministicNodeTerminal {
                outcome,
                error_class,
                ..
            } if matches!(outcome, NodeOutcome::Failed | NodeOutcome::Rejected)
                != error_class.is_some() =>
            {
                return Err(PersistenceError::InvalidDocument(
                    "node failure/rejection requires an error class and success/cancellation forbids one"
                        .to_owned(),
                ));
            }
            Self::AttemptUsageRecorded { usage, .. }
                if usage.input_units.is_none()
                    && usage.output_units.is_none()
                    && usage.duration_ms.is_none()
                    && usage.cost.is_none() =>
            {
                return Err(PersistenceError::InvalidDocument(
                    "an attempt usage fact must contain at least one observation".to_owned(),
                ));
            }
            Self::JoinSatisfied {
                rule: JoinRule::Quorum { required: 0 },
                ..
            } => {
                return Err(PersistenceError::InvalidDocument(
                    "join quorum must be greater than zero".to_owned(),
                ));
            }
            Self::JoinSatisfied {
                branches,
                retained_branches,
                rule,
                ..
            } => {
                check_references("event.branches", branches.len())?;
                check_references("event.retained_branches", retained_branches.len())?;
                for branch in branches {
                    check_references("event.branch.outputs", branch.outputs.len())?;
                    if branch
                        .outputs
                        .iter()
                        .collect::<std::collections::BTreeSet<_>>()
                        .len()
                        != branch.outputs.len()
                    {
                        return Err(PersistenceError::InvalidDocument(
                            "branch output references must be distinct".to_owned(),
                        ));
                    }
                }
                let branch_ids: std::collections::BTreeSet<_> =
                    branches.iter().map(|branch| &branch.branch).collect();
                let retained_ids: std::collections::BTreeSet<_> =
                    retained_branches.iter().collect();
                let successful = branches
                    .iter()
                    .filter(|branch| branch.outcome == RunOutcome::Succeeded)
                    .count();
                let rule_satisfied = match rule {
                    JoinRule::All | JoinRule::AnyCompletion => !branches.is_empty(),
                    JoinRule::FirstSuccess => successful > 0,
                    JoinRule::Quorum { required } => {
                        usize::try_from(*required).is_ok_and(|required| successful >= required)
                    }
                };
                if branch_ids.len() != branches.len()
                    || retained_ids.len() != retained_branches.len()
                    || !branch_ids.is_disjoint(&retained_ids)
                    || !rule_satisfied
                {
                    return Err(PersistenceError::InvalidDocument(
                        "join results must use distinct branches and truthfully satisfy the recorded rule"
                            .to_owned(),
                    ));
                }
            }
            Self::SubworkflowTerminal { outputs, .. } | Self::BranchTerminal { outputs, .. } => {
                check_references("event.outputs", outputs.len())?;
                if outputs
                    .iter()
                    .collect::<std::collections::BTreeSet<_>>()
                    .len()
                    != outputs.len()
                {
                    return Err(PersistenceError::InvalidDocument(
                        "branch/subworkflow output references must be distinct".to_owned(),
                    ));
                }
            }
            Self::ReconciliationPlanRecorded { items, .. } => {
                if items.len() > MAX_RECONCILIATION_PLAN_ITEMS {
                    return Err(PersistenceError::Bounds {
                        location: "event.reconciliation.items",
                        reason: format!(
                            "at most {MAX_RECONCILIATION_PLAN_ITEMS} items are allowed so actions, application, and revision pin fit one atomic commit"
                        ),
                    });
                }
                let unique: std::collections::BTreeSet<_> = items
                    .iter()
                    .map(|item| (item.node.as_ref(), item.execution.as_ref()))
                    .collect();
                let invalid_identity = items.iter().any(|item| {
                    item.node.is_none()
                        && item.execution.is_none()
                        && item.classification
                            != ReconciliationClassification::IncompatibleInterfaceOrSubworkflow
                });
                if unique.len() != items.len() || invalid_identity {
                    return Err(PersistenceError::InvalidDocument(
                        "reconciliation items must be distinct; only workflow-interface incompatibilities may omit both node and execution"
                            .to_owned(),
                    ));
                }
            }
            _ => {}
        }
        // Validate provider-neutral artifact references even before a commit request
        // derives its exact ownership/accounting set.
        let _ = self.required_artifacts()?;
        self.validate_workspace_run(run)?;
        Ok(())
    }

    fn validate_workspace_run(&self, run: &RunId) -> Result<(), PersistenceError> {
        let value_in_run = |value: &WorkspaceValueReference| value.scope().run() == run;
        let scope_in_run = |scope: &WorkspaceScope| scope.reference().run() == run;
        let valid = match self {
            Self::RunCreated {
                root_scope, inputs, ..
            } => {
                scope_in_run(root_scope)
                    && root_scope.kind().is_run_root()
                    && inputs.iter().all(value_in_run)
            }
            Self::RunTerminal { outputs, .. } => outputs.iter().all(value_in_run),
            Self::NodeBecameEligible { scope, .. } => scope.run() == run,
            Self::NodeOutputPublished { value, .. } => value_in_run(value),
            Self::DeterministicOutputPublished { value, .. } => value_in_run(value),
            Self::BranchScopeCreated { scope, .. } | Self::RepeatIterationCreated { scope, .. } => {
                scope_in_run(scope)
            }
            Self::BranchTerminal { outputs, .. } => outputs.iter().all(value_in_run),
            Self::JoinSatisfied { branches, .. } => branches
                .iter()
                .all(|branch| branch.scope.run() == run && branch.outputs.iter().all(value_in_run)),
            Self::SubworkflowCreated {
                child_run,
                scope,
                inputs,
                ..
            } => child_run != run && scope_in_run(scope) && inputs.iter().all(value_in_run),
            Self::SubworkflowTerminal {
                child_run, outputs, ..
            } => child_run != run && outputs.iter().all(|value| value.scope().run() == child_run),
            Self::SubworkflowOutputImported {
                child_value,
                parent_value,
                ..
            } => {
                child_value.scope().run() != run
                    && parent_value.scope().run() == run
                    && child_value.scope().run() != parent_value.scope().run()
            }
            Self::SubworkflowCancellationRequested { child_run, .. } => child_run != run,
            Self::ReconciliationRemediationCreated { scope, .. }
            | Self::RemediationWorkCreated { scope, .. } => scope.run() == run,
            _ => true,
        };
        if valid {
            Ok(())
        } else {
            Err(PersistenceError::InvalidDocument(
                "workspace scopes/value references in an event must belong to its run aggregate"
                    .to_owned(),
            ))
        }
    }
}

fn workspace_artifact_reference(
    reference: &milkdrift_capability::ArtifactReference,
) -> Result<ArtifactReference, PersistenceError> {
    let media_type = reference.media_type().ok_or_else(|| {
        PersistenceError::InvalidDocument(
            "scheduled artifact input requires an exact media type".to_owned(),
        )
    })?;
    let size_bytes = reference.size_bytes().ok_or_else(|| {
        PersistenceError::InvalidDocument(
            "scheduled artifact input requires an exact byte size".to_owned(),
        )
    })?;
    let artifact = ArtifactId::new(reference.identity()).map_err(|error| {
        PersistenceError::InvalidDocument(format!(
            "scheduled artifact input has an invalid identity: {error}"
        ))
    })?;
    let digest = ArtifactContentDigest::from_hex(reference.digest()).map_err(|error| {
        PersistenceError::InvalidDocument(format!(
            "scheduled artifact input has an invalid digest: {error}"
        ))
    })?;
    let media_type = MediaType::new(media_type).map_err(|error| {
        PersistenceError::InvalidDocument(format!(
            "scheduled artifact input has an invalid media type: {error}"
        ))
    })?;
    Ok(ArtifactReference::new(
        artifact, digest, media_type, size_bytes,
    ))
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use crate::{AttemptId, NodeExecutionId};
    use milkdrift_blueprint::NodeId;
    use milkdrift_capability::{
        ArtifactReference as CapabilityArtifactReference, CapabilityId, InputReference,
        InvocationId, InvocationRequest, InvocationValueReference, OperationId,
    };

    use super::*;

    fn scheduled_with_artifact(
        media_type: Option<String>,
        size_bytes: Option<u64>,
    ) -> Result<RunEventKind, Box<dyn std::error::Error>> {
        let invocation = InvocationId::new("invocation-artifact")?;
        let input = InputReference::new(
            "source",
            InvocationValueReference::Artifact {
                reference: CapabilityArtifactReference::new(
                    "artifact-source",
                    "a".repeat(64),
                    media_type,
                    size_bytes,
                )?,
            },
        )?;
        let request = InvocationRequest::new(
            invocation.clone(),
            CapabilityId::new("artifact-consumer")?,
            OperationId::new("artifact.consume")?,
            None,
            None,
            vec![input],
            BTreeMap::new(),
        )?;
        Ok(RunEventKind::NodeScheduled {
            node: NodeId::new("consume")?,
            execution: NodeExecutionId::new("execution-consume")?,
            attempt: AttemptId::new("attempt-consume")?,
            invocation,
            idempotency_key: None,
            request,
        })
    }

    #[test]
    fn scheduled_artifact_inputs_are_exact_atomic_ownership_requirements()
    -> Result<(), Box<dyn std::error::Error>> {
        let artifacts =
            scheduled_with_artifact(Some("application/octet-stream".to_owned()), Some(7))?
                .required_artifacts()?;
        assert_eq!(artifacts.len(), 1);
        assert_eq!(artifacts[0].artifact().as_str(), "artifact-source");
        assert_eq!(artifacts[0].digest().to_hex(), "a".repeat(64));
        assert_eq!(
            artifacts[0].media_type().as_str(),
            "application/octet-stream"
        );
        assert_eq!(artifacts[0].size_bytes(), 7);

        assert!(
            scheduled_with_artifact(None, Some(7))?
                .required_artifacts()
                .is_err()
        );
        assert!(
            scheduled_with_artifact(Some("application/octet-stream".to_owned()), None)?
                .required_artifacts()
                .is_err()
        );
        Ok(())
    }

    #[test]
    fn scheduled_context_manifest_is_an_exact_atomic_ownership_requirement()
    -> Result<(), Box<dyn std::error::Error>> {
        let invocation = InvocationId::new("invocation-context")?;
        let request = InvocationRequest::new(
            invocation.clone(),
            CapabilityId::new("model-provider")?,
            OperationId::new("model.generate")?,
            None,
            None,
            Vec::new(),
            BTreeMap::new(),
        )?
        .with_context_manifest(CapabilityArtifactReference::new(
            "artifact-context",
            "b".repeat(64),
            Some("application/vnd.milkdrift.context-manifest.v2+json".to_owned()),
            Some(42),
        )?)?;
        let event = RunEventKind::NodeScheduled {
            node: NodeId::new("generate")?,
            execution: NodeExecutionId::new("execution-generate")?,
            attempt: AttemptId::new("attempt-generate")?,
            invocation,
            idempotency_key: None,
            request,
        };

        let artifacts = event.required_artifacts()?;
        assert_eq!(artifacts.len(), 1);
        assert_eq!(artifacts[0].artifact().as_str(), "artifact-context");
        assert_eq!(artifacts[0].digest().to_hex(), "b".repeat(64));
        assert_eq!(
            artifacts[0].media_type().as_str(),
            "application/vnd.milkdrift.context-manifest.v2+json"
        );
        assert_eq!(artifacts[0].size_bytes(), 42);
        Ok(())
    }
}
