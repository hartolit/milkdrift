//! Actual CLI processes against a scripted public endpoint, with owned hard deadlines.
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
impl Drop for ChildOwner {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

struct Server {
    endpoint: String,
    stop: Arc<AtomicBool>,
    worker: Option<thread::JoinHandle<std::io::Result<()>>>,
}

impl Server {
    fn new(responses: Vec<String>) -> TestResult<Self> {
        let listener = TcpListener::bind("127.0.0.1:0")?;
        listener.set_nonblocking(true)?;
        let endpoint = format!("http://{}/", listener.local_addr()?);
        let stop = Arc::new(AtomicBool::new(false));
        let stopped = stop.clone();
        let worker = thread::spawn(move || {
            let mut responses = responses.into_iter();
            'connections: while !stopped.load(Ordering::SeqCst) {
                let (mut stream, _) = match listener.accept() {
                    Ok(connection) => connection,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(5));
                        continue;
                    }
                    Err(_) => break,
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
                    request.extend_from_slice(&chunk[..count]);
                    if let Some(end) = request.windows(4).position(|part| part == b"\r\n\r\n") {
                        let headers = String::from_utf8_lossy(&request[..end]);
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
                if let Some(response) = responses.next() {
                    let _ = stream.write_all(response.as_bytes());
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
}

impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

fn response(value: Value) -> String {
    http(
        200,
        json!({"protocol":{"major":2,"minor":3},"request_id":"fixture","value":value}).to_string(),
    )
}
fn negotiation() -> String {
    response(json!({"protocol":{"major":2,"minor":3},"service":"milkdrift"}))
}
fn http(status: u16, body: String) -> String {
    format!(
        "HTTP/1.1 {status} Fixture\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
}
fn run_state(terminal: Option<&str>) -> Value {
    json!({"run_id":"run-one","sequence":10,"lifecycle":if terminal.is_some(){"terminal"}else{"running"},"terminal":terminal,"workflow_id":"workflow-one","revision_id":"revision-one","semantic_digest":null,"nodes":[],"uncertainty_count":0})
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
    let deadline = Instant::now() + Duration::from_secs(8);
    let status = loop {
        if let Some(status) = child.0.try_wait()? {
            break status;
        }
        assert!(
            Instant::now() < deadline,
            "CLI exceeded the independent hard deadline"
        );
        thread::sleep(Duration::from_millis(10));
    };
    let output = std::fs::read_to_string(stdout)?;
    let errors = std::fs::read_to_string(stderr)?;
    assert!(!output.contains("private-fixture-token-do-not-echo"));
    let records = output
        .lines()
        .map(|line| {
            assert!(!line.chars().any(char::is_control));
            let value: Value = serde_json::from_str(line)?;
            assert_eq!(value["schema_version"], 2);
            Ok(value)
        })
        .collect::<TestResult<Vec<_>>>()?;
    Ok((status.code().ok_or("exit code absent")?, records, errors))
}

#[test]
fn malformed_arguments_and_missing_bounds_use_stdout_only() -> TestResult {
    for arguments in [
        vec!["run", "start"],
        vec!["run", "wait", "run-one"],
        vec!["capability", "list", "--follow"],
        vec!["blueprint", "show", "revision-one", "--document"],
    ] {
        let (exit, records, stderr) = invoke("http://127.0.0.1:1/", &arguments, false)?;
        assert_eq!(exit, 2);
        assert!(stderr.is_empty());
        assert_eq!(records.len(), 1);
        assert_eq!(records[0]["status"], "failure");
        assert_eq!(records[0]["error"]["code"], "invalid_input");
        assert_eq!(records[0]["final"], true);
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
        assert_eq!(records[0]["type"], kind);
        assert_eq!(records[0]["final"], true);
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
        let (actual, records, stderr) = invoke(
            &server.endpoint,
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
        assert_eq!(records[0]["type"], "run.wait");
        assert_eq!(records[0]["value"]["terminal"], terminal);
        assert_eq!(records[0]["command_id"], "command-fixture");
    }
    let server = Server::new(vec![negotiation(), response(run_state(None))])?;
    let (exit, records, _) = invoke(
        &server.endpoint,
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
    assert_eq!(records[0]["error"]["retryable"], Value::Null);
    Ok(())
}

#[test]
fn deadline_includes_negotiation_and_blocked_document_input() -> TestResult {
    let server = Server::new(vec![])?;
    let (exit, records, _) = invoke(
        &server.endpoint,
        &["--timeout-secs", "1", "run", "show", "run-one"],
        false,
    )?;
    assert_eq!(exit, 10);
    assert_eq!(records[0]["error"]["code"], "timeout");
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
    let refusal = json!({"protocol":{"major":2,"minor":3},"request_id":null,"code":"unauthorized","message":"private-fixture-token-do-not-echo","retryable":false,"details":{}});
    let server = Server::new(vec![
        negotiation(),
        response(json!([])),
        http(403, refusal.to_string()),
    ])?;
    let (exit, records, stderr) = invoke(
        &server.endpoint,
        &["--timeout-secs", "3", "capability", "list", "--follow"],
        false,
    )?;
    assert_eq!(exit, 3);
    assert!(stderr.is_empty());
    assert_eq!(records.len(), 2);
    assert_eq!(records[0]["final"], false);
    assert_eq!(records[1]["final"], true);
    assert_eq!(records[1]["error"]["code"], "unauthorized");
    Ok(())
}

#[test]
fn lost_stream_and_malformed_protocol_are_finite() -> TestResult {
    let server = Server::new(vec![negotiation(),response(json!([])),"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_owned()])?;
    let (exit, records, _) = invoke(
        &server.endpoint,
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
    assert_eq!(records.last().ok_or("final record absent")?["final"], true);
    let server = Server::new(vec![
        negotiation(),
        http(200, "not-json-secret-fixture".to_owned()),
    ])?;
    let (exit, records, _) = invoke(&server.endpoint, &["run", "show", "run-one"], false)?;
    assert_eq!(exit, 9);
    assert!(!records[0].to_string().contains("not-json-secret-fixture"));
    Ok(())
}
