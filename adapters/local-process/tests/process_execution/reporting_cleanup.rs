use super::*;
use milkdrift_capability::InvocationEventKind;
use milkdrift_runtime::ExecutorError;
use std::sync::mpsc;

use super::process_cleanup::{ProbeCleanup, process_alive, read_pids};

pub(super) fn finish_worker<T>(
    result: Result<T, mpsc::RecvTimeoutError>,
    receiver: &mpsc::Receiver<T>,
    worker: thread::JoinHandle<Result<(), &'static str>>,
) -> TestResult<T> {
    match result {
        Ok(result) => {
            worker.join().map_err(|_| "execution worker panicked")??;
            Ok(result)
        }
        Err(error) => {
            // Fixture processes have already been released/killed. Join a late result,
            // but never turn missing timely completion into a passing assertion.
            let completed = receiver.recv_timeout(Duration::from_secs(3)).is_ok();
            if completed || worker.is_finished() {
                worker
                    .join()
                    .map_err(|_| "execution worker panicked during fallback cleanup")??;
            }
            Err(format!("execution did not complete in time: {error}; completion after fixture cleanup observed={completed}").into())
        }
    }
}

#[test]
fn inherited_idle_pipes_cannot_hold_invocation_cleanup() -> TestResult {
    for mode in ["exit", "cancel", "timeout", "shutdown"] {
        let data = Arc::new(TestDataAccess::new()?);
        let directory = tempfile::tempdir()?;
        let pids = directory.path().join("pids");
        let release = directory.path().join("release");
        let cleanup = ProbeCleanup(pids.clone());
        let mut value = profile_value(
            &data.root,
            vec![
                json!("escaped-pipes"),
                json!(pids),
                json!(release),
                json!(if mode == "exit" { "exit" } else { "wait" }),
            ],
        )?;
        value
            .pointer_mut("/profile")
            .and_then(Value::as_object_mut)
            .ok_or("fixture object absent")?
            .insert(
                "inputs".to_owned(),
                json!([{ "input": "prompt", "relative_path": "prompt.txt" }]),
            );
        value
            .pointer_mut("/profile")
            .and_then(Value::as_object_mut)
            .ok_or("fixture object absent")?
            .insert(
                "stdin".to_owned(),
                json!({ "type": "input", "input": "prompt", "max_bytes": 65536 }),
            );
        value
            .pointer_mut("/profile/limits")
            .and_then(Value::as_object_mut)
            .ok_or("fixture object absent")?
            .insert(
                "wall_timeout_ms".to_owned(),
                json!(if mode == "timeout" { 800 } else { 30000 }),
            );
        let profile = parse_profile(&value)?;
        let request = request(
            &profile,
            "inherited-pipes",
            vec![input(
                "prompt",
                json!(["i".repeat(30000), "i".repeat(30000)]),
            )?],
        )?;
        let (host, snapshot) = setup(
            profile,
            data.clone(),
            Arc::new(InMemorySecretResolver::new()),
        )?;
        let reporter = Arc::new(TestReporter::default());
        let worker_host = host.clone();
        let worker_reporter = reporter.clone();
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
            sender
                .send(result)
                .map_err(|_| "execution result receiver disappeared")
        });
        let ready_deadline = Instant::now() + Duration::from_secs(3);
        while read_pids(&pids).len() < 2 && Instant::now() < ready_deadline {
            thread::sleep(Duration::from_millis(5));
        }
        let entered = read_pids(&pids);
        if mode == "cancel" {
            for sequence in 1..=3 {
                TaskExecutor::cancel(
                    &host,
                    &CancellationRequest::new(
                        request.invocation().clone(),
                        sequence,
                        "inherited pipes",
                    )?,
                )?;
            }
        } else if mode == "shutdown" {
            host.force_shutdown()?;
        }
        let result = receiver.recv_timeout(Duration::from_secs(2));
        let holder_alive = entered.get(1).map(|pid| process_alive(*pid)).transpose();
        fs::write(&release, b"release")?;
        drop(cleanup);
        let result = finish_worker(result, &receiver, worker)?;
        assert_eq!(entered.len(), 2, "fixture did not enter: {mode}");
        result?;
        assert_eq!(
            holder_alive?,
            Some(true),
            "fixture must still hold pipes at cleanup: {mode}"
        );
        let events = reporter.events()?;
        assert_eq!(
            terminal_status(&events),
            Some(TerminalStatus::Uncertain),
            "{mode}"
        );
        assert_eq!(
            terminal_failure_code(&events),
            Some("process_io_incomplete")
        );
        let terminal = events
            .iter()
            .find_map(|event| match event.kind() {
                InvocationEventKind::Terminal { terminal } => Some(terminal),
                _ => None,
            })
            .ok_or("missing terminal")?;
        let evidence = serde_json::to_value(terminal)?;
        let cleanup = evidence
            .pointer("/usage/extensions/org.milkdrift~1process-cleanup")
            .ok_or("cleanup evidence absent")?;
        assert_eq!(
            cleanup
                .get("local_io_joined")
                .ok_or("cleanup local_io_joined absent")?,
            true
        );
        assert_eq!(
            cleanup
                .get("parent_exit_observed")
                .ok_or("cleanup parent_exit_observed absent")?,
            true
        );
        assert_eq!(
            cleanup.pointer("/stdout/eof").ok_or("cleanup EOF absent")?,
            false
        );
        assert_eq!(
            cleanup.pointer("/stderr/eof").ok_or("cleanup EOF absent")?,
            false
        );
        assert_eq!(
            evidence
                .pointer("/failure/retryable")
                .ok_or("retry refusal absent")?,
            false
        );
        assert!(
            evidence
                .pointer("/usage/cost_micros")
                .ok_or("cost absent")?
                .is_null()
        );
        assert!(data.output("stdout")?.is_none());
        assert!(matches!(
            TaskExecutor::cancel(
                &host,
                &CancellationRequest::new(request.invocation().clone(), 4, "after cleanup")?
            ),
            Err(ExecutorError::Unavailable(_))
        ));
    }
    Ok(())
}

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
    reporting_rejection_with_inheritance(target, tree, false)
}

fn reporting_rejection_with_inheritance(
    target: Rejection,
    tree: bool,
    escaped: bool,
) -> TestResult {
    let data = Arc::new(TestDataAccess::new()?);
    let pid_directory = tempfile::tempdir()?;
    let pid_path = pid_directory
        .path()
        .join(if tree { "tree.pids" } else { "probe.pids" });
    let cleanup = ProbeCleanup(pid_path.clone());
    let arguments = if escaped {
        vec![
            json!("escaped-pipes"),
            json!(pid_path),
            json!(pid_directory.path().join("release")),
            json!("wait"),
        ]
    } else if tree {
        vec![json!("tree"), json!(pid_path), json!("30000")]
    } else {
        vec![json!("reporting-probe"), json!(pid_path)]
    };
    let mut value = profile_value(&data.root, arguments)?;
    value
        .pointer_mut("/profile/limits")
        .and_then(Value::as_object_mut)
        .ok_or("fixture object absent")?
        .insert("wall_timeout_ms".to_owned(), json!(30000));
    value
        .pointer_mut("/profile/limits")
        .and_then(Value::as_object_mut)
        .ok_or("fixture object absent")?
        .insert("heartbeat_interval_ms".to_owned(), json!(100));
    value
        .pointer_mut("/profile")
        .and_then(Value::as_object_mut)
        .ok_or("fixture object absent")?
        .insert(
            "inputs".to_owned(),
            json!([{ "input": "prompt", "relative_path": "prompt.txt" }]),
        );
    value
        .pointer_mut("/profile")
        .and_then(Value::as_object_mut)
        .ok_or("fixture object absent")?
        .insert(
            "stdin".to_owned(),
            json!({ "type": "input", "input": "prompt", "max_bytes": 65536 }),
        );
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
        expected_pids: if escaped {
            2
        } else if tree {
            3
        } else {
            1
        },
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
        sender
            .send(result)
            .map_err(|_| "execution result receiver disappeared")
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
    let result = finish_worker(result, &receiver, worker)?;
    assert_eq!(
        pids.len(),
        if escaped {
            2
        } else if tree {
            3
        } else {
            1
        },
        "fixture never entered for {target:?}"
    );
    let survivors: Vec<_> = survivors?.into_iter().filter(|(_, alive)| *alive).collect();
    if escaped {
        assert_eq!(survivors.len(), 1, "only the external pipe holder remains");
        assert_eq!(
            survivors.first().ok_or("missing surviving holder")?.0,
            *pids.get(1).ok_or("missing holder PID")?
        );
    } else {
        assert!(
            survivors.is_empty(),
            "live children after {target:?}: {survivors:?}"
        );
    }
    match target {
        Rejection::Panic => assert!(matches!(
            result,
            Err(ExecutorError::AdapterPanicked { after_entry: true })
        )),
        _ => assert!(
            matches!(result, Err(ExecutorError::BoundaryAfterEntry(message)) if message.contains("injected reporting failure"))
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
fn failed_reporting_and_unwinding_interrupt_inherited_idle_pipes() -> TestResult {
    for rejection in [Rejection::Initial, Rejection::Heartbeat, Rejection::Panic] {
        reporting_rejection_with_inheritance(rejection, false, true)?;
    }
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
    for mode in [
        "cancel",
        "timeout",
        "overflow",
        "shutdown",
        #[cfg(unix)]
        "stopped",
    ] {
        let data = Arc::new(TestDataAccess::new()?);
        let directory = tempfile::tempdir()?;
        let pid_path = directory.path().join("child.pid");
        let cleanup = ProbeCleanup(pid_path.clone());
        let mut value = profile_value(&data.root, vec![json!("reporting-probe"), json!(pid_path)])?;
        if mode == "timeout" {
            value
                .pointer_mut("/profile/limits")
                .and_then(Value::as_object_mut)
                .ok_or("fixture object absent")?
                .insert("wall_timeout_ms".to_owned(), json!(1000));
        }
        if mode == "overflow" {
            value
                .pointer_mut("/profile/stdout")
                .and_then(Value::as_object_mut)
                .ok_or("fixture object absent")?
                .insert("max_capture_bytes".to_owned(), json!(4));
            value
                .pointer_mut("/profile/stdout")
                .and_then(Value::as_object_mut)
                .ok_or("fixture object absent")?
                .insert("overflow_action".to_owned(), json!("terminate"));
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
            sender
                .send(result)
                .map_err(|_| "execution result receiver disappeared")
        });
        let deadline = Instant::now() + Duration::from_secs(3);
        while read_pids(&pid_path).is_empty() && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(5));
        }
        #[cfg(unix)]
        if mode == "stopped" {
            use rustix::process::{Pid, Signal, WaitOptions, kill_process, waitpid};
            let pid = Pid::from_raw(i32::try_from(
                *read_pids(&pid_path).first().ok_or("missing child")?,
            )?)
            .ok_or("invalid pid")?;
            kill_process(pid, Signal::STOP)?;
            // Observe the stop, then cancel: a stopped process cannot service TERM.
            // waitpid consumes only the stop notification; the adapter reaps exit.
            loop {
                if waitpid(Some(pid), WaitOptions::NOHANG | WaitOptions::UNTRACED)?
                    .is_some_and(|(_, status)| status.stopped())
                {
                    break;
                }
                assert!(Instant::now() < deadline, "child never stopped");
                thread::sleep(Duration::from_millis(5));
            }
        }
        if mode == "shutdown" {
            adapter.shutdown()?;
        }
        if mode == "cancel" || mode == "stopped" {
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
        let result = finish_worker(result, &receiver, worker)?;
        result?;
        assert_eq!(pids.len(), 1, "{mode} fixture did not enter");
        assert_eq!(alive?, vec![false], "{mode} left a live child");
        let events = reporter.events()?;
        match mode {
            "cancel" | "shutdown" | "stopped" => {
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
