//! Cancellation and shutdown leave signal delivery to the execution monitor.

use super::*;
use milkdrift_capability::InvocationEventKind;
use std::sync::mpsc;

use super::process_cleanup as cleanup;

struct PausedReporter {
    entered: mpsc::SyncSender<()>,
    resume: Mutex<mpsc::Receiver<()>>,
    accepted: TestReporter,
}

impl AdapterReporter for PausedReporter {
    fn invocation(&self, event: InvocationEvent) -> Result<(), AdapterError> {
        if matches!(event.kind(), InvocationEventKind::Progress { message, .. }
            if message.starts_with("local process started"))
        {
            self.entered
                .send(())
                .map_err(|_| AdapterError::external_failure("test observer disappeared"))?;
            self.resume
                .lock()
                .map_err(|_| AdapterError::external_failure("test gate poisoned"))?
                .recv_timeout(Duration::from_secs(10))
                .map_err(|_| AdapterError::external_failure("test monitor was not released"))?;
        }
        self.accepted.invocation(event)
    }

    fn heartbeat(&self) -> Result<(), AdapterError> {
        self.accepted.heartbeat()
    }
}

#[test]
fn cancellation_and_shutdown_signal_only_through_the_monitor() -> TestResult {
    for shutdown in [false, true] {
        let data = Arc::new(TestDataAccess::new()?);
        let directory = tempfile::tempdir()?;
        let pids = directory.path().join("pids");
        let proceed = directory.path().join("proceed");
        let responsive = directory.path().join("responsive");
        let cleanup = cleanup::ProbeCleanup(pids.clone());
        let profile = parse_profile(&profile_value(
            &data.root,
            vec![
                json!("reporting-probe"),
                json!(pids),
                json!(proceed),
                json!(responsive),
            ],
        )?)?;
        let request = request(&profile, "monitor-owned-shutdown", Vec::new())?;
        let adapter = Arc::new(LocalProcessAdapter::new(
            profile,
            data,
            Arc::new(InMemorySecretResolver::new()),
        )?);
        adapter.start()?;
        let snapshot =
            ResolvedCapabilitySnapshot::from_descriptor(adapter.descriptor(), request.operation())?;
        let (entered, entry) = mpsc::sync_channel(1);
        let (resume, resumed) = mpsc::sync_channel(1);
        let reporter = Arc::new(PausedReporter {
            entered,
            resume: Mutex::new(resumed),
            accepted: TestReporter::default(),
        });
        let worker_adapter = adapter.clone();
        let worker_reporter = reporter.clone();
        let worker_request = request.clone();
        let context = context()?;
        let (finished, completion) = mpsc::sync_channel(1);
        let worker = thread::spawn(move || {
            let result = worker_adapter.execute(
                &AdapterInvocation::with_context(&snapshot, &worker_request, &context),
                worker_reporter.as_ref(),
            );
            let _ = finished.send(result);
        });
        entry.recv_timeout(Duration::from_secs(5))?;
        if shutdown {
            adapter.shutdown()?;
        } else {
            assert!(
                adapter
                    .cancel(&CancellationRequest::new(
                        request.invocation().clone(),
                        1,
                        "monitor gate"
                    )?)?
                    .accepted()
            );
        }
        fs::write(proceed, b"continue")?;
        let deadline = Instant::now() + Duration::from_secs(3);
        while !responsive.exists() && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(5));
        }
        // A PID can still exist as a zombie. A new marker proves the child actually
        // ran after the request while its monitor remained paused.
        let still_running = responsive.exists();
        resume.send(())?;
        let result = completion.recv_timeout(Duration::from_secs(5));
        let pids = cleanup::read_pids(&pids);
        let alive = pids
            .iter()
            .map(|pid| cleanup::process_alive(*pid))
            .collect::<TestResult<Vec<_>>>();
        drop(cleanup);
        if worker.is_finished() {
            worker.join().map_err(|_| "shutdown worker panicked")?;
        }
        result??;
        assert!(
            still_running,
            "shutdown={shutdown} signalled outside the monitor"
        );
        assert_eq!(pids.len(), 1);
        assert_eq!(alive?, vec![false]);
        assert_eq!(
            terminal_status(&reporter.accepted.events()?),
            Some(TerminalStatus::Cancelled)
        );
        assert_eq!(adapter.health(1)?.current_load(), 0);
    }
    Ok(())
}
