use std::sync::{Arc, Barrier, atomic::Ordering};

use milkdrift_authority::ActorRef;
use milkdrift_capability::CapabilityDescriptorDocument;
use milkdrift_persistence::WorkerId;
use milkdrift_redb_store::RedbStore;

use crate::{
    DeterministicExecutor, ManualClock, RetryPolicy, RuntimeConfig, RuntimeService,
    SchedulerLimits, SequentialIdGenerator,
};

#[test]
fn structured_scan_claims_share_one_exact_budget() -> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let descriptor = CapabilityDescriptorDocument::from_json(include_bytes!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../capability/tests/fixtures/descriptor-v1.json"
    )))?;
    let runtime = RuntimeService::open_closed(
        Arc::new(RedbStore::open(directory.path())?),
        Arc::new(DeterministicExecutor::new(descriptor.body().clone())),
        Arc::new(ManualClock::new(100)),
        Arc::new(SequentialIdGenerator::new("scan-budget", 1)?),
        RuntimeConfig::new(
            WorkerId::new("worker-scan-budget")?,
            ActorRef::new("actor:scan-budget")?,
            100,
            10,
            SchedulerLimits::new(1, 1, 1, 1)?,
            RetryPolicy::new(1, Vec::new(), 1, 1, 0)?,
        )?,
    )?;
    assert_eq!(runtime.claim_structured_scan_visits(usize::MAX), usize::MAX);
    runtime.structured_scan_budget.store(5, Ordering::Release);
    runtime
        .structured_scan_budget_active
        .store(true, Ordering::Release);
    assert_eq!(runtime.claim_structured_scan_visits(0), 0);
    let barrier = Barrier::new(2);
    let mut claims = std::thread::scope(|scope| {
        let first = scope.spawn(|| {
            barrier.wait();
            runtime.claim_structured_scan_visits(4)
        });
        barrier.wait();
        let second = runtime.claim_structured_scan_visits(4);
        first
            .join()
            .map(|first| [first, second])
            .map_err(|_| "scan budget claimant panicked")
    })?;
    claims.sort_unstable();
    assert_eq!(claims, [1, 4]);
    assert_eq!(runtime.structured_scan_budget.load(Ordering::Acquire), 0);
    assert_eq!(runtime.claim_structured_scan_visits(usize::MAX), 0);
    runtime
        .structured_scan_budget_active
        .store(false, Ordering::Release);
    assert_eq!(runtime.claim_structured_scan_visits(3), 3);
    assert_eq!(runtime.structured_scan_budget.load(Ordering::Acquire), 0);
    Ok(())
}
