//! Page durable pending operations without occupying serving execution workers.
use super::{PeerService, ServingError, map_execution_persistence};
use milkdrift_authority::AuthorityEvaluator;
use milkdrift_capability::InvocationEvent;
use milkdrift_peer_protocol::{ObservationCategory, PeerObservation};
use milkdrift_persistence::PageSize;
use milkdrift_persistence::published::{PublishedInvocationPlan, PublishedInvocationSource};

impl PeerService {
    pub(crate) fn published_entry_allowed(
        &self,
        plan: &PublishedInvocationPlan,
    ) -> Result<bool, ServingError> {
        let PublishedInvocationSource::Serving { caller, execution } = &plan.source else {
            return Err(ServingError::Protocol(
                "expected a serving publication source".to_owned(),
            ));
        };
        let Some(milkdrift_persistence::PeerExecutionSnapshot::Hot(record)) = self
            .executions
            .peer_execution(caller, execution)
            .map_err(map_execution_persistence)?
        else {
            return Ok(false);
        };
        if record.published_invocation.as_ref() != Some(plan)
            || !matches!(
                record.phase,
                milkdrift_persistence::PeerExecutionPhase::AwaitingWorkflow { .. }
            )
            || record.cancellation.is_some()
            || self.now()? >= record.request.deadline_unix_ms
            || self.now()? >= plan.deadline_unix_ms
        {
            return Ok(false);
        }
        self.published_caller_allowed(plan)
    }

    fn published_caller_allowed(
        &self,
        plan: &PublishedInvocationPlan,
    ) -> Result<bool, ServingError> {
        let PublishedInvocationSource::Serving { caller, .. } = &plan.source else {
            return Err(ServingError::Protocol(
                "expected a serving publication source".to_owned(),
            ));
        };
        let relationship_available = match &caller.principal {
            milkdrift_peer_protocol::ServingPrincipal::Peer { peer } => {
                self.relationship(peer).is_ok()
            }
            milkdrift_peer_protocol::ServingPrincipal::Client { actor } => {
                self.client_grant(actor).is_ok()
            }
        };
        let mut request = plan.caller.request().clone();
        request.evaluated_at = milkdrift_authority::BoundaryTimeMillis::new(self.now()?);
        Ok(relationship_available
            && self
                .authority
                .evaluate(&request)
                .map_err(|error| ServingError::Unavailable(error.to_string()))?
                .is_allowed())
    }

    /// Advance one bounded page of already accepted workflow-backed operations. An unavailable
    /// owner leaves the saved association pending; it never returns the record to dispatch.
    pub(super) fn continue_published(&self) -> Result<(), ServingError> {
        let mut cursor = self.published_cursor.lock().map_err(|_| {
            ServingError::Unavailable("published observation cursor unavailable".to_owned())
        })?;
        let (records, next) = self
            .executions
            .published_serving_page(
                cursor.as_ref(),
                PageSize::new(self.config.workers.archive_batch_size)
                    .map_err(|e| ServingError::Protocol(e.to_string()))?,
            )
            .map_err(map_execution_persistence)?;
        for record in records {
            let Some(plan) = record.published_invocation.as_ref() else {
                continue;
            };
            let caller_allowed = self.published_caller_allowed(plan)?;
            let event = match self.capability_host.continue_published_invocation(
                plan,
                !caller_allowed
                    || record.cancellation.is_some()
                    || self.now()? >= record.request.deadline_unix_ms,
                record.last_observation_sequence.saturating_add(1),
            ) {
                Ok(Some(event)) => event,
                Ok(None) => continue,
                Err(error) => {
                    tracing::debug!(execution = %record.execution, reason = %error, "published serving continuation remains pending");
                    continue;
                }
            };
            let terminal = matches!(
                event.kind(),
                milkdrift_capability::InvocationEventKind::Terminal { .. }
            );
            let event = InvocationEvent::new(
                record.request.request.invocation().clone(),
                event.sequence(),
                event.kind().clone(),
            )
            .map_err(|e| ServingError::Protocol(e.to_string()))?;
            let observed = self
                .executions
                .append_peer_observation(
                    &record.caller,
                    &record.execution,
                    &PeerObservation {
                        execution: record.execution.clone(),
                        sequence: event.sequence(),
                        category: if terminal {
                            ObservationCategory::Terminal
                        } else {
                            ObservationCategory::Artifact
                        },
                        event,
                        observed_at_unix_ms: self.now()?,
                    },
                )
                .map_err(map_execution_persistence);
            if terminal
                && self
                    .executions
                    .peer_execution(&record.caller, &record.execution)
                    .map_err(map_execution_persistence)?
                    .is_some_and(|snapshot| match snapshot {
                        milkdrift_persistence::PeerExecutionSnapshot::Hot(record) => matches!(
                            record.phase,
                            milkdrift_persistence::PeerExecutionPhase::Terminal { .. }
                        ),
                        milkdrift_persistence::PeerExecutionSnapshot::Archived(_) => true,
                    })
            {
                self.capability_host
                    .release_published_invocation(plan)
                    .map_err(|e| ServingError::Unavailable(e.to_string()))?;
            }
            observed?;
        }
        *cursor = next;
        Ok(())
    }
}
