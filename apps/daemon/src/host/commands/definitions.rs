//! Blueprint and prompt-sequence validation/import planning.

use serde_json::{Value, json};

use super::super::{
    ActorSession, AuthorRef, AuthorityOperation, BlueprintRevisionDocument, CommandAccepted,
    CommandRequest, Owner, PromptSequenceDocument, PublicFailure, RequestedResourceFacts,
    RevisionStore, bounded, compile_prompt_sequence, invalid, public_persistence,
};

pub(super) fn blueprint(
    owner: &Owner,
    session: &ActorSession,
    request: &CommandRequest,
    document: &Value,
    store: bool,
) -> Result<CommandAccepted, PublicFailure> {
    let bytes = serde_json::to_vec(document).map_err(|_| invalid("invalid blueprint JSON"))?;
    let (_document, revision) = BlueprintRevisionDocument::from_json(&bytes)
        .map_err(|error| invalid(&bounded(&error.to_string())))?;
    let decision = authorize_definition(
        owner,
        session,
        revision.semantic().workflow().clone(),
        revision.id().clone(),
        store,
        if store {
            "command:import-blueprint"
        } else {
            "command:validate-blueprint"
        },
    )?;
    let replayed = if store {
        matches!(
            owner
                .store
                .put_revision(&revision)
                .map_err(public_persistence)?,
            milkdrift_persistence::ImmutableRevisionPut::AlreadyPresent
        )
    } else {
        false
    };
    if store {
        owner.record_security_decision(&decision)?;
    }
    Ok(CommandAccepted {
        command_id: request.command_id.clone(),
        replayed,
        resulting_sequence: None,
        result_type: if store {
            "blueprint_imported"
        } else {
            "blueprint_valid"
        }
        .to_owned(),
        value: if store {
            json!({
                "revision_id": revision.id().as_str(),
                "workflow_id": revision.semantic().workflow().as_str(),
                "semantic_digest": revision.content_digest().as_str(),
            })
        } else {
            json!({
                "revision_id": revision.id().as_str(),
                "semantic_digest": revision.content_digest().as_str(),
            })
        },
    })
}

pub(super) fn prompt_sequence(
    owner: &Owner,
    session: &ActorSession,
    request: &CommandRequest,
    document: &Value,
    store: bool,
) -> Result<CommandAccepted, PublicFailure> {
    let bytes =
        serde_json::to_vec(document).map_err(|_| invalid("invalid prompt-sequence JSON"))?;
    let document = PromptSequenceDocument::from_json(&bytes)
        .map_err(|error| invalid(&bounded(&error.to_string())))?;
    let author = AuthorRef::new(session.actor.as_str().to_owned())
        .map_err(|error| invalid(&error.to_string()))?;
    let compiled = compile_prompt_sequence(&document, author)
        .map_err(|error| invalid(&bounded(&error.to_string())))?;
    let revision = compiled.revision();
    let decision = authorize_definition(
        owner,
        session,
        revision.semantic().workflow().clone(),
        revision.id().clone(),
        store,
        if store {
            "command:import-prompt-sequence"
        } else {
            "command:validate-prompt-sequence"
        },
    )?;
    let replayed = if store {
        matches!(
            owner
                .store
                .put_revision(revision)
                .map_err(public_persistence)?,
            milkdrift_persistence::ImmutableRevisionPut::AlreadyPresent
        )
    } else {
        false
    };
    if store {
        owner.record_security_decision(&decision)?;
    }
    Ok(CommandAccepted {
        command_id: request.command_id.clone(),
        replayed,
        resulting_sequence: None,
        result_type: if store {
            "prompt_sequence_imported"
        } else {
            "prompt_sequence_valid"
        }
        .to_owned(),
        value: json!({
            "schema_version": 1,
            "sequence_id": document.sequence().id,
            "workflow_id": revision.semantic().workflow().as_str(),
            "revision_id": revision.id().as_str(),
            "semantic_digest": revision.content_digest().as_str(),
            "import_digest": compiled.import_digest(),
            "repository_profile_digest": compiled.repository_profile_digest(),
            "stages": compiled.stages(),
        }),
    })
}

fn authorize_definition(
    owner: &Owner,
    session: &ActorSession,
    workflow: milkdrift_blueprint::WorkflowId,
    revision: milkdrift_blueprint::RevisionId,
    store: bool,
    boundary: &str,
) -> Result<milkdrift_authority::AuthorityDecisionSnapshot, PublicFailure> {
    let mut resources = RequestedResourceFacts::empty();
    resources.workflow = Some(workflow);
    resources.revision = Some(revision);
    owner.authorize(
        session,
        if store {
            AuthorityOperation::ImportBlueprint
        } else {
            AuthorityOperation::ValidateBlueprint
        },
        resources,
        boundary,
    )
}
