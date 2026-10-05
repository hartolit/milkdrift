//! Authorized run inspection, including the shared controller account origin.
use super::{ControlService, attempt_inspection, status_from_projection};
use crate::{
    ControlCommandDocument, ControlError, NodeExecutionRead, OptimisticGuard, RunInspection,
};
use milkdrift_authority::{AuthorityBudget, AuthorityOperation, RequestedResourceFacts};
use milkdrift_workspace::{ArtifactId, CausalReference, RunId};

impl ControlService {
    pub(super) fn inspect_run_authorized(
        &self,
        document: &ControlCommandDocument,
        run: &RunId,
    ) -> Result<RunInspection, ControlError> {
        let mut value = self.inspect_run(run, document.guard())?;
        self.authorize_simple(
            document,
            AuthorityOperation::InspectRun,
            value.workflow.as_ref(),
            Some(run),
        )?;
        // A child-only read grant cannot disclose sibling use through a shared account.
        if let crate::ControllerAccountingRead::Active { account, .. } =
            &value.controller_accounting
        {
            let origin = account.declaration().controller_run();
            if origin != run {
                let parent = self.runtime.projection(origin)?;
                self.authorize_simple(
                    document,
                    AuthorityOperation::InspectRun,
                    parent.workflow(),
                    Some(origin),
                )?;
            }
        }
        for execution in &mut value.executions {
            self.filter_outputs(document, &mut execution.outputs)?;
            if let Some(attempt) = &mut execution.latest_attempt {
                self.filter_outputs(document, &mut attempt.outputs)?;
                if let Some(reference) = &attempt.context_manifest {
                    let artifact = ArtifactId::new(reference.identity().to_owned())
                        .map_err(|error| ControlError::InvalidContract(error.to_string()))?;
                    if !self.artifact_metadata_visible(document, &artifact)? {
                        attempt.context_manifest = None;
                        attempt.context_manifest_denied = true;
                    }
                }
            }
        }
        Ok(value)
    }

    fn filter_outputs(
        &self,
        document: &ControlCommandDocument,
        outputs: &mut Vec<milkdrift_runtime::PublishedNodeOutput>,
    ) -> Result<(), ControlError> {
        let mut visible = Vec::with_capacity(outputs.len());
        for output in outputs.drain(..) {
            if let Some(reference) = output.artifact()
                && !self.artifact_metadata_visible(document, reference.artifact())?
            {
                continue;
            }
            visible.push(output);
        }
        *outputs = visible;
        Ok(())
    }

    fn authorize_artifact_metadata(
        &self,
        document: &ControlCommandDocument,
        artifact: &ArtifactId,
    ) -> Result<(), ControlError> {
        let metadata = self.artifacts.metadata(artifact)?.ok_or_else(|| {
            ControlError::AuthorizationDenied {
                reasons: Vec::new(),
                decision_digest: None,
            }
        })?;
        let mut resources = RequestedResourceFacts::empty();
        resources.artifact = Some(artifact.clone());
        resources.artifact_sensitivity = Some(metadata.sensitivity());
        self.authorize(
            document,
            AuthorityOperation::ReadArtifactMetadata,
            resources,
            AuthorityBudget::default(),
        )
    }

    fn artifact_metadata_visible(
        &self,
        document: &ControlCommandDocument,
        artifact: &ArtifactId,
    ) -> Result<bool, ControlError> {
        match self.authorize_artifact_metadata(document, artifact) {
            Ok(()) => Ok(true),
            Err(ControlError::AuthorizationDenied { .. }) => Ok(false),
            Err(error) => Err(error),
        }
    }

    pub(super) fn authorize_timeline_artifacts(
        &self,
        document: &ControlCommandDocument,
        page: &crate::TimelinePage,
    ) -> Result<(), ControlError> {
        // This API returns exact journal facts. Refuse a protected page rather than
        // editing an event or presenting a filtered sequence as complete history.
        for event in &page.events {
            for reference in event.kind().required_artifacts()? {
                self.authorize_artifact_metadata(document, reference.artifact())?;
            }
            if let milkdrift_persistence::RunEventKind::ArtifactPublished { metadata } =
                event.kind()
            {
                for cause in std::iter::once(metadata.provenance().producer())
                    .chain(metadata.provenance().causes())
                {
                    self.authorize_causal_artifact(document, cause)?;
                }
            }
        }
        Ok(())
    }

    fn authorize_causal_artifact(
        &self,
        document: &ControlCommandDocument,
        cause: &CausalReference,
    ) -> Result<(), ControlError> {
        match cause {
            CausalReference::Artifact { reference } => {
                self.authorize_artifact_metadata(document, reference.artifact())
            }
            CausalReference::PeerClaim { reference, .. } => {
                self.authorize_causal_artifact(document, reference)
            }
            _ => Ok(()),
        }
    }

    pub(super) fn inspect_run(
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
            published_source: projection.published_source().cloned(),
            governing_agreement: projection.accepted_agreement().cloned(),
            agreement_adoptions: projection.agreement_adoptions(),
            controller_accounting: crate::ControllerAccountingRead::from_account(
                self.runtime.controller_account_for_run(run)?.as_ref(),
            )?,
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
}
