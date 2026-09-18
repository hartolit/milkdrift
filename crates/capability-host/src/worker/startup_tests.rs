use super::*;

#[test]
fn partial_startup_closes_both_queues_and_joins_before_returning() {
    let (execution, execute) = sync_channel(1);
    let (cancellation, cancel) = sync_channel(1);
    let exited = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let mut startup = WorkerStartup {
        execution: Some(execution),
        cancellation: Some(cancellation),
        joins: Vec::new(),
    };
    for receiver in [execute, cancel] {
        let exited = exited.clone();
        startup.joins.push(thread::spawn(move || {
            assert!(receiver.recv().is_err());
            exited.fetch_add(1, Ordering::SeqCst);
        }));
    }
    // The same guard is dropped by either execution- or cancellation-thread spawn failure.
    drop(startup);
    assert_eq!(exited.load(Ordering::SeqCst), 2);
}
