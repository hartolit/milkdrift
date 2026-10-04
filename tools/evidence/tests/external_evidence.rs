//! Hermetic contract tests for the operator-driven external evidence harness.

use std::{
    fs,
    path::Path,
    process::Command,
    sync::{Mutex, MutexGuard},
    time::Duration,
};

use milkdrift_evidence::application::run_command;
use serde_json::Value;

type TestResult = Result<(), Box<dyn std::error::Error + Send + Sync>>;

static HARNESS_PROCESS: Mutex<()> = Mutex::new(());

fn serialize_harness_process() -> MutexGuard<'static, ()> {
    // Only serializes independent child owners; a failed scenario leaves no shared fixture state.
    HARNESS_PROCESS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn evidence_command(output: &Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_milkdrift-external-evidence"));
    command.arg("--output").arg(output);
    command
}

fn read_report(output: &Path) -> Result<(String, Value), Box<dyn std::error::Error + Send + Sync>> {
    let text = fs::read_to_string(output.join("report.json"))?;
    let value = serde_json::from_str(&text)?;
    Ok((text, value))
}

#[test]
fn fixture_proves_the_harness_without_claiming_external_qualification() -> TestResult {
    let _harness = serialize_harness_process();
    let root = tempfile::tempdir()?;
    let output = root.path().join("fixture-evidence");
    let secret = "external-evidence-test-secret-9f8f623a";
    let result = run_command(
        evidence_command(&output)
            .args([
                "--fixture",
                "--allow-fixture",
                "--max-output-units",
                "2048",
                "--timeout-secs",
                // A workflow wait covers several sequential processes and acceptance steps.
                // Keep an explicit non-default bound without timing out valid debug-build work.
                "120",
            ])
            .arg("--secret-source")
            .arg("secret:test-only=env:MILKDRIFT_EVIDENCE_TEST_SECRET")
            .env("MILKDRIFT_EVIDENCE_TEST_SECRET", secret),
        None,
        // This child owns multiple workflow waits, daemon restarts and compiler invocations.
        // Its deadline must cover that sequence, including unoptimized executable hashing.
        Duration::from_secs(300),
    )?;
    assert!(result.status.success(), "fixture failed: {}", result.stderr);

    let (text, report) = read_report(&output)?;
    assert_eq!(
        report
            .pointer("/schema_version")
            .ok_or("missing /schema_version")?,
        1
    );
    assert_eq!(
        report
            .pointer("/fixture_mode")
            .ok_or("missing /fixture_mode")?,
        true
    );
    assert_eq!(
        report.pointer("/qualifying").ok_or("missing /qualifying")?,
        false
    );
    assert!(
        report
            .pointer("/configuration_digest")
            .ok_or("missing /configuration_digest")?
            .as_str()
            .is_some_and(|digest| digest.starts_with("b3_") && digest.len() == 67)
    );
    for scenario in ["process", "model"] {
        assert_eq!(
            report
                .get(scenario)
                .ok_or("scenario report absent")?
                .pointer("/outcome")
                .ok_or("missing /outcome")?,
            "succeeded"
        );
        assert_eq!(
            report
                .get(scenario)
                .ok_or("scenario report absent")?
                .pointer("/qualifying")
                .ok_or("missing /qualifying")?,
            false
        );
        assert_eq!(
            report
                .get(scenario)
                .ok_or("scenario report absent")?
                .pointer("/restart_boundaries/0/duplicate_attempts")
                .ok_or("missing /restart_boundaries/0/duplicate_attempts")?,
            false
        );
    }
    assert_eq!(
        report
            .pointer("/process/facts/distinct_process_invocations")
            .ok_or("missing /process/facts/distinct_process_invocations")?,
        5
    );
    assert_eq!(
        report
            .pointer("/process/facts/process_invocations")
            .ok_or("missing /process/facts/process_invocations")?
            .as_array()
            .map(Vec::len),
        Some(5)
    );
    assert_eq!(
        report
            .pointer("/model/facts/selected_count")
            .ok_or("missing /model/facts/selected_count")?,
        2
    );
    assert!(
        report
            .pointer("/model/facts/omitted_count")
            .ok_or("missing /model/facts/omitted_count")?
            .as_u64()
            .is_some_and(|count| count >= 1)
    );
    assert!(
        report
            .pointer("/model/facts/streaming_observations")
            .ok_or("missing /model/facts/streaming_observations")?
            .as_u64()
            .is_some_and(|count| count >= 1)
    );
    assert_eq!(
        report
            .pointer("/model/facts/usage/input_units")
            .ok_or("missing /model/facts/usage/input_units")?,
        19
    );
    assert_eq!(
        report
            .pointer("/model/facts/max_output_units")
            .ok_or("missing /model/facts/max_output_units")?,
        2048
    );
    assert_eq!(
        report
            .pointer("/model/facts/wait_timeout_secs")
            .ok_or("missing /model/facts/wait_timeout_secs")?,
        120
    );
    assert_eq!(
        report
            .pointer("/process/facts/wait_timeout_secs")
            .ok_or("missing /process/facts/wait_timeout_secs")?,
        120
    );
    let repository_facts = report
        .pointer("/process/facts")
        .ok_or("missing /process/facts")?;
    assert_eq!(
        repository_facts["repository_initial_commit"],
        repository_facts["repository_final_commit"]
    );
    assert_eq!(
        repository_facts["repository_initial_tree"],
        repository_facts["repository_final_tree"]
    );
    assert!(
        repository_facts["dirty_diff_bytes"]
            .as_u64()
            .is_some_and(|bytes| bytes > 0)
    );
    assert_ne!(
        repository_facts["dirty_diff_digest"],
        format!("b3_{}", blake3::hash(b""))
    );
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
    let _harness = serialize_harness_process();
    let root = tempfile::tempdir()?;
    let output = root.path().join("missing-resources");
    let result = run_command(
        &mut evidence_command(&output),
        None,
        Duration::from_secs(90),
    )?;
    assert!(!result.status.success());
    let (_, report) = read_report(&output)?;
    assert_eq!(
        report.pointer("/qualifying").ok_or("missing /qualifying")?,
        false
    );
    assert_eq!(
        report
            .pointer("/fixture_mode")
            .ok_or("missing /fixture_mode")?,
        false
    );
    assert_eq!(
        report
            .pointer("/process/outcome")
            .ok_or("missing /process/outcome")?,
        "not_run"
    );
    assert_eq!(
        report
            .pointer("/model/outcome")
            .ok_or("missing /model/outcome")?,
        "not_run"
    );
    assert!(
        report
            .pointer("/failure_reason")
            .ok_or("missing /failure_reason")?
            .as_str()
            .is_some_and(|reason| reason.contains("--agent-profile"))
    );
    Ok(())
}

#[test]
fn fixture_process_and_model_scenario_failures_exit_nonzero() -> TestResult {
    let _harness = serialize_harness_process();
    let root = tempfile::tempdir()?;
    for fault in ["process", "model", "model-truncated"] {
        let output = root.path().join(format!("{fault}-failure"));
        let result = run_command(
            evidence_command(&output).args([
                "--fixture",
                "--allow-fixture",
                "--fixture-failure",
                fault,
            ]),
            None,
            Duration::from_secs(300),
        )?;
        assert!(!result.status.success());
        let (_, report) = read_report(&output)?;
        assert_eq!(
            report.pointer("/qualifying").ok_or("missing /qualifying")?,
            false
        );
        let scenario = if fault == "process" {
            "process"
        } else {
            "model"
        };
        assert_eq!(
            report
                .get(scenario)
                .ok_or("scenario report absent")?
                .pointer("/outcome")
                .ok_or("missing /outcome")?,
            "failed",
            "{fault}: {report}"
        );
        let expected_reason = if fault == "model-truncated" {
            "model result not accepted: output_allowance_exhausted; invocation terminal=succeeded"
        } else {
            "fixture-injected"
        };
        assert!(
            report
                .get(scenario)
                .ok_or("scenario report absent")?
                .pointer("/failure_reason")
                .ok_or("missing /failure_reason")?
                .as_str()
                .is_some_and(|reason| reason.contains(expected_reason)),
            "{fault}: expected {expected_reason:?}; report={report}"
        );
        if scenario == "model" {
            assert_eq!(
                report
                    .pointer("/process/outcome")
                    .ok_or("missing /process/outcome")?,
                "succeeded"
            );
        }
    }
    Ok(())
}

#[test]
fn fixture_requires_acknowledgement_and_tracked_outputs_are_refused() -> TestResult {
    let _harness = serialize_harness_process();
    let root = tempfile::tempdir()?;
    let unacknowledged = root.path().join("unacknowledged");
    let result = run_command(
        evidence_command(&unacknowledged).arg("--fixture"),
        None,
        Duration::from_secs(90),
    )?;
    assert!(!result.status.success());
    assert!(result.stderr.contains("--fixture requires --allow-fixture"));
    assert!(!unacknowledged.exists());

    let repository = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()?;
    let tracked = repository.join("docs/external-evidence-test-output");
    let result = run_command(
        evidence_command(&tracked).args(["--fixture", "--allow-fixture"]),
        None,
        Duration::from_secs(90),
    )?;
    assert!(!result.status.success());
    assert!(result.stderr.contains("tracked source paths"));
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
        schema.pointer("/$id").ok_or("missing /$id")?,
        "https://milkdrift.dev/schema/external-evidence-report-v1.json"
    );
    assert_eq!(
        schema
            .pointer("/properties/schema_version/const")
            .ok_or("missing /properties/schema_version/const")?,
        1
    );
    assert_eq!(
        schema
            .pointer("/additionalProperties")
            .ok_or("missing /additionalProperties")?,
        false
    );
    assert_eq!(
        schema
            .pointer("/properties/validation/maxItems")
            .ok_or("missing /properties/validation/maxItems")?,
        4096
    );
    assert_eq!(
        schema
            .pointer("/properties/redactions/maxItems")
            .ok_or("missing /properties/redactions/maxItems")?,
        64
    );
    let model_facts = schema
        .pointer("/allOf/0/then/properties/model/properties/facts")
        .ok_or("missing /allOf/0/then/properties/model/properties/facts")?;
    assert_eq!(
        model_facts
            .pointer("/properties/finish_reason/const")
            .ok_or("missing /properties/finish_reason/const")?,
        "stop"
    );
    assert!(
        model_facts["required"]
            .as_array()
            .is_some_and(|required| required.iter().any(|field| field == "finish_reason"))
    );
    let process_facts = schema
        .pointer("/allOf/0/then/properties/process/properties/facts")
        .ok_or("missing /allOf/0/then/properties/process/properties/facts")?;
    for field in [
        "repository_initial_tree",
        "repository_final_tree",
        "dirty_diff_digest",
        "dirty_diff_bytes",
    ] {
        assert!(
            process_facts["required"]
                .as_array()
                .is_some_and(|required| required.iter().any(|value| value == field)),
            "{field}"
        );
    }
    assert_eq!(
        process_facts
            .pointer("/properties/dirty_diff_bytes/minimum")
            .ok_or("missing /properties/dirty_diff_bytes/minimum")?,
        1
    );
    assert_eq!(
        schema
            .pointer("/$defs/scenario/properties/facts/maxProperties")
            .ok_or("missing /$defs/scenario/properties/facts/maxProperties")?,
        64
    );
    assert_eq!(
        schema
            .pointer("/$defs/artifact/properties/digest/pattern")
            .ok_or("missing /$defs/artifact/properties/digest/pattern")?,
        "^[0-9a-f]{64}$"
    );
    assert!(
        schema
            .pointer("/required")
            .ok_or("missing /required")?
            .as_array()
            .is_some_and(|fields| fields.len() == 12)
    );
    Ok(())
}
