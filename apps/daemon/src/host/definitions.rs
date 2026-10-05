//! Authorized immutable workflow/revision lineage read-model ownership.

use super::{
    Owner, PublicFailure, read_model::internal, read_model::invalid, read_model::not_found,
    read_model::parse_revision_id, read_model::public_persistence, read_model::public_protocol,
    read_model::public_revision_summary, read_model::unauthorized,
};
use crate::auth::ActorSession;
use milkdrift_authority::{AuthorityOperation, RequestedResourceFacts, WorkflowRunScope};
use milkdrift_blueprint::{BlueprintRevisionDocument, WorkflowId};
use milkdrift_control::{ControlCommand, ControlResult};
use milkdrift_control_protocol::{
    Cursor, Page, RevisionDiffRead, RevisionRead, RevisionSummary as PublicRevisionSummary,
};
use milkdrift_persistence::{
    PageSize, RevisionCursor, RevisionFilter, RevisionPageQuery, RevisionStore,
};

mod comparison;

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
            name: stored.semantic().metadata().name().to_owned(),
            inputs: interface_fields(stored.semantic().interface().inputs()),
            outputs: interface_fields(stored.semantic().interface().outputs()),
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
        let workflows = if let Some(requested) = requested_workflow {
            Some(
                milkdrift_authority::WorkflowSet::new([requested])
                    .map_err(|error| invalid(&error.to_string()))?,
            )
        } else {
            match &session.grant.resources().workflow_run {
                WorkflowRunScope::Any => None,
                WorkflowRunScope::Workflows { workflows } => Some(workflows.clone()),
                WorkflowRunScope::Workflow { workflow } => Some(
                    milkdrift_authority::WorkflowSet::new([workflow.clone()])
                        .map_err(|error| invalid(&error.to_string()))?,
                ),
                WorkflowRunScope::Run { .. } => return Err(unauthorized()),
            }
        };
        // A run grant never grants general definition discovery, even with a supplied filter.
        if matches!(
            session.grant.resources().workflow_run,
            WorkflowRunScope::Run { .. }
        ) {
            return Err(unauthorized());
        }
        let limit = PageSize::new(limit).map_err(public_persistence)?;
        let authorize = |workflow| {
            let mut resources = RequestedResourceFacts::empty();
            resources.workflow = workflow;
            self.authorize(
                session,
                AuthorityOperation::InspectRevision,
                resources,
                "read:revisions",
            )
        };
        let decision = if let Some(workflows) = &workflows {
            let mut decisions = workflows
                .values()
                .iter()
                .map(|id| authorize(Some(id.clone())));
            let first = decisions.next().ok_or_else(internal)??;
            decisions.try_fold(first, |_, next| next)?
        } else {
            authorize(None)?
        };
        // The raw filter distinguishes discovery from an explicit workflow request. Exact grant
        // contents in the binding supply the authorized set, without growing the cursor per ID.
        let feed = format!("revisions:{}", workflow.unwrap_or("*"));
        let binding = session.cursor_binding(&feed);
        let filter = RevisionFilter { workflows };
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
                limit,
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
        let mut difference =
            comparison::changes(left_revision.semantic(), right_revision.semantic());
        let limit =
            usize::try_from(milkdrift_control_protocol::MAX_PAGE_ITEMS).map_err(|_| internal())?;
        let changes = difference.by_ref().take(limit).collect();
        let truncated = difference.next().is_some();
        Ok(RevisionDiffRead {
            from_revision: from.to_owned(),
            to_revision: to.to_owned(),
            changes,
            truncated,
        })
    }
}

fn interface_fields(
    fields: &std::collections::BTreeMap<
        milkdrift_blueprint::FieldId,
        milkdrift_blueprint::InterfaceField,
    >,
) -> Vec<milkdrift_control_protocol::WorkflowFieldRead> {
    fields
        .iter()
        .map(
            |(name, field)| milkdrift_control_protocol::WorkflowFieldRead {
                name: name.to_string(),
                schema: field.schema().id().to_string(),
                version: field.schema().version(),
                required: field.is_required(),
            },
        )
        .collect()
}
