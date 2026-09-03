//! Projection seeding and exact terminal-anchor reconstruction.

use super::DiscoveryState;
use crate::context::source::{
    ContextBuildError, event_actor, event_at, event_attempt, event_semantics, producer,
};

impl DiscoveryState<'_, '_, '_> {
    pub(super) fn seed_direct_inputs(&mut self) -> Result<(), ContextBuildError> {
        for input in self.request.direct_inputs {
            self.candidates
                .push(self.source.direct_candidate(&self.request, input)?);
        }
        Ok(())
    }

    pub(super) fn seed_projection(&mut self) -> Result<(), ContextBuildError> {
        self.seed_projection_join_exposure();
        self.seed_current_outputs()?;
        self.seed_active_terminal_events()?;
        self.seed_settled_terminal_events()
    }

    fn seed_current_outputs(&mut self) -> Result<(), ContextBuildError> {
        for execution in self.request.projection.current_node_executions() {
            let Some(execution_fact) = self.executions.get(execution.execution()) else {
                continue;
            };
            for output in execution.outputs() {
                let event = event_at(
                    self.source.store,
                    &self.request.identity.run,
                    output.sequence(),
                    self.request.through_sequence,
                )?;
                let attempt = event_attempt(event.kind());
                self.candidates.push(self.source.output_candidate(
                    &self.request,
                    execution_fact,
                    attempt,
                    output.value(),
                    output.artifact(),
                    &event,
                    producer(attempt.and_then(|attempt| self.attempts.get(attempt)), None),
                    false,
                    &mut self.distances,
                )?);
                self.indexed_sequences.insert(output.sequence());
            }
        }
        Ok(())
    }

    fn seed_active_terminal_events(&mut self) -> Result<(), ContextBuildError> {
        for execution in self.request.projection.node_executions().values() {
            let sequence = execution
                .deterministic_terminal()
                .map(|terminal| terminal.sequence())
                .or_else(|| {
                    execution
                        .attempts()
                        .last()
                        .and_then(|attempt| self.request.projection.attempts().get(attempt))
                        .and_then(|attempt| attempt.terminal())
                        .map(|terminal| terminal.sequence())
                });
            let Some(sequence) = sequence else {
                continue;
            };
            if !self.can_summarize_event() {
                break;
            }
            self.seed_terminal_event(execution.execution(), sequence)?;
        }
        Ok(())
    }

    fn seed_settled_terminal_events(&mut self) -> Result<(), ContextBuildError> {
        for execution in self.request.projection.settled_node_executions().values() {
            let Some(sequence) = execution.terminal_sequence() else {
                continue;
            };
            if !self.can_summarize_event() {
                break;
            }
            self.seed_terminal_event(execution.execution(), sequence)?;
        }
        Ok(())
    }

    fn seed_terminal_event(
        &mut self,
        execution: &milkdrift_persistence::NodeExecutionId,
        sequence: milkdrift_persistence::RunSequence,
    ) -> Result<(), ContextBuildError> {
        let event = event_at(
            self.source.store,
            &self.request.identity.run,
            sequence,
            self.request.through_sequence,
        )?;
        let Some((kind, roles)) = event_semantics(event.kind()) else {
            return Ok(());
        };
        let execution_fact = self.executions.get(execution);
        let attempt_id = event_attempt(event.kind());
        let attempt = attempt_id.and_then(|attempt| self.attempts.get(attempt));
        self.candidates.push(self.source.event_candidate(
            &self.request,
            &event,
            kind,
            roles,
            execution_fact,
            attempt_id,
            attempt,
            event_actor(event.kind()),
            false,
            &mut self.distances,
        )?);
        self.count_event_summary()?;
        self.indexed_sequences.insert(sequence);
        Ok(())
    }
}
