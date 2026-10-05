//! Both CLI preparation consumers retain exact uploaded references across encoding changes.
use super::{authoring::model_configuration_document, inputs::ModelFixture, support::*};
use milkdrift_control_client::SavedRunRequest;
use milkdrift_control_protocol::{InputUploadRequest, RunInput};
use milkdrift_peer_protocol::{DirectInvocationRequest, InvocationAcceptance};
use serde_json::Value;

fn prepare(
    daemon: &RunningDaemon,
    directory: &TempDir,
    invocation: bool,
    id: &str,
    path: &str,
    inputs: &[&str],
) -> TestResult<(bool, String)> {
    let mut args = if invocation {
        vec![
            "invocation",
            "prepare",
            "writing-model",
            "model.generate",
            "--host",
            "host:local",
            "--request-id",
            id,
            "--output",
            path,
        ]
    } else {
        vec![
            "run",
            "start",
            "prepared-run",
            "release-notes",
            "unused-preparation-revision",
            "--request-file",
            path,
            "--prepare-only",
        ]
    };
    for input in inputs {
        args.extend(["--input", input]);
    }
    cli(daemon, directory, id, &args, true)
}

fn reference(
    directory: &TempDir,
    path: &str,
    invocation: bool,
    index: usize,
) -> TestResult<String> {
    let value: Value = serde_json::from_slice(&fs::read(directory.path().join(path))?)?;
    let pointer = if invocation {
        format!("/request/inputs/{index}/value/reference/identity")
    } else {
        format!("/request/command/inputs/{index}/artifact_id")
    };
    Ok(value
        .pointer(&pointer)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("reference absent at {pointer}"))?
        .into())
}

async fn case(invocation: bool) -> TestResult {
    let directory = TempDir::new()?;
    let model = ModelFixture::start().await?;
    let config = model_configuration_document(&directory, model.address)?;
    let daemon = start(config.validate(directory.path())?, CONTROLLER_TOKEN).await?;
    model.checked(async |model| {
        daemon.checked(model, async |daemon| {
            fs::write(directory.path().join("one.txt"), b"first upload")?;
            fs::write(directory.path().join("two.txt"), b"different upload")?;
            for (id, path, inputs) in [
                ("x", "first.json", vec!["z:1:y=one.txt"]),
                ("x:0:z", "second.json", vec!["unused=one.txt", "y=two.txt"]),
                ("x", "retry.json", vec!["z:1:y=one.txt"]),
            ] {
                let (ok, message) = prepare(daemon, &directory, invocation, id, path, &inputs)?;
                assert!(ok, "{message}");
            }
            let first = reference(&directory, "first.json", invocation, 0)?;
            let second = reference(&directory, "second.json", invocation, 1)?;
            assert_ne!(first, second);
            assert_eq!(first, reference(&directory, "retry.json", invocation, 0)?);
            assert_eq!(daemon.client.artifact_range(&first, 0, 11).await?.bytes, b"first upload");
            assert_eq!(daemon.client.artifact_range(&second, 0, 15).await?.bytes, b"different upload");
            fs::write(directory.path().join("one.txt"), b"changed upload")?;
            let (ok, message) = prepare(daemon, &directory, invocation, "x", "conflict.json", &["z:1:y=one.txt"])?;
            assert!(!ok && message.contains("conflict"), "{message}");
            assert_eq!(daemon.client.artifact_range(&first, 0, 11).await?.bytes, b"first upload");

            // Simulate an upload retained by a previous CLI before it could save
            // the complete request. The public upload contract retains its exact ID.
            let namespace = if invocation { "invocation-input" } else { "run-input" };
            let legacy = InputUploadRequest::from_content("host:local".into(), format!("{namespace}:{}", blake3::hash(b"legacy:0:brief")), "text/plain".into(), "restricted".into(), b"legacy retained brief")?;
            let artifact = daemon.client.upload_input(&legacy).await?;
            assert_eq!(artifact, daemon.client.upload_input(&legacy).await?);
            if invocation {
                let mut saved: DirectInvocationRequest = serde_json::from_slice(&fs::read(directory.path().join("first.json"))?)?;
                let mut value = serde_json::to_value(&saved)?;
                *value.pointer_mut("/request/inputs").ok_or("inputs absent")? = serde_json::json!([{
                    "name":"brief", "value":{"type":"artifact", "reference":{
                        "identity":artifact.artifact_id, "digest":artifact.digest,
                        "media_type":artifact.content_type, "size_bytes":artifact.size
                    }}
                }]);
                saved = serde_json::from_value(value)?;
                let path = directory.path().join("legacy-invocation.json");
                fs::write(&path, serde_json::to_vec(&saved)?)?;
                for replayed in [false, true] {
                    let result = cli_ok(daemon, &directory, "legacy-submit", &["invocation", "submit", "legacy-invocation.json"])?;
                    let accepted: InvocationAcceptance = serde_json::from_value(result)?;
                    assert!(matches!(accepted, InvocationAcceptance::Accepted { replayed: actual, .. } if actual == replayed));
                    assert_eq!(serde_json::from_slice::<DirectInvocationRequest>(&fs::read(&path)?)?, saved);
                }
            } else {
                let revision = super::inputs::workflow(&daemon.client).await?;
                let saved = daemon.client.prepare_run(super::inputs::start_request("legacy-run", &revision, vec![RunInput { name: "brief".into(), artifact_id: artifact.artifact_id.clone() }])).await?;
                let path = directory.path().join("legacy-run.json");
                fs::write(&path, serde_json::to_vec(&saved)?)?;
                #[cfg(unix)] {
                    use std::os::unix::fs::PermissionsExt as _;
                    fs::set_permissions(&path, fs::Permissions::from_mode(0o600))?;
                }
                for replayed in [false, true] {
                    let result = cli_ok(daemon, &directory, &saved.request.command_id, &["run", "reconnect", "legacy-run.json"])?;
                    assert_eq!(result.get("replayed"), Some(&Value::Bool(replayed)));
                    assert_eq!(serde_json::from_slice::<SavedRunRequest>(&fs::read(&path)?)?, saved);
                }
                let finished = wait_for_run(&daemon.client, "legacy-run", Duration::from_secs(45), |run| run.terminal.is_some()).await?;
                assert_eq!(finished.terminal.as_deref(), Some("succeeded"));
            }
            assert_eq!(artifact, daemon.client.artifact_metadata(&artifact.artifact_id).await?);
            Ok(())
        }).await
    }).await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn run_upload_boundaries_retry_conflict_and_legacy_references() -> TestResult {
    case(false).await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn invocation_upload_boundaries_retry_conflict_and_legacy_references() -> TestResult {
    case(true).await
}
