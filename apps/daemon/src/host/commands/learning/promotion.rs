//! A policy is an operator's immutable, conditional publication authorization. Execution still
//! checks that operator's current grant through the ordinary publication owner.
use super::{
    ActorSession, CommandRequest, LearningOutcome, LearningReceiptReference, Owner, PublicFailure,
    Record, authorize_declaration, declaration_record, invalid, public_control, public_persistence,
    read,
};
use milkdrift_authority::{AuthorityOperation, RequestedResourceFacts};
use milkdrift_capability::OperationId;
use milkdrift_persistence::RunQueryStore;

#[allow(clippy::too_many_arguments)]
pub(super) fn preauthorize(
    owner: &mut Owner,
    session: &ActorSession,
    request: &CommandRequest,
    reference: LearningReceiptReference,
    executor: milkdrift_authority::ActorRef,
    method: Box<milkdrift_persistence::published::PublishedMethod>,
    expected_previous_version: u64,
) -> Result<Record, PublicFailure> {
    let (declaration, _) = declaration_record(owner, &reference)?;
    authorize_declaration(owner, session, request, &declaration)?;
    method.validate().map_err(public_persistence)?;
    if owner
        .store
        .run_summary(&declaration.proposal_run)
        .map_err(public_persistence)?
        .is_some()
        || method.revision != declaration.baseline
        || method.agreement != declaration.agreement
        || method.descriptor.identity() != &declaration.publication
        || method.descriptor.descriptor_revision() != declaration.generation
    {
        return Err(invalid(
            "automatic publication policy must fix the declared envelope before proposal generation",
        ));
    }
    let mut resources = RequestedResourceFacts::empty();
    resources.capability = Some(declaration.publication);
    resources.capability_operation =
        Some(OperationId::new("method.publish").map_err(|e| invalid(&e.to_string()))?);
    let authorization = owner.authorize(
        session,
        AuthorityOperation::AdministerCapabilities,
        resources,
        "method.publish",
    )?;
    owner.record_security_decision(&authorization)?;
    Ok(Record::Policy {
        declaration: reference,
        executor,
        method,
        expected_previous_version,
        authorization: Box::new(authorization),
    })
}

pub(super) fn automatic(
    owner: &mut Owner,
    session: &ActorSession,
    request: &CommandRequest,
    policy: LearningReceiptReference,
    comparison: LearningReceiptReference,
) -> Result<Record, PublicFailure> {
    let Record::Policy {
        declaration,
        executor,
        mut method,
        expected_previous_version,
        authorization,
    } = read(owner, &policy)?
    else {
        return Err(invalid(
            "automatic promotion requires an operator policy receipt",
        ));
    };
    if executor != session.actor {
        return Err(invalid(
            "automatic promotion executor differs from the operator policy",
        ));
    }
    let (decl, _) = declaration_record(owner, &declaration)?;
    authorize_declaration(owner, session, request, &decl)?;
    let Record::Comparison {
        declaration: compared,
        result,
        ..
    } = read(owner, &comparison)?
    else {
        return Err(invalid("automatic promotion requires a comparison receipt"));
    };
    if compared != declaration
        || result.outcome != LearningOutcome::Eligible
        || result.declaration != decl.digest().map_err(public_control)?
    {
        return Err(invalid(
            "automatic promotion did not satisfy its exact declared criterion",
        ));
    }
    method.revision = result.candidate;
    let fingerprint = super::super::super::receipts::command_fingerprint(session, request)?;
    let published = owner
        .workflow()?
        .publications
        .as_ref()
        .ok_or_else(|| invalid("publication unavailable"))?
        .publish(
            *method,
            Some(expected_previous_version),
            &authorization,
            &fingerprint,
        )
        .map_err(public_control)?;
    // The policy reference remains explicit in the receipt alongside the ordinary publication.
    Ok(Record::Promotion {
        comparison,
        policy: Some(policy),
        publication: reference(&published)?,
    })
}

// A publication already owns its complete method and authorization. Keeping that potentially
// large document again could exceed the learning receipt limit *after* publication committed.
// The digest and original request let operators inspect that exact publication through its owner.
pub(super) fn reference(
    record: &milkdrift_persistence::published::PublishedMethodRecord,
) -> Result<serde_json::Value, PublicFailure> {
    Ok(serde_json::json!({
        "capability": record.method.descriptor.identity(),
        "generation": record.method.descriptor.descriptor_revision(),
        "method_digest": record.method.digest().map_err(public_persistence)?,
        "publication_request": record.publication_request,
        "version": record.version
    }))
}
