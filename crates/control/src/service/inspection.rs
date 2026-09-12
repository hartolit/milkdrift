//! Authorized run inspection, including the shared controller account origin.
use super::{ControlService, attempt_inspection, status_from_projection};
use crate::{
    ControlCommandDocument, ControlError, NodeExecutionRead, OptimisticGuard, RunInspection,
};
use milkdrift_authority::AuthorityOperation;
use milkdrift_workspace::RunId;

impl ControlService {
    pub(super) fn inspect_run_authorized(
        &self,
        document: &ControlCommandDocument,
        run: &RunId,
    ) -> Result<RunInspection, ControlError> {
        let value = self.inspect_run(run, document.guard())?;
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
        Ok(value)
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
