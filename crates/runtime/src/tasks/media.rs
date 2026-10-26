//! Admission uses adapter-owned media rules; adapters still validate bytes at entry.

use super::{TaskRuntime, WorkflowRecord};
use crate::RuntimeError;
use milkdrift_blueprint::{TaskDefinition, TaskInputBinding, TaskInputValue};
use milkdrift_capability::TaskAuthoringContract;

impl TaskRuntime {
    pub(super) fn validate_input_media(
        record: &WorkflowRecord,
        definition: &TaskDefinition,
        contract: &TaskAuthoringContract,
    ) -> Result<(), RuntimeError> {
        for (name, binding) in &definition.inputs {
            let port = contract.inputs.iter().find(|port| &port.name == name);
            if port.is_none() && !contract.allows_additional_inputs {
                // Existing direct callers may retain context an operation does not consume.
                // Authoring offers only real ports; absence does not introduce a blanket
                // refusal of an otherwise valid accepted context binding.
                continue;
            }
            let facts = match binding {
                TaskInputBinding::Inline { value } => value_facts(value)?,
                TaskInputBinding::Supplied { name, .. } => record
                    .inputs
                    .get(name)
                    .map(value_facts)
                    .transpose()?
                    .flatten(),
                TaskInputBinding::SettledOutcome { .. } => {
                    Some((false, Some("application/json".into()), None))
                }
                TaskInputBinding::TaskOutput { task, output, .. } => {
                    let producer = record.tasks.get(task);
                    if let Some(reference) = producer
                        .and_then(|task| task.result.as_ref())
                        .and_then(|result| result.outputs.get(output))
                    {
                        Some((
                            true,
                            reference.media_type().map(str::to_owned),
                            reference.size_bytes(),
                        ))
                    } else {
                        // A promise bounds possible output, not its actual length. Do not reject
                        // a valid future small result merely because its producer allows more.
                        Some((
                            true,
                            producer
                                .and_then(|task| task.definition.outputs.get(output))
                                .and_then(|port| port.media_type.clone()),
                            None,
                        ))
                    }
                }
            };
            let Some((artifact, media, bytes)) = facts else {
                continue;
            };
            if artifact && port.is_some_and(|port| !port.accepts_artifacts) {
                return Err(invalid(
                    name,
                    "this executor input accepts inline content, not a result artifact",
                ));
            }
            if let Some(media) = media
                && (!contract.input_media.accepts(&media)
                    || port
                        .and_then(|port| port.media_type.as_ref())
                        .is_some_and(|expected| expected != &media))
            {
                return Err(invalid(
                    name,
                    &format!("the selected executor cannot interpret {media} here"),
                ));
            }
            if let Some(bytes) = bytes
                && port.is_some_and(|port| bytes > port.max_bytes)
            {
                return Err(invalid(
                    name,
                    "the captured content exceeds this executor input's size limit",
                ));
            }
        }
        Ok(())
    }

    pub(super) fn validate_known_media(
        &self,
        record: &WorkflowRecord,
        changed: &std::collections::BTreeSet<milkdrift_blueprint::TaskId>,
        new_definitions: &std::collections::BTreeSet<milkdrift_blueprint::TaskId>,
    ) -> Result<(), RuntimeError> {
        for task in record
            .tasks
            .values()
            .filter(|task| changed.contains(&task.id) && !task.removed && task.frozen.is_none())
        {
            if new_definitions.contains(&task.id) {
                self.validate_future_definition(record, &task.definition)?;
            } else if let Some((_, contract)) = self.task_authoring(&task.definition.capability)? {
                // Supplying content does not replace the accepted executor or require that it
                // is presently selectable. Validate known constraints without making input
                // delivery depend on transient model availability.
                Self::validate_input_media(record, &task.definition, &contract)?;
            }
        }
        Ok(())
    }
}

type InputFacts = (bool, Option<String>, Option<u64>);

fn value_facts(value: &TaskInputValue) -> Result<Option<InputFacts>, RuntimeError> {
    Ok(match value {
        TaskInputValue::Text { text } | TaskInputValue::TextFile { text, .. } => {
            Some((false, Some("text/plain".into()), Some(text.len() as u64)))
        }
        TaskInputValue::Json { value } => Some((
            false,
            Some("application/json".into()),
            Some(serde_json::to_vec(value)?.len() as u64),
        )),
        TaskInputValue::Artifact { reference } => Some((
            true,
            reference.media_type().map(str::to_owned),
            reference.size_bytes(),
        )),
        TaskInputValue::Absent => None,
    })
}

fn invalid(input: &str, reason: &str) -> RuntimeError {
    RuntimeError::InvalidTransition(format!("Input {input}: {reason}"))
}
