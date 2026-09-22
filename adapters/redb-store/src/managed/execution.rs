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
#[serde(deny_unknown_fields)]
pub(super) struct ChildLink {
    pub(super) parent: milkdrift_workspace::RunId,
    pub(super) sequence: milkdrift_persistence::RunSequence,
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
    let descriptor = current
        .capabilities
        .iter()
        .find(|d| {
            d.identity() == snapshot.capability()
                && d.descriptor_revision() == snapshot.descriptor_revision()
        })
        .ok_or_else(|| conflict("unpublished managed capability"))?;
    snapshot
        .validate_against(descriptor)
        .map_err(|e| invalid(&e.to_string()))?;
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
        use_record.editing = editing;
    } else if blockers.len() != 1
        || !matches!(blockers[0].phase, ManagedUsePhase::Quiescent { .. })
        || !linked(write, blockers[0], &use_record, None)?
    {
        return Err(busy("working area already has an exclusive editor"));
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
    let u = &record.uses[i];
    // Terminal observations never stand in for physical stop. Adapters may prove stop before
    // reporting; uncertainty/reporting loss keeps the use for authorized recovery.
    if u.parent.is_none()
        && !record.uses.iter().any(|c| c.parent.as_deref() == Some(id))
        && ((!u.entry_committed && matches!(u.phase, ManagedUsePhase::Reserved {}))
            || matches!(u.phase, ManagedUsePhase::Quiescent { .. }))
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
            RunEventKind::SubworkflowCreated { child_run, .. } => {
                let bytes = json::encode(
                    &ChildLink {
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
    let (
        ManagedExecution::Local {
            run: parent_run,
            execution: parent_execution,
            ..
        },
        ManagedExecution::Local { run: child_run, .. },
    ) = (&parent.execution, &child.execution)
    else {
        return Ok(false);
    };
    let table = write.open_table(MANAGED_LINKS).map_err(error::redb)?;
    let Some(bytes) = table.get(child_run.as_str()).map_err(error::redb)? else {
        return Ok(false);
    };
    let link: ChildLink = json::decode(bytes.value(), "managed child link")?;
    if &link.parent != parent_run
        || association.is_some_and(|a| a != format!("{}:{}", link.parent, link.sequence.get()))
    {
        return Ok(false);
    }
    let key = codec::run_sequence(link.parent.as_str(), link.sequence)?;
    let events = write.open_table(RUN_EVENTS).map_err(error::redb)?;
    let bytes = events
        .get(key.as_slice())
        .map_err(error::redb)?
        .ok_or_else(|| invalid("child link lost authoritative event"))?;
    let event = crate::journal::decode_stored_event(bytes.value())?;
    if !matches!(event.kind(), RunEventKind::SubworkflowCreated { child_run: run, parent_execution: execution, ownership: milkdrift_persistence::SubworkflowOwnership::Attached, .. } if run == child_run && execution == parent_execution)
    {
        return Ok(false);
    }
    drop(bytes);
    drop(events);
    // Runtime already enforces inherited basis when creating the child; compare those immutable
    // bindings again here to ensure a separately authorized run cannot borrow its resource claim.
    Ok(run_basis(write, parent_run)? == run_basis(write, child_run)?)
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
        return Ok(true);
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
