use std::collections::{BTreeMap, BTreeSet};

use milkdrift_blueprint::ContextSemanticRole;
use milkdrift_model::{ContextProducerFact, ContextSemanticKind, ContextSource};
use milkdrift_persistence::{
    EventCursor, EventPageQuery, NodeExecutionId, PageSize, RunEventKind, RunSequence,
};
use milkdrift_workspace::{SubworkflowId, WorkspaceValueReference};

use super::{
    AttemptFact, ContextBuildError, ContextCandidate, ContextCandidateSource, ContextSourceRequest,
    DurableContextCandidateSource, ExecutionFact, SOURCE_PAGE_SIZE, ancestor_depths,
    candidate_references_join_output, candidate_tail_start, event_actor, event_at, event_attempt,
    event_execution, event_semantics, persistence, producer, projected_attempt_fact,
    record_subworkflow_parent,
};

impl ContextCandidateSource for DurableContextCandidateSource<'_> {
    fn discover(
        &self,
        request: ContextSourceRequest<'_>,
    ) -> Result<Vec<ContextCandidate>, ContextBuildError> {
        let mut candidates = Vec::new();
        for input in request.direct_inputs {
            candidates.push(self.direct_candidate(&request, input)?);
        }

        let maximum_records = request.policy.budget().max_candidate_records;
        let first_sequence = candidate_tail_start(request.through_sequence.get(), maximum_records);
        let mut cursor = (first_sequence > 1).then(|| EventCursor {
            run: request.identity.run.clone(),
            next_sequence: RunSequence::new(first_sequence),
        });
        let mut scanned = 0_u32;
        let mut event_summaries = 0_u32;
        let mut current_revision = request.revision.id().clone();
        let mut executions = request
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
            .collect::<BTreeMap<_, _>>();
        let mut attempts = request
            .projection
            .attempts()
            .iter()
            .map(|(attempt, fact)| (attempt.clone(), projected_attempt_fact(fact)))
            .collect::<BTreeMap<_, _>>();
        let mut distances = BTreeMap::new();
        let mut subworkflow_parents = request
            .projection
            .subworkflows()
            .values()
            .map(|child| {
                (
                    child.subworkflow().clone(),
                    child.parent_execution().clone(),
                )
            })
            .collect::<BTreeMap<SubworkflowId, NodeExecutionId>>();
        let all_ancestors = ancestor_depths(
            request.revision.semantic(),
            &request.identity.node,
            Some(u16::MAX),
        );
        let mut join_exposed_values = BTreeSet::new();
        for join in request.projection.joins().values() {
            if executions.get(join.execution()).is_some_and(|execution| {
                execution.revision == *request.revision.id()
                    && all_ancestors.contains_key(&execution.node)
            }) {
                join_exposed_values.extend(
                    join.branches()
                        .iter()
                        .flat_map(|branch| branch.outputs.iter().cloned()),
                );
            }
        }
        let mut indexed_sequences = BTreeSet::new();
        for execution in request.projection.current_node_executions() {
            let Some(execution_fact) = executions.get(execution.execution()) else {
                continue;
            };
            for output in execution.outputs() {
                let event = event_at(
                    self.store,
                    &request.identity.run,
                    output.sequence(),
                    request.through_sequence,
                )?;
                let attempt = event_attempt(event.kind());
                candidates.push(self.output_candidate(
                    &request,
                    execution_fact,
                    attempt,
                    output.value(),
                    output.artifact(),
                    &event,
                    producer(attempt.and_then(|attempt| attempts.get(attempt)), None),
                    false,
                    &mut distances,
                )?);
                indexed_sequences.insert(output.sequence());
            }
        }
        for execution in request.projection.node_executions().values() {
            let sequence = execution
                .deterministic_terminal()
                .map(|terminal| terminal.sequence())
                .or_else(|| {
                    execution
                        .attempts()
                        .last()
                        .and_then(|attempt| request.projection.attempts().get(attempt))
                        .and_then(|attempt| attempt.terminal())
                        .map(|terminal| terminal.sequence())
                });
            let Some(sequence) = sequence else {
                continue;
            };
            if event_summaries >= request.policy.budget().max_event_summaries {
                break;
            }
            let event = event_at(
                self.store,
                &request.identity.run,
                sequence,
                request.through_sequence,
            )?;
            let Some((kind, roles)) = event_semantics(event.kind()) else {
                continue;
            };
            let execution_fact = executions.get(execution.execution());
            let attempt_id = event_attempt(event.kind());
            let attempt = attempt_id.and_then(|attempt| attempts.get(attempt));
            candidates.push(self.event_candidate(
                &request,
                &event,
                kind,
                roles,
                execution_fact,
                attempt_id,
                attempt,
                event_actor(event.kind()),
                false,
                &mut distances,
            )?);
            event_summaries += 1;
            indexed_sequences.insert(sequence);
        }
        for execution in request.projection.settled_node_executions().values() {
            let Some(sequence) = execution.terminal_sequence() else {
                continue;
            };
            if event_summaries >= request.policy.budget().max_event_summaries {
                break;
            }
            let event = event_at(
                self.store,
                &request.identity.run,
                sequence,
                request.through_sequence,
            )?;
            let Some((kind, roles)) = event_semantics(event.kind()) else {
                continue;
            };
            let execution_fact = executions.get(execution.execution());
            let attempt_id = event_attempt(event.kind());
            let attempt = attempt_id.and_then(|attempt| attempts.get(attempt));
            candidates.push(self.event_candidate(
                &request,
                &event,
                kind,
                roles,
                execution_fact,
                attempt_id,
                attempt,
                event_actor(event.kind()),
                false,
                &mut distances,
            )?);
            event_summaries += 1;
            indexed_sequences.insert(sequence);
        }
        'pages: loop {
            let remaining = maximum_records - scanned;
            if remaining == 0 {
                break;
            }
            let page_size = SOURCE_PAGE_SIZE.min(remaining);
            let page = self
                .store
                .events(
                    &EventPageQuery::new(
                        request.identity.run.clone(),
                        cursor,
                        PageSize::new(page_size).map_err(persistence)?,
                    )
                    .map_err(persistence)?,
                )
                .map_err(persistence)?;
            scanned = scanned
                .checked_add(
                    u32::try_from(page.events.len())
                        .map_err(|_| ContextBuildError::AccountingOverflow)?,
                )
                .ok_or(ContextBuildError::AccountingOverflow)?;
            for event in &page.events {
                if event.sequence() > request.through_sequence {
                    break 'pages;
                }
                match event.kind() {
                    RunEventKind::RunCreated { revision, .. }
                    | RunEventKind::RevisionPinned { revision, .. } => {
                        current_revision = revision.clone();
                    }
                    RunEventKind::NodeBecameEligible {
                        node,
                        execution,
                        scope,
                        ..
                    } => {
                        executions
                            .entry(execution.clone())
                            .or_insert_with(|| ExecutionFact {
                                execution: execution.clone(),
                                node: node.clone(),
                                scope: scope.clone(),
                                revision: current_revision.clone(),
                            });
                    }
                    RunEventKind::NodeScheduled {
                        execution,
                        attempt,
                        invocation,
                        ..
                    } => {
                        attempts.insert(
                            attempt.clone(),
                            AttemptFact {
                                execution: Some(execution.clone()),
                                invocation: Some(invocation.as_str().to_owned()),
                                ..AttemptFact::default()
                            },
                        );
                    }
                    RunEventKind::CapabilityResolved {
                        attempt, snapshot, ..
                    } => {
                        let fact = attempts.entry(attempt.clone()).or_default();
                        fact.capability = Some(snapshot.capability().as_str().to_owned());
                        fact.descriptor_revision = Some(snapshot.descriptor_revision());
                        fact.provider_profile = snapshot
                            .provider_profile()
                            .map(|profile| profile.as_str().to_owned());
                    }
                    RunEventKind::CapabilityResolutionDecisionRecorded {
                        attempt,
                        authorization,
                        ..
                    } => {
                        let fact = attempts.entry(attempt.clone()).or_default();
                        fact.peer = authorization
                            .request()
                            .resources
                            .peer
                            .as_ref()
                            .map(|peer| peer.as_str().to_owned())
                            .or_else(|| {
                                authorization
                                    .request()
                                    .provenance
                                    .peer
                                    .as_ref()
                                    .map(|peer| peer.as_str().to_owned())
                            });
                    }
                    RunEventKind::NodeOutputPublished {
                        execution,
                        attempt,
                        value,
                        artifact,
                        ..
                    } => {
                        if indexed_sequences.contains(&event.sequence()) {
                            continue;
                        }
                        if let Some(execution_fact) = executions.get(execution) {
                            candidates.push(self.output_candidate(
                                &request,
                                execution_fact,
                                Some(attempt),
                                value,
                                artifact.as_ref(),
                                event,
                                producer(attempts.get(attempt), None),
                                false,
                                &mut distances,
                            )?);
                        }
                    }
                    RunEventKind::DeterministicOutputPublished {
                        execution,
                        value,
                        artifact,
                    } => {
                        if indexed_sequences.contains(&event.sequence()) {
                            continue;
                        }
                        if let Some(execution_fact) = executions.get(execution) {
                            candidates.push(self.output_candidate(
                                &request,
                                execution_fact,
                                None,
                                value,
                                artifact.as_ref(),
                                event,
                                ContextProducerFact::default(),
                                false,
                                &mut distances,
                            )?);
                        }
                    }
                    RunEventKind::SubworkflowCreated {
                        subworkflow,
                        parent_execution,
                        ..
                    } => {
                        record_subworkflow_parent(
                            &mut subworkflow_parents,
                            subworkflow,
                            parent_execution,
                        )?;
                    }
                    RunEventKind::SubworkflowOutputImported {
                        subworkflow,
                        parent_value,
                        ..
                    } => {
                        let execution_fact = subworkflow_parents
                            .get(subworkflow)
                            .and_then(|execution| executions.get(execution))
                            .ok_or(ContextBuildError::RequiredUnavailable(
                                "subworkflow parent provenance",
                            ))?;
                        candidates.push(self.output_candidate(
                            &request,
                            execution_fact,
                            None,
                            parent_value,
                            None,
                            event,
                            ContextProducerFact::default(),
                            true,
                            &mut distances,
                        )?);
                    }
                    RunEventKind::JoinSatisfied {
                        execution,
                        branches,
                        ..
                    } => {
                        if executions.get(execution).is_some_and(|execution| {
                            execution.revision == *request.revision.id()
                                && all_ancestors.contains_key(&execution.node)
                        }) {
                            join_exposed_values.extend(
                                branches
                                    .iter()
                                    .flat_map(|branch| branch.outputs.iter().cloned()),
                            );
                        }
                        if event_summaries < request.policy.budget().max_event_summaries {
                            event_summaries += 1;
                            candidates.push(
                                self.event_candidate(
                                    &request,
                                    event,
                                    ContextSemanticKind::SuccessfulOutput,
                                    BTreeSet::from([ContextSemanticRole::Evidence]),
                                    event_execution(event.kind())
                                        .and_then(|execution| executions.get(execution)),
                                    None,
                                    None,
                                    None,
                                    false,
                                    &mut distances,
                                )?,
                            );
                        }
                    }
                    kind if !indexed_sequences.contains(&event.sequence())
                        && event_semantics(kind).is_some()
                        && event_summaries < request.policy.budget().max_event_summaries =>
                    {
                        event_summaries += 1;
                        let semantics = event_semantics(kind).ok_or_else(|| {
                            ContextBuildError::Policy("event semantics disappeared".to_owned())
                        })?;
                        let attempt_id = event_attempt(kind);
                        let attempt = attempt_id.and_then(|attempt| attempts.get(attempt));
                        let execution = event_execution(kind)
                            .and_then(|execution| executions.get(execution))
                            .or_else(|| {
                                attempt
                                    .and_then(|attempt| attempt.execution.as_ref())
                                    .and_then(|execution| executions.get(execution))
                            });
                        candidates.push(self.event_candidate(
                            &request,
                            event,
                            semantics.0,
                            semantics.1,
                            execution,
                            attempt_id,
                            attempt,
                            event_actor(kind),
                            false,
                            &mut distances,
                        )?);
                    }
                    _ => {}
                }
            }
            cursor = page.next;
            if cursor.is_none() {
                break;
            }
        }
        for selector in request.policy.selected_workspace_values() {
            let reference: WorkspaceValueReference =
                serde_json::from_str(selector).map_err(|_| {
                    ContextBuildError::RequiredUnavailable("selected workspace reference")
                })?;
            let source = ContextSource::WorkspaceValue {
                reference: reference.clone(),
            };
            if !candidates
                .iter()
                .any(|candidate| candidate.source.as_ref() == Some(&source))
            {
                candidates.push(self.explicit_workspace_candidate(&request, reference)?);
            }
        }
        for selector in request.policy.explicit_evidence() {
            let source: ContextSource = serde_json::from_str(selector).map_err(|_| {
                ContextBuildError::RequiredUnavailable("explicit evidence reference")
            })?;
            if candidates
                .iter()
                .any(|candidate| candidate.source.as_ref() == Some(&source))
            {
                continue;
            }
            candidates.push(self.explicit_candidate(
                &request,
                source,
                &executions,
                &attempts,
                &mut distances,
            )?);
        }
        for candidate in &mut candidates {
            if candidate_references_join_output(candidate, &join_exposed_values) {
                candidate.exposed_across_scope = true;
            }
        }
        for selected in request.policy.selected_executions() {
            if !candidates.iter().any(|candidate| {
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
        if candidates.len()
            > usize::try_from(request.policy.budget().max_candidate_records)
                .map_err(|_| ContextBuildError::AccountingOverflow)?
        {
            return Err(ContextBuildError::RequiredBudget("candidate count"));
        }
        Ok(candidates)
    }
}
