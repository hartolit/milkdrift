use super::{bump, busy, conflict, invalid, put_record, read_record, uses::locate};
use crate::{
    codec, error, json,
    schema::{MANAGED_LINKS, MANAGED_USES, RUN_EVENTS},
};
use milkdrift_capability::ResolvedCapabilitySnapshot;
use milkdrift_capability::managed::{
    MANAGED_BINDING_EXTENSION, MAX_MANAGED_USES, ManagedBinding, ManagedResourceKind,
};
use milkdrift_persistence::{
    AtomicRunCommitRequest, PeerExecutionRecord, PersistenceError, RunEventKind,
    managed::{ManagedExecution, ManagedUse, ManagedUsePhase},
};
use redb::ReadableTable;
use serde::{Deserialize, Serialize};

/// The index points to one authoritative parent event, not a second association authority.
#[derive(Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub(super) enum ChildLink {
    Subworkflow {
        parent: milkdrift_workspace::RunId,
        sequence: milkdrift_persistence::RunSequence,
    },
    Published {
        source: milkdrift_persistence::published::PublishedInvocationSource,
    },
}

fn acquire(
    write: &redb::WriteTransaction,
    snapshot: &ResolvedCapabilitySnapshot,
    execution: ManagedExecution,
) -> Result<(), PersistenceError> {
    let Some(extension) = snapshot
        .descriptor_extensions()
        .iter()
        .find(|(k, _)| k.as_str() == MANAGED_BINDING_EXTENSION)
        .map(|(_, v)| v)
    else {
        return Ok(());
    };
    let binding: ManagedBinding =
        serde_json::from_value(extension.value().clone()).map_err(|e| invalid(&e.to_string()))?;
    binding.validate().map_err(|e| invalid(&e.to_string()))?;
    let mut record = read_record(write, binding.installation.as_str())?
        .ok_or_else(|| conflict("managed generation absent"))?;
    let current = record
        .current
        .as_ref()
        .ok_or_else(|| conflict("managed generation not verified"))?;
    if !record.admission_open
        || record.pending.is_some()
        || record.removed
        || record.generation != binding.generation
        || current.recipe.digest != binding.recipe_digest
        || record.uses.len() >= MAX_MANAGED_USES
    {
        return Err(busy("managed generation is busy, unavailable or stale"));
    }
    if let Some(descriptor) = current.capabilities.iter().find(|d| {
        d.identity() == snapshot.capability()
            && d.descriptor_revision() == snapshot.descriptor_revision()
    }) {
        snapshot
            .validate_against(descriptor)
            .map_err(|e| invalid(&e.to_string()))?;
    } else {
        // A published method may retain an explicit dependency on this installation. Its ordinary
        // registered descriptor is a derived view of the separately authorized immutable method.
        crate::published::validate_managed_publication(write, snapshot)?;
    }
    if binding
        .resources
        .iter()
        .map(|r| &r.resource)
        .collect::<std::collections::BTreeSet<_>>()
        .len()
        != binding.resources.len()
    {
        return Err(invalid("duplicate managed dependency"));
    }
    for requirement in &binding.resources {
        if record.uses.iter().any(|usage| {
            matches!(usage.phase, ManagedUsePhase::Fencing { .. })
                && usage
                    .binding
                    .resources
                    .iter()
                    .any(|r| r.resource == requirement.resource)
        }) {
            return Err(busy("resource fencing excludes new accepted use"));
        }
        let r = current
            .resources
            .iter()
            .find(|r| r.name == requirement.resource)
            .ok_or_else(|| conflict("resource absent from generation"))?;
        if r.kind == ManagedResourceKind::Service
            && r.ownership == milkdrift_capability::managed::ResourceOwnership::Owned
            && !record.observation.as_ref().is_some_and(|o| o.running)
        {
            return Err(busy("owned service is stopped"));
        }
    }
    let id = execution.use_id();
    if write
        .open_table(MANAGED_USES)
        .map_err(error::redb)?
        .get(id.as_str())
        .map_err(error::redb)?
        .is_some()
    {
        return Err(conflict("duplicate managed use acceptance"));
    }
    let mut use_record = ManagedUse {
        id: id.clone(),
        execution,
        binding,
        claim: 1,
        editing: Vec::new(),
        entry_committed: false,
        execution_terminal: false,
        phase: ManagedUsePhase::Reserved {},
        parent: None,
    };
    let editing: Vec<_> = use_record
        .binding
        .resources
        .iter()
        .filter(|r| r.mutation)
        .map(|r| r.resource.clone())
        .collect();
    let blockers: Vec<_> = record
        .uses
        .iter()
        .filter(|u| u.editing.iter().any(|r| editing.contains(r)))
        .collect();
    if blockers.is_empty() {
        use_record.editing = editing.clone();
    } else if blockers.len() != 1
        || !matches!(blockers[0].phase, ManagedUsePhase::Quiescent { .. })
        || !linked(write, blockers[0], &use_record, None)?
    {
        return Err(busy("working area already has an exclusive editor"));
    }
    if let Some(parent) = blockers.first()
        && let ManagedUsePhase::Quiescent {
            evidence: milkdrift_persistence::managed::QuiescenceEvidence::NoExternalEntry { source },
        } = &parent.phase
    {
        let source = source.clone();
        let parent_id = parent.id.clone();
        let parent_index = record
            .uses
            .iter()
            .position(|usage| usage.id == parent_id)
            .ok_or_else(|| invalid("publication parent hold disappeared"))?;
        let plan = crate::published::association_in_transaction(write, &source)?
            .ok_or_else(|| invalid("publication resource hold lost acceptance"))?;
        if editing
            .iter()
            .any(|resource| !record.uses[parent_index].editing.contains(resource))
            || !publication_parent_matches(&record.uses[parent_index], &plan)
            || parent_cancelled(write, &record.uses[parent_index])?
        {
            return Err(busy("published resource parent is no longer active"));
        }
        let parent = &mut record.uses[parent_index];
        parent
            .editing
            .retain(|resource| !editing.contains(resource));
        parent.claim = parent
            .claim
            .checked_add(1)
            .ok_or_else(|| invalid("resource claim exhausted"))?;
        parent.phase = ManagedUsePhase::Suspended {
            child: id.clone(),
            evidence: milkdrift_persistence::managed::QuiescenceEvidence::NoExternalEntry {
                source,
            },
        };
        use_record.editing = editing;
        use_record.parent = Some(parent_id);
    }
    // An exact child can reserve lifetime protection while awaiting explicit transfer. It still
    // cannot cross final entry until the transaction commits its editing claim.
    if let ManagedExecution::Local { run, attempt, .. } = &use_record.execution {
        write
            .open_table(crate::schema::MANAGED_LOCAL_USES)
            .map_err(error::redb)?
            .insert(local_key(run, attempt).as_str(), id.as_str())
            .map_err(error::redb)?;
    }
    record.uses.push(use_record);
    bump(&mut record)?;
    write
        .open_table(MANAGED_USES)
        .map_err(error::redb)?
        .insert(id.as_str(), record.name.as_str())
        .map_err(error::redb)?;
    put_record(write, &record)
}

fn entry(write: &redb::WriteTransaction, id: &str) -> Result<(), PersistenceError> {
    let Some((mut record, i)) = locate(write, id)? else {
        return Ok(());
    };
    let u = &mut record.uses[i];
    if !record.admission_open
        || record.pending.is_some()
        || u.execution_terminal
        || !matches!(u.phase, ManagedUsePhase::Reserved {})
        || u.binding
            .resources
            .iter()
            .any(|r| r.mutation && !u.editing.contains(&r.resource))
    {
        return Err(busy(
            "resource editing transfer or maintenance blocks final entry",
        ));
    }
    u.entry_committed = true;
    bump(&mut record)?;
    put_record(write, &record)
}
fn terminal(write: &redb::WriteTransaction, id: &str) -> Result<(), PersistenceError> {
    let Some((mut record, i)) = locate(write, id)? else {
        return Ok(());
    };
    record.uses[i].execution_terminal = true;
    let stopped_or_never_entered =
        matches!(record.uses[i].phase, ManagedUsePhase::Quiescent { .. })
            || (!record.uses[i].entry_committed
                && matches!(record.uses[i].phase, ManagedUsePhase::Reserved {}));
    if let Some(parent_id) = &record.uses[i].parent
        && stopped_or_never_entered
        && let Some(parent_index) = record.uses.iter().position(|usage| &usage.id == parent_id)
        && let ManagedUsePhase::Suspended {
            child,
            evidence: milkdrift_persistence::managed::QuiescenceEvidence::NoExternalEntry { source },
        } = &record.uses[parent_index].phase
        && child == id
    {
        let evidence = milkdrift_persistence::managed::QuiescenceEvidence::NoExternalEntry {
            source: source.clone(),
        };
        let parent = &mut record.uses[parent_index];
        parent.claim = parent
            .claim
            .checked_add(1)
            .ok_or_else(|| invalid("resource claim exhausted"))?;
        parent.editing = parent
            .binding
            .resources
            .iter()
            .filter(|resource| resource.mutation)
            .map(|resource| resource.resource.clone())
            .collect();
        parent.phase = ManagedUsePhase::Quiescent { evidence };
        record.uses[i].parent = None;
    }
    let u = &record.uses[i];
    // Terminal observations never stand in for physical stop. Adapters may prove stop before
    // reporting; uncertainty/reporting loss keeps the use for authorized recovery.
    if u.parent.is_none()
        && !record.uses.iter().any(|c| c.parent.as_deref() == Some(id))
        && stopped_or_never_entered
    {
        remove_index(write, u)?;
        record.uses.remove(i);
        write
            .open_table(MANAGED_USES)
            .map_err(error::redb)?
            .remove(id)
            .map_err(error::redb)?;
    }
    bump(&mut record)?;
    put_record(write, &record)?;
    Ok(())
}

pub(crate) fn apply_run_resources(
    write: &redb::WriteTransaction,
    request: &AtomicRunCommitRequest,
) -> Result<(), PersistenceError> {
    let run = request.receipt().run();
    for event in request.events() {
        match event.kind() {
            RunEventKind::PublishedInvocationPlanned { plan, .. } => {
                link_published(write, plan)?;
                pending_publication(write, plan)?;
            }
            RunEventKind::SubworkflowCreated { child_run, .. } => {
                let bytes = json::encode(
                    &ChildLink::Subworkflow {
                        parent: run.clone(),
                        sequence: event.sequence(),
                    },
                    "managed child link",
                )?;
                if write
                    .open_table(MANAGED_LINKS)
                    .map_err(error::redb)?
                    .insert(child_run.as_str(), bytes.as_slice())
                    .map_err(error::redb)?
                    .is_some()
                {
                    return Err(conflict("child association already exists"));
                }
            }
            RunEventKind::CapabilityResolved {
                execution,
                attempt,
                snapshot,
                ..
            } if snapshot
                .descriptor_extensions()
                .keys()
                .any(|key| key.as_str() == MANAGED_BINDING_EXTENSION) =>
            {
                let scheduled = request
                    .events()
                    .iter()
                    .find_map(|e| match e.kind() {
                        RunEventKind::NodeScheduled {
                            attempt: a,
                            invocation,
                            ..
                        } if a == attempt => Some((invocation.clone(), e.sequence())),
                        _ => None,
                    })
                    .ok_or_else(|| {
                        invalid("resource resolution lacks same-transaction scheduling")
                    })?;
                acquire(
                    write,
                    snapshot,
                    ManagedExecution::Local {
                        run: run.clone(),
                        execution: execution.clone(),
                        attempt: attempt.clone(),
                        invocation: scheduled.0,
                        accepted_at: scheduled.1,
                    },
                )?;
            }
            RunEventKind::CapabilityAdapterEntryDecisionRecorded {
                attempt,
                authorization,
                controller_admission,
            } if authorization.is_allowed()
                && !matches!(
                    controller_admission,
                    milkdrift_persistence::ControllerAdmissionOutcome::Denied { .. }
                ) =>
            {
                if let Some(id) = local_use(write, run, attempt)? {
                    entry(write, &id)?;
                }
            }
            RunEventKind::NodeTerminal { attempt, .. } => {
                if let Some(id) = local_use(write, run, attempt)? {
                    terminal(write, &id)?;
                }
            }
            _ => {}
        }
    }
    Ok(())
}

// The active-use set is bounded globally by installations and per-installation admission. Avoid
// scanning execution history; the exact runtime index key below locates its one accepted hold.
fn local_key(
    run: &milkdrift_workspace::RunId,
    attempt: &milkdrift_persistence::AttemptId,
) -> String {
    format!("local:{run}:{attempt}")
}
pub(super) fn remove_index(
    write: &redb::WriteTransaction,
    usage: &ManagedUse,
) -> Result<(), PersistenceError> {
    if let ManagedExecution::Local { run, attempt, .. } = &usage.execution {
        write
            .open_table(crate::schema::MANAGED_LOCAL_USES)
            .map_err(error::redb)?
            .remove(local_key(run, attempt).as_str())
            .map_err(error::redb)?;
    }
    Ok(())
}
fn local_use(
    write: &redb::WriteTransaction,
    run: &milkdrift_workspace::RunId,
    attempt: &milkdrift_persistence::AttemptId,
) -> Result<Option<String>, PersistenceError> {
    write
        .open_table(crate::schema::MANAGED_LOCAL_USES)
        .map_err(error::redb)?
        .get(local_key(run, attempt).as_str())
        .map_err(error::redb)?
        .map(|v| Ok(v.value().to_owned()))
        .transpose()
}

pub(crate) fn accept_serving_resources(
    write: &redb::WriteTransaction,
    record: &PeerExecutionRecord,
) -> Result<(), PersistenceError> {
    acquire(
        write,
        &record.request.selection,
        ManagedExecution::Serving {
            execution: record.execution.clone(),
            invocation: record.managed_invocation()?,
        },
    )
}
pub(crate) fn enter_serving_resources(
    write: &redb::WriteTransaction,
    record: &PeerExecutionRecord,
) -> Result<(), PersistenceError> {
    entry(
        write,
        &milkdrift_persistence::managed::managed_use_id(&record.managed_invocation()?),
    )
}
pub(crate) fn terminal_serving_resources(
    write: &redb::WriteTransaction,
    record: &PeerExecutionRecord,
) -> Result<(), PersistenceError> {
    terminal(
        write,
        &milkdrift_persistence::managed::managed_use_id(&record.managed_invocation()?),
    )
}

pub(super) fn linked(
    write: &redb::WriteTransaction,
    parent: &ManagedUse,
    child: &ManagedUse,
    association: Option<&str>,
) -> Result<bool, PersistenceError> {
    let ManagedExecution::Local { run: child_run, .. } = &child.execution else {
        return Ok(false);
    };
    let mut current = child_run.clone();
    let table = write.open_table(MANAGED_LINKS).map_err(error::redb)?;
    for _ in 0..32 {
        let Some(bytes) = table.get(current.as_str()).map_err(error::redb)? else {
            return Ok(false);
        };
        match json::decode::<ChildLink>(bytes.value(), "managed child link")? {
            ChildLink::Subworkflow {
                parent: run,
                sequence,
            } => {
                let key = codec::run_sequence(run.as_str(), sequence)?;
                let event = {
                    let events = write.open_table(RUN_EVENTS).map_err(error::redb)?;
                    let bytes = events
                        .get(key.as_slice())
                        .map_err(error::redb)?
                        .ok_or_else(|| invalid("child link lost authoritative event"))?;
                    crate::journal::decode_stored_event(bytes.value())?
                };
                let RunEventKind::SubworkflowCreated {
                    child_run,
                    parent_execution,
                    ownership: milkdrift_persistence::SubworkflowOwnership::Attached,
                    ..
                } = event.kind()
                else {
                    return Ok(false);
                };
                if child_run != &current || run_basis(write, &run)? != run_basis(write, &current)? {
                    return Ok(false);
                }
                if let ManagedExecution::Local {
                    run: expected,
                    execution,
                    ..
                } = &parent.execution
                    && expected == &run
                    && execution == parent_execution
                {
                    return Ok(association
                        .is_none_or(|value| value == format!("{run}:{}", sequence.get())));
                }
                current = run;
            }
            ChildLink::Published { source } => {
                let Some(plan) = crate::published::association_in_transaction(write, &source)?
                else {
                    return Ok(false);
                };
                if plan.child_run != current
                    || !publication_parent_matches(parent, &plan)
                    || association.is_some_and(|value| value != plan.association_id())
                {
                    return Ok(false);
                }
                let basis = run_basis(write, &current)?;
                return Ok(basis.actor() == &plan.service.actor
                    && basis.grant() == &plan.service.grant
                    && basis.grant_revision() == plan.service.grant_revision
                    && basis.grant_digest() == &plan.service.grant_digest
                    && basis.revocation_generation() == plan.service.revocation_generation);
            }
        }
    }
    Err(invalid(
        "managed child ancestry exceeds the supported bound",
    ))
}

pub(super) fn publication_parent_matches(
    parent: &ManagedUse,
    plan: &milkdrift_persistence::published::PublishedInvocationPlan,
) -> bool {
    use milkdrift_persistence::published::PublishedInvocationSource;
    parent.execution.invocation() == &plan.invocation
        && match (&parent.execution, &plan.source) {
            (
                ManagedExecution::Local { run, attempt, .. },
                PublishedInvocationSource::Local {
                    run: accepted_run,
                    attempt: accepted_attempt,
                },
            ) => run == accepted_run && attempt == accepted_attempt,
            (
                ManagedExecution::Serving { execution, .. },
                PublishedInvocationSource::Serving {
                    execution: accepted_execution,
                    ..
                },
            ) => execution == accepted_execution,
            _ => false,
        }
}

pub(crate) fn link_published(
    write: &redb::WriteTransaction,
    plan: &milkdrift_persistence::published::PublishedInvocationPlan,
) -> Result<(), PersistenceError> {
    let bytes = json::encode(
        &ChildLink::Published {
            source: plan.source.clone(),
        },
        "managed child link",
    )?;
    if write
        .open_table(MANAGED_LINKS)
        .map_err(error::redb)?
        .insert(plan.child_run.as_str(), bytes.as_slice())
        .map_err(error::redb)?
        .is_some()
    {
        return Err(conflict("published child association already exists"));
    }
    Ok(())
}

fn run_basis(
    write: &redb::WriteTransaction,
    run: &milkdrift_workspace::RunId,
) -> Result<milkdrift_authority::ExecutionAuthorityBasis, PersistenceError> {
    let events = write.open_table(RUN_EVENTS).map_err(error::redb)?;
    // Runtime establishes the immutable basis at creation. Inspect only that bounded prefix.
    for sequence in 1..=8 {
        let key = codec::run_sequence(
            run.as_str(),
            milkdrift_persistence::RunSequence::new(sequence),
        )?;
        if let Some(bytes) = events.get(key.as_slice()).map_err(error::redb)?
            && let RunEventKind::ExecutionAuthorityEstablished { basis } =
                crate::journal::decode_stored_event(bytes.value())?.kind()
        {
            return Ok(basis.clone());
        }
    }
    Err(invalid(
        "accepted run has no immutable authority basis at creation",
    ))
}

pub(super) fn parent_authorized(
    write: &redb::WriteTransaction,
    parent: &ManagedUse,
    decision: &milkdrift_authority::AuthorityDecisionSnapshot,
) -> Result<bool, PersistenceError> {
    let ManagedExecution::Local { run, .. } = &parent.execution else {
        return Ok(false);
    };
    let basis = run_basis(write, run)?;
    let request = decision.request();
    Ok(decision.is_allowed()
        && basis.actor() == &request.actor
        && basis.grant() == &request.grant
        && basis.grant_revision() == request.grant_revision
        && basis.grant_digest() == &request.grant_digest
        && basis.revocation_generation() == request.revocation_generation)
}

pub(super) fn parent_cancelled(
    write: &redb::WriteTransaction,
    parent: &ManagedUse,
) -> Result<bool, PersistenceError> {
    let ManagedExecution::Local { run, .. } = &parent.execution else {
        let ManagedExecution::Serving { execution, .. } = &parent.execution else {
            return Ok(true);
        };
        let table = write
            .open_table(crate::schema::PEER_EXECUTIONS)
            .map_err(error::redb)?;
        let Some(bytes) = table.get(execution.as_str()).map_err(error::redb)? else {
            return Ok(true);
        };
        let record: PeerExecutionRecord = json::decode(bytes.value(), "peer execution")?;
        return Ok(record.cancellation.is_some() || !record.phase.is_active());
    };
    let summary = write
        .open_table(crate::schema::RUN_SUMMARIES)
        .map_err(error::redb)?;
    let bytes = summary
        .get(run.as_str())
        .map_err(error::redb)?
        .ok_or_else(|| invalid("parent run summary missing"))?;
    let summary: milkdrift_persistence::RunSummaryIndex =
        json::decode(bytes.value(), "run summary")?;
    Ok(matches!(
        summary.state,
        milkdrift_persistence::IndexedRunState::Cancelling
            | milkdrift_persistence::IndexedRunState::Terminal
    ))
}

/// The accepted action has no external entry closure. Its authoritative transaction is therefore
/// no-entry proof, independent of a child's later physical stop evidence.
pub(crate) fn pending_publication(
    write: &redb::WriteTransaction,
    plan: &milkdrift_persistence::published::PublishedInvocationPlan,
) -> Result<(), PersistenceError> {
    let id = milkdrift_persistence::managed::managed_use_id(&plan.invocation);
    let Some((mut record, index)) = locate(write, &id)? else {
        return Ok(());
    };
    let usage = &mut record.uses[index];
    if !matches!(usage.phase, ManagedUsePhase::Reserved {})
        || !publication_parent_matches(usage, plan)
    {
        return Err(conflict(
            "published no-entry evidence differs from its accepted hold",
        ));
    }
    usage.phase = ManagedUsePhase::Quiescent {
        evidence: milkdrift_persistence::managed::QuiescenceEvidence::NoExternalEntry {
            source: plan.source.clone(),
        },
    };
    bump(&mut record)?;
    put_record(write, &record)
}
