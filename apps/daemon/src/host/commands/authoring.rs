//! Stateless authoring over ordinary immutable definitions. The client retains mutations;
//! the daemon retains only saved revisions and the existing exact command receipts.

use milkdrift_blueprint::{
    AuthorRef, BlueprintRevision, BlueprintRevisionDocument, Mutation, MutationBatch, WorkflowId,
};
use milkdrift_control_protocol::{
    BlueprintDraft, BlueprintEdit, CommandAccepted, CommandRequest, ErrorCode,
};
use milkdrift_persistence::RevisionStore;
use serde_json::json;

use super::super::{ActorSession, Owner, PublicFailure, invalid, public_persistence};
use super::definitions::authorize_definition;

mod edits;
mod graph;
mod repair;
use graph::ModelWorkflow;
pub(super) use repair::prepare as repair;

type BuildResult<T> = Result<T, Box<dyn std::error::Error>>;
fn failure(error: impl std::fmt::Display) -> PublicFailure {
    invalid(&super::super::bounded(&error.to_string()))
}

fn base(
    owner: &Owner,
    session: &ActorSession,
    request: &CommandRequest,
    draft: &BlueprintDraft,
) -> Result<Option<BlueprintRevision>, PublicFailure> {
    if request.expected_revision != draft.base_revision || request.expected_sequence.is_some() {
        return Err(PublicFailure::new(
            ErrorCode::Conflict,
            "authoring requires the exact draft base revision and no run sequence",
            false,
        ));
    }
    draft
        .base_revision
        .as_ref()
        .map(|id| {
            let read = owner.revision(session, id)?;
            let bytes = serde_json::to_vec(&read.document).map_err(failure)?;
            let (_, revision) = BlueprintRevisionDocument::from_json(&bytes).map_err(failure)?;
            if revision.semantic().workflow().as_str() != draft.workflow_id {
                return Err(invalid("draft workflow differs from its base"));
            }
            Ok(revision)
        })
        .transpose()
}

fn candidate(
    draft: &BlueprintDraft,
    base: Option<&BlueprintRevision>,
    session: &ActorSession,
    reason: &str,
) -> Result<BlueprintRevision, PublicFailure> {
    if draft.mutations.is_empty()
        && let Some(base) = base
    {
        return Ok(base.clone());
    }
    let operations = draft
        .mutations
        .iter()
        .cloned()
        .map(serde_json::from_value::<Mutation>)
        .collect::<Result<Vec<_>, _>>()
        .map_err(failure)?;
    let batch = MutationBatch::new(operations).map_err(failure)?;
    let author = AuthorRef::new(session.actor.as_str()).map_err(failure)?;
    match base {
        Some(base) => base.revise(base.id(), batch, author, reason),
        None => BlueprintRevision::genesis(
            WorkflowId::new(&draft.workflow_id).map_err(failure)?,
            batch,
            author,
            reason,
        ),
    }
    .map_err(failure)
}

pub(super) fn construct(
    owner: &Owner,
    session: &ActorSession,
    request: &CommandRequest,
    draft: &BlueprintDraft,
    store: bool,
) -> Result<CommandAccepted, PublicFailure> {
    let base = base(owner, session, request, draft)?;
    let revision = candidate(draft, base.as_ref(), session, &request.reason)?;
    finish(
        owner,
        session,
        request,
        draft.clone(),
        revision,
        store,
        serde_json::Value::Null,
    )
}

pub(super) fn copy(
    owner: &Owner,
    session: &ActorSession,
    request: &CommandRequest,
    source: &str,
    workflow: &str,
    name: &str,
) -> Result<CommandAccepted, PublicFailure> {
    if request.expected_revision.as_deref() != Some(source) || request.expected_sequence.is_some() {
        return Err(super::super::read_model::conflict(
            "copy requires the exact source revision and no run sequence",
        ));
    }
    let read = owner.revision(session, source)?;
    let (_, source) =
        BlueprintRevisionDocument::from_json(&serde_json::to_vec(&read.document).map_err(failure)?)
            .map_err(failure)?;
    if source.semantic().workflow().as_str() == workflow {
        return Err(invalid(
            "an independent copy requires a different workflow identity",
        ));
    }
    if source.semantic().agreement().is_some() {
        return Err(invalid(
            "the governing agreement is bound to its workflow identity; reuse this exact definition or explicitly author another governed method",
        ));
    }
    let mut mutations = graph::difference(None, &source);
    let metadata = source.semantic().metadata();
    // Preserve descriptive extensions and every graph fact; the source is retained in immutable
    // revision provenance and the exact copy command, without transferring any execution state.
    *mutations
        .first_mut()
        .ok_or_else(super::super::read_model::internal)? = Mutation::SetMetadata {
        metadata: milkdrift_blueprint::BlueprintMetadata::new(
            name,
            metadata.description(),
            metadata.labels().clone(),
            metadata.extensions().clone(),
        )
        .map_err(failure)?,
    };
    let revision = BlueprintRevision::genesis(
        WorkflowId::new(workflow).map_err(failure)?,
        MutationBatch::new(mutations).map_err(failure)?,
        AuthorRef::new(session.actor.as_str()).map_err(failure)?,
        format!("Copy of {}. {}", source.id(), request.reason),
    )
    .map_err(failure)?;
    finish(
        owner,
        session,
        request,
        BlueprintDraft {
            workflow_id: workflow.into(),
            base_revision: None,
            mutations: vec![],
        },
        revision,
        true,
        json!({"copied_from":source.id()}),
    )
}

pub(super) fn execute(
    owner: &Owner,
    session: &ActorSession,
    request: &CommandRequest,
    draft: &BlueprintDraft,
    edit: Option<&BlueprintEdit>,
    save: bool,
) -> Result<CommandAccepted, PublicFailure> {
    let base = base(owner, session, request, draft)?;
    let workflow = WorkflowId::new(&draft.workflow_id).map_err(failure)?;
    let mut model = if base.is_none() && draft.mutations.is_empty() {
        ModelWorkflow::empty(&draft.workflow_id)
    } else {
        let revision = candidate(draft, base.as_ref(), session, &request.reason)?;
        ModelWorkflow::read(&revision).map_err(failure)?
    };
    if let Some(edit) = edit {
        model.edit(owner, session, edit).map_err(failure)?;
    }
    // Recheck every selected model, including mutations supplied without an editor gesture.
    // Missing and hidden capabilities have the same diagnostic.
    model.authorize_models(owner, session)?;
    if save {
        model.complete().map_err(failure)?;
    }
    let generated = model
        .build(
            workflow,
            AuthorRef::new(session.actor.as_str()).map_err(failure)?,
            &request.reason,
        )
        .map_err(failure)?;
    let operations = graph::difference(base.as_ref(), &generated);
    let pending = BlueprintDraft {
        workflow_id: draft.workflow_id.clone(),
        base_revision: draft.base_revision.clone(),
        mutations: operations
            .into_iter()
            .map(serde_json::to_value)
            .collect::<Result<_, _>>()
            .map_err(failure)?,
    };
    let revision = candidate(&pending, base.as_ref(), session, &request.reason)?;
    let view = model.view();
    finish(owner, session, request, pending, revision, save, view)
}

fn finish(
    owner: &Owner,
    session: &ActorSession,
    request: &CommandRequest,
    mut draft: BlueprintDraft,
    revision: BlueprintRevision,
    store: bool,
    view: serde_json::Value,
) -> Result<CommandAccepted, PublicFailure> {
    let decision = authorize_definition(
        owner,
        session,
        revision.semantic().workflow().clone(),
        revision.id().clone(),
        store,
        "command:author-blueprint",
    )?;
    let document: serde_json::Value = serde_json::from_slice(
        &BlueprintRevisionDocument::new(&revision)
            .to_canonical_json()
            .map_err(failure)?,
    )
    .map_err(failure)?;
    if store {
        draft.base_revision = Some(revision.id().to_string());
        draft.mutations.clear();
    }
    let result = CommandAccepted {
        command_id: request.command_id.clone(),
        replayed: false,
        resulting_sequence: None,
        result_type: if store {
            "blueprint_saved"
        } else {
            "blueprint_draft"
        }
        .to_owned(),
        value: json!({"draft":draft, "revision_id":revision.id(), "document":document, "workflow":view}),
    };
    // Refuse an oversized reply before storing anything; it must fit the normal public reader.
    milkdrift_control_protocol::encode_json(&result).map_err(failure)?;
    if store {
        owner
            .store
            .put_revision(&revision)
            .map_err(public_persistence)?;
        owner.record_security_decision(&decision)?;
    }
    Ok(result)
}
