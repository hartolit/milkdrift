//! Failure evidence contains selected public state, never command arguments or response payloads.

use std::time::{Duration, Instant};

use milkdrift_contracts::truncate_utf8;
use serde_json::{Value, json};

use super::{CliOutput, CliRunner};

pub(super) fn command_identity(arguments: &[&str]) -> &'static str {
    for pair in arguments.windows(2) {
        match pair {
            ["run", "wait"] => return "run wait",
            ["run", "show"] => return "run show",
            ["run", "timeline"] => return "run timeline",
            ["run", "start"] => return "run start",
            ["run", "list"] => return "run list",
            ["daemon", "health"] => return "daemon health",
            ["daemon", "readiness"] => return "daemon readiness",
            ["capability", "show"] => return "capability show",
            ["invocation", "wait"] => return "invocation wait",
            ["invocation", "observations"] => return "invocation observations",
            ["attempt", "inspect"] => return "attempt inspect",
            ["blueprint", "import"] => return "blueprint import",
            _ => {}
        }
    }
    "command (arguments withheld)"
}

fn label(value: &Value) -> Value {
    value.as_str().map_or(Value::Null, |text| {
        Value::String(
            truncate_utf8(text, 128)
                .chars()
                .filter(|c| c.is_ascii_alphanumeric() || ":_-().".contains(*c))
                .collect(),
        )
    })
}

fn attempt(value: &Value) -> Value {
    json!({
        "attempt_id": label(value.pointer("/attempt_id").unwrap_or(&Value::Null)),
        "state": label(value.pointer("/state").unwrap_or(&Value::Null)),
        "terminal": label(value.pointer("/terminal").unwrap_or(&Value::Null)),
        "uncertain": value.pointer("/uncertain").unwrap_or(&Value::Null).as_bool(),
        "outputs": value.pointer("/outputs").unwrap_or(&Value::Null).as_array().map(Vec::len),
        // Retain this known diagnostic classification without copying arbitrary adapter text.
        "lease_renewal_rejected": value.pointer("/terminal_detail").unwrap_or(&Value::Null).as_str().is_some_and(|text| text.contains("heartbeat requires a still-valid active lease and later expiration"))
    })
}

fn selected_state(document: &Value) -> Value {
    let value = document.pointer("/value").unwrap_or(&Value::Null);
    match document.pointer("/type").unwrap_or(&Value::Null).as_str() {
        Some("run.show" | "run.wait") => json!({
            "run_id": label(value.pointer("/run_id").unwrap_or(&Value::Null)), "sequence": value.pointer("/sequence").unwrap_or(&Value::Null).as_u64(),
            "lifecycle": label(value.pointer("/lifecycle").unwrap_or(&Value::Null)), "terminal": label(value.pointer("/terminal").unwrap_or(&Value::Null)),
            "uncertainty_count": value.pointer("/uncertainty_count").unwrap_or(&Value::Null).as_u64(),
            "nodes": value.pointer("/nodes").unwrap_or(&Value::Null).as_array().map(|nodes| nodes.iter().take(8).map(|node| json!({
                "node_id": label(node.pointer("/node_id").unwrap_or(&Value::Null)), "state": label(node.pointer("/state").unwrap_or(&Value::Null)),
                "latest_attempt_id": label(node.pointer("/latest_attempt_id").unwrap_or(&Value::Null)),
                "attempt": attempt(node.pointer("/latest_attempt").unwrap_or(&Value::Null))
            })).collect::<Vec<_>>())
        }),
        Some("run.timeline") => json!({
            "events": value.pointer("/items").unwrap_or(&Value::Null).as_array().map(|items| items.iter().take(16).map(|item| json!({
                "sequence": item.pointer("/sequence").unwrap_or(&Value::Null).as_u64(), "timestamp_ms": item.pointer("/timestamp_ms").unwrap_or(&Value::Null).as_u64(),
                "category": label(item.pointer("/category").unwrap_or(&Value::Null)), "node_id": label(item.pointer("/node_id").unwrap_or(&Value::Null)),
                "attempt_id": label(item.pointer("/attempt_id").unwrap_or(&Value::Null)), "outcome": label(item.pointer("/detail/outcome").unwrap_or(&Value::Null))
            })).collect::<Vec<_>>()),
            "more": !value.pointer("/next_cursor").unwrap_or(&Value::Null).is_null()
        }),
        Some("daemon.health" | "daemon.readiness") => json!({
            "ready": value.pointer("/ready").unwrap_or(&Value::Null).as_bool(), "live": value.pointer("/live").unwrap_or(&Value::Null).as_bool(),
            "state": label(value.pointer("/state").unwrap_or(&Value::Null)), "active_effects": value.pointer("/active_effects").unwrap_or(&Value::Null).as_u64(),
            "queued_requests": value.pointer("/queued_requests").unwrap_or(&Value::Null).as_u64(),
            "failure_present": !value.pointer("/last_failure").unwrap_or(&Value::Null).is_null()
        }),
        Some("capability.show") => json!({
            "capabilities": value.as_array().map(|items| items.iter().take(4).map(|item| json!({
                "capability_id": label(item.pointer("/capability_id").unwrap_or(&Value::Null)), "generation": item.pointer("/generation").unwrap_or(&Value::Null).as_u64(),
                "health": label(item.pointer("/health").unwrap_or(&Value::Null)), "available": item.pointer("/available").unwrap_or(&Value::Null).as_bool(),
                "active_permits": item.pointer("/active_permits").unwrap_or(&Value::Null).as_u64()
            })).collect::<Vec<_>>())
        }),
        Some("attempt.inspect") => attempt(value),
        _ => Value::Null,
    }
}

pub(super) fn output_summary(output: &CliOutput) -> String {
    captured_summary(&output.stdout, output.stderr.len())
}

pub(super) fn captured_summary(stdout: &str, stderr_bytes: usize) -> String {
    let summary = match serde_json::from_str::<Value>(stdout) {
        Ok(document) => json!({
            "status": label(document.pointer("/status").unwrap_or(&Value::Null)),
            "classification": label(document.pointer("/error/classification").unwrap_or(&Value::Null)),
            "state": selected_state(&document)
        })
        .to_string(),
        Err(_) => "no single JSON response; content withheld".to_owned(),
    };
    let bounded = truncate_utf8(&summary, 2_048);
    format!(
        "{bounded}{}; stdout_bytes={}; stderr_bytes={}",
        if bounded.len() < summary.len() {
            " [truncated]"
        } else {
            ""
        },
        stdout.len(),
        stderr_bytes
    )
}

impl CliRunner {
    /// Collects at most four sanitized public reads within six seconds after a failure.
    ///
    /// Each diagnostic has at most two seconds including child exit. Errors become evidence;
    /// callers keep the original failure. Cleanup's separate two-second reap bound still applies.
    #[must_use]
    pub fn run_diagnostics(&self, run: &str, capability: Option<&str>) -> String {
        let deadline = Instant::now() + Duration::from_secs(6);
        let mut commands = vec![
            vec!["run", "show", run],
            vec!["run", "timeline", run, "--limit", "16"],
            vec!["daemon", "health"],
        ];
        if let Some(capability) = capability {
            commands.push(vec!["capability", "show", capability]);
        }
        let mut summaries = Vec::new();
        for arguments in commands {
            let identity = command_identity(&arguments);
            if Instant::now() >= deadline {
                summaries.push(format!("{identity}: diagnostic budget exhausted"));
                continue;
            }
            let probe_deadline = deadline.min(Instant::now() + Duration::from_secs(2));
            let mut bounded = vec!["--timeout-secs", "1"];
            bounded.extend(arguments);
            let summary = match self.run_until(&bounded, None, probe_deadline) {
                Ok(output) => format!(
                    "exit={:?}; elapsed_ms={}; {}",
                    output.status.code(),
                    output.elapsed.as_millis(),
                    output_summary(&output)
                ),
                Err(error) => format!("unavailable: {error}"),
            };
            summaries.push(format!("{identity}: {summary}"));
        }
        format!("diagnostics=[{}]", summaries.join("; "))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diagnostics_keep_state_without_payloads_credentials_or_unbounded_text() {
        let secret = "sentinel-private-input-credential";
        let response = json!({
            "type":"run.show", "status":"success", "command_id":secret,
            "value": {"run_id":"test-run", "sequence":42, "lifecycle":"running", "uncertainty_count":1,
                "inputs":secret, "controller_accounting":secret,
                "nodes":[{"node_id":"model", "state":"uncertain", "latest_attempt_id":"attempt-one", "latest_attempt":{
                    "attempt_id":"attempt-one", "state":"uncertain", "terminal":null, "uncertain":true,
                    "terminal_detail":secret, "context":secret, "outputs":[{"content":secret}]
                }}]}, "error":{"detail":secret}
        }).to_string();
        let summary = captured_summary(&response, secret.len());
        assert!(summary.contains("attempt-one"));
        assert!(summary.contains("uncertain"));
        assert!(summary.contains("42"));
        assert!(!summary.contains(secret));
        assert!(!captured_summary(secret, secret.len()).contains(secret));
        let timeline = json!({"status":"success", "type":"run.timeline", "value":{"items":vec![json!({"sequence":1,"attempt_id":"x".repeat(1000),"summary":secret});1000]}}).to_string();
        let summary = captured_summary(&timeline, 0);
        assert!(summary.contains("[truncated]"));
        assert!(summary.len() < 2200);
        assert!(!summary.contains(secret));
        assert_eq!(
            command_identity(&["--reason", secret, "run", "wait", secret]),
            "run wait"
        );
    }
}
