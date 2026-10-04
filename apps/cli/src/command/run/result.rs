//! Formatting only: outcome, evidence, outputs and action hints come from the daemon.

use crate::{error::CliError, session::CliSession};
use milkdrift_control_protocol::RunResultRead;
use std::path::Path;

pub(super) async fn execute(
    session: &CliSession,
    run: &str,
    details: bool,
    field: Option<&str>,
    output: Option<&Path>,
) -> Result<(), CliError> {
    let value = session.client().run_result(run).await?;
    let download = if let (Some(field), Some(output)) = (field, output) {
        let result = value.outputs.iter().find(|value| value.name == field)
            .ok_or_else(|| CliError::Invalid("declared final output is unavailable; inspect the run outcome and read permissions".into()))?;
        Some(
            crate::command::artifact::download(session, &result.artifact.artifact_id, output)
                .await?,
        )
    } else {
        None
    };
    if session.cli().json {
        let mut result =
            serde_json::to_value(&value).map_err(|error| CliError::Internal(error.to_string()))?;
        if let Some(download) = download {
            result
                .as_object_mut()
                .ok_or_else(|| CliError::Internal("run result is not an object".into()))?
                .insert("download".into(), download);
        }
        session.output("run.result", &result)?;
    } else {
        render(&mut std::io::stdout().lock(), &value, details)?;
        if let Some(download) = download {
            session.output("artifact.get", &download)?;
        }
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

fn render(
    out: &mut impl std::io::Write,
    value: &RunResultRead,
    details: bool,
) -> Result<(), CliError> {
    let run = &value.run;
    writeln!(
        out,
        "Workflow: {} (version {})",
        safe(value.workflow_name.as_deref().unwrap_or("name unavailable")),
        value
            .version
            .map_or_else(|| "unknown".into(), |v| v.to_string())
    )?;
    writeln!(
        out,
        "Run: {}\nRevision: {}\nSequence: {}",
        safe(&run.run_id),
        safe(run.revision_id.as_deref().unwrap_or("unknown")),
        run.sequence
    )?;
    writeln!(
        out,
        "Workflow outcome: {}",
        safe(run.terminal.as_deref().unwrap_or(&run.lifecycle))
    )?;
    writeln!(
        out,
        "Unresolved external outcomes: {}",
        run.uncertainty_count
    )?;
    for node in &run.nodes {
        writeln!(out, "  Step {}: {}", safe(&node.node_id), safe(&node.state))?;
        if let Some(attempt) = &node.latest_attempt {
            writeln!(
                out,
                "    Invocation: {}{}",
                safe(attempt.terminal.as_deref().unwrap_or(&attempt.state)),
                if attempt.uncertain {
                    " (uncertain)"
                } else {
                    ""
                }
            )?;
            if let Some(generation) = &attempt.model_generation {
                writeln!(
                    out,
                    "    Model finish: {}",
                    safe(generation.finish_reason.as_deref().unwrap_or("unknown"))
                )?;
            }
            if let Some(acceptance) = &attempt.result_acceptance {
                writeln!(
                    out,
                    "    Required result: {} ({})",
                    if acceptance.accepted {
                        "accepted"
                    } else {
                        "rejected"
                    },
                    safe(&acceptance.reason)
                )?;
            }
            if details {
                writeln!(
                    out,
                    "    Attempt: {}\n    Model/profile: {} / {}",
                    safe(&attempt.attempt_id),
                    safe(attempt.capability_id.as_deref().unwrap_or("unknown")),
                    safe(attempt.provider_profile.as_deref().unwrap_or("none"))
                )?;
                writeln!(
                    out,
                    "    Limits: {}",
                    safe(
                        &serde_json::to_string(&attempt.model_generation)
                            .map_err(|error| CliError::Internal(error.to_string()))?
                    )
                )?;
                writeln!(
                    out,
                    "    Usage: {}",
                    match &attempt.usage {
                        None => "unknown".into(),
                        Some(usage) => safe(
                            &serde_json::to_string(usage)
                                .map_err(|error| CliError::Internal(error.to_string()))?
                        ),
                    }
                )?;
                writeln!(
                    out,
                    "    Selected inputs: {}",
                    safe(
                        &serde_json::to_string(&attempt.context)
                            .map_err(|error| CliError::Internal(error.to_string()))?
                    )
                )?;
                if let Some(detail) = &attempt.terminal_detail {
                    writeln!(out, "    Evidence: {}", safe(detail))?;
                }
            }
        } else if node.latest_attempt_id.is_some() {
            writeln!(
                out,
                "    Attempt evidence unavailable under this read scope"
            )?;
        }
    }
    if value.outputs.is_empty() {
        writeln!(
            out,
            "Final output: unavailable (no readable successful terminal output)"
        )?;
    }
    for output in &value.outputs {
        writeln!(
            out,
            "Final output {}: {} ({} bytes)",
            safe(&output.name),
            safe(&output.artifact.artifact_id),
            output.artifact.size
        )?;
        if let Some(preview) = &output.preview {
            writeln!(out, "{}", safe(preview))?;
        }
        if output.preview_truncated {
            writeln!(out, "[preview truncated; download the complete output]")?;
        }
        writeln!(
            out,
            "Download: run result {} --field {} --output NEW_FILE",
            safe(&run.run_id),
            safe(&output.name)
        )?;
    }
    if value.outputs_restricted {
        writeln!(out, "Some output is outside your read permissions.")?;
    }
    if value.truncated {
        writeln!(
            out,
            "View bounded; use run show, exact attempt reads or paged timeline for additional evidence."
        )?;
    }
    if run.terminal.is_some() {
        writeln!(
            out,
            "This run has ended; further work requires an explicitly linked new run."
        )?;
    }
    if value
        .actions
        .iter()
        .any(|action| action == "start_linked_run")
    {
        writeln!(
            out,
            "New run: --evidence recovery_observation={} run start NEW_RUN {} {} --request-file NEW_REQUEST_FILE (supply required inputs; admission rechecks authority)",
            safe(&run.run_id),
            safe(run.workflow_id.as_deref().unwrap_or("WORKFLOW")),
            safe(run.revision_id.as_deref().unwrap_or("REVISION"))
        )?;
    }
    writeln!(
        out,
        "Permitted next operations: {}",
        if value.actions.is_empty() {
            "none in this view".into()
        } else {
            safe(&value.actions.join(", "))
        }
    )?;
    out.flush()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn result_rendering_propagates_output_failure() -> Result<(), Box<dyn std::error::Error>> {
        let value = serde_json::from_value(serde_json::json!({
            "run": {
                "run_id": "output-fixture", "sequence": 10, "lifecycle": "running",
                "terminal": null, "workflow_id": "workflow", "revision_id": "revision",
                "semantic_digest": null, "nodes": [], "governing_agreement": null,
                "agreement_adoptions": 0, "uncertainty_count": 1
            },
            "workflow_name": "Readout", "version": 2, "truncated": false,
            "outputs": [], "outputs_restricted": true, "actions": []
        }))?;
        let mut output = Vec::new();
        super::render(&mut output, &value, true)?;
        let text = String::from_utf8(output)?;
        assert!(text.contains("Workflow: Readout (version 2)"));
        assert!(text.contains("Unresolved external outcomes: 1"));
        assert!(text.contains("Some output is outside your read permissions."));
        let mut full = std::io::Cursor::new([0_u8; 0]);
        let error = super::render(&mut full, &value, true)
            .err()
            .ok_or("full output accepted")?;
        assert_eq!(crate::error::exit_code(&error), 9);
        Ok(())
    }

    #[test]
    fn terminal_text_is_data() {
        assert_eq!(
            super::safe("notes\n\u{1b}[31m\r\u{202e}"),
            "notes\n\\u{1b}[31m\\u{d}\\u{202e}"
        );
    }
}
