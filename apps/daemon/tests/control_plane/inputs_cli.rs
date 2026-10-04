//! Real CLI and daemon processes recover retained requests across client and daemon exits.
use super::{
    binary::{BinaryDaemon, ChildOwner},
    inputs::{ModelFixture, workflow},
    support::*,
};
use serde_json::Value;
use std::{path::Path, process::Stdio};

async fn cli(
    endpoint: &Url,
    directory: &Path,
    arguments: &[&str],
) -> TestResult<std::process::Output> {
    let executable = std::env::current_exe()?
        .parent()
        .and_then(|path| path.parent())
        .ok_or("binary directory")?
        .join(format!("milkdrift{}", std::env::consts::EXE_SUFFIX));
    let endpoint = endpoint.to_string();
    let directory = directory.to_owned();
    let arguments: Vec<String> = arguments.iter().map(|value| (*value).into()).collect();
    let seconds: u64 = arguments
        .windows(2)
        .find(|pair| pair[0] == "--timeout-secs")
        .map(|pair| pair[1].parse())
        .transpose()?
        .unwrap_or(10);
    let result = tokio::task::spawn_blocking(move || -> Result<std::process::Output, String> {
        let mut command = std::process::Command::new(executable);
        command
            .args(["--endpoint", &endpoint, "--token-file"])
            .arg(directory.join("controller.token"))
            .arg("--json");
        if !arguments
            .iter()
            .any(|argument| argument == "--timeout-secs")
        {
            command.args(["--timeout-secs", "10"]);
        }
        let mut child = ChildOwner(Some(
            command
                .args(arguments)
                .current_dir(directory)
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .map_err(|error| error.to_string())?,
        ));
        for _ in 0..(seconds + 5) * 20 {
            if child
                .0
                .as_mut()
                .ok_or("child absent")?
                .try_wait()
                .map_err(|error| error.to_string())?
                .is_some()
            {
                return child
                    .0
                    .take()
                    .ok_or("child absent")?
                    .wait_with_output()
                    .map_err(|error| error.to_string());
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        Err("CLI watchdog expired".into())
    })
    .await?;
    result.map_err(Into::into)
}

fn success(output: &std::process::Output) -> TestResult<Value> {
    assert!(
        output.status.success(),
        "{} {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    Ok(serde_json::from_slice::<Value>(&output.stdout)?["value"].clone())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn actual_cli_retains_inputs_and_reconnects_after_client_and_daemon_exit() -> TestResult {
    let directory = tempfile::tempdir()?;
    let model = ModelFixture::start().await?;
    let (daemon, path) = BinaryDaemon::configured(&directory, &model).await?;
    let endpoint = daemon.endpoint.clone();
    let revision = workflow(&daemon.client).await?;
    let brief = include_bytes!("../../../../examples/operator/release-notes/harbor-brief.txt");
    fs::write(directory.path().join("brief.txt"), brief)?;
    let prepared = success(
        &cli(
            &daemon.endpoint,
            directory.path(),
            &[
                "--command-id",
                "retained-start",
                "run",
                "start",
                "retained",
                "release-notes",
                &revision,
                "--input",
                "brief=brief.txt",
                "--request-file",
                "retained.json",
                "--prepare-only",
            ],
        )
        .await?,
    )?;
    assert_eq!(prepared["command_id"], "retained-start");
    assert_eq!(prepared["host"], "host:local");
    assert!(
        model
            .requests
            .lock()
            .map_err(|_| "fixture lock")?
            .is_empty()
    );
    assert!(daemon.client.run("retained").await.is_err());
    let retained = fs::read(directory.path().join("retained.json"))?;
    let text = std::str::from_utf8(&retained)?;
    assert!(!text.contains(CONTROLLER_TOKEN));
    assert!(!text.contains("brief.txt"));
    assert!(!text.contains("Harbor Host"));
    fs::write(
        directory.path().join("brief.txt"),
        "CHANGED AFTER PREPARATION",
    )?;
    let accepted = success(
        &cli(
            &daemon.endpoint,
            directory.path(),
            &["run", "reconnect", "retained.json"],
        )
        .await?,
    )?;
    assert_eq!(accepted["command_id"], "retained-start");
    wait_for_run(
        &daemon.client,
        "retained",
        Duration::from_secs(45),
        |state| state.terminal.as_deref() == Some("succeeded"),
    )
    .await?;
    let replay = success(
        &cli(
            &daemon.endpoint,
            directory.path(),
            &["run", "reconnect", "retained.json"],
        )
        .await?,
    )?;
    assert_eq!(replay["replayed"], true);
    assert_eq!(fs::read(directory.path().join("retained.json"))?, retained);
    {
        let requests = model.requests.lock().map_err(|_| "fixture lock")?;
        assert_eq!(requests.len(), 2);
        for body in requests.iter() {
            assert!(body.to_string().contains("Harbor Host 1.4"));
            assert!(!body.to_string().contains("CHANGED AFTER PREPARATION"));
        }
    }
    drop(daemon);
    let restarted = BinaryDaemon::start(&path, endpoint, directory.path()).await?;
    assert_eq!(
        success(
            &cli(
                &restarted.endpoint,
                directory.path(),
                &["run", "reconnect", "retained.json"]
            )
            .await?
        )?["replayed"],
        true
    );
    let mut altered: milkdrift_control_client::SavedRunRequest =
        milkdrift_control_protocol::decode_json(&retained)?;
    altered.request.reason = "changed exact request".into();
    fs::write(
        directory.path().join("retained.json"),
        serde_json::to_vec(&altered)?,
    )?;
    let conflict = cli(
        &restarted.endpoint,
        directory.path(),
        &["run", "reconnect", "retained.json"],
    )
    .await?;
    assert_eq!(conflict.status.code(), Some(4));
    assert_eq!(model.requests.lock().map_err(|_| "fixture lock")?.len(), 2);
    Ok(())
}

struct LostReplyProxy {
    endpoint: Url,
    accepted: Arc<std::sync::atomic::AtomicBool>,
    task: JoinHandle<std::io::Result<()>>,
}
impl Drop for LostReplyProxy {
    fn drop(&mut self) {
        self.task.abort();
    }
}
impl LostReplyProxy {
    async fn start(endpoint: Url) -> TestResult<Self> {
        use std::sync::atomic::{AtomicBool, Ordering};
        let accepted = Arc::new(AtomicBool::new(false));
        let observed = accepted.clone();
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(10))
            .build()?;
        let app = axum::Router::new().fallback(move |request: axum::extract::Request| {
            let endpoint = endpoint.clone();
            let http = http.clone();
            let observed = observed.clone();
            async move {
                let (parts, body) = request.into_parts();
                let path = parts.uri.path();
                let is_start = path == "/v1/commands" && parts.method == axum::http::Method::POST;
                let bytes =
                    axum::body::to_bytes(body, milkdrift_control_protocol::MAX_DOCUMENT_BYTES)
                        .await
                        .map_err(|_| axum::http::StatusCode::BAD_REQUEST)?;
                let url = endpoint
                    .join(
                        parts
                            .uri
                            .path_and_query()
                            .map_or(path, |value| value.as_str()),
                    )
                    .map_err(|_| axum::http::StatusCode::BAD_GATEWAY)?;
                let mut headers = parts.headers;
                headers.remove(axum::http::header::HOST);
                let response = http
                    .request(parts.method, url)
                    .headers(headers)
                    .body(bytes)
                    .send()
                    .await
                    .map_err(|_| axum::http::StatusCode::BAD_GATEWAY)?;
                let status = response.status();
                let body = response
                    .bytes()
                    .await
                    .map_err(|_| axum::http::StatusCode::BAD_GATEWAY)?;
                if is_start && status.is_success() && !observed.swap(true, Ordering::SeqCst) {
                    // The daemon's acceptance reply exists, but the first client never receives it.
                    tokio::time::sleep(Duration::from_secs(5)).await;
                }
                Ok::<_, axum::http::StatusCode>((
                    status,
                    [(axum::http::header::CONTENT_TYPE, "application/json")],
                    body,
                ))
            }
        });
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let endpoint = Url::parse(&format!("http://{}/", listener.local_addr()?))?;
        let task = tokio::spawn(async move { axum::serve(listener, app).await });
        Ok(Self {
            endpoint,
            accepted,
            task,
        })
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn actual_cli_lost_start_reply_and_wait_deadline_recover_without_reexecution() -> TestResult {
    let directory = tempfile::tempdir()?;
    let model = ModelFixture::start().await?;
    let (daemon, _) = BinaryDaemon::configured(&directory, &model).await?;
    let revision = workflow(&daemon.client).await?;
    fs::write(
        directory.path().join("brief.txt"),
        include_bytes!("../../../../examples/operator/release-notes/lantern-brief.txt"),
    )?;
    let proxy = LostReplyProxy::start(daemon.endpoint.clone()).await?;
    let lost = cli(
        &proxy.endpoint,
        directory.path(),
        &[
            "--timeout-secs",
            "2",
            "--command-id",
            "lost-start",
            "run",
            "start",
            "lost",
            "release-notes",
            &revision,
            "--input",
            "brief=brief.txt",
            "--request-file",
            "lost.json",
            "--wait",
        ],
    )
    .await?;
    assert_eq!(
        lost.status.code(),
        Some(10),
        "{}",
        String::from_utf8_lossy(&lost.stdout)
    );
    assert!(
        proxy.accepted.load(std::sync::atomic::Ordering::SeqCst),
        "client left before daemon accepted start"
    );
    let records: Vec<Value> = std::str::from_utf8(&lost.stdout)?
        .lines()
        .map(serde_json::from_str)
        .collect::<Result<_, _>>()?;
    assert_eq!(records.len(), 2);
    assert_eq!(records[0]["type"], "run.prepared");
    assert_eq!(records[0]["final"], false);
    assert_eq!(records[0]["value"]["command_id"], "lost-start");
    assert_eq!(records[1]["error"]["classification"], "timeout");
    assert_eq!(records[1]["final"], true);
    fs::remove_file(directory.path().join("brief.txt"))?;
    let recovered = cli(
        &daemon.endpoint,
        directory.path(),
        &[
            "--timeout-secs",
            "45",
            "run",
            "reconnect",
            "lost.json",
            "--wait",
        ],
    )
    .await?;
    assert!(
        recovered.status.success(),
        "{}",
        String::from_utf8_lossy(&recovered.stdout)
    );
    let records: Vec<Value> = std::str::from_utf8(&recovered.stdout)?
        .lines()
        .map(serde_json::from_str)
        .collect::<Result<_, _>>()?;
    assert_eq!(records.len(), 2);
    assert_eq!(records[0]["value"]["run_id"], "lost");
    assert_eq!(records[1]["value"]["terminal"], "succeeded");
    assert_eq!(
        records
            .iter()
            .filter(|record| record["final"] == true)
            .count(),
        1
    );
    assert_eq!(model.requests.lock().map_err(|_| "fixture lock")?.len(), 2);
    let replay = success(
        &cli(
            &daemon.endpoint,
            directory.path(),
            &["run", "reconnect", "lost.json"],
        )
        .await?,
    )?;
    assert_eq!(replay["replayed"], true);
    assert_eq!(model.requests.lock().map_err(|_| "fixture lock")?.len(), 2);
    Ok(())
}
