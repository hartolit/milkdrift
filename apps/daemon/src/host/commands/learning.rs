//! Learning composes existing authorized reads and immutable command receipts. It has no table,
//! background loop or execution queue. A failed comparison never edits the publication inventory.
mod observation;
mod promotion;
mod proposal;
use super::super::{
    Owner, PublicFailure, internal, invalid, not_found, public_control, public_persistence,
};
use crate::auth::ActorSession;
use milkdrift_authority::{AuthorityOperation, RequestedResourceFacts};
use milkdrift_blueprint::RevisionId;
use milkdrift_capability::{CapabilityId, OperationId};
use milkdrift_control::{
    ControlCommand, ControlResult, WorkflowProposalDocument,
    learning::{
        KnowledgeSelection, LearningComparison, LearningDeclaration, LearningOutcome,
        LearningReceiptReference, LearningRequest,
    },
};
use milkdrift_control_protocol::{CommandAccepted, CommandRequest};
use milkdrift_persistence::{
    ApplicationCommandStore, ArtifactStore, RevisionStore, RunQueryStore,
    managed::ManagedResourceStore,
};
use milkdrift_workspace::ArtifactReference;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum Record {
    Selection {
        selection: KnowledgeSelection,
        pages: Vec<Value>,
        artifact: ArtifactReference,
    },
    Declaration {
        declaration: LearningDeclaration,
        digest: String,
        recorded_at: u64,
    },
    Candidate {
        declaration: LearningReceiptReference,
        revision: RevisionId,
        proposal: Value,
        expected_benefit: String,
        applicability: String,
        counterevidence: String,
    },
    Comparison {
        declaration: LearningReceiptReference,
        candidate: LearningReceiptReference,
        result: LearningComparison,
        observations: Vec<(
            Option<milkdrift_control::learning::MethodEvaluation>,
            Option<milkdrift_control::learning::MethodEvaluation>,
        )>,
    },
    Promotion {
        comparison: LearningReceiptReference,
        policy: Option<LearningReceiptReference>,
        publication: Value,
    },
    Policy {
        declaration: LearningReceiptReference,
        executor: milkdrift_authority::ActorRef,
        method: Box<milkdrift_persistence::published::PublishedMethod>,
        expected_previous_version: u64,
        authorization: Box<milkdrift_authority::AuthorityDecisionSnapshot>,
    },
}

pub(super) fn execute(
    owner: &mut Owner,
    session: &ActorSession,
    request: &CommandRequest,
    document: &Value,
) -> Result<CommandAccepted, PublicFailure> {
    let command: LearningRequest =
        serde_json::from_value(document.clone()).map_err(|e| invalid(&e.to_string()))?;
    let operation = command.operation();
    let authority_operation = match command {
        LearningRequest::Candidate { .. } => AuthorityOperation::ProposeOffline,
        LearningRequest::Inspect { .. } => AuthorityOperation::InspectRevision,
        _ => AuthorityOperation::AdministerCapabilities,
    };
    let mut resources = RequestedResourceFacts::empty();
    resources.capability =
        Some(CapabilityId::new("milkdrift-workflow-control").map_err(|e| invalid(&e.to_string()))?);
    resources.capability_operation =
        Some(OperationId::new(operation).map_err(|e| invalid(&e.to_string()))?);
    if let LearningRequest::Candidate { proposal, .. } = &command {
        let p = WorkflowProposalDocument::from_json(
            &serde_json::to_vec(proposal).map_err(|_| internal())?,
        )
        .map_err(public_control)?;
        resources.workflow = Some(p.proposal().workflow().clone());
    }
    let decision = owner.authorize(session, authority_operation, resources, operation)?;
    owner.record_security_decision(&decision)?;
    let record = match command {
        LearningRequest::Select { selection } => select(owner, session, request, selection)?,
        LearningRequest::Declare { declaration } => declare(owner, session, request, declaration)?,
        LearningRequest::Candidate {
            declaration,
            proposal,
            expected_benefit,
            applicability,
            counterevidence,
        } => candidate(
            owner,
            session,
            request,
            declaration,
            proposal,
            expected_benefit,
            applicability,
            counterevidence,
        )?,
        LearningRequest::Compare {
            declaration,
            candidate,
        } => {
            let (decl, _) = declaration_record(owner, &declaration)?;
            authorize_declaration(owner, session, request, &decl)?;
            let Record::Candidate {
                declaration: source,
                revision,
                ..
            } = read(owner, &candidate)?
            else {
                return Err(invalid("candidate receipt required"));
            };
            if source != declaration {
                return Err(invalid("candidate belongs to another declaration"));
            }
            let observations = decl
                .pairs
                .iter()
                .map(|pair| {
                    Ok((
                        observation::read(
                            owner,
                            session,
                            request,
                            &decl,
                            &pair.baseline,
                            &decl.baseline,
                            &pair.input,
                        )?,
                        observation::read(
                            owner,
                            session,
                            request,
                            &decl,
                            &pair.candidate,
                            &revision,
                            &pair.input,
                        )?,
                    ))
                })
                .collect::<Result<Vec<_>, PublicFailure>>()?;
            let result =
                milkdrift_control::learning::compare_methods(&decl, &revision, &observations)
                    .map_err(public_control)?;
            Record::Comparison {
                declaration,
                candidate,
                result,
                observations,
            }
        }
        LearningRequest::Preauthorize {
            declaration,
            executor,
            method,
            expected_previous_version,
        } => promotion::preauthorize(
            owner,
            session,
            request,
            declaration,
            executor,
            method,
            expected_previous_version,
        )?,
        LearningRequest::AutoPromote { policy, comparison } => {
            promotion::automatic(owner, session, request, policy, comparison)?
        }
        LearningRequest::Promote {
            comparison,
            method,
            expected_previous_version,
        } => {
            let Record::Comparison {
                declaration,
                result,
                ..
            } = read(owner, &comparison)?
            else {
                return Err(invalid("comparison receipt required"));
            };
            let (decl, _) = declaration_record(owner, &declaration)?;
            authorize_declaration(owner, session, request, &decl)?;
            if result.outcome != LearningOutcome::Eligible
                || result.declaration != decl.digest().map_err(public_control)?
                || method.revision != result.candidate
                || method.agreement != decl.agreement
                || method.descriptor.identity() != &decl.publication
                || method.descriptor.descriptor_revision() != decl.generation
            {
                return Err(invalid(
                    "promotion does not match an eligible exact comparison",
                ));
            }
            let mut resources = RequestedResourceFacts::empty();
            resources.capability = Some(decl.publication.clone());
            resources.capability_operation =
                Some(OperationId::new("method.publish").map_err(|e| invalid(&e.to_string()))?);
            let authorization = owner.authorize(
                session,
                AuthorityOperation::AdministerCapabilities,
                resources,
                "method.publish",
            )?;
            owner.record_security_decision(&authorization)?;
            let fingerprint = super::super::receipts::command_fingerprint(session, request)?;
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
            Record::Promotion {
                comparison,
                policy: None,
                publication: promotion::reference(&published)?,
            }
        }
        LearningRequest::Inspect { receipt } => {
            let record = read(owner, &receipt)?;
            authorize_record(owner, session, request, &record)?;
            record
        }
    };
    let value = serde_json::to_value(record).map_err(|_| internal())?;
    if serde_json::to_vec(&value).map_err(|_| internal())?.len() > 262_144 {
        return Err(invalid(
            "selected learning result exceeds 262144 bytes; select smaller pages",
        ));
    }
    Ok(CommandAccepted {
        command_id: request.command_id.clone(),
        replayed: false,
        resulting_sequence: None,
        result_type: operation.into(),
        value,
    })
}

fn read(owner: &Owner, reference: &LearningReceiptReference) -> Result<Record, PublicFailure> {
    let receipt = owner
        .store
        .application_command_receipt(&reference.actor, &reference.command)
        .map_err(public_persistence)?
        .ok_or_else(not_found)?;
    let result = super::super::receipts::stored_application_result(&receipt)?;
    if ![
        "learning.select",
        "learning.declare",
        "learning.candidate",
        "learning.compare",
        "learning.promote",
        "learning.preauthorize",
        "learning.auto_promote",
    ]
    .contains(&result.result_type.as_str())
    {
        return Err(invalid("reference is not an accepted learning receipt"));
    }
    serde_json::from_value(result.value).map_err(|_| internal())
}

fn declaration_record(
    owner: &Owner,
    reference: &LearningReceiptReference,
) -> Result<(LearningDeclaration, u64), PublicFailure> {
    match read(owner, reference)? {
        Record::Declaration {
            declaration,
            digest,
            recorded_at,
        } if declaration.digest().map_err(public_control)? == digest => {
            Ok((declaration, recorded_at))
        }
        _ => Err(invalid("exact evaluation declaration receipt required")),
    }
}

fn revision(
    owner: &Owner,
    session: &ActorSession,
    request: &CommandRequest,
    revision: &RevisionId,
) -> Result<milkdrift_blueprint::BlueprintRevision, PublicFailure> {
    owner.execute_control_result(
        session,
        request,
        None,
        None,
        ControlCommand::InspectRevision {
            revision: revision.clone(),
        },
        "learning-revision",
    )?;
    owner
        .store
        .revision(revision)
        .map_err(public_persistence)?
        .ok_or_else(not_found)
}

fn authorize_artifact(
    owner: &mut Owner,
    session: &ActorSession,
    reference: &ArtifactReference,
) -> Result<(), PublicFailure> {
    // Use the existing public metadata and content boundary, including sensitivity and identity
    // refusal. The one-byte read establishes content authority without eagerly loading a document.
    let metadata = owner.artifact_metadata(session, reference.artifact().as_str())?;
    let actual = owner
        .store
        .metadata(reference.artifact())
        .map_err(public_persistence)?
        .ok_or_else(not_found)?;
    if actual.reference() != reference {
        return Err(invalid("selected artifact does not match retained bytes"));
    }
    for cause in actual
        .provenance()
        .causes()
        .iter()
        .chain([actual.provenance().producer()])
    {
        if let milkdrift_workspace::CausalReference::WorkspaceValue { reference } = cause {
            let mut resources = RequestedResourceFacts::empty();
            resources.run = Some(reference.scope().run().clone());
            resources.workspace_scope = Some(reference.scope().scope().clone());
            owner.authorize(
                session,
                AuthorityOperation::ReadWorkspaceValue,
                resources,
                "learning-source-scope",
            )?;
        }
    }
    let _ = metadata;
    owner.artifact_range(session, reference.artifact().as_str(), 0, 1, "learning")?;
    Ok(())
}

fn authorize_resource(
    owner: &Owner,
    session: &ActorSession,
    target: &milkdrift_capability::managed::ManagedName,
    operation: &str,
) -> Result<(), PublicFailure> {
    let mut resources = RequestedResourceFacts::empty();
    resources.capability =
        Some(CapabilityId::new(format!("managed.{target}")).map_err(|e| invalid(&e.to_string()))?);
    resources.capability_operation =
        Some(OperationId::new(operation).map_err(|e| invalid(&e.to_string()))?);
    owner.authorize(
        session,
        AuthorityOperation::AdministerCapabilities,
        resources,
        operation,
    )?;
    Ok(())
}

fn select(
    owner: &mut Owner,
    session: &ActorSession,
    request: &CommandRequest,
    selection: KnowledgeSelection,
) -> Result<Record, PublicFailure> {
    if selection.artifacts.len() > 32 || selection.pages.len() > 8 || selection.pages.is_empty() {
        return Err(invalid(
            "selection requires 1..=8 exact pages and at most 32 artifacts",
        ));
    }
    revision(owner, session, request, &selection.method)?;
    let installation = owner
        .store
        .managed_installation(&selection.workspace)
        .map_err(public_persistence)?
        .ok_or_else(not_found)?;
    if installation.removed {
        return Err(invalid("knowledge setup is removed"));
    }
    let mut resources = RequestedResourceFacts::empty();
    resources.capability = Some(
        CapabilityId::new(format!("managed.{}", selection.workspace.as_str()))
            .map_err(|e| invalid(&e.to_string()))?,
    );
    resources.capability_operation =
        Some(OperationId::new("resource.inspect").map_err(|e| invalid(&e.to_string()))?);
    owner.authorize(
        session,
        AuthorityOperation::AdministerCapabilities,
        resources,
        "learning-workspace",
    )?;
    authorize_artifact(owner, session, &selection.guidance)?;
    for artifact in &selection.artifacts {
        authorize_artifact(owner, session, artifact)?;
    }
    if let Some(previous) = &selection.supersedes {
        let Record::Selection {
            selection: prior, ..
        } = read(owner, previous)?
        else {
            return Err(invalid("superseded selection receipt required"));
        };
        if prior.workspace != selection.workspace {
            return Err(invalid(
                "supersession must preserve its applicable workspace",
            ));
        }
        if prior.method != selection.method {
            let Some(approval) = &selection.approval else {
                return Err(invalid(
                    "changing the guidance's applicable method requires its promotion receipt",
                ));
            };
            let Record::Promotion { comparison, .. } = read(owner, approval)? else {
                return Err(invalid("guidance method transition requires promotion"));
            };
            let Record::Comparison { result, .. } = read(owner, &comparison)? else {
                return Err(internal());
            };
            if result.baseline != prior.method || result.candidate != selection.method {
                return Err(invalid(
                    "guidance supersession differs from the approved method transition",
                ));
            }
        }
    }
    if let Some(approval) = &selection.approval {
        let Record::Promotion { comparison, .. } = read(owner, approval)? else {
            return Err(invalid("approved guidance requires a promotion receipt"));
        };
        let Record::Comparison { result, .. } = read(owner, &comparison)? else {
            return Err(invalid("guidance promotion comparison is unavailable"));
        };
        if result.candidate != selection.method || result.outcome != LearningOutcome::Eligible {
            return Err(invalid("guidance approval names another method"));
        }
    }
    let mut pages = Vec::new();
    for page in &selection.pages {
        if page.first == 0 || page.count == 0 || page.count > 64 {
            return Err(invalid("source page requires first >= 1 and count 1..=64"));
        }
        let result = owner.execute_control_result(
            session,
            request,
            None,
            None,
            ControlCommand::InspectTimeline {
                run: page.run.clone(),
                after: Some(milkdrift_persistence::RunSequence::new(page.first)),
                limit: milkdrift_persistence::PageSize::new(page.count)
                    .map_err(public_persistence)?,
            },
            "learning-page",
        )?;
        let ControlResult::Timeline { value } = result else {
            return Err(internal());
        };
        if value.events.len() != page.count as usize
            || value
                .events
                .first()
                .is_none_or(|e| e.sequence().get() != page.first)
        {
            return Err(invalid(
                "required source page is missing; missing evidence is not an absent failure",
            ));
        }
        // Use the same non-secret projection as ordinary timeline reads. Selecting history
        // must not turn private branch inputs, prompts or invocation arguments into a new
        // broadly readable artifact. Content requires an explicitly authorized artifact selection.
        let events = value
            .events
            .iter()
            .map(super::super::read_model::public_timeline)
            .collect::<Vec<_>>();
        pages.push(serde_json::json!({"run":page.run,"first":page.first,"count":page.count,"events":events}));
    }
    let bytes = serde_json::to_vec(
        &serde_json::json!({"schema_version":1,"selection":selection,"pages":pages}),
    )
    .map_err(|_| internal())?;
    if bytes.len() > 245_760 {
        return Err(invalid(
            "selected knowledge exceeds 245760 bytes; reduce the exact source pages",
        ));
    }
    let upload = milkdrift_control_protocol::InputUploadRequest::from_content(
        owner.host_id.to_string(),
        format!(
            "learning:{}",
            super::super::receipts::command_fingerprint(session, request)?
        ),
        "application/json".into(),
        "restricted".into(),
        &bytes,
    )
    .map_err(|e| invalid(&e.to_string()))?;
    let uploaded = owner.upload_input(session, &upload)?;
    let artifact = owner
        .store
        .metadata(
            &milkdrift_workspace::ArtifactId::new(uploaded.artifact_id)
                .map_err(|e| invalid(&e.to_string()))?,
        )
        .map_err(public_persistence)?
        .ok_or_else(internal)?
        .reference()
        .clone();
    Ok(Record::Selection {
        selection,
        pages,
        artifact,
    })
}

fn declare(
    owner: &mut Owner,
    session: &ActorSession,
    request: &CommandRequest,
    declaration: LearningDeclaration,
) -> Result<Record, PublicFailure> {
    declaration.validate().map_err(public_control)?;
    authorize_declaration(owner, session, request, &declaration)?;
    if owner
        .store
        .run_summary(&declaration.proposal_run)
        .map_err(public_persistence)?
        .is_some()
    {
        return Err(invalid("proposal run must not exist before declaration"));
    }
    let Record::Selection { selection, .. } = read(owner, &declaration.selection)? else {
        return Err(invalid(
            "source selection must be accepted before declaration",
        ));
    };
    if selection.method != declaration.baseline {
        return Err(invalid("source selection names a different baseline"));
    }
    let source = selection
        .artifacts
        .iter()
        .chain([&selection.guidance])
        .map(|a| a.artifact())
        .collect::<std::collections::BTreeSet<_>>();
    for pair in &declaration.pairs {
        if source.contains(pair.input.artifact())
            || selection
                .artifacts
                .iter()
                .chain([&selection.guidance])
                .any(|reference| reference.digest() == pair.input.digest())
        {
            return Err(invalid(
                "held-out evaluation input leaked into source selection",
            ));
        }
        for slot in [&pair.baseline, &pair.candidate] {
            authorize_resource(owner, session, &slot.target, "resource.inspect")?;
            authorize_resource(owner, session, &slot.workspace, "resource.inspect")?;
            if observation::accepted(owner, slot)?.is_some() {
                return Err(invalid(
                    "evaluation invocations must not exist before declaration",
                ));
            }
            let worker = owner
                .store
                .managed_installation(&slot.workspace)
                .map_err(public_persistence)?
                .ok_or_else(not_found)?;
            if worker.removed
                || worker.pending.is_some()
                || worker.generation != slot.workspace_generation
                || worker
                    .current
                    .as_ref()
                    .is_none_or(|setup| setup.recipe.digest != slot.workspace_configuration)
            {
                return Err(invalid(
                    "evaluation worker differs from the declared prepared generation",
                ));
            }
            let record = owner
                .store
                .managed_installation(&slot.target)
                .map_err(public_persistence)?
                .ok_or_else(not_found)?;
            let setup = record.current.as_ref().ok_or_else(not_found)?;
            let protection = setup
                .protection
                .as_ref()
                .ok_or_else(|| invalid("evaluation target is unprotected"))?;
            if record.removed
                || record.pending.is_some()
                || record.generation != slot.generation
                || setup.recipe.digest != slot.configuration
                || protection.policy.required_checks != declaration.checks
                || protection.agreement != declaration.agreement
                || protection
                    .policy
                    .digest()
                    .map_err(|e| invalid(&e.to_string()))?
                    != declaration.policy
                || protection.policy.verifier != declaration.verifier
            {
                return Err(invalid("evaluation target differs from declaration"));
            }
        }
    }
    Ok(Record::Declaration {
        digest: declaration.digest().map_err(public_control)?,
        declaration,
        recorded_at: owner.now()?,
    })
}

fn authorize_declaration(
    owner: &mut Owner,
    session: &ActorSession,
    request: &CommandRequest,
    declaration: &LearningDeclaration,
) -> Result<(), PublicFailure> {
    let base = revision(owner, session, request, &declaration.baseline)?;
    if base
        .semantic()
        .agreement()
        .is_none_or(|a| a.digest() != declaration.agreement)
    {
        return Err(invalid("baseline agreement differs from declaration"));
    }
    if !base
        .semantic()
        .interface()
        .inputs()
        .keys()
        .any(|key| key.as_str() == declaration.input_field.as_str())
        || !base
            .semantic()
            .interface()
            .outputs()
            .keys()
            .any(|key| key.as_str() == declaration.candidate_output.as_str())
    {
        return Err(invalid(
            "evaluation input and candidate output must name declared workflow fields",
        ));
    }
    for reference in declaration
        .pairs
        .iter()
        .map(|p| &p.input)
        .chain(declaration.provenance.iter())
    {
        authorize_artifact(owner, session, reference)?;
    }
    for pair in &declaration.pairs {
        for slot in [&pair.baseline, &pair.candidate] {
            authorize_resource(owner, session, &slot.workspace, "resource.inspect")?;
            authorize_resource(owner, session, &slot.target, "resource.inspect")?;
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn candidate(
    owner: &mut Owner,
    session: &ActorSession,
    request: &CommandRequest,
    declaration_ref: LearningReceiptReference,
    proposal: Value,
    expected_benefit: String,
    applicability: String,
    counterevidence: String,
) -> Result<Record, PublicFailure> {
    if [&expected_benefit, &applicability, &counterevidence]
        .into_iter()
        .any(|s| s.trim().is_empty() || s.len() > 4096)
    {
        return Err(invalid(
            "candidate requires bounded benefit, applicability and counterevidence",
        ));
    }
    let (declaration, declared_at) = declaration_record(owner, &declaration_ref)?;
    let Record::Selection {
        selection,
        pages,
        artifact,
    } = read(owner, &declaration.selection)?
    else {
        return Err(invalid("source selection absent"));
    };
    let parsed = WorkflowProposalDocument::from_json(
        &serde_json::to_vec(&proposal).map_err(|_| internal())?,
    )
    .map_err(public_control)?;
    let p = parsed.proposal();
    if expected_benefit != p.rationale()
        || applicability != p.assumptions().join("; ")
        || counterevidence != p.risk_notes().join("; ")
    {
        return Err(invalid(
            "candidate hypothesis must retain the model's rationale, assumptions and risk notes",
        ));
    }
    if p.base_revision() != &declaration.baseline
        || p.run().is_some()
        || p.proposer() != &session.actor
    {
        return Err(invalid(
            "candidate must propose a reusable revision against the declared baseline",
        ));
    }
    for source in selection
        .artifacts
        .iter()
        .chain([&selection.guidance, &artifact])
    {
        authorize_artifact(owner, session, source)?;
    }
    proposal::verify(
        owner,
        session,
        request,
        p,
        &artifact,
        &selection,
        &declaration,
        declared_at,
    )?;
    let mut permitted = selection
        .artifacts
        .iter()
        .chain([&selection.guidance])
        .map(|a| a.artifact().as_str().to_owned())
        .collect::<std::collections::BTreeSet<_>>();
    for page in pages {
        if let Some(events) = page["events"].as_array() {
            for event in events {
                if let Some(id) = event["detail"]["event_id"].as_str() {
                    permitted.insert(id.into());
                }
            }
        }
    }
    if p.evidence().is_empty()
        || p.evidence()
            .iter()
            .any(|e| !permitted.contains(e.id.as_str()))
    {
        return Err(invalid(
            "candidate cites invented or unselected source evidence",
        ));
    }
    let base = revision(owner, session, request, &declaration.baseline)?;
    let preview = base
        .revise(
            p.base_revision(),
            p.mutation().clone(),
            milkdrift_blueprint::AuthorRef::new(session.actor.as_str())
                .map_err(|e| invalid(&e.to_string()))?,
            "validate evidence-derived candidate before retention".to_owned(),
        )
        .map_err(|e| invalid(&e.to_string()))?;
    milkdrift_blueprint::validate_agreement_adoption(&base, &preview)
        .map_err(|e| invalid(&e.to_string()))?;
    if base.semantic() == preview.semantic() {
        return Err(invalid("candidate made no semantic method change"));
    }
    if request.expected_sequence.is_some()
        || request
            .expected_revision
            .as_ref()
            .is_some_and(|revision| revision != p.base_revision().as_str())
    {
        return Err(invalid(
            "reusable candidate guard differs from its exact declared baseline",
        ));
    }
    // The declaration already binds this immutable base. Preserve any caller guard and supply
    // that same exact revision to the ordinary proposal owner's mandatory control guard.
    let mut guarded = request.clone();
    guarded.expected_revision = Some(p.base_revision().to_string());
    let submitted = super::proposals::submit(owner, session, &guarded, &proposal)?;
    let submission: Value = submitted.value;
    let revision_id: RevisionId =
        serde_json::from_value(submission["proposed_revision"].clone()).map_err(|_| internal())?;
    Ok(Record::Candidate {
        declaration: declaration_ref,
        revision: revision_id,
        proposal: submission,
        expected_benefit,
        applicability,
        counterevidence,
    })
}

fn authorize_record(
    owner: &mut Owner,
    session: &ActorSession,
    request: &CommandRequest,
    record: &Record,
) -> Result<(), PublicFailure> {
    match record {
        Record::Selection {
            selection,
            artifact,
            ..
        } => {
            revision(owner, session, request, &selection.method)?;
            authorize_resource(owner, session, &selection.workspace, "resource.inspect")?;
            authorize_artifact(owner, session, artifact)?;
            for reference in selection.artifacts.iter().chain([&selection.guidance]) {
                authorize_artifact(owner, session, reference)?;
            }
            for page in &selection.pages {
                owner.execute_control_result(
                    session,
                    request,
                    None,
                    None,
                    ControlCommand::InspectTimeline {
                        run: page.run.clone(),
                        after: Some(milkdrift_persistence::RunSequence::new(page.first)),
                        limit: milkdrift_persistence::PageSize::new(page.count)
                            .map_err(public_persistence)?,
                    },
                    "learning-inspect",
                )?;
            }
        }
        Record::Declaration { declaration, .. } => {
            authorize_declaration(owner, session, request, declaration)?
        }
        Record::Policy { declaration, .. } => {
            let (declaration, _) = declaration_record(owner, declaration)?;
            authorize_declaration(owner, session, request, &declaration)?;
        }
        Record::Candidate {
            declaration,
            revision: candidate,
            ..
        } => {
            let (declaration, _) = declaration_record(owner, declaration)?;
            revision(owner, session, request, &declaration.baseline)?;
            revision(owner, session, request, candidate)?;
            let Record::Selection {
                selection,
                artifact,
                ..
            } = read(owner, &declaration.selection)?
            else {
                return Err(internal());
            };
            for reference in selection
                .artifacts
                .iter()
                .chain([&selection.guidance, &artifact])
            {
                authorize_artifact(owner, session, reference)?;
            }
        }
        Record::Comparison {
            declaration,
            observations,
            ..
        } => {
            let (declaration, _) = declaration_record(owner, declaration)?;
            authorize_declaration(owner, session, request, &declaration)?;
            for pair in &declaration.pairs {
                for slot in [&pair.baseline, &pair.candidate] {
                    authorize_resource(owner, session, &slot.target, "resource.evidence")?;
                }
            }
            for observation in observations.iter().flat_map(|(a, b)| [a, b]).flatten() {
                owner.execute_control_result(
                    session,
                    request,
                    None,
                    None,
                    ControlCommand::InspectRun {
                        run: observation.run.clone(),
                    },
                    "learning-comparison-history",
                )?;
            }
        }
        Record::Promotion { comparison, .. } => {
            let Record::Comparison { declaration, .. } = read(owner, comparison)? else {
                return Err(invalid("promotion comparison absent"));
            };
            let (declaration, _) = declaration_record(owner, &declaration)?;
            authorize_declaration(owner, session, request, &declaration)?;
        }
    }
    Ok(())
}
