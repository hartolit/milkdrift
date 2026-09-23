use super::{conflict, invalid, read_record, same_receipt_scope, validate_authorization};
use crate::{
    RedbStore, codec, error, json,
    schema::{MANAGED_EVALUATIONS, MANAGED_RECEIPTS},
};
use milkdrift_authority::AuthorityDecisionSnapshot;
use milkdrift_capability::{
    BoundedJson,
    managed::{MANAGED_SCHEMA_VERSION, ManagedAction, ManagedRequest},
};
use milkdrift_persistence::{PersistenceError, managed::ResourceReceipt};
use milkdrift_workspace::CandidateEvaluation;
use redb::{ReadableTable, ReadableTableMetadata};

pub(super) fn begin(
    store: &RedbStore,
    request: &ManagedRequest,
    authorization: &AuthorityDecisionSnapshot,
    evidence: &CandidateEvaluation,
) -> Result<ResourceReceipt, PersistenceError> {
    request.validate().map_err(|e| invalid(&e.to_string()))?;
    validate_authorization(request, authorization)?;
    evidence.validate().map_err(|e| invalid(&e.to_string()))?;
    let ManagedAction::Evaluate { candidate } = &request.action else {
        return Err(invalid("evaluation request required"));
    };
    if evidence.complete
        || candidate.digest() != evidence.subject.artifact.digest().to_string()
        || candidate.identity() != evidence.subject.artifact.artifact().as_str()
        || candidate.size_bytes() != Some(evidence.subject.artifact.size_bytes())
        || evidence.subject.target != request.installation
    {
        return Err(invalid("evaluation differs from accepted candidate"));
    }
    let write = store.database().begin_write().map_err(error::redb)?;
    let key = codec::pair(
        authorization.request().actor.as_str(),
        request.command.as_str(),
    )?;
    let mut receipts = write.open_table(MANAGED_RECEIPTS).map_err(error::redb)?;
    if let Some(bytes) = receipts.get(key.as_slice()).map_err(error::redb)? {
        let prior: ResourceReceipt = json::decode(bytes.value(), "managed receipt")?;
        if !same_receipt_scope(&prior, request, authorization) {
            return Err(conflict("evaluation command key"));
        }
        return Ok(prior);
    }
    let record = read_record(&write, request.installation.as_str())?
        .ok_or_else(|| conflict("evaluation target absent"))?;
    let setup = record
        .current
        .as_ref()
        .ok_or_else(|| conflict("evaluation target unprepared"))?;
    let protection = setup
        .protection
        .as_ref()
        .ok_or_else(|| conflict("target has no protected policy"))?;
    if record.version != request.expected_version
        || record.generation != evidence.subject.generation
        || record.removed
        || record.pending.is_some()
        || setup.recipe.digest != evidence.subject.configuration
        || protection.agreement != evidence.subject.agreement
        || protection
            .policy
            .digest()
            .map_err(|e| invalid(&e.to_string()))?
            != evidence.subject.policy
    {
        return Err(conflict("stale evaluation target or policy"));
    }
    let mut table = write.open_table(MANAGED_EVALUATIONS).map_err(error::redb)?;
    if table.len().map_err(error::redb)? >= 4096 {
        return Err(super::busy("retained evaluation limit reached"));
    }
    if table
        .get(evidence.identity.as_str())
        .map_err(error::redb)?
        .is_some()
    {
        return Err(conflict("evaluation identity"));
    }
    let bytes = json::encode(evidence, "managed evaluation")?;
    table
        .insert(evidence.identity.as_str(), bytes.as_slice())
        .map_err(error::redb)?;
    let mut response = record.view();
    response.state = "evaluating".to_owned();
    response.evaluation = Some(
        BoundedJson::new(serde_json::to_value(evidence).map_err(|e| invalid(&e.to_string()))?)
            .map_err(|e| invalid(&e.to_string()))?,
    );
    let receipt = ResourceReceipt {
        schema_version: MANAGED_SCHEMA_VERSION,
        request: request.clone(),
        authorization: authorization.clone(),
        response,
    };
    let bytes = json::encode(&receipt, "managed receipt")?;
    receipts
        .insert(key.as_slice(), bytes.as_slice())
        .map_err(error::redb)?;
    drop(table);
    drop(receipts);
    store
        .faults
        .check(crate::fault::FaultPoint::BeforeManagedCommit)?;
    write.commit().map_err(error::redb)?;
    store
        .faults
        .check(crate::fault::FaultPoint::AfterManagedCommit)?;
    Ok(receipt)
}
pub(super) fn finish(
    store: &RedbStore,
    evidence: &CandidateEvaluation,
) -> Result<(), PersistenceError> {
    evidence.validate().map_err(|e| invalid(&e.to_string()))?;
    let write = store.database().begin_write().map_err(error::redb)?;
    let mut table = write.open_table(MANAGED_EVALUATIONS).map_err(error::redb)?;
    let old: CandidateEvaluation = table
        .get(evidence.identity.as_str())
        .map_err(error::redb)?
        .map(|b| json::decode(b.value(), "managed evaluation"))
        .transpose()?
        .ok_or_else(|| conflict("missing evaluation intent"))?;
    if old.complete {
        if old == *evidence {
            return Ok(());
        }
        return Err(conflict("evaluation result is immutable"));
    }
    if !evidence.complete
        || old.subject != evidence.subject
        || old.started_at != evidence.started_at
        || old.expires_at != evidence.expires_at
        || old
            .checks
            .iter()
            .map(|c| &c.name)
            .ne(evidence.checks.iter().map(|c| &c.name))
    {
        return Err(conflict("evaluation result differs from intent"));
    }
    let bytes = json::encode(evidence, "managed evaluation")?;
    table
        .insert(evidence.identity.as_str(), bytes.as_slice())
        .map_err(error::redb)?;
    drop(table);
    store
        .faults
        .check(crate::fault::FaultPoint::BeforeManagedCommit)?;
    write.commit().map_err(error::redb)?;
    store
        .faults
        .check(crate::fault::FaultPoint::AfterManagedCommit)
}
pub(super) fn read(
    store: &RedbStore,
    identity: &str,
) -> Result<Option<CandidateEvaluation>, PersistenceError> {
    let read = store.database().begin_read().map_err(error::redb)?;
    read.open_table(MANAGED_EVALUATIONS)
        .map_err(error::redb)?
        .get(identity)
        .map_err(error::redb)?
        .map(|b| decode(identity, b.value()))
        .transpose()
}
pub(crate) fn decode(key: &str, bytes: &[u8]) -> Result<CandidateEvaluation, PersistenceError> {
    let evidence: CandidateEvaluation = json::decode(bytes, "managed evaluation")?;
    evidence.validate().map_err(|e| invalid(&e.to_string()))?;
    if evidence.identity != key {
        return Err(invalid("evaluation key differs"));
    }
    Ok(evidence)
}
