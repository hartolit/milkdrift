//! Branch, join, and subworkflow exposure under exact historical ownership.

use milkdrift_model::ContextProducerFact;
use milkdrift_persistence::{BranchResultReference, NodeExecutionId, RunEventEnvelope};
use milkdrift_workspace::{SubworkflowId, WorkspaceValueReference};

use super::DiscoveryState;
use crate::context::source::{
    ContextBuildError, candidate_references_join_output, record_subworkflow_parent,
};

impl DiscoveryState<'_, '_, '_> {
    pub(super) fn seed_projection_join_exposure(&mut self) {
        for join in self.request.projection.joins().values() {
            if self.join_is_visible(join.execution()) {
                self.join_exposed_values.extend(
                    join.branches()
                        .iter()
                        .flat_map(|branch| branch.outputs.iter().cloned()),
                );
            }
        }
    }

    pub(super) fn expose_join_outputs(
        &mut self,
        execution: &NodeExecutionId,
        branches: &[BranchResultReference],
    ) {
        if self.join_is_visible(execution) {
            self.join_exposed_values.extend(
                branches
                    .iter()
                    .flat_map(|branch| branch.outputs.iter().cloned()),
            );
        }
    }

    pub(super) fn record_subworkflow_parent(
        &mut self,
        subworkflow: &SubworkflowId,
        parent_execution: &NodeExecutionId,
    ) -> Result<(), ContextBuildError> {
        record_subworkflow_parent(&mut self.subworkflow_parents, subworkflow, parent_execution)
    }

    pub(super) fn import_subworkflow_output(
        &mut self,
        event: &RunEventEnvelope,
        subworkflow: &SubworkflowId,
        parent_value: &WorkspaceValueReference,
    ) -> Result<(), ContextBuildError> {
        let execution_fact = self
            .subworkflow_parents
            .get(subworkflow)
            .and_then(|execution| self.executions.get(execution))
            .ok_or(ContextBuildError::RequiredUnavailable(
                "subworkflow parent provenance",
            ))?;
        self.candidates.push(self.source.output_candidate(
            &self.request,
            execution_fact,
            None,
            parent_value,
            None,
            event,
            ContextProducerFact::default(),
            true,
            &mut self.distances,
        )?);
        Ok(())
    }

    pub(super) fn apply_join_exposure(&mut self) {
        for candidate in &mut self.candidates {
            if candidate_references_join_output(candidate, &self.join_exposed_values) {
                candidate.exposed_across_scope = true;
            }
        }
    }

    fn join_is_visible(&self, execution: &NodeExecutionId) -> bool {
        self.executions.get(execution).is_some_and(|execution| {
            execution.revision == *self.request.revision.id()
                && self.all_ancestors.contains_key(&execution.node)
        })
    }
}
