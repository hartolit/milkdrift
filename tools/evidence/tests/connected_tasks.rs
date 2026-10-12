//! The finite connected-task lane must execute real binaries and clean up on both outcomes.

use milkdrift_evidence::{
    EvidenceResult,
    application::{application_binary, run_command},
};
use serde_json::Value;
use std::{fs, path::Path, process::Command, time::Duration};

async fn assert_stopped(root: &Path) -> EvidenceResult {
    let ready: Value = serde_json::from_slice(&fs::read(root.join("ready.json"))?)?;
    let instances = ready
        .get("daemons")
        .and_then(Value::as_array)
        .ok_or("ready daemon list missing")?;
    assert_eq!(instances.len(), 2);
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(1))
        .build()?;
    for instance in instances {
        let endpoint = instance
            .get("endpoint")
            .and_then(Value::as_str)
            .ok_or("ready endpoint missing")?;
        assert!(
            client
                .get(format!("{endpoint}/v1/session"))
                .send()
                .await
                .is_err(),
            "owned daemon is still serving after fixture exit"
        );
    }
    Ok(())
}

#[tokio::test]
async fn real_cli_and_public_client_replay_discover_and_preserve_private_captured_inputs()
-> EvidenceResult {
    let directory = tempfile::tempdir()?;
    let root = directory.path().join("connected");
    let output = run_command(
        Command::new(env!("CARGO_BIN_EXE_connected-tasks"))
            .args(["check", "--root"])
            .arg(&root)
            .arg("--daemon")
            .arg(application_binary("milkdrift-daemon")?)
            .arg("--cli")
            .arg(application_binary("milkdrift")?),
        None,
        Duration::from_secs(300),
    )?;
    assert!(
        output.status.success(),
        "connected task evidence failed: {}",
        output.stderr
    );
    assert!(!output.stdout.contains("private-brief-sentinel"));
    assert!(!output.stderr.contains("private-brief-sentinel"));
    let report: Value = serde_json::from_slice(&fs::read(root.join("check.json"))?)?;
    assert_eq!(
        report.get("outcome").and_then(Value::as_str),
        Some("passed")
    );
    assert_eq!(report.get("qualifying"), Some(&Value::Bool(false)));
    for fact in [
        "cli_submission",
        "public_client_discovery",
        "exact_replay_after_restart",
        "captured_input_survives_local_edit",
    ] {
        assert_eq!(
            report.get(fact),
            Some(&Value::Bool(true)),
            "missing observed {fact}"
        );
    }
    assert_eq!(
        report.get("model_requests").and_then(Value::as_u64),
        Some(1)
    );
    let private: Value = serde_json::from_slice(&fs::read(root.join("browser.json"))?)?;
    for name in ["a", "b"] {
        let token = private
            .get(name)
            .and_then(|value| value.get("token"))
            .and_then(Value::as_str)
            .ok_or("credential absent")?;
        assert!(
            !output.stdout.contains(token)
                && !output.stderr.contains(token)
                && !report.to_string().contains(token)
        );
    }
    assert_stopped(&root).await
}

#[tokio::test]
async fn failed_connected_scenario_reaps_both_production_daemons_without_publishing_success()
-> EvidenceResult {
    let directory = tempfile::tempdir()?;
    let root = directory.path().join("failed-connected");
    let output = run_command(
        Command::new(env!("CARGO_BIN_EXE_connected-tasks"))
            .args(["check", "--root"])
            .arg(&root)
            .arg("--daemon")
            .arg(application_binary("milkdrift-daemon")?)
            .arg("--cli")
            .arg(directory.path().join("missing-cli")),
        None,
        Duration::from_secs(180),
    )?;
    assert!(!output.status.success());
    assert!(!root.join("check.json").exists());
    assert_stopped(&root).await
}
