use std::sync::Arc;

use milkdrift_capability::PeerId;
use milkdrift_peer_protocol::{ExecutionLimits, PeerExecutionId, ServingCaller};
use milkdrift_persistence::WorkerId;
use milkdrift_redb_store::RedbStore;

use super::PeerStoreReporter;
use crate::{PeerClock, PeerClockError};

struct FixedClock;
impl PeerClock for FixedClock {
    fn now_unix_ms(&self) -> Result<u64, PeerClockError> {
        Ok(100)
    }
}

#[test]
fn rejected_report_exposes_failure_to_retain_uncertainty() -> Result<(), Box<dyn std::error::Error>>
{
    let directory = tempfile::tempdir()?;
    let reporter = PeerStoreReporter {
        caller: ServingCaller::peer(&PeerId::new("host")?, &PeerId::new("caller")?),
        execution: PeerExecutionId::new("missing-execution")?,
        executions: Arc::new(RedbStore::open(directory.path())?),
        clock: Arc::new(FixedClock),
        lease_ms: 10,
        limits: ExecutionLimits {
            nested_invocations: None,
            artifact_bytes: 0,
            duration_ms: 10,
            cost_micros: 0,
            cost_currency: None,
            input_units: None,
            output_units: None,
            observations: 1,
        },
        input_artifact_bytes: 0,
        deadline_unix_ms: 110,
        worker: WorkerId::new("report-worker")?,
        claim_generation: 1,
    };
    let error = reporter.reject_report("peer_report_rejected", "invalid observation");
    let detail = error.to_string();
    assert!(detail.contains("peer_report_rejected: invalid observation"));
    assert!(detail.contains("uncertainty could not be persisted"));
    Ok(())
}
