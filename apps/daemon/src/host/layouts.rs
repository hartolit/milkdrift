//! Presentation-only layout validation, authorization, persistence, and read ownership.

use super::{
    ActorSession, ApplicationCommandEffect, ApplicationEffectReference, ApplicationLayoutStore,
    ApplicationLayoutUpdate, AuthorityOperation, CommandAccepted, CommandRequest, IntegrityDigest,
    LayoutDocument, LayoutOwner, Owner, PublicFailure, RequestedResourceFacts, RevisionStore,
    TimestampMillis, WorkflowId, bounded, corruption, internal, invalid, not_found,
    parse_revision_id, public_persistence, public_protocol,
};

pub(super) fn execute(
    owner: &Owner,
    session: &ActorSession,
    request: &CommandRequest,
    layout: &LayoutDocument,
) -> Result<CommandAccepted, PublicFailure> {
    layout.validate().map_err(public_protocol)?;
    let revision = parse_revision_id(&layout.revision_id)?;
    let workflow =
        WorkflowId::new(layout.workflow_id.clone()).map_err(|error| invalid(&error.to_string()))?;
    let mut resources = RequestedResourceFacts::empty();
    resources.workflow = Some(workflow.clone());
    resources.revision = Some(revision.clone());
    resources.layout_owner = Some(LayoutOwner::Shared);
    let decision = owner.authorize(
        session,
        AuthorityOperation::WriteLayout,
        resources,
        "command:put-layout",
    )?;
    let stored_revision = owner
        .store
        .revision(&revision)
        .map_err(public_persistence)?
        .ok_or_else(not_found)?;
    if stored_revision.semantic().workflow() != &workflow {
        return Err(not_found());
    }
    owner.record_security_decision(&decision)?;
    Ok(CommandAccepted {
        command_id: request.command_id.clone(),
        replayed: false,
        resulting_sequence: None,
        result_type: "layout_updated".to_owned(),
        value: serde_json::to_value(layout).map_err(|_| internal())?,
    })
}

pub(super) fn application_effect(
    session: &ActorSession,
    layout: &LayoutDocument,
    completed_at: TimestampMillis,
) -> Result<(Option<ApplicationEffectReference>, ApplicationCommandEffect), PublicFailure> {
    let workflow =
        WorkflowId::new(layout.workflow_id.clone()).map_err(|error| invalid(&error.to_string()))?;
    let revision = parse_revision_id(&layout.revision_id)?;
    let digest = IntegrityDigest::new(layout.digest.clone()).map_err(public_persistence)?;
    let reference = ApplicationEffectReference::Layout {
        workflow: workflow.clone(),
        revision: revision.clone(),
        generation: layout.generation,
        digest: digest.clone(),
    };
    let document = milkdrift_control_protocol::encode_json(layout).map_err(public_protocol)?;
    Ok((
        Some(reference),
        ApplicationCommandEffect::PutLayout(ApplicationLayoutUpdate {
            layout_schema_version: layout.schema_version,
            workflow,
            revision,
            generation: layout.generation,
            digest,
            author: session.actor.clone(),
            updated_at: completed_at,
            document,
        }),
    ))
}

pub(super) fn read(
    owner: &Owner,
    session: &ActorSession,
    workflow: &str,
    revision: &str,
) -> Result<LayoutDocument, PublicFailure> {
    let workflow_id =
        WorkflowId::new(workflow.to_owned()).map_err(|error| invalid(&error.to_string()))?;
    let revision_id = parse_revision_id(revision)?;
    let mut resources = RequestedResourceFacts::empty();
    resources.workflow = Some(workflow_id.clone());
    resources.revision = Some(revision_id.clone());
    resources.layout_owner = Some(LayoutOwner::Shared);
    owner.authorize(
        session,
        AuthorityOperation::ReadLayout,
        resources,
        "read:layout",
    )?;
    let stored = owner
        .store
        .application_layout(&workflow_id, &revision_id)
        .map_err(public_persistence)?
        .ok_or_else(not_found)?;
    let layout: LayoutDocument = milkdrift_control_protocol::decode_json(stored.document())
        .map_err(|error| corruption(&bounded(&error.to_string())))?;
    layout
        .validate()
        .map_err(|error| corruption(&bounded(&error.to_string())))?;
    Ok(layout)
}
