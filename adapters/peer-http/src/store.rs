use milkdrift_peer_protocol::{InvocationAcceptance, InvocationLookup, RemoteExecutionStatus};
use milkdrift_persistence::{PeerExecutionPhase, PeerExecutionRecord};

pub(crate) fn acceptance(record: &PeerExecutionRecord, replayed: bool) -> InvocationAcceptance {
    InvocationAcceptance::Accepted {
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
    }
}

pub(crate) fn lookup(record: &PeerExecutionRecord) -> InvocationLookup {
    InvocationLookup::Known {
        execution: record.execution.clone(),
        request_digest: record.request.request_digest.clone(),
        status: public_status(record),
        last_sequence: record.last_observation_sequence,
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
