use milkdrift_peer_protocol::{
    ArchivedExecutionSummary, InvocationAcceptance, InvocationLookup, ObservationHistory,
    RemoteExecutionStatus,
};
use milkdrift_persistence::{
    PeerArchivedDisposition, PeerExecutionPhase, PeerExecutionRecord, PeerExecutionSnapshot,
    PeerExecutionTombstone,
};

pub(crate) fn acceptance(snapshot: &PeerExecutionSnapshot, replayed: bool) -> InvocationAcceptance {
    match snapshot {
        PeerExecutionSnapshot::Hot(record) => InvocationAcceptance::Accepted {
            request_id: record.request.request_id.clone(),
            execution: record.execution.clone(),
            request_digest: record.request.request_digest.clone(),
            accepted_at_unix_ms: record.accepted_at_unix_ms,
            lease_expires_at_unix_ms: record
                .phase
                .claim()
                .map_or(record.request.deadline_unix_ms, |claim| {
                    claim.lease_expires_at_unix_ms
                }),
            replayed,
        },
        PeerExecutionSnapshot::Archived(tombstone) => InvocationAcceptance::Archived {
            request_id: tombstone.request_id.clone(),
            execution: tombstone.execution.clone(),
            request_digest: tombstone.request_digest.clone(),
            accepted_at_unix_ms: tombstone.accepted_at_unix_ms,
            summary: Box::new(archived_summary(tombstone)),
        },
    }
}

pub(crate) fn lookup(snapshot: &PeerExecutionSnapshot) -> InvocationLookup {
    InvocationLookup::Known {
        request_id: snapshot.request_id().clone(),
        execution: snapshot.execution().clone(),
        request_digest: snapshot.request_digest().to_owned(),
        accepted_at_unix_ms: match snapshot {
            PeerExecutionSnapshot::Hot(record) => record.accepted_at_unix_ms,
            PeerExecutionSnapshot::Archived(tombstone) => tombstone.accepted_at_unix_ms,
        },
        status: snapshot_status(snapshot),
        last_sequence: snapshot.last_observation_sequence(),
        history: match snapshot {
            PeerExecutionSnapshot::Hot(_) => ObservationHistory::Hot,
            PeerExecutionSnapshot::Archived(tombstone) => ObservationHistory::Archived {
                summary: Box::new(archived_summary(tombstone)),
            },
        },
    }
}

pub(crate) const fn public_status(record: &PeerExecutionRecord) -> RemoteExecutionStatus {
    match record.phase {
        PeerExecutionPhase::DispatchAvailable { .. }
        | PeerExecutionPhase::DispatchClaimed { .. }
        | PeerExecutionPhase::CancellationRequested { evidence: None, .. } => {
            RemoteExecutionStatus::Accepted
        }
        PeerExecutionPhase::Entered { .. }
        | PeerExecutionPhase::CancellationRequested {
            evidence: Some(_), ..
        } => RemoteExecutionStatus::Running,
        PeerExecutionPhase::Terminal { .. } => RemoteExecutionStatus::Terminal,
        PeerExecutionPhase::Uncertain { .. } => RemoteExecutionStatus::OutcomeUnknown,
    }
}

pub(crate) const fn snapshot_status(snapshot: &PeerExecutionSnapshot) -> RemoteExecutionStatus {
    match snapshot {
        PeerExecutionSnapshot::Hot(record) => public_status(record),
        PeerExecutionSnapshot::Archived(tombstone) => match tombstone.disposition {
            PeerArchivedDisposition::Terminal { .. } => RemoteExecutionStatus::Terminal,
            PeerArchivedDisposition::Uncertain { .. } => RemoteExecutionStatus::OutcomeUnknown,
        },
    }
}

pub(crate) fn archived_summary(tombstone: &PeerExecutionTombstone) -> ArchivedExecutionSummary {
    let (status, final_observation, uncertainty_reason) = match &tombstone.disposition {
        PeerArchivedDisposition::Terminal { observation } => (
            RemoteExecutionStatus::Terminal,
            Some((**observation).clone()),
            None,
        ),
        PeerArchivedDisposition::Uncertain { reason, .. } => (
            RemoteExecutionStatus::OutcomeUnknown,
            None,
            Some(reason.clone()),
        ),
    };
    ArchivedExecutionSummary {
        status,
        last_sequence: tombstone.last_observation_sequence,
        observation_digest: tombstone.observation_digest.clone(),
        archived_at_unix_ms: tombstone.archived_at_unix_ms,
        final_observation,
        uncertainty_reason,
    }
}
