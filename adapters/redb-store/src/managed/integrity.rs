use super::{decode_record, invalid};
use crate::{
    RedbStore, error, json,
    schema::{MANAGED_INSTALLATIONS, MANAGED_TRANSITIONS, MANAGED_USES},
};
use milkdrift_persistence::{
    PersistenceError,
    managed::{ManagedExecution, ManagedTransition, ResourceReceipt},
};
use redb::ReadableTable;

pub(super) fn verify(store: &RedbStore) -> Result<(), PersistenceError> {
    let read = store.database().begin_read().map_err(error::redb)?;
    let installations = read
        .open_table(MANAGED_INSTALLATIONS)
        .map_err(error::redb)?;
    let uses = read.open_table(MANAGED_USES).map_err(error::redb)?;
    // Startup visits the bounded live inventory only. Lifetime receipts and transition history
    // are checked by the ordinary resumable integrity scanner, not an unbounded startup pass.
    for row in installations.iter().map_err(error::redb)? {
        let (key, value) = row.map_err(error::redb)?;
        record(&read, key.value(), value.value())?;
    }
    for row in uses.iter().map_err(error::redb)? {
        let (id, name) = row.map_err(error::redb)?;
        use_index(&read, id.value(), name.value())?;
    }
    Ok(())
}

pub(crate) fn record(
    read: &redb::ReadTransaction,
    key: &str,
    bytes: &[u8],
) -> Result<(), PersistenceError> {
    let record = decode_record(bytes)?;
    if key != record.name.as_str() {
        return Err(invalid("managed inventory key mismatch"));
    }
    let uses = read.open_table(MANAGED_USES).map_err(error::redb)?;
    let mut editors = std::collections::BTreeSet::new();
    for u in &record.uses {
        if u.id != u.execution.use_id()
            || u.binding.installation != record.name
            || u.claim == 0
            || u.binding.generation != record.generation
            || u.editing.iter().any(|r| !editors.insert(r))
        {
            return Err(invalid(
                "managed use identity, generation or exclusive editing conflict",
            ));
        }
        if uses
            .get(u.id.as_str())
            .map_err(error::redb)?
            .is_none_or(|v| v.value() != record.name.as_str())
        {
            return Err(invalid("managed hold missing its index"));
        }
        let evidence = match &u.phase {
            milkdrift_persistence::managed::ManagedUsePhase::Quiescent { evidence }
            | milkdrift_persistence::managed::ManagedUsePhase::Suspended { evidence, .. } => {
                Some(evidence)
            }
            _ => None,
        };
        if let Some(milkdrift_persistence::managed::QuiescenceEvidence::NoExternalEntry {
            source,
        }) = evidence
        {
            let plan = crate::published::association_read(read, source)?
                .ok_or_else(|| invalid("no-entry proof lost its publication acceptance"))?;
            if !super::execution::publication_parent_matches(u, &plan) {
                return Err(invalid(
                    "no-entry proof belongs to a different resource use",
                ));
            }
        }
        match &u.execution {
            ManagedExecution::Local {
                run,
                execution,
                attempt,
                invocation,
                accepted_at,
            } => {
                let key = crate::codec::run_sequence(run.as_str(), *accepted_at)?;
                let table = read
                    .open_table(crate::schema::RUN_EVENTS)
                    .map_err(error::redb)?;
                let bytes = table
                    .get(key.as_slice())
                    .map_err(error::redb)?
                    .ok_or_else(|| invalid("managed use lost runtime acceptance"))?;
                let event = crate::journal::decode_stored_event(bytes.value())?;
                if !matches!(event.kind(), milkdrift_persistence::RunEventKind::NodeScheduled { execution: e, attempt: a, invocation: i, .. } if e == execution && a == attempt && i == invocation)
                {
                    return Err(invalid("managed use differs from runtime acceptance"));
                }
                let index = read
                    .open_table(crate::schema::MANAGED_LOCAL_USES)
                    .map_err(error::redb)?;
                if index
                    .get(format!("local:{run}:{attempt}").as_str())
                    .map_err(error::redb)?
                    .is_none_or(|id| id.value() != u.id)
                {
                    return Err(invalid("managed local lookup differs from acceptance"));
                }
            }
            ManagedExecution::Serving {
                execution,
                invocation,
            } => {
                let snapshot =
                    crate::peer::snapshot_in_read_transaction_text(read, execution.as_str())?;
                match snapshot {
                    milkdrift_persistence::PeerExecutionSnapshot::Hot(accepted) => {
                        if accepted.managed_invocation()? != *invocation {
                            return Err(invalid("managed use differs from serving acceptance"));
                        }
                        let binding = accepted
                            .request
                            .selection
                            .descriptor_extensions()
                            .iter()
                            .find(|(k, _)| {
                                k.as_str()
                                    == milkdrift_capability::managed::MANAGED_BINDING_EXTENSION
                            })
                            .ok_or_else(|| invalid("serving acceptance lacks managed binding"))?;
                        let binding: milkdrift_capability::managed::ManagedBinding =
                            serde_json::from_value(binding.1.value().clone())
                                .map_err(|e| invalid(&e.to_string()))?;
                        if binding != u.binding {
                            return Err(invalid(
                                "managed dependency differs from serving acceptance",
                            ));
                        }
                    }
                    milkdrift_persistence::PeerExecutionSnapshot::Archived(accepted) => {
                        let setup = record
                            .current
                            .as_ref()
                            .ok_or_else(|| invalid("archived use has no retained setup"))?;
                        let descriptor = setup
                            .capabilities
                            .iter()
                            .find(|d| {
                                d.identity() == &accepted.capability
                                    && d.descriptor_revision() == accepted.capability_generation
                            })
                            .ok_or_else(|| invalid("archived use lost exact generation"))?;
                        let snapshot =
                            milkdrift_capability::ResolvedCapabilitySnapshot::from_descriptor(
                                descriptor,
                                &accepted.operation,
                            )
                            .map_err(|e| invalid(&e.to_string()))?;
                        let binding = snapshot
                            .descriptor_extensions()
                            .iter()
                            .find(|(key, _)| {
                                key.as_str()
                                    == milkdrift_capability::managed::MANAGED_BINDING_EXTENSION
                            })
                            .ok_or_else(|| invalid("archived generation lost managed binding"))?;
                        let binding: milkdrift_capability::managed::ManagedBinding =
                            serde_json::from_value(binding.1.value().clone())
                                .map_err(|e| invalid(&e.to_string()))?;
                        if accepted.managed_invocation()? != *invocation
                            || accepted.capability_digest != snapshot.digest()
                            || binding != u.binding
                        {
                            return Err(invalid(
                                "archived resource use differs from exact accepted identity",
                            ));
                        }
                    }
                }
            }
        }
        if let Some(parent) = &u.parent
            && !record.uses.iter().any(|p| p.id == *parent && matches!(&p.phase, milkdrift_persistence::managed::ManagedUsePhase::Suspended { child, .. } if child == &u.id)) { return Err(invalid("editing child lost exact suspended parent")); }
    }
    if let Some(pending) = &record.pending {
        let transitions = read.open_table(MANAGED_TRANSITIONS).map_err(error::redb)?;
        let bytes = transitions
            .get(format!("{}:{}", record.name, pending.change.identity).as_str())
            .map_err(error::redb)?
            .ok_or_else(|| invalid("pending transition has no durable evidence"))?;
        let saved: ManagedTransition = json::decode(bytes.value(), "managed transition")?;
        if &saved != pending {
            return Err(invalid("pending transition evidence differs"));
        }
    }
    Ok(())
}

pub(crate) fn use_index(
    read: &redb::ReadTransaction,
    id: &str,
    name: &str,
) -> Result<(), PersistenceError> {
    let installations = read
        .open_table(MANAGED_INSTALLATIONS)
        .map_err(error::redb)?;
    let bytes = installations
        .get(name)
        .map_err(error::redb)?
        .ok_or_else(|| invalid("orphan managed use index"))?;
    let record = decode_record(bytes.value())?;
    if !record.uses.iter().any(|u| u.id == id) {
        return Err(invalid("orphan managed use"));
    }
    Ok(())
}

pub(crate) fn receipt(key: &[u8], bytes: &[u8]) -> Result<(), PersistenceError> {
    let receipt: ResourceReceipt = json::decode(bytes, "managed receipt")?;
    receipt
        .request
        .validate()
        .map_err(|e| invalid(&e.to_string()))?;
    receipt
        .response
        .validate_for(&receipt.request)
        .map_err(|e| invalid(&e.to_string()))?;
    super::validate_authorization(&receipt.request, &receipt.authorization)?;
    if receipt.schema_version != milkdrift_capability::managed::MANAGED_SCHEMA_VERSION
        || key
            != crate::codec::pair(
                receipt.authorization.request().actor.as_str(),
                receipt.request.command.as_str(),
            )?
            .as_slice()
        || !receipt.authorization.is_allowed()
        || receipt.response.installation != receipt.request.installation
    {
        return Err(invalid("managed receipt identity mismatch"));
    }
    Ok(())
}

pub(crate) fn transition(key: &str, bytes: &[u8]) -> Result<(), PersistenceError> {
    let transition: ManagedTransition = json::decode(bytes, "managed transition")?;
    transition.change.candidate.validate()?;
    if !key.ends_with(&format!(":{}", transition.change.identity))
        || transition.next_step as usize != transition.evidence.len()
        || transition.change.steps.is_empty()
        || transition.change.steps.len() > 16
        || transition.evidence.len() > transition.change.steps.len()
    {
        return Err(invalid("managed transition evidence mismatch"));
    }
    Ok(())
}
