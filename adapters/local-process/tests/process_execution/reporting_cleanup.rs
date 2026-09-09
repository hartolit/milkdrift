use super::*;
use milkdrift_capability::InvocationEventKind;
use milkdrift_runtime::ExecutorError;
use std::sync::mpsc;

#[path = "../support/process_cleanup.rs"]
mod process_cleanup;
use process_cleanup::{ProbeCleanup, process_alive, read_pids};

#[derive(Clone, Copy, Debug)]
enum Rejection {
    Initial,
    Stdout,
    Stderr,
    Heartbeat,
    Panic,
}

struct RejectingReporter {
    target: Rejection,
    pid_path: PathBuf,
    expected_pids: usize,
    accepted: TestReporter,
    rejected: AtomicUsize,
}

impl RejectingReporter {
    fn reject(&self) -> Result<(), AdapterError> {
        let deadline = Instant::now() + Duration::from_secs(3);
        while read_pids(&self.pid_path).len() < self.expected_pids && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(5));
        }
        self.rejected.fetch_add(1, Ordering::SeqCst);
        if matches!(self.target, Rejection::Panic) {
            std::panic::resume_unwind(Box::new("injected reporter panic"));
        }
        Err(AdapterError::external_failure("injected reporting failure"))
    }
}

impl AdapterReporter for RejectingReporter {
    fn invocation(&self, event: InvocationEvent) -> Result<(), AdapterError> {
        if let InvocationEventKind::Progress { message, .. } = event.kind() {
            let reject = match self.target {
                Rejection::Initial | Rejection::Panic => {
                    message.starts_with("local process started")
                }
                Rejection::Stdout => message.starts_with("stdout: "),
                Rejection::Stderr => message.starts_with("stderr: "),
                Rejection::Heartbeat => false,
            };
            if reject {
                // Even the initial-report case must prove the child entered and left
                // its pipes open, rather than killing it before fixture startup.
                return self.reject();
            }
        }
        self.accepted.invocation(event)
    }

    fn heartbeat(&self) -> Result<(), AdapterError> {
        if matches!(self.target, Rejection::Heartbeat) {
            self.reject()
        } else {
            self.accepted.heartbeat()
        }
    }
}

fn reporting_rejection(target: Rejection, tree: bool) -> TestResult {
    let data = Arc::new(TestDataAccess::new()?);
    let pid_directory = tempfile::tempdir()?;
    let pid_path = pid_directory
        .path()
        .join(if tree { "tree.pids" } else { "probe.pids" });
    let cleanup = ProbeCleanup(pid_path.clone());
    let arguments = if tree {
        vec![json!("tree"), json!(pid_path), json!("30000")]
    } else {
        vec![json!("reporting-probe"), json!(pid_path)]
    };
    let mut value = profile_value(&data.root, arguments)?;
    value["profile"]["limits"]["wall_timeout_ms"] = json!(30000);
    value["profile"]["limits"]["heartbeat_interval_ms"] = json!(100);
    value["profile"]["inputs"] = json!([{ "input": "prompt", "relative_path": "prompt.txt" }]);
    value["profile"]["stdin"] = json!({ "type": "input", "input": "prompt", "max_bytes": 65536 });
    let profile = parse_profile(&value)?;
    let request = request(
        &profile,
        "reporting-cleanup",
        vec![input(
            "prompt",
            json!(["i".repeat(30000), "i".repeat(30000)]),
        )?],
    )?;
    let (host, snapshot) = setup(profile, data, Arc::new(InMemorySecretResolver::new()))?;
    let reporter = Arc::new(RejectingReporter {
        target,
        pid_path: pid_path.clone(),
        expected_pids: if tree { 3 } else { 1 },
        accepted: TestReporter::default(),
        rejected: AtomicUsize::new(0),
    });
    let worker_reporter = reporter.clone();
    let worker_host = host.clone();
    let worker_request = request.clone();
    let context = context()?;
    let (sender, receiver) = mpsc::sync_channel(1);
    let worker = thread::spawn(move || {
        let result = worker_host.execute_exact_with_context(
            &snapshot,
            &worker_request,
            &context,
            worker_reporter.as_ref(),
        );
        let _ = sender.send(result);
    });
    let result = receiver.recv_timeout(Duration::from_secs(5));
    let pids = read_pids(&pid_path);
    let survivors = pids
        .iter()
        .copied()
        .map(|pid| Ok((pid, process_alive(pid)?)))
        .collect::<TestResult<Vec<_>>>();
    // Kill fixture processes even when the adapter returns early or hangs. A second
    // bounded receive avoids hiding a regression behind an unbounded test join.
    drop(cleanup);
    if result.is_err() {
        let _ = receiver.recv_timeout(Duration::from_secs(3));
    }
    if worker.is_finished() {
        worker.join().map_err(|_| "execution thread panicked")?;
    }
    assert_eq!(
        pids.len(),
        if tree { 3 } else { 1 },
        "fixture never entered for {target:?}"
    );
    let survivors: Vec<_> = survivors?.into_iter().filter(|(_, alive)| *alive).collect();
    assert!(
        survivors.is_empty(),
        "live children after {target:?}: {survivors:?}"
    );
    match target {
        Rejection::Panic => assert!(matches!(
            result?,
            Err(ExecutorError::AdapterPanicked { after_entry: true })
        )),
        _ => assert!(
            matches!(result?, Err(ExecutorError::BoundaryAfterEntry(message)) if message.contains("injected reporting failure"))
        ),
    }
    assert_eq!(reporter.rejected.load(Ordering::SeqCst), 1);
    assert_eq!(terminal_status(&reporter.accepted.events()?), None);
    let cancellation = TaskExecutor::cancel(
        &host,
        &CancellationRequest::new(request.invocation().clone(), 1, "after cleanup")?,
    );
    assert!(matches!(cancellation, Err(ExecutorError::Unavailable(_))));
    Ok(())
}

#[test]
fn initial_report_rejection_terminates_child() -> TestResult {
    reporting_rejection(Rejection::Initial, false)
}

#[test]
fn stdout_report_rejection_terminates_child() -> TestResult {
    reporting_rejection(Rejection::Stdout, false)
}

#[test]
fn stderr_report_rejection_terminates_child() -> TestResult {
    reporting_rejection(Rejection::Stderr, false)
}

#[test]
fn heartbeat_rejection_terminates_child() -> TestResult {
    reporting_rejection(Rejection::Heartbeat, false)
}

#[test]
fn reporter_panic_terminates_child_before_host_catches_it() -> TestResult {
    reporting_rejection(Rejection::Panic, false)
}

#[test]
fn termination_paths_settle_the_child_and_keep_terminal_meaning() -> TestResult {
    for mode in ["cancel", "timeout", "overflow", "shutdown"] {
        let data = Arc::new(TestDataAccess::new()?);
        let directory = tempfile::tempdir()?;
        let pid_path = directory.path().join("child.pid");
        let cleanup = ProbeCleanup(pid_path.clone());
        let mut value = profile_value(&data.root, vec![json!("reporting-probe"), json!(pid_path)])?;
        if mode == "timeout" {
            value["profile"]["limits"]["wall_timeout_ms"] = json!(1000);
        }
        if mode == "overflow" {
            value["profile"]["stdout"]["max_capture_bytes"] = json!(4);
            value["profile"]["stdout"]["overflow_action"] = json!("terminate");
        }
        let profile = parse_profile(&value)?;
        let request = request(&profile, "termination-cleanup", Vec::new())?;
        let adapter = Arc::new(LocalProcessAdapter::new(
            profile,
            data,
            Arc::new(InMemorySecretResolver::new()),
        )?);
        adapter.start()?;
        let snapshot =
            ResolvedCapabilitySnapshot::from_descriptor(adapter.descriptor(), request.operation())?;
        let reporter = Arc::new(TestReporter::default());
        let worker_adapter = adapter.clone();
        let worker_reporter = reporter.clone();
        let worker_request = request.clone();
        let context = context()?;
        let (sender, receiver) = mpsc::sync_channel(1);
        let worker = thread::spawn(move || {
            let result = worker_adapter.execute(
                &AdapterInvocation::with_context(&snapshot, &worker_request, &context),
                worker_reporter.as_ref(),
            );
            let _ = sender.send(result);
        });
        let deadline = Instant::now() + Duration::from_secs(3);
        while read_pids(&pid_path).is_empty() && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(5));
        }
        if mode == "shutdown" {
            adapter.shutdown()?;
        }
        if mode == "cancel" {
            adapter.cancel(&CancellationRequest::new(
                request.invocation().clone(),
                1,
                "cleanup test",
            )?)?;
        }
        let result = receiver.recv_timeout(Duration::from_secs(5));
        let pids = read_pids(&pid_path);
        let alive = pids
            .iter()
            .map(|pid| process_alive(*pid))
            .collect::<TestResult<Vec<_>>>();
        drop(cleanup);
        if result.is_err() {
            let _ = receiver.recv_timeout(Duration::from_secs(3));
        }
        if worker.is_finished() {
            worker.join().map_err(|_| "termination worker panicked")?;
        }
        result??;
        assert_eq!(pids.len(), 1, "{mode} fixture did not enter");
        assert_eq!(alive?, vec![false], "{mode} left a live child");
        let events = reporter.events()?;
        match mode {
            "cancel" | "shutdown" => {
                assert_eq!(terminal_status(&events), Some(TerminalStatus::Cancelled))
            }
            "timeout" => assert_eq!(terminal_failure_code(&events), Some("process_timeout")),
            "overflow" => assert_eq!(
                terminal_failure_code(&events),
                Some("process_output_overflow")
            ),
            _ => unreachable!(),
        }
        assert_eq!(adapter.health(1)?.current_load(), 0);
    }
    Ok(())
}

#[cfg(unix)]
#[test]
fn reporting_rejection_terminates_owned_descendants() -> TestResult {
    reporting_rejection(Rejection::Initial, true)?;
    reporting_rejection(Rejection::Heartbeat, true)
}
