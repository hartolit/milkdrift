//! Authorized bounded run collection and timeline read-model ownership.

use super::{
    ActorSession, AuthorityOperation, ControlCommand, ControlResult, Cursor, ErrorCode, Owner,
    Page, PageSize, PublicFailure, RequestedResourceFacts, RunId, RunQueryStore, RunRead,
    RunSequence, RunSummaryCursor, RunSummaryFilter, RunSummaryPageQuery, TimelineEntry,
    WorkflowId, WorkflowRunScope, cursor_binding, internal, invalid, not_found, parse_run_state,
    public_persistence, public_protocol, public_timeline, unauthorized,
};

impl Owner {
    pub(super) fn runs(
        &self,
        session: &ActorSession,
        state: Option<&str>,
        workflow: Option<&str>,
        cursor: Option<&Cursor>,
        limit: u32,
    ) -> Result<Page<RunRead>, PublicFailure> {
        let indexed_state = state.map(parse_run_state).transpose()?;
        let requested_workflow = workflow
            .map(|value| WorkflowId::new(value.to_owned()))
            .transpose()
            .map_err(|error| invalid(&error.to_string()))?;
        if let WorkflowRunScope::Run {
            run,
            workflow: allowed_workflow,
        } = &session.grant.resources().workflow_run
        {
            if cursor.is_some()
                || requested_workflow.as_ref().is_some_and(|value| {
                    allowed_workflow
                        .as_ref()
                        .is_some_and(|allowed| value != allowed)
                })
            {
                return Err(unauthorized());
            }
            let value = self.run_read(session, run.as_str())?;
            let state_matches = state.is_none_or(|expected| value.lifecycle == expected);
            let workflow_matches = requested_workflow
                .as_ref()
                .is_none_or(|expected| value.workflow_id.as_deref() == Some(expected.as_str()));
            return Ok(Page {
                items: if state_matches && workflow_matches {
                    vec![value]
                } else {
                    Vec::new()
                },
                next_cursor: None,
                observed_cursor: None,
            });
        }
        let workflow_id = match &session.grant.resources().workflow_run {
            WorkflowRunScope::Any => requested_workflow,
            WorkflowRunScope::Workflow { workflow: allowed } => {
                if requested_workflow
                    .as_ref()
                    .is_some_and(|value| value != allowed)
                {
                    return Err(unauthorized());
                }
                Some(allowed.clone())
            }
            WorkflowRunScope::Run { .. } => unreachable!("exact run scope returned above"),
        };
        let feed = format!(
            "runs:{}:{}",
            state.unwrap_or("*"),
            workflow_id.as_ref().map_or("*", WorkflowId::as_str)
        );
        let mut resources = RequestedResourceFacts::empty();
        resources.workflow = workflow_id.clone();
        let decision = self.authorize(
            session,
            AuthorityOperation::InspectRun,
            resources,
            "read:runs",
        )?;
        let binding = cursor_binding(session, &feed)?;
        let filter = RunSummaryFilter {
            state: indexed_state,
            workflow: workflow_id,
        };
        let internal_cursor = cursor
            .map(|cursor| {
                cursor
                    .key_for_bound(&feed, &binding, session.cursor_key())
                    .map_err(public_protocol)
            })
            .transpose()?
            .map(|value| RunId::new(value).map_err(|error| invalid(&error.to_string())))
            .transpose()?
            .map(|run| RunSummaryCursor::for_query(run, filter.clone()));
        let page = self
            .store
            .run_summaries(&RunSummaryPageQuery {
                filter,
                cursor: internal_cursor,
                limit: PageSize::new(limit).map_err(public_persistence)?,
            })
            .map_err(public_persistence)?;
        let mut runs = Vec::with_capacity(page.runs.len());
        for summary in &page.runs {
            runs.push(self.run_read(session, summary.run.as_str())?);
        }
        let next_cursor = page
            .next
            .as_ref()
            .map(|cursor| {
                Cursor::new_bound_key(
                    &feed,
                    cursor.after_run().as_str(),
                    binding.clone(),
                    decision.digest(),
                    session.cursor_key(),
                )
                .map_err(public_protocol)
            })
            .transpose()?;
        Ok(Page {
            items: runs,
            next_cursor,
            observed_cursor: None,
        })
    }

    pub(super) fn timeline(
        &self,
        session: &ActorSession,
        run: &str,
        cursor: Option<&Cursor>,
        limit: u32,
    ) -> Result<Page<TimelineEntry>, PublicFailure> {
        let run_id = RunId::new(run.to_owned()).map_err(|error| invalid(&error.to_string()))?;
        let feed = format!("timeline:{run}");
        let decision = self.authorize_run_read(
            session,
            run,
            AuthorityOperation::InspectTimeline,
            "read:timeline",
        )?;
        let binding = cursor_binding(session, &feed)?;
        let next_sequence = cursor
            .map(|cursor| {
                cursor
                    .position_for_bound(&feed, &binding, session.cursor_key())
                    .map_err(public_protocol)
            })
            .transpose()?
            .map(|position| position.saturating_add(1))
            .unwrap_or(1);
        let result = match self.inspect_control(
            session,
            ControlCommand::InspectTimeline {
                run: run_id,
                after: Some(RunSequence::new(next_sequence)),
                limit: PageSize::new(limit).map_err(public_persistence)?,
            },
            None,
            "timeline",
        ) {
            Err(error) if error.code == ErrorCode::Unauthorized => return Err(not_found()),
            result => result?,
        };
        let ControlResult::Timeline { value } = result else {
            return Err(internal());
        };
        let items = value.events.iter().map(public_timeline).collect::<Vec<_>>();
        let next_cursor = value
            .next_sequence
            .map(|sequence| {
                Cursor::new_bound(
                    &feed,
                    sequence.get().saturating_sub(1),
                    binding.clone(),
                    decision.digest(),
                    session.cursor_key(),
                )
                .map_err(public_protocol)
            })
            .transpose()?;
        let observed_cursor = if value.observed_head == RunSequence::ZERO {
            None
        } else {
            Some(
                Cursor::new_bound(
                    &feed,
                    value.observed_head.get(),
                    binding,
                    decision.digest(),
                    session.cursor_key(),
                )
                .map_err(public_protocol)?,
            )
        };
        Ok(Page {
            items,
            next_cursor,
            observed_cursor,
        })
    }
}
