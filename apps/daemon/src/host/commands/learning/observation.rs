//! Read only the declared run. Candidate reports cannot supply repair counts or trusted passes.
use super::{
    ActorSession, CommandRequest, Owner, PublicFailure, authorize_artifact, internal, invalid,
    public_persistence,
};
use milkdrift_blueprint::RevisionId;
use milkdrift_control::{
    ControlCommand, ControlResult,
    learning::{EvaluationSlot, LearningDeclaration, MethodEvaluation},
};
use milkdrift_persistence::{
    PeerExecutionStore, RunEventKind, WorkspaceStore, managed::ManagedResourceStore,
};
use milkdrift_workspace::ArtifactReference;
use std::collections::BTreeMap;

pub(super) fn accepted(
    owner: &Owner,
    slot: &EvaluationSlot,
) -> Result<Option<milkdrift_persistence::PeerExecutionSnapshot>, PublicFailure> {
    let caller = milkdrift_peer_protocol::ServingCaller {
        host: owner.host_id.clone(),
        principal: milkdrift_peer_protocol::ServingPrincipal::Client {
            actor: slot.invocation.actor.clone(),
        },
    };
    let key = milkdrift_peer_protocol::PeerRequestId::new(slot.invocation.request.as_str())
        .map_err(|e| invalid(&e.to_string()))?;
    owner
        .store
        .peer_execution_by_request(&caller, &key)
        .map_err(public_persistence)
}

fn completion(
    owner: &Owner,
    accepted: &milkdrift_persistence::PeerExecutionSnapshot,
) -> Result<Option<(bool, u64)>, PublicFailure> {
    use milkdrift_persistence::{PeerExecutionPhase, PeerExecutionSnapshot};
    let observation = match accepted {
        PeerExecutionSnapshot::Hot(record) => {
            let PeerExecutionPhase::Terminal { sequence, .. } = record.phase else {
                return Ok(None);
            };
            owner
                .store
                .peer_observations(
                    &record.caller,
                    &record.execution,
                    sequence.saturating_sub(1),
                    milkdrift_persistence::PageSize::new(1).map_err(public_persistence)?,
                )
                .map_err(public_persistence)?
                .observations
                .into_iter()
                .next()
        }
        PeerExecutionSnapshot::Archived(record) => {
            record.disposition.terminal_observation().cloned()
        }
    };
    let Some(observation) = observation else {
        return Ok(None);
    };
    let milkdrift_capability::InvocationEventKind::Terminal { terminal } = observation.event.kind()
    else {
        return Err(invalid("serving completion has no terminal observation"));
    };
    Ok(Some((
        terminal.status() == milkdrift_capability::TerminalStatus::Success,
        observation.observed_at_unix_ms,
    )))
}

pub(super) fn read(
    owner: &mut Owner,
    session: &ActorSession,
    request: &CommandRequest,
    declaration: &LearningDeclaration,
    slot: &EvaluationSlot,
    method: &RevisionId,
    input: &ArtifactReference,
) -> Result<Option<MethodEvaluation>, PublicFailure> {
    super::authorize_resource(owner, session, &slot.target, "resource.evidence")?;
    let Some(accepted) = accepted(owner, slot)? else {
        return Ok(None);
    };
    let plan = match &accepted {
        milkdrift_persistence::PeerExecutionSnapshot::Hot(record) => {
            record.published_invocation.as_ref()
        }
        milkdrift_persistence::PeerExecutionSnapshot::Archived(record) => {
            record.published_invocation.as_ref()
        }
    };
    let Some(plan) = plan else {
        // Acceptance may be followed by a preparation refusal before a child exists.
        // There is no run, usage or verification history to manufacture for that slot.
        return Ok(None);
    };
    let child = plan.child_run.clone();
    let inspected = owner.execute_control_result(
        session,
        request,
        None,
        None,
        ControlCommand::InspectRun { run: child.clone() },
        "learning-evaluation-run",
    )?;
    let ControlResult::RunInspection { value: run } = inspected else {
        return Err(internal());
    };
    let mut observed = MethodEvaluation {
        invocation: slot.invocation.clone(),
        run: child.clone(),
        method: method.clone(),
        input: input.clone(),
        history_complete: false,
        succeeded: false,
        violation: None,
        duration_ms: None,
        usage: None,
        allowance: None,
        output: None,
        submissions: Vec::new(),
    };
    if run
        .governing_agreement
        .as_ref()
        .is_none_or(|a| a.agreement_digest() != declaration.agreement)
    {
        observed.violation = Some("run did not accept the declared agreement".into());
    }
    if let milkdrift_control::ControllerAccountingRead::Active { account, .. } =
        &run.controller_accounting
    {
        observed.allowance = Some(account.declaration().budget().clone());
        if account.blocked().is_none() && account.reservations().is_empty() {
            observed.usage = Some(account.settled());
        }
    }
    let mut next = Some(milkdrift_persistence::RunSequence::FIRST);
    let mut evaluation_attempts = BTreeMap::new();
    let mut workspace_used = false;
    let mut has_unread_descendants = false;
    let mut evaluations = BTreeMap::new();
    let started = match &accepted {
        milkdrift_persistence::PeerExecutionSnapshot::Hot(record) => record.accepted_at_unix_ms,
        milkdrift_persistence::PeerExecutionSnapshot::Archived(record) => {
            record.accepted_at_unix_ms
        }
    };
    let mut terminal = None;
    let mut actual_input = false;
    let mut output_known = false;
    // Fixed scan bound is part of this finite reader. Overflow remains explicitly incomplete,
    // even if the compact run projection happens to show a successful final candidate.
    for _ in 0..64 {
        let page = owner.execute_control_result(
            session,
            request,
            None,
            None,
            ControlCommand::InspectTimeline {
                run: child.clone(),
                after: next,
                limit: milkdrift_persistence::PageSize::new(64).map_err(public_persistence)?,
            },
            "learning-evaluation-page",
        )?;
        let ControlResult::Timeline { value: page } = page else {
            return Err(internal());
        };
        for event in &page.events {
            if event.sequence() > run.sequence {
                break;
            }
            match event.kind() {
                RunEventKind::RunCreated {
                    revision, inputs, ..
                } => {
                    if revision != method {
                        return Err(invalid("evaluation run selected another starting method"));
                    }
                    for reference in inputs
                        .iter()
                        .filter(|r| r.key() == &declaration.input_field)
                    {
                        if owner
                            .store
                            .value(reference)
                            .map_err(public_persistence)?
                            .is_some_and(|entry| entry.value().as_artifact() == Some(input))
                        {
                            actual_input = true;
                        }
                    }
                }
                RunEventKind::RunTerminal {
                    outcome, outputs, ..
                } => {
                    observed.succeeded = *outcome == milkdrift_persistence::RunOutcome::Succeeded;
                    terminal = Some(event.occurred_at().get());
                    output_known = true;
                    if let Some(reference) = outputs
                        .iter()
                        .find(|r| r.key() == &declaration.candidate_output)
                    {
                        let mut resources = milkdrift_authority::RequestedResourceFacts::empty();
                        resources.run = Some(child.clone());
                        resources.workspace_scope = Some(reference.scope().scope().clone());
                        owner.authorize(
                            session,
                            milkdrift_authority::AuthorityOperation::ReadWorkspaceValue,
                            resources,
                            "learning-result",
                        )?;
                        if let Some(value) =
                            owner.store.value(reference).map_err(public_persistence)?
                        {
                            observed.output = value.value().as_artifact().cloned();
                            if let Some(artifact) = &observed.output {
                                authorize_artifact(owner, session, artifact)?;
                            }
                        } else {
                            output_known = false;
                        }
                    }
                }
                RunEventKind::CapabilityEntryDecisionRecorded { authorization, .. }
                | RunEventKind::CapabilityAdapterEntryDecisionRecorded { authorization, .. }
                | RunEventKind::CapabilityResolutionDenied { authorization, .. }
                    if !authorization.is_allowed() =>
                {
                    observed.violation =
                        Some("evaluation attempted work outside its current authority".into());
                }
                RunEventKind::CapabilityResolved {
                    attempt,
                    requirement,
                    ..
                } if requirement.operation().as_str() == "resource.evaluate_candidate" => {
                    evaluation_attempts.insert(attempt.clone(), event.sequence().get());
                }
                RunEventKind::CapabilityResolved { snapshot, .. }
                    if snapshot.category()
                        == &milkdrift_capability::CapabilityCategory::Process =>
                {
                    let binding = snapshot
                        .descriptor_extensions()
                        .iter()
                        .find(|(key, _)| {
                            key.as_str() == milkdrift_capability::managed::MANAGED_BINDING_EXTENSION
                        })
                        .map(|(_, value)| {
                            serde_json::from_value::<milkdrift_capability::managed::ManagedBinding>(
                                value.value().clone(),
                            )
                        })
                        .transpose()
                        .map_err(|_| invalid("invalid recorded workspace binding"))?;
                    // Scheduling and resolution share one durable commit, with NodeScheduled
                    // first. Inspect the exact binding here instead of waiting for a later
                    // scheduling event that will never arrive for this attempt.
                    if let Some(binding) = binding {
                        if binding.installation == slot.workspace
                            && binding.recipe_digest == slot.workspace_configuration
                            && binding.generation == slot.workspace_generation
                        {
                            workspace_used = true;
                        } else {
                            observed.violation =
                                Some("process used another workspace or recipe generation".into());
                        }
                    } else {
                        observed.violation =
                            Some("process has no recorded managed workspace ownership".into());
                    }
                }
                RunEventKind::SubworkflowCreated { .. }
                | RunEventKind::PublishedInvocationPlanned { .. } => {
                    // This finite reader cannot qualify an unexamined descendant. Its use is still
                    // charged by the enclosing account, but missing verification remains unknown.
                    has_unread_descendants = true;
                }
                RunEventKind::NodeOutputPublished {
                    attempt,
                    artifact: Some(artifact),
                    ..
                } if evaluation_attempts.contains_key(attempt) => {
                    authorize_artifact(owner, session, artifact)?;
                    if artifact.size_bytes() > 65_536 {
                        return Err(invalid("verifier response exceeds learning read bound"));
                    }
                    let mut bytes = Vec::new();
                    while bytes.len() as u64 != artifact.size_bytes() {
                        let chunk = owner.artifact_range(
                            session,
                            artifact.artifact().as_str(),
                            bytes.len() as u64,
                            u32::try_from(artifact.size_bytes() - bytes.len() as u64)
                                .map_err(|_| internal())?
                                .min(16_384),
                            "learning-verification",
                        )?;
                        if chunk.bytes.is_empty() {
                            return Err(invalid("verifier response unavailable"));
                        }
                        bytes.extend(chunk.bytes);
                    }
                    if !artifact.verifies(&bytes) {
                        return Err(invalid("verifier response integrity mismatch"));
                    }
                    let response: serde_json::Value = serde_json::from_slice(&bytes)
                        .map_err(|_| invalid("invalid verifier response"))?;
                    let Some(identity) = response
                        .pointer("/evaluation/identity")
                        .and_then(serde_json::Value::as_str)
                    else {
                        continue;
                    };
                    let evaluation = owner
                        .store
                        .managed_evaluation(identity)
                        .map_err(public_persistence)?
                        .ok_or_else(|| {
                            invalid("host-owned verification evidence is unavailable")
                        })?;
                    let accepted: milkdrift_workspace::CandidateEvaluation =
                        serde_json::from_value(response["evaluation"].clone())
                            .map_err(|_| invalid("invalid accepted verification receipt"))?;
                    accepted
                        .validate()
                        .map_err(|_| invalid("invalid accepted verification receipt"))?;
                    // The immutable command receipt records acceptance before the verifier
                    // finishes. Its subject and identity bind the private completed observation;
                    // the initial unknown checks must not be mistaken for the final evidence.
                    if accepted.subject != evaluation.subject
                        || accepted.started_at != evaluation.started_at
                        || accepted.expires_at != evaluation.expires_at
                    {
                        return Err(invalid(
                            "uploaded verifier report contradicts its private record",
                        ));
                    }
                    if evaluations.insert(attempt.clone(), evaluation).is_some() {
                        return Err(invalid("duplicate verifier output for one attempt"));
                    }
                }
                _ => {}
            }
        }
        if page
            .events
            .last()
            .is_some_and(|e| e.sequence() >= run.sequence)
        {
            observed.history_complete = actual_input
                && output_known
                && workspace_used
                && !has_unread_descendants
                && evaluations.len() == evaluation_attempts.len();
            break;
        }
        next = page.next_sequence;
        if next.is_none() {
            break;
        }
    }
    // Internal success alone does not establish that the callable method returned its result.
    // A lost/failed public copy remains incomplete, and result delivery consumes elapsed time.
    let completed = completion(owner, &accepted)?;
    observed.succeeded &= completed.is_some_and(|(succeeded, _)| succeeded);
    observed.duration_ms = completed.and_then(|(_, end)| {
        terminal
            .filter(|internal_end| *internal_end <= end)
            .and_then(|_| end.checked_sub(started))
    });
    let mut ordered = evaluations
        .into_iter()
        .map(|(attempt, evaluation)| {
            evaluation_attempts
                .get(&attempt)
                .map(|sequence| (*sequence, evaluation))
                .ok_or_else(internal)
        })
        .collect::<Result<Vec<_>, _>>()?;
    ordered.sort_by_key(|(sequence, _)| *sequence);
    observed.submissions = ordered
        .into_iter()
        .map(|(_, evaluation)| evaluation)
        .collect();
    Ok(Some(observed))
}
