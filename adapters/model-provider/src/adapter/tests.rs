use super::*;

fn limits() -> MaterializationLimits {
    MaterializationLimits {
        max_files: 2,
        max_file_bytes: 5,
        max_total_bytes: 8,
        max_path_bytes: 64,
        max_directory_depth: 4,
        chunk_bytes: 4,
    }
}

#[test]
fn aggregate_materialization_ledger_enforces_count_bytes_and_exact_boundary() {
    let mut exact = MaterializationLedger::new(limits());
    assert!(exact.record(3).is_ok());
    assert!(exact.record(5).is_ok());
    assert!(exact.record(0).is_err());

    let mut bytes = MaterializationLedger::new(limits());
    assert!(bytes.record(4).is_ok());
    assert!(bytes.record(5).is_err());

    let mut file = MaterializationLedger::new(limits());
    assert!(file.record(6).is_err());
}

#[test]
fn active_registration_is_removed_during_panic_unwind() -> Result<(), Box<dyn std::error::Error>> {
    let invocation = milkdrift_capability::InvocationId::new("invocation-panic-cleanup")?;
    let active = Mutex::new(BTreeMap::from([(
        invocation.clone(),
        Arc::new(AtomicBool::new(false)),
    )]));
    let unwind = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _guard = ActiveInvocationGuard {
            active: &active,
            invocation,
        };
        std::panic::resume_unwind(Box::new("contained test panic"));
    }));
    assert!(unwind.is_err());
    assert!(
        active
            .lock()
            .map_err(|_| "active invocation test lock is poisoned")?
            .is_empty()
    );
    Ok(())
}
