//! File state is an unfinished edit batch, never an executable workflow or a model selection rule.

use crate::{
    error::CliError,
    session::CliSession,
    workflow_args::{WorkflowArgs, WorkflowCommand},
};
use milkdrift_control_protocol::{
    BlueprintDraft, BlueprintEdit, Command, MAX_DOCUMENT_BYTES, ModelInputSource,
};
use serde_json::json;
use std::path::Path;

mod file;
use file::DraftFile;

pub(super) async fn execute(session: &CliSession, args: &WorkflowArgs) -> Result<(), CliError> {
    let command = &args.command;
    match command {
        WorkflowCommand::List {
            workflow,
            limit,
            cursor,
        } => {
            let page = CliSession::page_request(*limit, cursor.as_deref())?;
            return session.output(
                "workflow.list",
                &session
                    .client()
                    .revisions(workflow.as_deref(), &page)
                    .await?,
            );
        }
        WorkflowCommand::Show { revision } => {
            let mut read = session.client().revision(revision).await?;
            read.document = None;
            return session.output("workflow.show", &read);
        }
        WorkflowCommand::Copy {
            revision,
            workflow,
            name,
            file,
        } => {
            let destination = DraftFile::open(file, true, args.expected_edit.as_deref())?;
            let request = session.command_request_with_revision(
                Command::CopyBlueprint {
                    source_revision: revision.clone(),
                    workflow_id: workflow.clone(),
                    name: name.clone(),
                },
                revision,
            )?;
            let copied = session.client().submit(&request).await?;
            let draft: BlueprintDraft = serde_json::from_value(
                copied
                    .value
                    .get("draft")
                    .ok_or_else(|| CliError::Internal("response has no draft".into()))?
                    .clone(),
            )
            .map_err(|error| CliError::Internal(error.to_string()))?;
            let token = destination.replace(&draft)?;
            return session.output("workflow.copy", &json!({"file":file,"edit_token":token,"copied_from":revision,"workflow_id":workflow,"revision_id":draft.base_revision}));
        }
        _ => {}
    }
    if matches!(command, WorkflowCommand::Models) {
        let models = session
            .client()
            .capabilities()
            .await?
            .into_iter()
            .filter(|value| {
                value.current
                    && !value.draining
                    && value.operations.iter().any(|op| op == "model.generate")
            })
            .collect::<Vec<_>>();
        return session.output("workflow.models", &models);
    }
    let path = path(command)?;
    let creating = matches!(
        command,
        WorkflowCommand::New { .. } | WorkflowCommand::Open { .. }
    );
    let file = DraftFile::open(path, creating, args.expected_edit.as_deref())?;
    let draft = match command {
        WorkflowCommand::New { workflow, .. } => BlueprintDraft {
            workflow_id: workflow.clone(),
            base_revision: None,
            mutations: vec![],
        },
        WorkflowCommand::Open { revision, .. } => {
            let read = session.client().revision(revision).await?;
            BlueprintDraft {
                workflow_id: read.summary.workflow_id,
                base_revision: Some(revision.clone()),
                mutations: vec![],
            }
        }
        _ => file.draft()?,
    };
    let edit = edit(session, command).await?;
    let save = matches!(command, WorkflowCommand::Save { .. });
    let operation = Command::AuthorBlueprint {
        draft: draft.clone(),
        edit,
        save,
    };
    let request = match &draft.base_revision {
        Some(revision) => session.command_request_with_revision(operation, revision)?,
        None => session.command_request(operation)?,
    };
    let accepted = session.client().submit(&request).await?;
    let returned: BlueprintDraft = serde_json::from_value(
        accepted
            .value
            .get("draft")
            .ok_or_else(|| CliError::Internal("response has no draft".into()))?
            .clone(),
    )
    .map_err(|error| CliError::Internal(error.to_string()))?;
    let token = if matches!(command, WorkflowCommand::Inspect { .. }) {
        file.token()
    } else {
        file.replace(&returned)?
    };
    session.output("workflow.author", &json!({"file":path,"edit_token":token,"base_revision":returned.base_revision,"pending_mutations":returned.mutations.len(),"revision_id":accepted.value.get("revision_id").ok_or_else(|| CliError::Internal("response has no revision_id".into()))?,"saved":save,"workflow":accepted.value.get("workflow").ok_or_else(|| CliError::Internal("response has no workflow".into()))?}))
}

fn path(command: &WorkflowCommand) -> Result<&Path, CliError> {
    Ok(match command {
        WorkflowCommand::New { file, .. }
        | WorkflowCommand::Open { file, .. }
        | WorkflowCommand::Inspect { file }
        | WorkflowCommand::Add { file, .. }
        | WorkflowCommand::Prompt { file, .. }
        | WorkflowCommand::OutputLimit { file, .. }
        | WorkflowCommand::Model { file, .. }
        | WorkflowCommand::Input { file, .. }
        | WorkflowCommand::RemoveInput { file, .. }
        | WorkflowCommand::RenameInput { file, .. }
        | WorkflowCommand::Connect { file, .. }
        | WorkflowCommand::Disconnect { file, .. }
        | WorkflowCommand::Output { file, .. }
        | WorkflowCommand::ClearOutput { file }
        | WorkflowCommand::Remove { file, .. }
        | WorkflowCommand::Move { file, .. }
        | WorkflowCommand::Rename { file, .. }
        | WorkflowCommand::Save { file } => file,
        WorkflowCommand::Models
        | WorkflowCommand::List { .. }
        | WorkflowCommand::Show { .. }
        | WorkflowCommand::Copy { .. } => {
            return Err(CliError::Internal("model listing has no draft".to_owned()));
        }
    })
}
async fn text(session: &CliSession, path: &Path) -> Result<String, CliError> {
    String::from_utf8(
        session
            .read_bounded(path, MAX_DOCUMENT_BYTES, "prompt")
            .await?,
    )
    .map_err(|_| CliError::Invalid("prompt must be UTF-8".to_owned()))
}
async fn edit(
    session: &CliSession,
    command: &WorkflowCommand,
) -> Result<Option<BlueprintEdit>, CliError> {
    Ok(Some(match command {
        WorkflowCommand::New { name, .. } | WorkflowCommand::Rename { name, .. } => {
            BlueprintEdit::Rename { name: name.clone() }
        }
        WorkflowCommand::Add {
            step,
            model,
            prompt,
            maximum_output_units,
            ..
        } => BlueprintEdit::AddModel {
            step: step.clone(),
            capability: model.clone(),
            prompt: text(session, prompt).await?,
            maximum_output_units: *maximum_output_units,
        },
        WorkflowCommand::Prompt { step, prompt, .. } => BlueprintEdit::Prompt {
            step: step.clone(),
            prompt: text(session, prompt).await?,
        },
        WorkflowCommand::OutputLimit {
            step,
            maximum_output_units,
            ..
        } => BlueprintEdit::OutputLimit {
            step: step.clone(),
            maximum_output_units: *maximum_output_units,
        },
        WorkflowCommand::Model { step, model, .. } => BlueprintEdit::Model {
            step: step.clone(),
            capability: model.clone(),
        },
        WorkflowCommand::Input { name, .. } => BlueprintEdit::Input { name: name.clone() },
        WorkflowCommand::RemoveInput { name, .. } => {
            BlueprintEdit::RemoveInput { name: name.clone() }
        }
        WorkflowCommand::RenameInput { name, new_name, .. } => BlueprintEdit::RenameInput {
            name: name.clone(),
            new_name: new_name.clone(),
        },
        WorkflowCommand::Connect {
            step,
            input,
            run_input,
            from_step,
            ..
        } => BlueprintEdit::Connect {
            step: step.clone(),
            input: input.clone(),
            source: match (run_input, from_step) {
                (Some(name), None) => ModelInputSource::RunInput { name: name.clone() },
                (None, Some(step)) => ModelInputSource::Step { step: step.clone() },
                _ => {
                    return Err(CliError::Invalid(
                        "select exactly one input source".to_owned(),
                    ));
                }
            },
        },
        WorkflowCommand::Disconnect { step, input, .. } => BlueprintEdit::Disconnect {
            step: step.clone(),
            input: input.clone(),
        },
        WorkflowCommand::Output { step, name, .. } => BlueprintEdit::Output {
            step: step.clone(),
            name: name.clone(),
        },
        WorkflowCommand::ClearOutput { .. } => BlueprintEdit::ClearOutput {},
        WorkflowCommand::Remove { step, .. } => BlueprintEdit::Remove { step: step.clone() },
        WorkflowCommand::Move { step, before, .. } => BlueprintEdit::Move {
            step: step.clone(),
            before: before.clone(),
        },
        _ => return Ok(None),
    }))
}
