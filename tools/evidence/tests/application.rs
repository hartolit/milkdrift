//! Exercise the shared harness at real child-process boundaries.

use std::{
    io::{Read as _, Write as _},
    net::{TcpListener, TcpStream},
    path::PathBuf,
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};

use milkdrift_evidence::{
    EvidenceResult,
    application::{CliRunner, OwnedChild, ensure, wait_for_run},
};

fn runner(executable: PathBuf, endpoint: String) -> CliRunner {
    CliRunner {
        executable,
        endpoint,
        token_file: PathBuf::from("credential-file-must-not-appear"),
        forbidden_storage_path: PathBuf::from("store-must-not-appear"),
    }
}

#[cfg(unix)]
#[test]
fn fixture_output_failure_exits_without_bypassing_child_cleanup() -> EvidenceResult {
    use std::{
        os::{fd::OwnedFd, unix::net::UnixStream},
        process::{Command, Stdio},
    };
    for (executable, argument) in [
        (env!("CARGO_BIN_EXE_evidence-process-helper"), "emit"),
        (
            env!("CARGO_BIN_EXE_headless-cli-evidence"),
            "--fixture-artifact",
        ),
    ] {
        let (writer, reader) = UnixStream::pair()?;
        drop(reader);
        let mut child = OwnedChild::spawn(
            Command::new(executable)
                .arg(argument)
                .stdin(Stdio::null())
                .stdout(Stdio::from(OwnedFd::from(writer)))
                .stderr(Stdio::null()),
        )?;
        let started = Instant::now();
        let status = loop {
            if let Some(status) = child.try_wait()? {
                break status;
            }
            ensure(
                started.elapsed() < Duration::from_secs(5),
                "fixture ignored failed stdout",
            )?;
            thread::sleep(Duration::from_millis(5));
        };
        child.terminate()?;
        assert_eq!(status.code(), Some(1));
    }
    Ok(())
}

fn accept(listener: &TcpListener) -> EvidenceResult<TcpStream> {
    listener.set_nonblocking(true)?;
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        match listener.accept() {
            Ok((stream, _)) => {
                stream.set_nonblocking(false)?;
                stream.set_read_timeout(Some(Duration::from_secs(4)))?;
                return Ok(stream);
            }
            Err(error)
                if error.kind() == std::io::ErrorKind::WouldBlock && Instant::now() < deadline =>
            {
                thread::sleep(Duration::from_millis(5))
            }
            Err(error) => return Err(error.into()),
        }
    }
}

#[test]
fn enclosing_deadline_kills_and_reaps_stalled_cli_without_restarting_budget() -> EvidenceResult {
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let runner = runner(
        PathBuf::from(env!("CARGO_BIN_EXE_evidence-process-helper")),
        listener.local_addr()?.to_string(),
    );
    let started = Instant::now();
    let deadline = started + Duration::from_secs(2);
    thread::scope(|scope| -> EvidenceResult {
        let command = scope.spawn(|| {
            runner.run_until(
                &[
                    "--timeout-secs",
                    "30",
                    "run",
                    "show",
                    "private-input-must-not-appear",
                ],
                None,
                deadline,
            )
        });
        let mut control = accept(&listener)?;
        let mut pid = [0; 4];
        control.read_exact(&mut pid)?;
        let failure = command
            .join()
            .map_err(|_| "harness thread panicked")?
            .err()
            .ok_or("stalled child succeeded")?
            .to_string();
        assert!(
            failure.contains("CLI run show: child command exceeded its deadline"),
            "{failure}"
        );
        assert!(failure.contains("cleanup=reaped"), "{failure}");
        assert!(failure.contains(&format!("pid={}", u32::from_be_bytes(pid))));
        assert!(!failure.contains("must-not-appear"));
        assert_eq!(
            control.read(&mut [0])?,
            0,
            "owned child still holds its socket"
        );
        Ok(())
    })?;
    assert!(
        started.elapsed() < Duration::from_secs(4),
        "nested command restarted its 30-second budget"
    );
    Ok(())
}

#[test]
fn diagnostic_failures_preserve_primary_error_and_have_one_finite_budget() -> EvidenceResult {
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let runner = runner(
        PathBuf::from(env!("CARGO_BIN_EXE_evidence-process-helper")),
        listener.local_addr()?.to_string(),
    );
    let started = Instant::now();
    thread::scope(|scope| -> EvidenceResult {
        let (finished, completion) = mpsc::sync_channel(1);
        let worker = scope.spawn(move || {
            let result = wait_for_run(
                &runner,
                "private-input-must-not-appear",
                Duration::from_secs(2),
                |_| false,
            );
            finished
                .send(result)
                .map_err(|_| "diagnostic completion receiver disappeared")
        });
        // The primary child and three diagnostic children all stall. Every one must be
        // reaped before the next is started, even though no diagnostic succeeds.
        for _ in 0..4 {
            let mut control = accept(&listener)?;
            control.read_exact(&mut [0; 4])?;
            assert_eq!(
                control.read(&mut [0])?,
                0,
                "failed probe retained its child"
            );
        }
        let failure = completion
            .recv_timeout(Duration::from_secs(3))?
            .err()
            .ok_or("stalled run succeeded")?
            .to_string();
        assert!(
            failure.starts_with("CLI run show: child command exceeded its deadline"),
            "{failure}"
        );
        for identity in ["run show", "run timeline", "daemon health"] {
            assert!(
                failure.contains(&format!("{identity}: unavailable")),
                "{failure}"
            );
        }
        assert!(!failure.contains("must-not-appear"));
        assert!(failure.len() < 4096);
        worker
            .join()
            .map_err(|_| "diagnostic observer panicked")??;
        Ok(())
    })?;
    assert!(
        started.elapsed() < Duration::from_secs(11),
        "diagnostics escaped their six-second allowance"
    );
    Ok(())
}

#[test]
fn successful_capture_reaps_child_after_explicit_release() -> EvidenceResult {
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let runner = runner(
        PathBuf::from(env!("CARGO_BIN_EXE_evidence-process-helper")),
        listener.local_addr()?.to_string(),
    );
    thread::scope(|scope| -> EvidenceResult {
        let command = scope.spawn(|| runner.success(&["run", "show", "case"]));
        let mut control = accept(&listener)?;
        control.read_exact(&mut [0; 4])?;
        control.write_all(&[1])?;
        command.join().map_err(|_| "harness thread panicked")??;
        assert_eq!(control.read(&mut [0])?, 0);
        Ok(())
    })
}

#[test]
fn scenario_assertion_failure_drops_and_reaps_owned_child() -> EvidenceResult {
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let mut control = None;
    let result = (|| -> EvidenceResult {
        let _child = OwnedChild::spawn(
            std::process::Command::new(env!("CARGO_BIN_EXE_evidence-process-helper"))
                .arg("--endpoint")
                .arg(listener.local_addr()?.to_string()),
        )?;
        let mut stream = accept(&listener)?;
        stream.read_exact(&mut [0; 4])?;
        control = Some(stream);
        ensure(false, "original scenario assertion")
    })();
    assert_eq!(
        result
            .err()
            .ok_or("assertion unexpectedly succeeded")?
            .to_string(),
        "original scenario assertion"
    );
    assert_eq!(control.ok_or("child never entered")?.read(&mut [0])?, 0);
    Ok(())
}
