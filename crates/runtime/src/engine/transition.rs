//! One operation-scoped owner for projected structured transition output.

use super::RuntimeService;
use crate::RuntimeError;
use crate::projection::RunProjection;
use milkdrift_persistence::{
    MAX_EVENTS_PER_COMMIT, MAX_WORKSPACE_MUTATIONS_PER_COMMIT, RunEventEnvelope, RunEventKind,
    TimestampMillis, WorkspaceMutation,
};
use milkdrift_workspace::RunId;

/// Borrows exactly the mutable state and output buffers of one commit-planning operation.
///
/// This owner cannot escape the operation. Event allocation, sequencing, projection, and
/// buffer bounds advance together through [`Self::push_event`]; workspace accumulation is
/// checked through [`Self::push_workspace`]. Durable I/O and authority evaluation remain on
/// `RuntimeService` and are intentionally not hidden here.
pub(super) struct PlanTransition<'a> {
    runtime: &'a RuntimeService,
    run: &'a RunId,
    occurred_at: TimestampMillis,
    projection: &'a mut RunProjection,
    events: &'a mut Vec<RunEventEnvelope>,
    workspace: &'a mut Vec<WorkspaceMutation>,
}

impl<'a> PlanTransition<'a> {
    pub(super) fn new(
        runtime: &'a RuntimeService,
        run: &'a RunId,
        occurred_at: TimestampMillis,
        projection: &'a mut RunProjection,
        events: &'a mut Vec<RunEventEnvelope>,
        workspace: &'a mut Vec<WorkspaceMutation>,
    ) -> Self {
        Self {
            runtime,
            run,
            occurred_at,
            projection,
            events,
            workspace,
        }
    }

    pub(super) const fn run(&self) -> &RunId {
        self.run
    }

    pub(super) const fn occurred_at(&self) -> TimestampMillis {
        self.occurred_at
    }

    pub(super) const fn projection(&self) -> &RunProjection {
        self.projection
    }

    pub(super) fn workspace(&self) -> &[WorkspaceMutation] {
        self.workspace
    }

    pub(super) fn event_count(&self) -> usize {
        self.events.len()
    }

    pub(super) fn events(&self) -> &[RunEventEnvelope] {
        self.events
    }

    pub(super) fn has_event_capacity(&self, additional: usize, ceiling: usize) -> bool {
        ceiling <= MAX_EVENTS_PER_COMMIT && self.events.len().saturating_add(additional) <= ceiling
    }

    pub(super) fn push_event(&mut self, kind: RunEventKind) -> Result<(), RuntimeError> {
        if !self.has_event_capacity(1, MAX_EVENTS_PER_COMMIT) {
            return Err(RuntimeError::Scheduling(
                "event commit bound reached while driving structured work".to_owned(),
            ));
        }
        let event = RunEventEnvelope::new(
            self.runtime.next_event_id()?,
            self.run.clone(),
            self.projection.sequence().next()?,
            self.occurred_at,
            kind,
        )?;
        self.projection.apply_replayed(&event)?;
        self.events.push(event);
        Ok(())
    }

    pub(super) fn push_workspace(
        &mut self,
        mutation: WorkspaceMutation,
    ) -> Result<(), RuntimeError> {
        if self.workspace.len() >= MAX_WORKSPACE_MUTATIONS_PER_COMMIT {
            return Err(RuntimeError::Scheduling(
                "workspace mutation commit bound reached while driving structured work".to_owned(),
            ));
        }
        self.workspace.push(mutation);
        Ok(())
    }
}
