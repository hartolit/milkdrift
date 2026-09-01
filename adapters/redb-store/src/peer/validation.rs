//! Validation of durable peer execution documents and authority envelopes.

use milkdrift_authority::{AuthorityDecisionSnapshot, AuthorityOperation};
use milkdrift_persistence::{
    PEER_EXECUTION_RECORD_SCHEMA_VERSION_V2, PEER_EXECUTION_RECORD_SCHEMA_VERSION_V3,
    PEER_EXECUTION_TOMBSTONE_SCHEMA_VERSION_V1, PeerAdmission, PeerArchivedDisposition,
    PeerCatalogState, PeerExecutionPhase, PeerExecutionRecord, PeerExecutionTombstone,
    PeerRelationshipState, PersistenceError,
};

use super::{MAX_UNCERTAINTY_REASON_BYTES, corruption, invalid, valid_prefixed_blake3};

pub(super) fn validate_relationship(value: &PeerRelationshipState) -> Result<(), PersistenceError> {
    if value.generation == 0 || value.expires_at_unix_ms == 0 || value.maximum_active == 0 {
        return Err(invalid("peer relationship persistence facts are invalid"));
    }
    Ok(())
}

pub(super) fn validate_catalog(value: &PeerCatalogState) -> Result<(), PersistenceError> {
    if value.relationship_generation == 0
        || value.generation == 0
        || value.expires_at_unix_ms == 0
        || !valid_prefixed_blake3(&value.digest)
    {
        return Err(invalid("peer catalog persistence facts are invalid"));
    }
    Ok(())
}

pub(super) fn validate_admission(value: &PeerAdmission<'_>) -> Result<(), PersistenceError> {
    value
        .request
        .validate()
        .map_err(|cause| invalid(&cause.to_string()))?;
    let decision_request = value.authority.request();
    let resources = &decision_request.resources;
    let provenance = &decision_request.provenance;
    let delegated = &value.request.delegation.provenance;
    if !value.authority.is_allowed()
        || decision_request.operation != AuthorityOperation::InvokePeerCapability
        || decision_request.actor != value.request.delegation.actor
        || resources.peer.as_ref() != Some(value.owner_peer)
        || resources.capability.as_ref() != Some(value.request.selection.capability())
        || resources.capability_operation.as_ref() != Some(value.request.selection.operation())
        || provenance
            .revision
            .as_ref()
            .map(ToString::to_string)
            .as_deref()
            != Some(delegated.revision.as_str())
        || provenance.node.as_ref().map(ToString::to_string).as_deref()
            != Some(delegated.node.as_str())
        || provenance.execution.as_deref() != Some(delegated.execution.as_str())
        || provenance.attempt.as_deref() != Some(delegated.attempt.as_str())
        || provenance.descriptor_revision != Some(value.request.selection.descriptor_revision())
        || value.relationship_generation == 0
        || value.accepted_at_unix_ms == 0
        || value.maximum_global_active == 0
        || value.maximum_dispatch_queue == 0
        || value.maximum_hot_terminal_records == 0
        || value.archive_batch_size == 0
        || value.archive_terminal_before_or_at_unix_ms == 0
        || value.maximum_hot_terminal_records < u64::from(value.maximum_global_active)
    {
        return Err(invalid("peer admission persistence facts are invalid"));
    }
    Ok(())
}

pub(super) fn validate_entry_authority(
    record: &PeerExecutionRecord,
    authority: &AuthorityDecisionSnapshot,
) -> Result<(), PersistenceError> {
    let accepted = record.authority.request();
    let entry = authority.request();
    if !authority.is_allowed()
        || entry.operation != AuthorityOperation::InvokePeerCapability
        || entry.actor != record.request.delegation.actor
        || entry.resources != accepted.resources
        || entry.budget != accepted.budget
        || entry.provenance != accepted.provenance
    {
        return Err(invalid(
            "peer adapter-entry authority does not match the accepted execution envelope",
        ));
    }
    Ok(())
}

pub(super) fn validate_record(record: &PeerExecutionRecord) -> Result<(), PersistenceError> {
    record
        .request
        .validate()
        .map_err(|cause| corruption(format!("stored peer request is invalid: {cause}")))?;
    if !matches!(
        record.schema_version,
        PEER_EXECUTION_RECORD_SCHEMA_VERSION_V2 | PEER_EXECUTION_RECORD_SCHEMA_VERSION_V3
    ) || record.relationship_generation == 0
        || record.acceptance_sequence == 0
        || record.accepted_at_unix_ms == 0
        || record.revision == 0
        || u64::from(record.accounting.observations) != record.last_observation_sequence
        || record.last_observation_sequence > u64::from(record.request.limits.observations)
        || record.accounting.artifact_bytes > record.request.limits.artifact_bytes
        || (record.schema_version == PEER_EXECUTION_RECORD_SCHEMA_VERSION_V3
            && record.accounting.artifact_bytes
                < record.request.input_artifact_bytes().map_err(|cause| {
                    corruption(format!("stored peer input is invalid: {cause}"))
                })?)
        || !valid_prefixed_blake3(&record.observation_digest)
    {
        return Err(corruption(
            "stored peer execution primary facts are invalid",
        ));
    }
    if let PeerExecutionPhase::Terminal { sequence, .. } = record.phase
        && sequence != record.last_observation_sequence
    {
        return Err(corruption(
            "stored peer terminal sequence disagrees with its head",
        ));
    }
    if matches!(
        record.phase,
        PeerExecutionPhase::CancellationRequested { .. }
    ) != record.cancellation.is_some()
        && record.phase.is_active()
    {
        return Err(corruption(
            "stored peer cancellation phase disagrees with its facts",
        ));
    }
    Ok(())
}

pub(super) fn validate_tombstone(
    tombstone: &PeerExecutionTombstone,
) -> Result<(), PersistenceError> {
    if tombstone.schema_version != PEER_EXECUTION_TOMBSTONE_SCHEMA_VERSION_V1
        || tombstone.relationship_generation == 0
        || tombstone.acceptance_sequence == 0
        || tombstone.accepted_at_unix_ms == 0
        || tombstone.catalog_generation == 0
        || tombstone.capability_generation == 0
        || tombstone.authority.grant_revision == 0
        || tombstone.authority.policy_version == 0
        || tombstone.archived_at_unix_ms == 0
        || tombstone.compacted_through_sequence != tombstone.last_observation_sequence
        || u64::from(tombstone.accounting.observations) != tombstone.last_observation_sequence
        || !valid_prefixed_blake3(&tombstone.request_digest)
        || !valid_prefixed_blake3(&tombstone.catalog_digest)
        || !valid_capability_digest(&tombstone.capability_digest)
        || !valid_prefixed_blake3(&tombstone.authority.decision_digest)
        || !valid_prefixed_blake3(&tombstone.observation_digest)
    {
        return Err(corruption(
            "stored peer execution tombstone facts are invalid",
        ));
    }
    match &tombstone.disposition {
        PeerArchivedDisposition::Terminal { observation } => {
            observation.validate().map_err(|cause| {
                corruption(format!("archived terminal summary is invalid: {cause}"))
            })?;
            if observation.execution != tombstone.execution
                || observation.sequence != tombstone.last_observation_sequence
                || observation.event.kind().terminal().is_none()
                || observation.observed_at_unix_ms > tombstone.archived_at_unix_ms
            {
                return Err(corruption(
                    "archived terminal summary disagrees with its tombstone",
                ));
            }
        }
        PeerArchivedDisposition::Uncertain {
            uncertain_at_unix_ms,
            reason,
        } => {
            if *uncertain_at_unix_ms == 0
                || *uncertain_at_unix_ms > tombstone.archived_at_unix_ms
                || reason.is_empty()
                || reason.len() > MAX_UNCERTAINTY_REASON_BYTES
            {
                return Err(corruption("archived uncertainty summary is invalid"));
            }
        }
    }
    if tombstone.cancellation.as_ref().is_some_and(|cancellation| {
        cancellation.request.execution != tombstone.execution
            || cancellation
                .acknowledgement
                .as_ref()
                .is_some_and(|acknowledgement| acknowledgement.execution != tombstone.execution)
    }) {
        return Err(corruption(
            "archived cancellation facts target another execution",
        ));
    }
    Ok(())
}

fn valid_capability_digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}
