//! Validation of durable peer execution documents and authority envelopes.

use milkdrift_authority::{AuthorityDecisionSnapshot, AuthorityOperation};
use milkdrift_contracts::is_canonical_blake3_digest;
use milkdrift_persistence::{
    PeerAdmission, PeerArchivedDisposition, PeerExecutionPhase, PeerExecutionRecord,
    PeerExecutionTombstone, PersistenceError, SERVING_EXECUTION_RECORD_SCHEMA_VERSION,
    SERVING_EXECUTION_TOMBSTONE_SCHEMA_VERSION, ServingCallerState, ServingCatalogState,
};

use super::{MAX_UNCERTAINTY_REASON_BYTES, corruption, invalid};

pub(super) fn validate_relationship(value: &ServingCallerState) -> Result<(), PersistenceError> {
    if value.generation == 0 || value.expires_at_unix_ms == 0 || value.maximum_active == 0 {
        return Err(invalid("peer relationship persistence facts are invalid"));
    }
    Ok(())
}

pub(super) fn validate_catalog(value: &ServingCatalogState) -> Result<(), PersistenceError> {
    if value.relationship_generation == 0
        || value.generation == 0
        || value.expires_at_unix_ms == 0
        || !is_canonical_blake3_digest(&value.digest)
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
    let origin = value.request.authorization.origin();
    let delegated = origin.workflow();
    let operation = match &value.request.authorization {
        milkdrift_peer_protocol::ServingAuthorization::Peer(_) => {
            AuthorityOperation::InvokePeerCapability
        }
        milkdrift_peer_protocol::ServingAuthorization::Client(client) => {
            if decision_request.grant != client.grant
                || decision_request.grant_revision != client.grant_revision
                || decision_request.grant_digest != client.grant_digest
                || decision_request.revocation_generation != client.revocation_generation
            {
                return Err(invalid(
                    "client admission must retain its exact authenticated grant",
                ));
            }
            AuthorityOperation::InvokeCapability
        }
    };
    if !value.authority.is_allowed()
        || decision_request.operation != operation
        || &decision_request.actor != value.request.authorization.actor()
        || &value.request.authorization.caller() != value.caller
        || resources.peer.as_ref() != value.caller.peer_identity()
        || resources.capability.as_ref() != Some(value.request.selection.capability())
        || resources.capability_operation.as_ref() != Some(value.request.selection.operation())
        || provenance
            .revision
            .as_ref()
            .map(ToString::to_string)
            .as_deref()
            != delegated.map(|value| value.revision.as_str())
        || provenance.node.as_ref().map(ToString::to_string).as_deref()
            != delegated.map(|value| value.node.as_str())
        || provenance.execution.as_deref() != delegated.map(|value| value.execution.as_str())
        || provenance.attempt.as_deref() != delegated.map(|value| value.attempt.as_str())
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
        || entry.operation != accepted.operation
        || &entry.actor != record.request.authorization.actor()
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
    if let Some(plan) = &record.published_invocation {
        plan.validate()?;
        if plan.source
            != (milkdrift_persistence::published::PublishedInvocationSource::Serving {
                caller: record.caller.clone(),
                execution: record.execution.clone(),
            })
            || plan.invocation != record.managed_invocation()?
            || plan.capability != *record.request.selection.capability()
            || plan.generation != record.request.selection.descriptor_revision()
            || plan.request.inputs() != record.request.request.inputs()
            || record.phase.entry_evidence().is_none()
                && !matches!(record.phase, PeerExecutionPhase::Terminal { .. })
        {
            return Err(corruption(
                "published serving link differs from accepted operation",
            ));
        }
    } else if matches!(record.phase, PeerExecutionPhase::AwaitingWorkflow { .. }) {
        return Err(corruption("pending published operation has no association"));
    }
    if record.schema_version != SERVING_EXECUTION_RECORD_SCHEMA_VERSION
        || record.relationship_generation == 0
        || record.acceptance_sequence == 0
        || record.accepted_at_unix_ms == 0
        || record.revision == 0
        || u64::from(record.accounting.observations) != record.last_observation_sequence
        || record.last_observation_sequence > u64::from(record.request.limits.observations)
        || record.accounting.outputs > 256
        || record.accounting.outputs > record.accounting.observations
        || record.accounting.artifact_bytes > record.request.limits.artifact_bytes
        || (record.accounting.artifact_bytes
            < record
                .request
                .input_artifact_bytes()
                .map_err(|cause| corruption(format!("stored peer input is invalid: {cause}")))?)
        || !is_canonical_blake3_digest(&record.observation_digest)
    {
        return Err(corruption(
            "stored peer execution primary facts are invalid",
        ));
    }
    if record.caller != record.request.authorization.caller()
        || &record.authority.request().actor != record.request.authorization.actor()
    {
        return Err(corruption(
            "stored serving caller contradicts accepted authority",
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
    if let Some(plan) = &tombstone.published_invocation {
        plan.validate()?;
        if plan.source
            != (milkdrift_persistence::published::PublishedInvocationSource::Serving {
                caller: tombstone.caller.clone(),
                execution: tombstone.execution.clone(),
            })
            || plan.invocation != tombstone.managed_invocation()?
            || plan.capability != tombstone.capability
            || plan.generation != tombstone.capability_generation
        {
            return Err(corruption(
                "archived published operation lost exact linkage",
            ));
        }
    }
    if tombstone.output_observations.len() != tombstone.accounting.outputs as usize
        || tombstone.output_observations.len() > 256
    {
        return Err(corruption(
            "archived output manifest contradicts its bounded accounting",
        ));
    }
    let mut prior = 0;
    for output in &tombstone.output_observations {
        output
            .validate()
            .map_err(|error| corruption(error.to_string()))?;
        if output.execution != tombstone.execution
            || output.sequence <= prior
            || output.sequence > tombstone.last_observation_sequence
            || output.event.kind().output().is_none()
        {
            return Err(corruption("archived output identity or order is invalid"));
        }
        prior = output.sequence;
    }
    if tombstone.caller != tombstone.authorization.caller()
        || &tombstone.authority.actor != tombstone.authorization.actor()
    {
        return Err(corruption(
            "archived serving caller contradicts accepted authority",
        ));
    }
    if tombstone.schema_version != SERVING_EXECUTION_TOMBSTONE_SCHEMA_VERSION
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
        || !is_canonical_blake3_digest(&tombstone.request_digest)
        || !is_canonical_blake3_digest(&tombstone.catalog_digest)
        || !valid_capability_digest(&tombstone.capability_digest)
        || !is_canonical_blake3_digest(&tombstone.authority.decision_digest)
        || !is_canonical_blake3_digest(&tombstone.observation_digest)
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
