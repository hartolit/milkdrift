use super::{bump, busy, conflict, invalid, put_record, read_record, uses::locate};
use crate::{error, schema::MANAGED_USES};
use milkdrift_capability::ResolvedCapabilitySnapshot;
use milkdrift_capability::managed::{
    MANAGED_BINDING_EXTENSION, MAX_MANAGED_USES, ManagedBinding, ManagedResourceKind,
};
use milkdrift_persistence::{
    PeerExecutionRecord, PersistenceError,
    managed::{ManagedExecution, ManagedUse, ManagedUsePhase},
};
use redb::ReadableTable;

pub(super) fn acquire(
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
        return Err(conflict(
            "managed capability is absent from its exact installed generation",
        ));
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
    if record
        .uses
        .iter()
        .any(|u| u.editing.iter().any(|r| editing.contains(r)))
    {
        return Err(busy("working area already has an exclusive editor"));
    }
    use_record.editing = editing;
    record.uses.push(use_record);
    bump(&mut record)?;
    write
        .open_table(MANAGED_USES)
        .map_err(error::redb)?
        .insert(id.as_str(), record.name.as_str())
        .map_err(error::redb)?;
    put_record(write, &record)
}

pub(super) fn entry(write: &redb::WriteTransaction, id: &str) -> Result<(), PersistenceError> {
    let Some((mut record, i)) = locate(write, id)? else {
        return Ok(());
    };
    let u = record
        .uses
        .get_mut(i)
        .ok_or_else(|| invalid("use index has no hold"))?;
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
    let (before, after) = record
        .uses
        .split_at_mut_checked(i)
        .ok_or_else(|| invalid("use index has no hold"))?;
    let (u, after) = after
        .split_first_mut()
        .ok_or_else(|| invalid("use index has no hold"))?;
    u.execution_terminal = true;
    let stopped_or_never_entered = matches!(u.phase, ManagedUsePhase::Quiescent { .. })
        || (!u.entry_committed && matches!(u.phase, ManagedUsePhase::Reserved {}));
    // Terminal observations never stand in for physical stop. Adapters may prove stop before
    // reporting; uncertainty/reporting loss keeps the use for authorized recovery.
    if u.parent.is_none()
        && !before
            .iter()
            .chain(after.iter())
            .any(|c| c.parent.as_deref() == Some(id))
        && stopped_or_never_entered
    {
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

pub(crate) fn apply_task_resources(
    write: &redb::WriteTransaction,
    old: Option<&milkdrift_persistence::WorkflowSnapshot>,
    next: &milkdrift_persistence::WorkflowSnapshot,
) -> Result<(), PersistenceError> {
    let previous = old.map_or(&[][..], |snapshot| snapshot.managed_executions.as_slice());
    for prior in previous {
        if !next
            .managed_executions
            .iter()
            .any(|fact| fact.invocation == prior.invocation)
        {
            return Err(invalid("managed task acceptance cannot be removed"));
        }
    }
    if next.managed_executions.len() > 4096 {
        return Err(invalid("managed task acceptance count exceeds its bound"));
    }
    let mut identities = std::collections::BTreeSet::new();
    let mut tasks = std::collections::BTreeSet::new();
    for fact in &next.managed_executions {
        if !identities.insert(&fact.invocation)
            || !tasks.insert(&fact.task)
            || fact.accepted_generation == 0
            || fact.accepted_generation > next.generation
            || !fact
                .selection
                .descriptor_extensions()
                .keys()
                .any(|key| key.as_str() == MANAGED_BINDING_EXTENSION)
            || fact.entered != next.entered_invocations.contains(&fact.invocation)
        {
            return Err(invalid(
                "managed task acceptance contradicts its bounded frozen entry",
            ));
        }
        let prior = previous
            .iter()
            .find(|prior| prior.invocation == fact.invocation);
        if let Some(prior) = prior {
            if prior.task != fact.task
                || prior.accepted_generation != fact.accepted_generation
                || prior.selection != fact.selection
                || (prior.entered && !fact.entered)
                || (prior.settled && (!fact.settled || prior.entered != fact.entered))
            {
                return Err(invalid(
                    "managed task frozen selection and history are immutable",
                ));
            }
        } else {
            if fact.entered || fact.settled || fact.accepted_generation != next.generation {
                return Err(invalid(
                    "managed task requires fresh no-entry acceptance before entry or settlement",
                ));
            }
            acquire(
                write,
                &fact.selection,
                ManagedExecution::Task {
                    workflow: next.workflow.clone(),
                    task: fact.task.clone(),
                    invocation: fact.invocation.clone(),
                    accepted_generation: fact.accepted_generation,
                },
            )?;
        }
        let id = milkdrift_persistence::managed::managed_use_id(&fact.invocation);
        if fact.entered && prior.is_some_and(|prior| !prior.entered) {
            entry(write, &id)?;
        }
        if fact.settled && prior.is_some_and(|prior| !prior.settled) {
            terminal(write, &id)?;
        }
    }
    Ok(())
}

// The active-use set is bounded globally by installations and per-installation admission. Avoid
// scanning execution history; the exact runtime index key below locates its one accepted hold.
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
