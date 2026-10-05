//! Actual CLI processes against a scripted public endpoint, with owned hard deadlines.
use milkdrift_control_protocol::ProtocolVersion;
use serde_json::{Value, json};
use std::{
    fs::File,
    io::{Read as _, Write as _},
    net::TcpListener,
    process::{Child, Command, Stdio},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant},
};
type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

struct ChildOwner(Child);
impl ChildOwner {
    fn wait_until(&mut self, maximum: Duration) -> std::io::Result<std::process::ExitStatus> {
        let started = Instant::now();
        loop {
            if let Some(status) = self.0.try_wait()? {
                return Ok(status);
            }
            if started.elapsed() >= maximum {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::TimedOut,
                    "CLI exceeded the independent hard deadline",
                ));
            }
            thread::sleep(Duration::from_millis(10));
        }
    }

    fn finish(&mut self) -> std::io::Result<()> {
        if self.0.try_wait()?.is_none() {
            self.0.kill()?;
        }
        self.wait_until(Duration::from_secs(5))?;
        Ok(())
    }
}

impl Drop for ChildOwner {
    fn drop(&mut self) {
        if let Err(error) = self.finish() {
            if thread::panicking() {
                #[expect(
                    clippy::print_stderr,
                    reason = "An unwinding fixture cannot return child cleanup failure or panic again."
                )]
                {
                    eprintln!("CLI fixture child cleanup unconfirmed: {error}");
                }
            } else {
                #[expect(
                    clippy::panic,
                    reason = "A test must fail when its owned child cannot be reaped; ordinary paths call finish explicitly."
                )]
                {
                    panic!("CLI fixture child cleanup unconfirmed: {error}");
                }
            }
        }
    }
}

struct Server {
    endpoint: String,
    stop: Arc<AtomicBool>,
    worker: Option<thread::JoinHandle<std::io::Result<()>>>,
}

impl Server {
    fn new(responses: Vec<String>) -> TestResult<Self> {
        Self::with_response_hook(responses, |_| Ok(()))
    }

    fn with_response_hook(
        responses: Vec<String>,
        mut before_response: impl FnMut(usize) -> std::io::Result<()> + Send + 'static,
    ) -> TestResult<Self> {
        let listener = TcpListener::bind("127.0.0.1:0")?;
        listener.set_nonblocking(true)?;
        let endpoint = format!("http://{}/", listener.local_addr()?);
        let stop = Arc::new(AtomicBool::new(false));
        let stopped = stop.clone();
        let worker = thread::spawn(move || {
            let mut responses = responses.into_iter();
            let mut response_index = 0;
            'connections: while !stopped.load(Ordering::SeqCst) {
                let (mut stream, _) = match listener.accept() {
                    Ok(connection) => connection,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(5));
                        continue;
                    }
                    Err(error) => return Err(error),
                };
                stream.set_nonblocking(false)?;
                stream.set_read_timeout(Some(Duration::from_secs(1)))?;
                stream.set_write_timeout(Some(Duration::from_secs(1)))?;
                let mut request = Vec::new();
                loop {
                    let mut chunk = [0; 4096];
                    let count = match stream.read(&mut chunk) {
                        Ok(0) | Err(_) => continue 'connections,
                        Ok(count) => count,
                    };
                    if request.len() + count > 8192 {
                        return Err(std::io::Error::other("incomplete fixture request"));
                    }
                    request.extend_from_slice(
                        chunk
                            .get(..count)
                            .ok_or_else(|| std::io::Error::other("read exceeds buffer"))?,
                    );
                    if let Some(end) = request.windows(4).position(|part| part == b"\r\n\r\n") {
                        let headers = String::from_utf8_lossy(
                            request
                                .get(..end)
                                .ok_or_else(|| std::io::Error::other("header exceeds request"))?,
                        );
                        let length = headers
                            .lines()
                            .find_map(|line| {
                                line.split_once(':')
                                    .filter(|(key, _)| key.eq_ignore_ascii_case("content-length"))
                                    .and_then(|(_, value)| value.trim().parse::<usize>().ok())
                            })
                            .unwrap_or(0);
                        if request.len() >= end + 4 + length {
                            break;
                        }
                    }
                }
                before_response(response_index)?;
                response_index += 1;
                if let Some(response) = responses.next() {
                    stream.write_all(response.as_bytes())?;
                } else {
                    // A deliberately stalled connection proves the caller's overall bound.
                    while !stopped.load(Ordering::SeqCst) {
                        thread::sleep(Duration::from_millis(5));
                    }
                }
            }
            Ok(())
        });
        Ok(Self {
            endpoint,
            stop,
            worker: Some(worker),
        })
    }

    fn finish(&mut self) -> std::io::Result<()> {
        self.stop.store(true, Ordering::SeqCst);
        let started = Instant::now();
        while self
            .worker
            .as_ref()
            .is_some_and(|worker| !worker.is_finished())
        {
            if started.elapsed() >= Duration::from_secs(3) {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::TimedOut,
                    "CLI fixture server cleanup unconfirmed",
                ));
            }
            thread::sleep(Duration::from_millis(5));
        }
        if let Some(worker) = self.worker.take() {
            worker
                .join()
                .map_err(|_| std::io::Error::other("CLI fixture server panicked"))??;
        }
        Ok(())
    }

    fn invoke(
        mut self,
        arguments: &[&str],
        keep_stdin_open: bool,
    ) -> TestResult<(i32, Vec<Value>, String)> {
        let outcome = invoke(&self.endpoint, arguments, keep_stdin_open);
        match (outcome, self.finish()) {
            (Ok(value), Ok(())) => Ok(value),
            (Err(error), Ok(())) => Err(error),
            (Ok(_), Err(cleanup)) => Err(cleanup.into()),
            (Err(error), Err(cleanup)) => Err(format!("{error}; server cleanup: {cleanup}").into()),
        }
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        if let Err(error) = self.finish() {
            if thread::panicking() {
                #[expect(
                    clippy::print_stderr,
                    reason = "An unwinding fixture cannot return server cleanup failure or panic again."
                )]
                {
                    eprintln!("CLI fixture server cleanup unconfirmed: {error}");
                }
            } else {
                #[expect(
                    clippy::panic,
                    reason = "A test must fail when its owned server fails cleanup; invoke finishes normal paths explicitly."
                )]
                {
                    panic!("CLI fixture server cleanup unconfirmed: {error}");
                }
            }
        }
    }
}

fn response(value: Value) -> String {
    http(
        200,
        json!({"protocol":ProtocolVersion::CURRENT,"request_id":"fixture","value":value})
            .to_string(),
    )
}
fn negotiation() -> String {
    response(json!({"protocol":ProtocolVersion::CURRENT,"service":"milkdrift"}))
}
fn http(status: u16, body: String) -> String {
    format!(
        "HTTP/1.1 {status} Fixture\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
}
fn run_state(terminal: Option<&str>) -> Value {
    let lifecycle = if terminal.is_some() {
        "terminal"
    } else {
        "running"
    };
    json!({
        "run_id": "run-one",
        "sequence": 10,
        "lifecycle": lifecycle,
        "terminal": terminal,
        "workflow_id": "workflow-one",
        "revision_id": "revision-one",
        "semantic_digest": null,
        "nodes": [],
        "governing_agreement": null, "agreement_adoptions": 0, "uncertainty_count": 0
    })
}

fn invoke(
    endpoint: &str,
    arguments: &[&str],
    keep_stdin_open: bool,
) -> TestResult<(i32, Vec<Value>, String)> {
    let root = tempfile::tempdir()?;
    let stdout = root.path().join("stdout");
    let stderr = root.path().join("stderr");
    let mut child = ChildOwner(
        Command::new(env!("CARGO_BIN_EXE_milkdrift"))
            .env_remove("MILKDRIFT_TOKEN_FILE")
            .env_remove("MILKDRIFT_TOKEN_ENV")
            .env("MILKDRIFT_TOKEN", "private-fixture-token-do-not-echo")
            .args([
                "--endpoint",
                endpoint,
                "--json",
                "--command-id",
                "command-fixture",
            ])
            .args(arguments)
            .stdin(if keep_stdin_open {
                Stdio::piped()
            } else {
                Stdio::null()
            })
            .stdout(File::create(&stdout)?)
            .stderr(File::create(&stderr)?)
            .spawn()?,
    );
    let status = match (child.wait_until(Duration::from_secs(8)), child.finish()) {
        (Ok(status), Ok(())) => status,
        (Err(error), Ok(())) => return Err(error.into()),
        (Ok(_), Err(cleanup)) => return Err(cleanup.into()),
        (Err(error), Err(cleanup)) => {
            return Err(format!("{error}; child cleanup: {cleanup}").into());
        }
    };
    let output = std::fs::read_to_string(stdout)?;
    let errors = std::fs::read_to_string(stderr)?;
    assert!(!output.contains("private-fixture-token-do-not-echo"));
    let records = output
        .lines()
        .map(|line| {
            assert!(!line.chars().any(char::is_control));
            let value: Value = serde_json::from_str(line)?;
            assert_eq!(
                value
                    .pointer("/schema_version")
                    .ok_or("missing fixture field")?,
                2
            );
            Ok(value)
        })
        .collect::<TestResult<Vec<_>>>()?;
    Ok((status.code().ok_or("exit code absent")?, records, errors))
}

#[test]
fn server_finish_returns_worker_failure() -> TestResult {
    let mut server = Server {
        endpoint: "unused".into(),
        stop: Arc::new(AtomicBool::new(false)),
        worker: Some(thread::spawn(|| {
            Err(std::io::Error::other("injected response failure"))
        })),
    };
    let failure = server
        .finish()
        .err()
        .ok_or("worker failure was discarded")?;
    assert!(failure.to_string().contains("injected response failure"));
    assert!(server.worker.is_none());
    Ok(())
}

#[cfg(unix)]
#[test]
fn child_finish_stops_and_reaps_a_running_process() -> TestResult {
    let mut child = ChildOwner(Command::new("/bin/sleep").arg("30").spawn()?);
    assert!(child.0.try_wait()?.is_none());
    child.finish()?;
    assert!(child.0.try_wait()?.is_some());
    child.finish()?;
    Ok(())
}

#[cfg(unix)]
#[test]
fn closed_stdout_returns_internal_exit_for_success_and_failure_documents() -> TestResult {
    use std::os::{fd::OwnedFd, unix::net::UnixStream};
    for arguments in [["--json", "--help"], ["--json", "not-a-command"]] {
        let (writer, reader) = UnixStream::pair()?;
        drop(reader);
        let mut child = ChildOwner(
            Command::new(env!("CARGO_BIN_EXE_milkdrift"))
                .args(arguments)
                .stdin(Stdio::null())
                .stdout(Stdio::from(OwnedFd::from(writer)))
                .stderr(Stdio::null())
                .spawn()?,
        );
        let status = child.wait_until(Duration::from_secs(8))?;
        child.finish()?;
        assert_eq!(status.code(), Some(9));
    }
    Ok(())
}

#[test]
fn malformed_arguments_and_missing_bounds_use_stdout_only() -> TestResult {
    for arguments in [
        vec!["run", "start"],
        vec!["run", "wait", "run-one"],
        vec!["run", "reconnect", "retained.json", "--wait"],
        vec![
            "run",
            "start",
            "run-one",
            "workflow",
            "revision",
            "--request-file",
            "retained.json",
            "--wait",
        ],
        vec!["capability", "list", "--follow"],
        vec!["blueprint", "show", "revision-one", "--document"],
    ] {
        let (exit, records, stderr) = invoke("http://127.0.0.1:1/", &arguments, false)?;
        assert_eq!(exit, 2);
        assert!(stderr.is_empty());
        assert_eq!(records.len(), 1);
        assert_eq!(
            records
                .first()
                .ok_or("missing fixture record")?
                .pointer("/status")
                .ok_or("missing fixture field")?,
            "failure"
        );
        assert_eq!(
            records
                .first()
                .ok_or("missing fixture record")?
                .pointer("/error/code")
                .ok_or("missing fixture field")?,
            "invalid_input"
        );
        assert_eq!(
            records
                .first()
                .ok_or("missing fixture record")?
                .pointer("/final")
                .ok_or("missing fixture field")?,
            true
        );
    }
    Ok(())
}

#[test]
fn help_and_version_are_json_without_a_session() -> TestResult {
    for (argument, kind) in [("--help", "help"), ("--version", "version")] {
        let (exit, records, stderr) = invoke("http://127.0.0.1:1/", &[argument], false)?;
        assert_eq!(exit, 0);
        assert!(stderr.is_empty());
        assert_eq!(records.len(), 1);
        assert_eq!(
            records
                .first()
                .ok_or("missing fixture record")?
                .pointer("/type")
                .ok_or("missing fixture field")?,
            kind
        );
        assert_eq!(
            records
                .first()
                .ok_or("missing fixture record")?
                .pointer("/final")
                .ok_or("missing fixture field")?,
            true
        );
    }
    Ok(())
}

#[test]
fn wait_has_typed_terminal_results_and_bounded_polling() -> TestResult {
    for (terminal, exit) in [("succeeded", 0), ("failed", 8), ("cancelled", 8)] {
        let server = Server::new(vec![
            negotiation(),
            response(run_state(None)),
            response(run_state(Some(terminal))),
        ])?;
        let (actual, records, stderr) = server.invoke(
            &[
                "--timeout-secs",
                "3",
                "run",
                "wait",
                "run-one",
                "--poll-ms",
                "10",
            ],
            false,
        )?;
        assert_eq!(actual, exit);
        assert!(stderr.is_empty());
        assert_eq!(records.len(), 1);
        assert_eq!(
            records
                .first()
                .ok_or("missing fixture record")?
                .pointer("/type")
                .ok_or("missing fixture field")?,
            "run.wait"
        );
        assert_eq!(
            records
                .first()
                .ok_or("missing fixture record")?
                .pointer("/value/terminal")
                .ok_or("missing fixture field")?,
            terminal
        );
        assert_eq!(
            records
                .first()
                .ok_or("missing fixture record")?
                .pointer("/command_id")
                .ok_or("missing fixture field")?,
            "command-fixture"
        );
    }
    let server = Server::new(vec![negotiation(), response(run_state(None))])?;
    let (exit, records, _) = server.invoke(
        &[
            "--timeout-secs",
            "3",
            "run",
            "wait",
            "run-one",
            "--max-polls",
            "1",
        ],
        false,
    )?;
    assert_eq!(exit, 10);
    assert_eq!(
        records
            .first()
            .ok_or("missing fixture record")?
            .pointer("/error/retryable")
            .ok_or("missing fixture field")?,
        &Value::Null
    );
    Ok(())
}

#[test]
fn deadline_includes_negotiation_and_blocked_document_input() -> TestResult {
    let server = Server::new(vec![])?;
    let (exit, records, _) =
        server.invoke(&["--timeout-secs", "1", "run", "show", "run-one"], false)?;
    assert_eq!(exit, 10);
    assert_eq!(
        records
            .first()
            .ok_or("missing fixture record")?
            .pointer("/error/code")
            .ok_or("missing fixture field")?,
        "timeout"
    );
    let root = tempfile::tempdir()?;
    let output = root.path().join("blueprint.json");
    let (exit, records, _) = invoke(
        "http://127.0.0.1:1/",
        &[
            "--timeout-secs",
            "1",
            "sequence",
            "compile",
            "-",
            "--author",
            "human:fixture",
            "--output",
            output.to_str().ok_or("output path is not UTF-8")?,
        ],
        true,
    )?;
    assert_eq!(exit, 10);
    assert_eq!(records.len(), 1);
    assert!(!output.exists());
    Ok(())
}

#[test]
fn authorization_loss_ends_json_lines_with_one_redacted_final_record() -> TestResult {
    let refusal = json!({"protocol":ProtocolVersion::CURRENT,"request_id":null,"code":"unauthorized","message":"private-fixture-token-do-not-echo","retryable":false,"details":{}});
    let server = Server::new(vec![
        negotiation(),
        response(json!([])),
        http(403, refusal.to_string()),
    ])?;
    let (exit, records, stderr) = server.invoke(
        &["--timeout-secs", "3", "capability", "list", "--follow"],
        false,
    )?;
    assert_eq!(exit, 3);
    assert!(stderr.is_empty());
    assert_eq!(records.len(), 2);
    assert_eq!(
        records
            .first()
            .ok_or("missing fixture record")?
            .pointer("/final")
            .ok_or("missing fixture field")?,
        false
    );
    assert_eq!(
        records
            .get(1)
            .ok_or("missing fixture record")?
            .pointer("/final")
            .ok_or("missing fixture field")?,
        true
    );
    assert_eq!(
        records
            .get(1)
            .ok_or("missing fixture record")?
            .pointer("/error/code")
            .ok_or("missing fixture field")?,
        "unauthorized"
    );
    Ok(())
}

#[test]
fn lost_stream_and_malformed_protocol_are_finite() -> TestResult {
    let server = Server::new(vec![negotiation(),response(json!([])),"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_owned()])?;
    let (exit, records, _) = server.invoke(
        &[
            "--timeout-secs",
            "3",
            "--max-reconnects",
            "0",
            "capability",
            "list",
            "--follow",
        ],
        false,
    )?;
    assert_eq!(exit, 10);
    assert_eq!(
        records
            .last()
            .ok_or("final record absent")?
            .get("final")
            .ok_or("final flag absent")?,
        true
    );
    let server = Server::new(vec![
        negotiation(),
        http(200, "not-json-secret-fixture".to_owned()),
    ])?;
    let (exit, records, _) = server.invoke(&["run", "show", "run-one"], false)?;
    assert_eq!(exit, 9);
    assert!(
        !records
            .first()
            .ok_or("missing fixture record")?
            .to_string()
            .contains("not-json-secret-fixture")
    );
    Ok(())
}

#[test]
fn artifact_download_verifies_ranges_digest_and_preserves_existing_files() -> TestResult {
    let root = tempfile::tempdir()?;
    let path = root.path().join("result.txt");
    let bytes = "complete notes";
    let metadata = json!({"artifact_id":"notes", "digest":blake3::hash(bytes.as_bytes()).to_hex().to_string(), "size":bytes.len(), "content_type":"text/plain", "disposition_name":null, "sensitivity":"restricted"});
    for (body, range, succeeds) in [
        (bytes, "bytes 0-13/14", true),
        ("changed notes!", "bytes 0-13/14", false),
        ("short", "bytes 0-13/14", false),
        (bytes, "bytes 1-14/15", false),
        (bytes, "invalid", false),
    ] {
        let range_response = format!(
            "HTTP/1.1 206 Partial Content\r\nContent-Type: text/plain\r\nContent-Range: {range}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        let server = Server::new(vec![
            negotiation(),
            response(metadata.clone()),
            range_response,
        ])?;
        let (exit, _, _) = server.invoke(
            &[
                "artifact",
                "get",
                "notes",
                "--output",
                path.to_str().ok_or("path")?,
            ],
            false,
        )?;
        assert_eq!(exit == 0, succeeds);
        if succeeds {
            assert_eq!(std::fs::read_to_string(&path)?, bytes);
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt as _;
                assert_eq!(
                    std::fs::metadata(&path)?.permissions().mode() & 0o777,
                    0o600
                );
            }
            std::fs::remove_file(&path)?;
        } else {
            assert!(!path.exists());
        }
    }
    std::fs::write(&path, "keep")?;
    let server = Server::new(vec![negotiation(), response(metadata)])?;
    let (exit, _, _) = server.invoke(
        &[
            "artifact",
            "get",
            "notes",
            "--output",
            path.to_str().ok_or("path")?,
        ],
        false,
    )?;
    assert_ne!(exit, 0);
    assert_eq!(std::fs::read_to_string(&path)?, "keep");
    Ok(())
}

#[test]
fn result_download_reports_one_final_outcome_after_verification() -> TestResult {
    let root = tempfile::tempdir()?;
    let path = root.path().join("notes.txt");
    let metadata = json!({"artifact_id":"notes", "digest":blake3::hash(b"notes").to_hex().to_string(), "size":5, "content_type":"text/plain", "disposition_name":null, "sensitivity":"restricted"});
    let result = json!({"run":run_state(Some("succeeded")), "workflow_name":"Release notes", "version":1, "truncated":false, "outputs":[{"name":"notes", "artifact":metadata, "preview":"notes", "preview_truncated":false}], "outputs_restricted":false, "actions":[]});
    for (body, succeeds) in [("notes", true), ("wrong", false)] {
        let server = Server::new(vec![
            negotiation(),
            response(result.clone()),
            response(metadata.clone()),
            format!(
                "HTTP/1.1 206 Partial Content\r\nContent-Type: text/plain\r\nContent-Range: bytes 0-4/5\r\nContent-Length: 5\r\nConnection: close\r\n\r\n{body}"
            ),
        ])?;
        let (exit, records, _) = server.invoke(
            &[
                "run",
                "result",
                "run-one",
                "--field",
                "notes",
                "--output",
                path.to_str().ok_or("path")?,
            ],
            false,
        )?;
        assert_eq!(exit == 0, succeeds, "{records:?}");
        assert_eq!(records.len(), 1, "{records:?}");
        assert_eq!(
            records
                .first()
                .ok_or("missing fixture record")?
                .pointer("/final")
                .ok_or("missing fixture field")?,
            true
        );
        assert_eq!(
            records
                .first()
                .ok_or("missing fixture record")?
                .pointer("/type")
                .ok_or("missing fixture field")?,
            "run.result"
        );
        assert_eq!(path.exists(), succeeds);
        if succeeds {
            assert_eq!(
                records
                    .first()
                    .ok_or("missing fixture record")?
                    .pointer("/value/download/digest")
                    .ok_or("missing fixture field")?,
                metadata.pointer("/digest").ok_or("missing fixture field")?
            );
            assert_eq!(std::fs::read_to_string(&path)?, body);
            std::fs::remove_file(&path)?;
        }
    }
    Ok(())
}

#[test]
fn artifact_download_never_removes_a_competing_destination() -> TestResult {
    for replacement in ["file", "directory", "symlink"] {
        #[cfg(not(unix))]
        if replacement == "symlink" {
            continue;
        }
        for outcome in ["complete", "digest", "truncated", "timeout"] {
            let root = tempfile::tempdir()?;
            let path = root.path().join("result");
            let target = root.path().join("unrelated");
            std::fs::write(&target, b"unrelated writer")?;
            let bytes = "complete notes";
            let metadata = json!({"artifact_id":"notes", "digest":blake3::hash(bytes.as_bytes()).to_hex().to_string(), "size":bytes.len(), "content_type":"text/plain", "disposition_name":null, "sensitivity":"restricted"});
            let body = match outcome {
                "digest" => "changed notes!",
                "truncated" => "short",
                _ => bytes,
            };
            let mut responses = vec![negotiation(), response(metadata)];
            if outcome != "timeout" {
                responses.push(format!("HTTP/1.1 206 Partial Content\r\nContent-Type: text/plain\r\nContent-Range: bytes 0-13/14\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()));
            }
            let destination = path.clone();
            #[cfg(unix)]
            let other = target.clone();
            let server = Server::with_response_hook(responses, move |index| {
                if index == 2 {
                    // The content request follows staging creation. Replace the
                    // final name before sending bytes or letting the deadline fire.
                    assert!(!destination.exists());
                    match replacement {
                        "file" => std::fs::write(&destination, b"unrelated writer")?,
                        "directory" => {
                            std::fs::create_dir(&destination)?;
                            std::fs::write(destination.join("keep"), b"unrelated writer")?;
                        }
                        #[cfg(unix)]
                        "symlink" => std::os::unix::fs::symlink(&other, &destination)?,
                        _ => return Err(std::io::Error::other("unknown fixture replacement")),
                    }
                }
                Ok(())
            })?;
            let (exit, records, _) = server.invoke(
                &[
                    "--timeout-secs",
                    "1",
                    "artifact",
                    "get",
                    "notes",
                    "--output",
                    path.to_str().ok_or("path")?,
                ],
                false,
            )?;
            assert_ne!(
                exit, 0,
                "{replacement}/{outcome} reported successful publication"
            );
            assert_eq!(
                records
                    .iter()
                    .filter(|record| record.get("final") == Some(&Value::Bool(true)))
                    .count(),
                1
            );
            let data = if replacement == "directory" {
                path.join("keep")
            } else {
                path.clone()
            };
            assert_eq!(std::fs::read(data)?, b"unrelated writer");
            assert_eq!(std::fs::read(target)?, b"unrelated writer");
            if replacement == "symlink" {
                assert!(std::fs::symlink_metadata(&path)?.file_type().is_symlink());
            }
            assert_eq!(
                std::fs::read_dir(root.path())?.count(),
                2,
                "staging remained after {outcome}"
            );
        }
    }
    Ok(())
}

#[test]
fn stream_duplicates_and_expired_cursors_use_a_fresh_authorized_view() -> TestResult {
    use milkdrift_control_protocol::Cursor;
    let event = |position, observation: Value| -> TestResult<String> {
        Ok(format!(
            "data: {}\n\n",
            json!({"protocol":ProtocolVersion::CURRENT,"cursor":Cursor::new("run:run-one",position)?,"feed":"run:run-one","observed_at_ms":1,"observation":observation})
        ))
    };
    let status = json!({"type":"run_status","value":run_state(None)});
    let sse = |body: String| {
        format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        )
    };
    let body = [
        event(10, status.clone())?,
        event(10, status.clone())?,
        event(8, status.clone())?,
        event(12, status.clone())?,
        event(
            18,
            json!({"type":"resync_required","value":{"reason":"cursor expired"}}),
        )?,
    ]
    .concat();
    let fresh = json!({"run":run_state(None),"workflow_name":"Example","version":1,"truncated":false,"outputs":[],"outputs_restricted":false,"actions":["pause"]});
    let next = [
        event(20, status)?,
        event(
            21,
            json!({"type":"stream_closing","value":{"reason":"authorization changed"}}),
        )?,
    ]
    .concat();
    let server = Server::new(vec![
        negotiation(),
        response(json!({"items":[],"next_cursor":null,"observed_cursor":null})),
        sse(body),
        response(fresh),
        sse(next),
    ])?;
    let (exit, records, _) = server.invoke(
        &[
            "--timeout-secs",
            "5",
            "run",
            "timeline",
            "run-one",
            "--follow",
        ],
        false,
    )?;
    assert_ne!(exit, 0);
    let observations = records
        .iter()
        .filter(|r| r.get("type").and_then(Value::as_str) == Some("run.observation"))
        .collect::<Vec<_>>();
    assert_eq!(observations.len(), 5, "{records:?}");
    assert_eq!(
        records
            .iter()
            .filter(|r| r.get("type").and_then(Value::as_str) == Some("run.result.fresh"))
            .count(),
        1
    );
    assert_eq!(
        observations
            .get(2)
            .ok_or("missing fixture record")?
            .pointer("/value/observation/type")
            .ok_or("missing fixture field")?,
        "resync_required"
    );
    assert_eq!(
        observations
            .get(4)
            .ok_or("missing fixture record")?
            .pointer("/value/observation/type")
            .ok_or("missing fixture field")?,
        "stream_closing"
    );
    Ok(())
}

#[test]
fn cli_refuses_mismatched_negotiation_success_and_error_versions() -> TestResult {
    for minor in [
        ProtocolVersion::CURRENT.minor - 1,
        ProtocolVersion::CURRENT.minor + 1,
    ] {
        let protocol = ProtocolVersion {
            major: ProtocolVersion::CURRENT.major,
            minor,
        };
        for replies in [
            vec![response(json!({"protocol": protocol, "service":"milkdrift"}))],
            vec![negotiation(), http(200, json!({"protocol": protocol, "request_id":"fixture", "value":run_state(None)}).to_string())],
            vec![negotiation(), http(403, json!({"protocol": protocol, "request_id":null, "code":"unauthorized", "message":"unsupported-error-fixture", "retryable":false, "details":{}}).to_string())],
        ] {
            let server = Server::new(replies)?;
            let (exit, records, stderr) = server.invoke(&["run", "show", "run-one"], false)?;
            assert_eq!(exit, 9);
            assert!(stderr.is_empty());
            assert_eq!(records.len(), 1);
            assert_eq!(records.first().ok_or("missing fixture record")?.pointer("/final").ok_or("missing fixture field")?, true);
            assert!(!records.first().ok_or("missing fixture record")?.to_string().contains("unsupported-error-fixture"));
        }
    }
    Ok(())
}

#[test]
fn prepared_direct_requests_preserve_the_servers_exact_document() -> TestResult {
    use milkdrift_capability::{
        AdmissionConstraints, BoundedJson, CancellationBehavior, CapabilityCategory, CapabilityId,
        DescriptorBuilder, IdempotencyBehavior, IdempotencyKey, InvocationId, InvocationRequest,
        Locality, OperationContract, OperationId, PeerId, ResolvedCapabilitySnapshot,
        SchemaContract, SchemaId, SideEffectClass, StreamingMode,
    };
    use milkdrift_peer_protocol::{
        CatalogDigest, DirectInvocationRequest, ExecutionLimits, PeerRequestId,
    };
    use std::collections::{BTreeMap, BTreeSet};
    for (effect, idempotency, expected_key) in [
        (
            SideEffectClass::IdempotentWrite,
            IdempotencyBehavior::CapabilityScoped,
            Some("stable-request"),
        ),
        (
            SideEffectClass::ReadOnly,
            IdempotencyBehavior::Unsupported,
            None,
        ),
        (
            SideEffectClass::NonIdempotentWrite,
            IdempotencyBehavior::Unsupported,
            None,
        ),
    ] {
        let operation = OperationId::new("test.execute")?;
        let capability = CapabilityId::new("test-capability")?;
        let schema = SchemaContract::new(
            SchemaId::new("test.input")?,
            1,
            BoundedJson::new(json!({"type":"object"}))?,
        )?;
        let descriptor = DescriptorBuilder::new(
            capability.clone(),
            1,
            CapabilityCategory::Tool,
            AdmissionConstraints::new(1, 0)?,
            Locality::Local,
        )
        .operations(BTreeMap::from([(
            operation.clone(),
            OperationContract::new(
                schema.clone(),
                schema,
                BTreeSet::from([StreamingMode::None]),
                CancellationBehavior::Unsupported,
                idempotency,
                effect,
                BTreeMap::new(),
            )?,
        )]))
        .build()?;
        let returned = DirectInvocationRequest {
            host: PeerId::new("host-test")?,
            request_id: PeerRequestId::new("stable-request")?,
            catalog_generation: 19,
            catalog_digest: CatalogDigest::new(format!("b3_{}", "1".repeat(64)))?,
            selection: ResolvedCapabilitySnapshot::from_descriptor(&descriptor, &operation)?,
            request: InvocationRequest::new(
                InvocationId::new("stable-request")?,
                capability,
                operation,
                None,
                expected_key.map(IdempotencyKey::new).transpose()?,
                vec![],
                BTreeMap::new(),
            )?,
            // Intentionally unrelated to the client clock: the CLI must not renew it.
            deadline_unix_ms: 12345,
            limits: ExecutionLimits {
                nested_invocations: None,
                artifact_bytes: 4096,
                duration_ms: 1000,
                cost_micros: 0,
                cost_currency: None,
                input_units: None,
                output_units: None,
                observations: 16,
            },
        };
        let server = Server::new(vec![
            negotiation(),
            response(serde_json::to_value(&returned)?),
        ])?;
        let directory = tempfile::tempdir()?;
        let inputs = directory.path().join("inputs.json");
        let output = directory.path().join("request.json");
        std::fs::write(&inputs, b"[]")?;
        let (exit, records, errors) = server.invoke(
            &[
                "invocation",
                "prepare",
                "test-capability",
                "test.execute",
                "--host",
                "host-test",
                "--request-id",
                "stable-request",
                "--inputs",
                inputs.to_str().ok_or("input path")?,
                "--output",
                output.to_str().ok_or("output path")?,
            ],
            false,
        )?;
        assert_eq!(exit, 0, "{records:?}: {errors}");
        let prepared: DirectInvocationRequest = serde_json::from_slice(&std::fs::read(output)?)?;
        assert_eq!(prepared, returned);
        prepared.selection.validate_request(&prepared.request)?;
        assert_eq!(
            prepared.request.idempotency_key().map(|key| key.as_str()),
            expected_key
        );
    }
    Ok(())
}
