//! Explicit-source completion and final discovery-bound validation.

use milkdrift_model::ContextSource;
use milkdrift_workspace::WorkspaceValueReference;

use super::DiscoveryState;
use crate::context::source::ContextBuildError;

impl DiscoveryState<'_, '_, '_> {
    pub(super) fn complete_explicit_sources(&mut self) -> Result<(), ContextBuildError> {
        for selector in self.request.policy.selected_workspace_values() {
            let reference: WorkspaceValueReference =
                serde_json::from_str(selector).map_err(|_| {
                    ContextBuildError::RequiredUnavailable("selected workspace reference")
                })?;
            let source = ContextSource::WorkspaceValue {
                reference: reference.clone(),
            };
            if !self
                .candidates
                .iter()
                .any(|candidate| candidate.source.as_ref() == Some(&source))
            {
                self.candidates.push(
                    self.source
                        .explicit_workspace_candidate(&self.request, reference)?,
                );
            }
        }
        for selector in self.request.policy.explicit_evidence() {
            let source: ContextSource = serde_json::from_str(selector).map_err(|_| {
                ContextBuildError::RequiredUnavailable("explicit evidence reference")
            })?;
            if self
                .candidates
                .iter()
                .any(|candidate| candidate.source.as_ref() == Some(&source))
            {
                continue;
            }
            self.candidates.push(self.source.explicit_candidate(
                &self.request,
                source,
                &self.executions,
                &self.attempts,
                &mut self.distances,
            )?);
        }
        Ok(())
    }

    pub(super) fn validate_required_sources(&self) -> Result<(), ContextBuildError> {
        for selected in self.request.policy.selected_executions() {
            if !self.candidates.iter().any(|candidate| {
                candidate
                    .execution
                    .as_ref()
                    .is_some_and(|execution| execution.as_str() == selected)
            }) {
                return Err(ContextBuildError::RequiredUnavailable(
                    "selected execution has no durable evidence",
                ));
            }
        }
        if self.candidates.len()
            > usize::try_from(self.request.policy.budget().max_candidate_records)
                .map_err(|_| ContextBuildError::AccountingOverflow)?
        {
            return Err(ContextBuildError::RequiredBudget("candidate count"));
        }
        Ok(())
    }
}
