use super::{bump, busy, conflict, invalid, put_record, read_record};
use crate::{RedbStore, error, schema::MANAGED_USES};
use milkdrift_persistence::{
    PersistenceError,
    managed::{ManagedUse, ManagedUsePhase, QuiescenceEvidence},
};
use redb::ReadableTable;

pub(super) fn begin_resolution(
    store: &RedbStore,
    request: &milkdrift_capability::managed::ManagedRequest,
    authorization: &milkdrift_authority::AuthorityDecisionSnapshot,
) -> Result<milkdrift_persistence::managed::ResourceReceipt, PersistenceError> {
    use milkdrift_capability::managed::{MANAGED_SCHEMA_VERSION, ManagedAction};
    use milkdrift_persistence::managed::ResourceReceipt;
    request.validate().map_err(|e| invalid(&e.to_string()))?;
    super::validate_authorization(request, authorization)?;
    if !authorization.is_allowed() {
        return Err(conflict("resolution requires authorization"));
    }
    let ManagedAction::Resolve {
        use_id,
        expected_claim,
    } = &request.action
    else {
        return Err(invalid("resolution action required"));
    };
    let write = store.database().begin_write().map_err(error::redb)?;
    let key = crate::codec::pair(
        authorization.request().actor.as_str(),
        request.command.as_str(),
    )?;
    if let Some(bytes) = write
        .open_table(super::MANAGED_RECEIPTS)
        .map_err(error::redb)?
        .get(key.as_slice())
        .map_err(error::redb)?
    {
        let prior: ResourceReceipt = crate::json::decode(bytes.value(), "managed receipt")?;
        if !super::same_receipt_scope(&prior, request, authorization) {
            return Err(conflict("resolution command conflict"));
        }
        return Ok(prior);
    }
    let (mut record, i) =
        locate(&write, use_id)?.ok_or_else(|| conflict("unknown resolution use"))?;
    if record.name != request.installation
        || record.version != request.expected_version
        || record.uses[i].claim != *expected_claim
        || record.pending.is_some()
    {
        return Err(conflict("stale resolution guard"));
    }
    if record
        .uses
        .iter()
        .any(|u| u.parent.as_deref() == Some(use_id))
    {
        return Err(busy(
            "settle exact child ownership before resolving this use",
        ));
    }
    if record.uses.iter().enumerate().any(|(other_index, other)| {
        other_index != i
            && !matches!(&other.phase, ManagedUsePhase::Suspended { child, .. } if child == use_id)
            && other.binding.resources.iter().any(|r| {
                record.uses[i]
                    .binding
                    .resources
                    .iter()
                    .any(|target| target.resource == r.resource)
            })
    }) {
        return Err(busy(
            "other accepted readers or writers prevent disruption of the shared resource",
        ));
    }
    let usage = &mut record.uses[i];
    let service = record.current.as_ref().and_then(|setup| {
        setup.resources.iter().find(|r| {
            r.kind == milkdrift_capability::managed::ManagedResourceKind::Service
                && usage
                    .binding
                    .resources
                    .iter()
                    .any(|required| required.resource == r.name)
        })
    });
    let physical_identity = match &usage.phase {
        ManagedUsePhase::Reserved {} => {
            service.map_or_else(|| format!("mdtask-{use_id}"), |r| r.identity.clone())
        }
        ManagedUsePhase::Entered { physical_identity }
        | ManagedUsePhase::Fencing { physical_identity } => physical_identity.clone(),
        ManagedUsePhase::Quiescent {
            evidence:
                QuiescenceEvidence::PhysicalStop {
                    physical_identity, ..
                },
        } => physical_identity.clone(),
        ManagedUsePhase::Quiescent {
            evidence: QuiescenceEvidence::NoExternalEntry { .. },
        } => {
            return Err(busy(
                "a pending workflow has no physical writer to fence; cancel its exact invocation",
            ));
        }
        ManagedUsePhase::Suspended { .. } => {
            return Err(busy("suspended use requires child settlement"));
        }
    };
    usage.claim = usage
        .claim
        .checked_add(1)
        .ok_or_else(|| invalid("claim exhausted"))?;
    usage.phase = ManagedUsePhase::Fencing { physical_identity };
    bump(&mut record)?;
    let receipt = ResourceReceipt {
        schema_version: MANAGED_SCHEMA_VERSION,
        request: request.clone(),
        authorization: authorization.clone(),
        response: record.view(),
    };
    let bytes = crate::json::encode(&receipt, "managed receipt")?;
    write
        .open_table(super::MANAGED_RECEIPTS)
        .map_err(error::redb)?
        .insert(key.as_slice(), bytes.as_slice())
        .map_err(error::redb)?;
    put_record(&write, &record)?;
    store
        .faults
        .check(crate::fault::FaultPoint::BeforeManagedCommit)?;
    write.commit().map_err(error::redb)?;
    store
        .faults
        .check(crate::fault::FaultPoint::AfterManagedCommit)?;
    Ok(receipt)
}

pub(super) fn locate(
    write: &redb::WriteTransaction,
    id: &str,
) -> Result<Option<(milkdrift_persistence::managed::InstallationRecord, usize)>, PersistenceError> {
    let name = write
        .open_table(MANAGED_USES)
        .map_err(error::redb)?
        .get(id)
        .map_err(error::redb)?
        .map(|v| v.value().to_owned());
    let Some(name) = name else {
        return Ok(None);
    };
    let record = read_record(write, &name)?
        .ok_or_else(|| invalid("use index refers to missing installation"))?;
    let index = record
        .uses
        .iter()
        .position(|u| u.id == id)
        .ok_or_else(|| invalid("use index has no hold"))?;
    Ok(Some((record, index)))
}

pub(super) fn read_use(
    store: &RedbStore,
    id: &str,
) -> Result<Option<ManagedUse>, PersistenceError> {
    let read = store.database().begin_read().map_err(error::redb)?;
    let name = read
        .open_table(MANAGED_USES)
        .map_err(error::redb)?
        .get(id)
        .map_err(error::redb)?
        .map(|v| v.value().to_owned());
    let Some(name) = name else {
        return Ok(None);
    };
    let table = read
        .open_table(super::MANAGED_INSTALLATIONS)
        .map_err(error::redb)?;
    let bytes = table
        .get(name.as_str())
        .map_err(error::redb)?
        .ok_or_else(|| invalid("use index refers to missing installation"))?;
    let record = super::decode_record(bytes.value())?;
    record
        .uses
        .into_iter()
        .find(|u| u.id == id)
        .map(Some)
        .ok_or_else(|| invalid("use index has no hold"))
}

pub(super) fn enter(
    store: &RedbStore,
    id: &str,
    claim: u64,
    physical: &str,
) -> Result<ManagedUse, PersistenceError> {
    if physical.is_empty() || physical.len() > 256 {
        return Err(invalid("invalid physical use identity"));
    }
    let write = store.database().begin_write().map_err(error::redb)?;
    let (mut record, i) =
        locate(&write, id)?.ok_or_else(|| conflict("use has no durable acceptance"))?;
    let u = &mut record.uses[i];
    if !record.admission_open
        || record.pending.is_some()
        || u.claim != claim
        || !u.entry_committed
        || u.execution_terminal
        || u.binding
            .resources
            .iter()
            .any(|r| r.mutation && !u.editing.contains(&r.resource))
    {
        return Err(busy("use does not own current entry and editing claims"));
    }
    if !matches!(u.phase, ManagedUsePhase::Reserved {}) {
        return Err(conflict("resource use cannot enter twice"));
    }
    u.phase = ManagedUsePhase::Entered {
        physical_identity: physical.to_owned(),
    };
    let result = u.clone();
    bump(&mut record)?;
    put_record(&write, &record)?;
    write.commit().map_err(error::redb)?;
    Ok(result)
}

pub(super) fn quiesce(
    store: &RedbStore,
    id: &str,
    claim: u64,
    evidence: &QuiescenceEvidence,
) -> Result<(), PersistenceError> {
    let QuiescenceEvidence::PhysicalStop {
        physical_identity: stopped_identity,
        observation_digest,
        ..
    } = evidence
    else {
        return Err(invalid(
            "only the publication acceptance transaction can prove no external entry",
        ));
    };
    if !milkdrift_contracts::is_canonical_blake3_digest(observation_digest) {
        return Err(invalid("invalid stop evidence"));
    }
    let write = store.database().begin_write().map_err(error::redb)?;
    let (mut record, i) = locate(&write, id)?.ok_or_else(|| conflict("unknown use"))?;
    let u = &mut record.uses[i];
    if u.claim != claim {
        return Err(conflict("stale stop evidence"));
    }
    match &u.phase {
        ManagedUsePhase::Entered { physical_identity }
        | ManagedUsePhase::Fencing { physical_identity }
            if physical_identity == stopped_identity => {}
        ManagedUsePhase::Quiescent { evidence: old } if old == evidence => return Ok(()),
        _ => {
            return Err(conflict(
                "stop evidence does not bind the entered physical identity",
            ));
        }
    }
    let fenced = matches!(u.phase, ManagedUsePhase::Fencing { .. });
    u.phase = ManagedUsePhase::Quiescent {
        evidence: evidence.clone(),
    };
    if fenced
        && record.current.as_ref().is_some_and(|setup| {
            setup.resources.iter().any(|r| {
                r.kind == milkdrift_capability::managed::ManagedResourceKind::Service
                    && r.ownership == milkdrift_capability::managed::ResourceOwnership::Owned
                    && &r.identity == stopped_identity
            })
        })
    {
        record.desired_running = false;
        record.observation = Some(milkdrift_persistence::managed::ManagedObservation {
            digest: observation_digest.clone(),
            summary: "owned service physically fenced; prior operation outcome remains unchanged"
                .to_owned(),
            running: false,
        });
    }
    bump(&mut record)?;
    put_record(&write, &record)?;
    write.commit().map_err(error::redb)
}

pub(super) fn release(store: &RedbStore, id: &str, claim: u64) -> Result<(), PersistenceError> {
    let write = store.database().begin_write().map_err(error::redb)?;
    let Some((mut record, i)) = locate(&write, id)? else {
        return Ok(());
    };
    let u = &record.uses[i];
    if u.claim != claim
        || !matches!(u.phase, ManagedUsePhase::Quiescent { .. })
        || u.parent.is_some()
        || (!u.execution_terminal
            && matches!(
                u.phase,
                ManagedUsePhase::Quiescent {
                    evidence: QuiescenceEvidence::NoExternalEntry { .. }
                }
            ))
        || record.uses.iter().any(|c| c.parent.as_deref() == Some(id))
    {
        return Err(busy(
            "physical quiescence and settled child ownership are required",
        ));
    }
    super::execution::remove_index(&write, &record.uses[i])?;
    record.uses.remove(i);
    write
        .open_table(MANAGED_USES)
        .map_err(error::redb)?
        .remove(id)
        .map_err(error::redb)?;
    bump(&mut record)?;
    put_record(&write, &record)?;
    write.commit().map_err(error::redb)
}

pub(super) fn transfer(
    store: &RedbStore,
    request: &milkdrift_capability::managed::ManagedRequest,
    authorization: &milkdrift_authority::AuthorityDecisionSnapshot,
) -> Result<milkdrift_persistence::managed::ResourceReceipt, PersistenceError> {
    use milkdrift_capability::managed::{MANAGED_SCHEMA_VERSION, ManagedAction};
    use milkdrift_persistence::managed::ResourceReceipt;
    request.validate().map_err(|e| invalid(&e.to_string()))?;
    super::validate_authorization(request, authorization)?;
    if !authorization.is_allowed() {
        return Err(conflict("transfer requires current authorization"));
    }
    let (h, returning) = match &request.action {
        ManagedAction::Handoff { transfer } => (transfer, None),
        ManagedAction::Return {
            transfer,
            resume_parent,
        } => (transfer, Some(*resume_parent)),
        _ => return Err(invalid("editing transfer action required")),
    };
    let write = store.database().begin_write().map_err(error::redb)?;
    let key = crate::codec::pair(
        authorization.request().actor.as_str(),
        request.command.as_str(),
    )?;
    if let Some(bytes) = write
        .open_table(super::MANAGED_RECEIPTS)
        .map_err(error::redb)?
        .get(key.as_slice())
        .map_err(error::redb)?
    {
        let prior: ResourceReceipt = crate::json::decode(bytes.value(), "managed receipt")?;
        if !super::same_receipt_scope(&prior, request, authorization) {
            return Err(conflict("transfer command conflict"));
        }
        return Ok(prior);
    }
    let mut record = read_record(&write, h.installation.as_str())?
        .ok_or_else(|| conflict("missing installation"))?;
    if record.version != request.expected_version
        || record.pending.is_some()
        || !record.admission_open
        || record.generation != h.generation
        || h.parent == h.child
    {
        return Err(conflict("handoff generation or admission conflict"));
    }
    let pi = record
        .uses
        .iter()
        .position(|u| u.id == h.parent)
        .ok_or_else(|| conflict("missing parent hold"))?;
    let ci = record
        .uses
        .iter()
        .position(|u| u.id == h.child)
        .ok_or_else(|| conflict("missing child hold"))?;
    let parent = record.uses[pi].clone();
    let child = record.uses[ci].clone();
    if parent.claim != h.parent_claim
        || child.claim != h.child_claim
        || parent.binding.generation != child.binding.generation
        || !super::execution::linked(&write, &parent, &child, Some(&h.association))?
    {
        return Err(conflict(
            "handoff requires exact claims and accepted inherited lineage",
        ));
    }
    if let Some(resume) = returning {
        if !matches!(&parent.phase, ManagedUsePhase::Suspended { child, .. } if child == &h.child)
            || child.parent.as_deref() != Some(&h.parent)
            || !matches!(child.phase, ManagedUsePhase::Quiescent { .. })
        {
            return Err(busy(
                "child physical quiescence is required before returning editing",
            ));
        }
        if resume
            && (parent.execution_terminal
                || super::execution::parent_cancelled(&write, &parent)?
                || !super::execution::parent_authorized(&write, &parent, authorization)?)
        {
            return Err(busy("cancelled parent cannot resume editing"));
        }
        let evidence = match parent.phase {
            ManagedUsePhase::Suspended { evidence, .. } => evidence,
            _ => return Err(conflict("parent not suspended")),
        };
        record.uses[pi].claim = parent
            .claim
            .checked_add(1)
            .ok_or_else(|| invalid("claim exhausted"))?;
        record.uses[pi].editing = if resume {
            parent
                .binding
                .resources
                .iter()
                .filter(|r| r.mutation)
                .map(|r| r.resource.clone())
                .collect()
        } else {
            Vec::new()
        };
        // Current authority grants the still-active parent a new physical entry epoch. The
        // caller must consume this claim; callbacks from its earlier epoch remain stale.
        record.uses[pi].phase = if resume {
            ManagedUsePhase::Reserved {}
        } else {
            ManagedUsePhase::Quiescent { evidence }
        };
        super::execution::remove_index(&write, &record.uses[ci])?;
        record.uses.remove(ci);
        write
            .open_table(MANAGED_USES)
            .map_err(error::redb)?
            .remove(h.child.as_str())
            .map_err(error::redb)?;
    } else {
        if parent.execution_terminal
            || super::execution::parent_cancelled(&write, &parent)?
            || !super::execution::parent_authorized(&write, &parent, authorization)?
        {
            return Err(busy(
                "handoff requires a live parent and its current authority",
            ));
        }
        let ManagedUsePhase::Quiescent { evidence } = &parent.phase else {
            return Err(busy(
                "parent writer must physically stop before child entry",
            ));
        };
        if !matches!(child.phase, ManagedUsePhase::Reserved {})
            || child.entry_committed
            || child.parent.is_some()
        {
            return Err(conflict("child already entered or linked"));
        }
        let requested: Vec<_> = child
            .binding
            .resources
            .iter()
            .filter(|r| r.mutation)
            .map(|r| r.resource.clone())
            .collect();
        if requested.is_empty() || requested.iter().any(|r| !parent.editing.contains(r)) {
            return Err(conflict("child mutation exceeds the exact parent claim"));
        }
        record.uses[pi].editing.retain(|r| !requested.contains(r));
        record.uses[pi].claim = parent
            .claim
            .checked_add(1)
            .ok_or_else(|| invalid("claim exhausted"))?;
        record.uses[pi].phase = ManagedUsePhase::Suspended {
            child: h.child.clone(),
            evidence: evidence.clone(),
        };
        record.uses[ci].editing = requested;
        record.uses[ci].claim = child
            .claim
            .checked_add(1)
            .ok_or_else(|| invalid("claim exhausted"))?;
        record.uses[ci].parent = Some(h.parent.clone());
    }
    bump(&mut record)?;
    let receipt = ResourceReceipt {
        schema_version: MANAGED_SCHEMA_VERSION,
        request: request.clone(),
        authorization: authorization.clone(),
        response: record.view(),
    };
    let bytes = crate::json::encode(&receipt, "managed receipt")?;
    write
        .open_table(super::MANAGED_RECEIPTS)
        .map_err(error::redb)?
        .insert(key.as_slice(), bytes.as_slice())
        .map_err(error::redb)?;
    put_record(&write, &record)?;
    store
        .faults
        .check(crate::fault::FaultPoint::BeforeManagedCommit)?;
    write.commit().map_err(error::redb)?;
    store
        .faults
        .check(crate::fault::FaultPoint::AfterManagedCommit)?;
    Ok(receipt)
}
