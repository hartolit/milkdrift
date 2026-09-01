//! Hermetic contract tests for the operator-driven external evidence harness.

use std::{fs, path::Path, process::Command};

use serde_json::Value;

type TestResult = Result<(), Box<dyn std::error::Error>>;

fn evidence_command(output: &Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_milkdrift-external-evidence"));
    command.arg("--output").arg(output);
    command
}

fn read_report(output: &Path) -> Result<(String, Value), Box<dyn std::error::Error>> {
    let text = fs::read_to_string(output.join("report.json"))?;
    let value = serde_json::from_str(&text)?;
    Ok((text, value))
}

#[test]
fn fixture_proves_the_harness_without_claiming_external_qualification() -> TestResult {
    let root = tempfile::tempdir()?;
    let output = root.path().join("fixture-evidence");
    let secret = "external-evidence-test-secret-9f8f623a";
    let result = evidence_command(&output)
        .args(["--fixture", "--allow-fixture"])
        .arg("--secret-source")
        .arg("secret:test-only=env:MILKDRIFT_EVIDENCE_TEST_SECRET")
        .env("MILKDRIFT_EVIDENCE_TEST_SECRET", secret)
        .output()?;
    assert!(
        result.status.success(),
        "fixture failed: {}",
        String::from_utf8_lossy(&result.stderr)
    );

    let (text, report) = read_report(&output)?;
    assert_eq!(report["schema_version"], 1);
    assert_eq!(report["fixture_mode"], true);
    assert_eq!(report["qualifying"], false);
    assert!(
        report["configuration_digest"]
            .as_str()
            .is_some_and(|digest| digest.starts_with("b3_") && digest.len() == 67)
    );
    for scenario in ["process", "model"] {
        assert_eq!(report[scenario]["outcome"], "succeeded");
        assert_eq!(report[scenario]["qualifying"], false);
        assert_eq!(
            report[scenario]["restart_boundaries"][0]["duplicate_attempts"],
            false
        );
    }
    assert_eq!(
        report["process"]["facts"]["distinct_process_invocations"],
        5
    );
    assert_eq!(
        report["process"]["facts"]["process_invocations"]
            .as_array()
            .map(Vec::len),
        Some(5)
    );
    assert_eq!(report["model"]["facts"]["selected_count"], 2);
    assert!(
        report["model"]["facts"]["omitted_count"]
            .as_u64()
            .is_some_and(|count| count >= 1)
    );
    assert!(
        report["model"]["facts"]["streaming_observations"]
            .as_u64()
            .is_some_and(|count| count >= 1)
    );
    assert_eq!(report["model"]["facts"]["usage"]["input_units"], 19);
    assert!(output.join("session/data").is_dir());

    let lower = text.to_ascii_lowercase();
    assert!(!text.contains(secret));
    assert!(!lower.contains("\"authorization\""));
    assert!(!lower.contains("\"environment\""));
    assert!(!text.contains("Using only the selected evidence"));
    assert!(!text.contains("def add(a, b)"));
    Ok(())
}

#[test]
fn missing_real_resources_are_non_qualifying_and_fail_closed() -> TestResult {
    let root = tempfile::tempdir()?;
    let output = root.path().join("missing-resources");
    let result = evidence_command(&output).output()?;
    assert!(!result.status.success());
    let (_, report) = read_report(&output)?;
    assert_eq!(report["qualifying"], false);
    assert_eq!(report["fixture_mode"], false);
    assert_eq!(report["process"]["outcome"], "not_run");
    assert_eq!(report["model"]["outcome"], "not_run");
    assert!(
        report["failure_reason"]
            .as_str()
            .is_some_and(|reason| reason.contains("--agent-profile"))
    );
    Ok(())
}

#[test]
fn fixture_process_and_model_scenario_failures_exit_nonzero() -> TestResult {
    let root = tempfile::tempdir()?;
    for fault in ["process", "model"] {
        let output = root.path().join(format!("{fault}-failure"));
        let result = evidence_command(&output)
            .args(["--fixture", "--allow-fixture", "--fixture-failure", fault])
            .output()?;
        assert!(!result.status.success());
        let (_, report) = read_report(&output)?;
        assert_eq!(report["qualifying"], false);
        assert_eq!(report[fault]["outcome"], "failed");
        assert!(
            report[fault]["failure_reason"]
                .as_str()
                .is_some_and(|reason| reason.contains("fixture-injected"))
        );
        if fault == "model" {
            assert_eq!(report["process"]["outcome"], "succeeded");
        }
    }
    Ok(())
}

#[test]
fn fixture_requires_acknowledgement_and_tracked_outputs_are_refused() -> TestResult {
    let root = tempfile::tempdir()?;
    let unacknowledged = root.path().join("unacknowledged");
    let result = evidence_command(&unacknowledged)
        .arg("--fixture")
        .output()?;
    assert!(!result.status.success());
    assert!(String::from_utf8_lossy(&result.stderr).contains("--fixture requires --allow-fixture"));
    assert!(!unacknowledged.exists());

    let repository = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()?;
    let tracked = repository.join("docs/external-evidence-test-output");
    let result = evidence_command(&tracked)
        .args(["--fixture", "--allow-fixture"])
        .output()?;
    assert!(!result.status.success());
    assert!(String::from_utf8_lossy(&result.stderr).contains("tracked source paths"));
    assert!(!tracked.exists());
    Ok(())
}

#[test]
fn committed_report_schema_is_strict_and_versioned() -> TestResult {
    let repository = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()?;
    let schema: Value = serde_json::from_slice(&fs::read(
        repository.join("docs/reference/external-evidence-report-v1.schema.json"),
    )?)?;
    assert_eq!(
        schema["$id"],
        "https://milkdrift.dev/schema/external-evidence-report-v1.json"
    );
    assert_eq!(schema["properties"]["schema_version"]["const"], 1);
    assert_eq!(schema["additionalProperties"], false);
    assert_eq!(schema["properties"]["validation"]["maxItems"], 4096);
    assert_eq!(schema["properties"]["redactions"]["maxItems"], 64);
    assert_eq!(
        schema["$defs"]["scenario"]["properties"]["facts"]["maxProperties"],
        64
    );
    assert_eq!(
        schema["$defs"]["artifact"]["properties"]["digest"]["pattern"],
        "^[0-9a-f]{64}$"
    );
    assert!(
        schema["required"]
            .as_array()
            .is_some_and(|fields| fields.len() == 12)
    );
    Ok(())
}
