//! Formatting only: outcome, evidence, outputs and action hints come from the daemon.

use crate::{error::CliError, session::CliSession};
use milkdrift_control_protocol::RunResultRead;
use std::{fmt::Write as _, path::Path};

pub(super) async fn execute(
    session: &CliSession,
    run: &str,
    details: bool,
    field: Option<&str>,
    output: Option<&Path>,
) -> Result<(), CliError> {
    let value = session.client().run_result(run).await?;
    if session.cli().json {
        session.output("run.result", &value)?;
    } else {
        print!("{}", render(&value, details));
    }
    if let (Some(field), Some(output)) = (field, output) {
        let result = value.outputs.iter().find(|value| value.name == field)
            .ok_or_else(|| CliError::Invalid("declared final output is unavailable; inspect the run outcome and read permissions".into()))?;
        crate::command::artifact::download(session, &result.artifact.artifact_id, output).await?;
    }
    Ok(())
}

// Escape terminal controls, including bidi formatting; preserve readable line breaks in prose.
fn safe(value: &str) -> String {
    value
        .chars()
        .flat_map(|c| {
            if (c.is_control() && c != '\n' && c != '\t')
                || matches!(c, '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')
            {
                c.escape_unicode().collect::<Vec<_>>()
            } else {
                vec![c]
            }
        })
        .collect()
}

fn render(value: &RunResultRead, details: bool) -> String {
    let mut out = String::new();
    let run = &value.run;
    let _ = writeln!(
        out,
        "Workflow: {} (version {})",
        safe(value.workflow_name.as_deref().unwrap_or("name unavailable")),
        value
            .version
            .map_or_else(|| "unknown".into(), |v| v.to_string())
    );
    let _ = writeln!(
        out,
        "Run: {}\nRevision: {}\nSequence: {}",
        safe(&run.run_id),
        safe(run.revision_id.as_deref().unwrap_or("unknown")),
        run.sequence
    );
    let _ = writeln!(
        out,
        "Workflow outcome: {}",
        safe(run.terminal.as_deref().unwrap_or(&run.lifecycle))
    );
    let _ = writeln!(
        out,
        "Unresolved external outcomes: {}",
        run.uncertainty_count
    );
    for node in &run.nodes {
        let _ = writeln!(out, "  Step {}: {}", safe(&node.node_id), safe(&node.state));
        if let Some(attempt) = &node.latest_attempt {
            let _ = writeln!(
                out,
                "    Invocation: {}{}",
                safe(attempt.terminal.as_deref().unwrap_or(&attempt.state)),
                if attempt.uncertain {
                    " (uncertain)"
                } else {
                    ""
                }
            );
            if let Some(generation) = &attempt.model_generation {
                let _ = writeln!(
                    out,
                    "    Model finish: {}",
                    safe(generation.finish_reason.as_deref().unwrap_or("unknown"))
                );
            }
            if let Some(acceptance) = &attempt.result_acceptance {
                let _ = writeln!(
                    out,
                    "    Required result: {} ({})",
                    if acceptance.accepted {
                        "accepted"
                    } else {
                        "rejected"
                    },
                    safe(&acceptance.reason)
                );
            }
            if details {
                let _ = writeln!(
                    out,
                    "    Attempt: {}\n    Model/profile: {} / {}",
                    safe(&attempt.attempt_id),
                    safe(attempt.capability_id.as_deref().unwrap_or("unknown")),
                    safe(attempt.provider_profile.as_deref().unwrap_or("none"))
                );
                let _ = writeln!(
                    out,
                    "    Limits: {}",
                    serde_json::to_string(&attempt.model_generation).unwrap_or_default()
                );
                let _ = writeln!(
                    out,
                    "    Usage: {}",
                    attempt.usage.as_ref().map_or_else(
                        || "unknown".into(),
                        |usage| serde_json::to_string(usage).unwrap_or_default()
                    )
                );
                let _ = writeln!(
                    out,
                    "    Selected inputs: {}",
                    serde_json::to_string(&attempt.context).unwrap_or_default()
                );
                if let Some(detail) = &attempt.terminal_detail {
                    let _ = writeln!(out, "    Evidence: {}", safe(detail));
                }
            }
        } else if node.latest_attempt_id.is_some() {
            let _ = writeln!(
                out,
                "    Attempt evidence unavailable under this read scope"
            );
        }
    }
    if value.outputs.is_empty() {
        let _ = writeln!(
            out,
            "Final output: unavailable (no readable successful terminal output)"
        );
    }
    for output in &value.outputs {
        let _ = writeln!(
            out,
            "Final output {}: {} ({} bytes)",
            safe(&output.name),
            safe(&output.artifact.artifact_id),
            output.artifact.size
        );
        if let Some(preview) = &output.preview {
            let _ = writeln!(out, "{}", safe(preview));
        }
        if output.preview_truncated {
            let _ = writeln!(out, "[preview truncated; download the complete output]");
        }
        let _ = writeln!(
            out,
            "Download: run result {} --field {} --output NEW_FILE",
            safe(&run.run_id),
            safe(&output.name)
        );
    }
    if value.outputs_restricted {
        let _ = writeln!(out, "Some output is outside your read permissions.");
    }
    if value.truncated {
        let _ = writeln!(
            out,
            "View bounded; use run show, exact attempt reads or paged timeline for additional evidence."
        );
    }
    if run.terminal.is_some() {
        let _ = writeln!(
            out,
            "This run has ended; further work requires an explicitly linked new run."
        );
    }
    let _ = writeln!(
        out,
        "Permitted next operations: {}",
        if value.actions.is_empty() {
            "none in this view".into()
        } else {
            value.actions.join(", ")
        }
    );
    out
}

#[cfg(test)]
mod tests {
    #[test]
    fn terminal_text_is_data() {
        assert_eq!(
            super::safe("notes\n\u{1b}[31m\r\u{202e}"),
            "notes\n\\u{1b}[31m\\u{d}\\u{202e}"
        );
    }
}
