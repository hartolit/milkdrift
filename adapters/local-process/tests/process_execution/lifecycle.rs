use super::*;

#[test]
fn nonzero_exit_signal_and_timeout_are_typed_failures() -> TestResult {
    for (suffix, arguments, timeout) in [
        ("nonzero", vec![json!("exit"), json!("7")], 5000_u64),
        ("signal", vec![json!("signal")], 5000_u64),
        ("timeout", vec![json!("sleep"), json!("5000")], 50_u64),
    ] {
        let data = Arc::new(TestDataAccess::new()?);
        let mut value = profile_value(&data.root, arguments)?;
        value["profile"]["limits"]["wall_timeout_ms"] = json!(timeout);
        value["profile"]["limits"]["heartbeat_interval_ms"] = json!(25);
        let profile = parse_profile(&value)?;
        let request = request(&profile, &format!("invocation-{suffix}"), Vec::new())?;
        let (host, snapshot) = setup(profile, data, Arc::new(InMemorySecretResolver::new()))?;
        let reporter = TestReporter::default();
        host.execute_exact_with_context(&snapshot, &request, &context()?, &reporter)?;
        assert_eq!(
            terminal_status(&reporter.events()?),
            Some(TerminalStatus::Failure)
        );
    }
    Ok(())
}

#[test]
fn explicit_cancellation_observes_terminal_group_cleanup() -> TestResult {
    let data = Arc::new(TestDataAccess::new()?);
    let profile = parse_profile(&profile_value(
        &data.root,
        vec![json!("sleep"), json!("10000")],
    )?)?;
    let request = request(&profile, "invocation-cancel", Vec::new())?;
    let (host, snapshot) = setup(profile, data, Arc::new(InMemorySecretResolver::new()))?;
    let reporter = Arc::new(TestReporter::default());
    let thread_host = host.clone();
    let thread_reporter = reporter.clone();
    let thread_request = request.clone();
    let context = context()?;
    let handle = thread::spawn(move || {
        thread_host.execute_exact_with_context(
            &snapshot,
            &thread_request,
            &context,
            thread_reporter.as_ref(),
        )
    });
    reporter.wait_until_started()?;
    let acknowledgement = TaskExecutor::cancel(
        &host,
        &CancellationRequest::new(request.invocation().clone(), 1, "test cancellation")?,
    )?;
    assert!(acknowledgement.accepted());
    assert!(!acknowledgement.terminal_boundary());
    handle.join().map_err(|_| "execution thread panicked")??;
    assert_eq!(
        terminal_status(&reporter.events()?),
        Some(TerminalStatus::Cancelled)
    );
    Ok(())
}

#[cfg(unix)]
#[test]
fn cancellation_terminates_child_and_grandchild_in_owned_group() -> TestResult {
    let data = Arc::new(TestDataAccess::new()?);
    let pid_owner = tempfile::NamedTempFile::new()?;
    let pid_path = pid_owner.path().to_string_lossy().to_string();
    let profile = parse_profile(&profile_value(
        &data.root,
        vec![json!("tree"), json!(pid_path), json!("10000")],
    )?)?;
    let request = request(&profile, "invocation-tree", Vec::new())?;
    let (host, snapshot) = setup(profile, data, Arc::new(InMemorySecretResolver::new()))?;
    let reporter = Arc::new(TestReporter::default());
    let thread_host = host.clone();
    let thread_reporter = reporter.clone();
    let thread_request = request.clone();
    let context = context()?;
    let handle = thread::spawn(move || {
        thread_host.execute_exact_with_context(
            &snapshot,
            &thread_request,
            &context,
            thread_reporter.as_ref(),
        )
    });
    reporter.wait_until_started()?;
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        let pids = fs::read_to_string(&pid_path)?;
        if pids.lines().count() >= 3 || Instant::now() >= deadline {
            break;
        }
        thread::sleep(Duration::from_millis(10));
    }
    let pids = fs::read_to_string(&pid_path)?;
    assert!(pids.lines().count() >= 3);
    let acknowledgement = TaskExecutor::cancel(
        &host,
        &CancellationRequest::new(request.invocation().clone(), 1, "tree cancellation")?,
    )?;
    assert!(acknowledgement.accepted());
    assert!(!acknowledgement.terminal_boundary());
    handle.join().map_err(|_| "execution thread panicked")??;
    assert_eq!(
        terminal_status(&reporter.events()?),
        Some(TerminalStatus::Cancelled)
    );
    for pid in pids.lines() {
        let pid: i32 = pid.parse()?;
        let pid = rustix::process::Pid::from_raw(pid).ok_or("invalid fixture pid")?;
        assert_eq!(
            rustix::process::test_kill_process(pid).err(),
            Some(rustix::io::Errno::SRCH)
        );
    }
    Ok(())
}

#[cfg(unix)]
#[test]
fn child_exit_tears_down_a_descendant_that_keeps_inherited_pipes_open() -> TestResult {
    let data = Arc::new(TestDataAccess::new()?);
    let pid_owner = tempfile::NamedTempFile::new()?;
    let pid_path = pid_owner.path().to_string_lossy().to_string();
    let profile = parse_profile(&profile_value(
        &data.root,
        vec![
            json!("exit-with-pipe-holder"),
            json!(pid_path),
            json!("2000"),
        ],
    )?)?;
    let request = request(&profile, "invocation-pipe-holder", Vec::new())?;
    let (host, snapshot) = setup(profile, data, Arc::new(InMemorySecretResolver::new()))?;
    let reporter = TestReporter::default();
    host.execute_exact_with_context(&snapshot, &request, &context()?, &reporter)?;
    let events = reporter.events()?;
    assert_eq!(terminal_status(&events), Some(TerminalStatus::Failure));
    assert_eq!(
        terminal_failure_code(&events),
        Some("process_descendant_contract_violated")
    );
    let pid: i32 = fs::read_to_string(pid_owner.path())?.trim().parse()?;
    let pid = rustix::process::Pid::from_raw(pid).ok_or("invalid fixture pid")?;
    assert_eq!(
        rustix::process::test_kill_process(pid).err(),
        Some(rustix::io::Errno::SRCH)
    );
    Ok(())
}
