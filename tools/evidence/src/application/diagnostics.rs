//! Failure evidence contains selected public state, never command arguments or response payloads.

use std::time::{Duration, Instant};

use milkdrift_contracts::truncate_utf8;
use serde_json::{Value, json};

use super::{CliOutput, CliRunner};

pub(super) fn command_identity(arguments: &[&str]) -> &'static str {
    for pair in arguments.windows(2) {
        match pair {
            ["workflow", "show"] => return "workflow show",
            ["daemon", "health"] => return "daemon health",
            ["daemon", "readiness"] => return "daemon readiness",
            ["capability", "show"] => return "capability show",
            ["invocation", "wait"] => return "invocation wait",
            ["invocation", "observations"] => return "invocation observations",
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

fn selected_state(document: &Value) -> Value {
    let value = document.pointer("/value").unwrap_or(&Value::Null);
    match document.pointer("/type").unwrap_or(&Value::Null).as_str() {
        Some("workflow.show") => json!({
            "workflow": label(value.pointer("/id").unwrap_or(&Value::Null)),
            "generation": value.pointer("/generation").and_then(Value::as_u64),
            "state": label(value.pointer("/state").unwrap_or(&Value::Null)),
            "tasks": value.pointer("/tasks").and_then(Value::as_array).map(|items|items.iter().take(8).map(|item|json!({"id":label(item.pointer("/id").unwrap_or(&Value::Null)),"state":label(item.pointer("/state").unwrap_or(&Value::Null)),"stop_requested":item.pointer("/stop_requested").and_then(Value::as_bool)})).collect::<Vec<_>>())
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
    pub fn workflow_diagnostics(&self, run: &str, capability: Option<&str>) -> String {
        let deadline = Instant::now() + Duration::from_secs(6);
        let mut commands = vec![vec!["workflow", "show", run], vec!["daemon", "health"]];
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
        let response = json!({"type":"workflow.show","status":"success","command_id":secret,
            "value":{"id":"work","generation":42,"state":"active","tasks":[{"id":"task-one","state":"uncertain","stop_requested":true,"instructions":secret,"frozen":secret}]},"error":{"detail":secret}}).to_string();
        let summary = captured_summary(&response, secret.len());
        assert!(
            summary.contains("task-one") && summary.contains("uncertain") && summary.contains("42")
        );
        assert!(!summary.contains(secret));
        assert!(!captured_summary(secret, secret.len()).contains(secret));
        assert!(summary.len() < 2200);
        assert_eq!(
            command_identity(&["--reason", secret, "workflow", "show", secret]),
            "workflow show"
        );
    }
}
