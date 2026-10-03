use super::{authoring::model_configuration, support::*};
fn cli(
    daemon: &RunningDaemon,
    directory: &TempDir,
    arguments: &[&str],
    stdin: Option<&str>,
) -> TestResult<std::process::Output> {
    use std::io::Write as _;
    let executable = std::env::current_exe()?
        .parent()
        .and_then(|path| path.parent())
        .ok_or("binary directory")?
        .join(format!("milkdrift{}", std::env::consts::EXE_SUFFIX));
    let mut child = std::process::Command::new(executable)
        .arg("--endpoint")
        .arg(daemon.endpoint.as_str())
        .arg("--token-file")
        .arg(directory.path().join("controller.token"))
        .args(["--json", "--timeout-secs", "10", "workflow"])
        .args(arguments)
        .current_dir(directory.path())
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()?;
    if let Some(text) = stdin {
        child
            .stdin
            .take()
            .ok_or("stdin pipe")?
            .write_all(text.as_bytes())?;
    }
    drop(child.stdin.take());
    Ok(child.wait_with_output()?)
}
fn cli_ok(
    daemon: &RunningDaemon,
    directory: &TempDir,
    arguments: &[&str],
    stdin: Option<&str>,
) -> TestResult<serde_json::Value> {
    let output = cli(daemon, directory, arguments, stdin)?;
    assert!(
        output.status.success(),
        "{arguments:?}: {} {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    Ok(serde_json::from_slice::<serde_json::Value>(&output.stdout)?["value"].clone())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn actual_cli_authors_prompts_connections_and_guarded_files() -> TestResult {
    let directory = tempfile::tempdir()?;
    let listener = std::net::TcpListener::bind("127.0.0.1:0")?;
    listener.set_nonblocking(true)?;
    let daemon = start(
        model_configuration(&directory, listener.local_addr()?)?,
        CONTROLLER_TOKEN,
    )
    .await?;
    fs::write(
        directory.path().join("draft.txt"),
        include_str!("../../../../examples/operator/release-notes/draft.txt"),
    )?;
    let review = include_str!("../../../../examples/operator/release-notes/review.txt");
    cli_ok(&daemon, &directory, &["models"], None)?;
    let created = cli_ok(
        &daemon,
        &directory,
        &[
            "new",
            "release-notes",
            "--name",
            "Release notes",
            "--file",
            "notes.json",
        ],
        None,
    )?;
    cli_ok(&daemon, &directory, &["input", "notes.json", "brief"], None)?;
    assert!(
        !cli(
            &daemon,
            &directory,
            &[
                "--expected-edit",
                created["edit_token"].as_str().ok_or("token")?,
                "rename",
                "notes.json",
                "Stale"
            ],
            None
        )?
        .status
        .success()
    );
    for step in ["draft", "review", "unused"] {
        cli_ok(
            &daemon,
            &directory,
            &[
                "add",
                "notes.json",
                step,
                "--model",
                "writing-model",
                "--prompt",
                "draft.txt",
                "--maximum-output-units",
                "512",
            ],
            None,
        )?;
    }
    cli_ok(
        &daemon,
        &directory,
        &["move", "notes.json", "unused", "--before", "draft"],
        None,
    )?;
    cli_ok(
        &daemon,
        &directory,
        &["remove", "notes.json", "unused"],
        None,
    )?;
    cli_ok(
        &daemon,
        &directory,
        &["prompt", "notes.json", "review", "--prompt", "-"],
        Some(review),
    )?;
    cli_ok(
        &daemon,
        &directory,
        &["model", "notes.json", "review", "writing-model"],
        None,
    )?;
    for step in ["draft", "review"] {
        cli_ok(
            &daemon,
            &directory,
            &[
                "connect",
                "notes.json",
                step,
                "brief",
                "--run-input",
                "brief",
            ],
            None,
        )?;
    }
    cli_ok(
        &daemon,
        &directory,
        &[
            "connect",
            "notes.json",
            "review",
            "draft",
            "--from-step",
            "draft",
        ],
        None,
    )?;
    cli_ok(
        &daemon,
        &directory,
        &["output", "notes.json", "review", "--name", "notes"],
        None,
    )?;
    let before = fs::read(directory.path().join("notes.json"))?;
    for arguments in [
        vec![
            "new",
            "other",
            "--name",
            "Overwrite",
            "--file",
            "notes.json",
        ],
        vec!["move", "notes.json", "review", "--before", "draft"],
        vec!["model", "notes.json", "review", "hidden-model"],
        vec![
            "connect",
            "notes.json",
            "draft",
            "bad",
            "--from-step",
            "review",
        ],
    ] {
        assert!(!cli(&daemon, &directory, &arguments, None)?.status.success());
        assert_eq!(fs::read(directory.path().join("notes.json"))?, before);
    }
    let saved = cli_ok(&daemon, &directory, &["save", "notes.json"], None)?;
    let revision = saved["revision_id"].as_str().ok_or("revision")?;
    let read = cli_ok(
        &daemon,
        &directory,
        &["open", revision, "--file", "reopened.json"],
        None,
    )?;
    assert_eq!(read["workflow"], saved["workflow"]);
    assert_eq!(read["workflow"]["steps"][1]["prompt"], review);
    let file_before = fs::read(directory.path().join("reopened.json"))?;
    let inspected = cli_ok(&daemon, &directory, &["inspect", "reopened.json"], None)?;
    assert_eq!(inspected["workflow"], saved["workflow"]);
    assert_eq!(
        fs::read(directory.path().join("reopened.json"))?,
        file_before
    );
    let unchanged = cli_ok(&daemon, &directory, &["save", "reopened.json"], None)?;
    assert_eq!(unchanged["revision_id"], revision);
    assert_eq!(
        fs::read(directory.path().join("reopened.json"))?,
        file_before
    );
    let old = daemon.client.revision(revision).await?.document;
    cli_ok(
        &daemon,
        &directory,
        &["prompt", "reopened.json", "review", "--prompt", "-"],
        Some("Check every claim against the brief."),
    )?;
    let edited = cli_ok(&daemon, &directory, &["save", "reopened.json"], None)?;
    assert_ne!(edited["revision_id"], revision);
    assert_eq!(daemon.client.revision(revision).await?.document, old);
    cli_ok(
        &daemon,
        &directory,
        &[
            "new",
            "meeting-summary",
            "--name",
            "Meeting summary",
            "--file",
            "meeting.json",
        ],
        None,
    )?;
    cli_ok(
        &daemon,
        &directory,
        &[
            "add",
            "meeting.json",
            "summarize",
            "--model",
            "writing-model",
            "--prompt",
            "-",
            "--maximum-output-units",
            "256",
        ],
        Some(include_str!(
            "../../../../examples/operator/release-notes/meeting.txt"
        )),
    )?;
    cli_ok(
        &daemon,
        &directory,
        &["input", "meeting.json", "brief"],
        None,
    )?;
    cli_ok(
        &daemon,
        &directory,
        &[
            "connect",
            "meeting.json",
            "summarize",
            "brief",
            "--run-input",
            "brief",
        ],
        None,
    )?;
    cli_ok(
        &daemon,
        &directory,
        &["output", "meeting.json", "summarize"],
        None,
    )?;
    cli_ok(&daemon, &directory, &["save", "meeting.json"], None)?;
    assert!(
        daemon
            .client
            .runs(
                None,
                None,
                &PageRequest {
                    limit: 20,
                    cursor: None
                }
            )
            .await?
            .items
            .is_empty()
    );
    assert_eq!(
        listener
            .accept()
            .err()
            .ok_or("unexpected model request")?
            .kind(),
        std::io::ErrorKind::WouldBlock
    );
    daemon.stop().await?;
    Ok(())
}
