//! Ordered durable context discovery coordinated by one private phase state.

use std::collections::{BTreeMap, BTreeSet};

use milkdrift_blueprint::RevisionId;
use milkdrift_persistence::{EventCursor, NodeExecutionId, RunSequence};
use milkdrift_workspace::{SubworkflowId, WorkspaceValueReference};

use super::{
    AttemptFact, ContextBuildError, ContextCandidate, ContextCandidateSource, ContextSourceRequest,
    DurableContextCandidateSource, ExecutionFact, ancestor_depths, candidate_tail_start,
    projected_attempt_fact,
};

mod completion;
mod event;
mod exposure;
mod journal;
mod projection;

impl ContextCandidateSource for DurableContextCandidateSource<'_> {
    fn discover(
        &self,
        request: ContextSourceRequest<'_>,
    ) -> Result<Vec<ContextCandidate>, ContextBuildError> {
        DiscoveryState::new(self, request).discover()
    }
}

struct DiscoveryState<'source, 'store, 'request> {
    source: &'source DurableContextCandidateSource<'store>,
    request: ContextSourceRequest<'request>,
    candidates: Vec<ContextCandidate>,
    maximum_records: u32,
    cursor: Option<EventCursor>,
    scanned: u32,
    event_summaries: u32,
    current_revision: RevisionId,
    executions: BTreeMap<NodeExecutionId, ExecutionFact>,
    attempts: BTreeMap<milkdrift_persistence::AttemptId, AttemptFact>,
    distances: BTreeMap<RevisionId, BTreeMap<milkdrift_blueprint::NodeId, u16>>,
    subworkflow_parents: BTreeMap<SubworkflowId, NodeExecutionId>,
    all_ancestors: BTreeMap<milkdrift_blueprint::NodeId, u16>,
    join_exposed_values: BTreeSet<WorkspaceValueReference>,
    indexed_sequences: BTreeSet<RunSequence>,
}

impl<'source, 'store, 'request> DiscoveryState<'source, 'store, 'request> {
    fn new(
        source: &'source DurableContextCandidateSource<'store>,
        request: ContextSourceRequest<'request>,
    ) -> Self {
        let maximum_records = request.policy.budget().max_candidate_records;
        let first_sequence = candidate_tail_start(request.through_sequence.get(), maximum_records);
        let cursor = (first_sequence > 1).then(|| EventCursor {
            run: request.identity.run.clone(),
            next_sequence: RunSequence::new(first_sequence),
        });
        let current_revision = request.revision.id().clone();
        let executions = request
            .projection
            .current_node_executions()
            .map(|execution| {
                (
                    execution.execution().clone(),
                    ExecutionFact {
                        execution: execution.execution().clone(),
                        node: execution.node().clone(),
                        scope: execution.scope().clone(),
                        revision: execution.revision().clone(),
                    },
                )
            })
            .collect();
        let attempts = request
            .projection
            .attempts()
            .iter()
            .map(|(attempt, fact)| (attempt.clone(), projected_attempt_fact(fact)))
            .collect();
        let subworkflow_parents = request
            .projection
            .subworkflows()
            .values()
            .map(|child| {
                (
                    child.subworkflow().clone(),
                    child.parent_execution().clone(),
                )
            })
            .collect();
        let all_ancestors = ancestor_depths(
            request.revision.semantic(),
            &request.identity.node,
            Some(u16::MAX),
        );
        Self {
            source,
            request,
            candidates: Vec::new(),
            maximum_records,
            cursor,
            scanned: 0,
            event_summaries: 0,
            current_revision,
            executions,
            attempts,
            distances: BTreeMap::new(),
            subworkflow_parents,
            all_ancestors,
            join_exposed_values: BTreeSet::new(),
            indexed_sequences: BTreeSet::new(),
        }
    }

    fn discover(mut self) -> Result<Vec<ContextCandidate>, ContextBuildError> {
        self.seed_direct_inputs()?;
        self.seed_projection()?;
        self.fold_journal()?;
        self.complete_explicit_sources()?;
        self.apply_join_exposure();
        self.validate_required_sources()?;
        Ok(self.candidates)
    }

    fn can_summarize_event(&self) -> bool {
        self.event_summaries < self.request.policy.budget().max_event_summaries
    }

    fn count_event_summary(&mut self) -> Result<(), ContextBuildError> {
        self.event_summaries = self
            .event_summaries
            .checked_add(1)
            .ok_or(ContextBuildError::AccountingOverflow)?;
        Ok(())
    }
}
