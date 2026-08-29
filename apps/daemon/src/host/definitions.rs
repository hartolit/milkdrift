//! Authorized immutable workflow/revision lineage read-model ownership.

use super::*;

impl Owner {
    pub(super) fn revision(
        &self,
        session: &ActorSession,
        revision: &str,
    ) -> Result<RevisionRead, PublicFailure> {
        let revision_id = parse_revision_id(revision)?;
        let command = ControlCommand::InspectRevision {
            revision: revision_id.clone(),
        };
        let result = self.inspect_control(session, command, None, "revision")?;
        let ControlResult::RevisionInspection { value } = result else {
            return Err(internal());
        };
        let stored = self
            .store
            .revision(&revision_id)
            .map_err(public_persistence)?
            .ok_or_else(not_found)?;
        let document = BlueprintRevisionDocument::new(&stored)
            .to_canonical_json()
            .map_err(|error| invalid(&error.to_string()))?;
        Ok(RevisionRead {
            summary: PublicRevisionSummary {
                revision_id: value.revision.as_str().to_owned(),
                workflow_id: value.workflow.as_str().to_owned(),
                lineage_sequence: value.lineage_sequence,
                semantic_digest: value.content_digest.as_str().to_owned(),
                parents: value
                    .parents
                    .iter()
                    .map(|parent| parent.as_str().to_owned())
                    .collect(),
            },
            author: value.author.as_str().to_owned(),
            reason: value.reason,
            node_count: u32::try_from(value.node_count).unwrap_or(u32::MAX),
            edge_count: u32::try_from(value.edge_count).unwrap_or(u32::MAX),
            document: serde_json::from_slice(&document).ok(),
        })
    }

    pub(super) fn revisions(
        &self,
        session: &ActorSession,
        workflow: Option<&str>,
        cursor: Option<&Cursor>,
        limit: u32,
    ) -> Result<Page<PublicRevisionSummary>, PublicFailure> {
        let requested_workflow = workflow
            .map(|value| WorkflowId::new(value.to_owned()))
            .transpose()
            .map_err(|error| invalid(&error.to_string()))?;
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
            WorkflowRunScope::Run { .. } => return Err(unauthorized()),
        };
        let feed = format!(
            "revisions:{}",
            workflow_id.as_ref().map_or("*", WorkflowId::as_str)
        );
        let mut resources = RequestedResourceFacts::empty();
        resources.workflow = workflow_id.clone();
        let decision = self.authorize(
            session,
            AuthorityOperation::InspectRevision,
            resources,
            "read:revisions",
        )?;
        let binding = cursor_binding(session, &feed)?;
        let filter = RevisionFilter {
            workflow: workflow_id,
        };
        let internal_cursor = cursor
            .map(|cursor| {
                cursor
                    .key_for_bound(&feed, &binding, session.cursor_key())
                    .map_err(public_protocol)
            })
            .transpose()?
            .map(|value| parse_revision_id(&value))
            .transpose()?
            .map(|revision| RevisionCursor::new(revision, filter.clone()));
        let page = self
            .store
            .revisions(&RevisionPageQuery {
                filter,
                cursor: internal_cursor,
                limit: PageSize::new(limit).map_err(public_persistence)?,
            })
            .map_err(public_persistence)?;
        let next_cursor = page
            .next
            .as_ref()
            .map(|cursor| {
                Cursor::new_bound_key(
                    &feed,
                    cursor.after_revision().as_str(),
                    binding.clone(),
                    decision.digest(),
                    session.cursor_key(),
                )
                .map_err(public_protocol)
            })
            .transpose()?;
        Ok(Page {
            items: page.revisions.iter().map(public_revision_summary).collect(),
            next_cursor,
            observed_cursor: None,
        })
    }

    pub(super) fn revision_diff(
        &self,
        session: &ActorSession,
        from: &str,
        to: &str,
    ) -> Result<RevisionDiffRead, PublicFailure> {
        let left = self.revision(session, from)?;
        let right = self.revision(session, to)?;
        if left.summary.workflow_id != right.summary.workflow_id {
            return Err(invalid("revision diff requires one workflow lineage"));
        }
        let left_revision = self
            .store
            .revision(&parse_revision_id(from)?)
            .map_err(public_persistence)?
            .ok_or_else(not_found)?;
        let right_revision = self
            .store
            .revision(&parse_revision_id(to)?)
            .map_err(public_persistence)?
            .ok_or_else(not_found)?;
        let mut changes = Vec::new();
        diff_keys(
            "node",
            left_revision.semantic().nodes(),
            right_revision.semantic().nodes(),
            &mut changes,
        );
        diff_keys(
            "edge",
            left_revision.semantic().edges(),
            right_revision.semantic().edges(),
            &mut changes,
        );
        let truncated = changes.len() > 1_024;
        changes.truncate(1_024);
        Ok(RevisionDiffRead {
            from_revision: from.to_owned(),
            to_revision: to.to_owned(),
            changes,
            truncated,
        })
    }
}
