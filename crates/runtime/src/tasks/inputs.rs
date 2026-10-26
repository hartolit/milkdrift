//! Readiness keeps typed absence separate from empty, but supplied, values.

use std::collections::BTreeMap;

use milkdrift_blueprint::{TaskInputBinding, TaskInputValue, TaskOrderCondition};

use super::{TaskRecord, TaskState, WorkflowRecord};

pub(super) fn resolve_inputs(
    workflow: &WorkflowRecord,
    task: &TaskRecord,
) -> Result<BTreeMap<String, TaskInputValue>, String> {
    if let Some(original) = &task.definition.corrective_of
        && workflow
            .tasks
            .get(original)
            .is_none_or(|value| !value.state.is_settled())
    {
        return Err(format!(
            "corrected task {original} has not settled; replacement effects cannot enter"
        ));
    }
    let legacy = task.definition.dependencies.is_legacy();
    for (dependency, condition) in task
        .definition
        .dependencies
        .conditions(&task.definition.inputs)
    {
        let producer = workflow
            .tasks
            .get(&dependency)
            .ok_or_else(|| format!("dependency {dependency} is absent"))?;
        let satisfied = match condition {
            TaskOrderCondition::Settled => producer.state.is_settled(),
            TaskOrderCondition::Success => {
                producer.state == TaskState::Completed
                    || !legacy
                        && producer
                            .adopted_correction
                            .as_ref()
                            .and_then(|id| workflow.tasks.get(id))
                            .is_some_and(|task| task.state == TaskState::Completed)
            }
        };
        if !satisfied {
            return Err(format!("dependency {dependency} is {:?}", producer.state));
        }
    }
    task.definition
        .inputs
        .iter()
        .map(|(name, binding)| {
            let value = match binding {
                TaskInputBinding::Inline { value } => value.clone(),
                TaskInputBinding::Supplied { name, optional } => {
                    supplied_input(workflow.inputs.get(name), name, *optional)?
                }
                TaskInputBinding::TaskOutput {
                    task,
                    output,
                    optional,
                } => {
                    let producer = workflow
                        .tasks
                        .get(task)
                        .ok_or_else(|| format!("producer {task} is absent"))?;
                    if !optional && producer.state != TaskState::Completed {
                        return Err(format!(
                            "required output {task}/{output} needs a successful producer; producer is {:?}",
                            producer.state
                        ));
                    }
                    if *optional && !producer.state.is_settled() {
                        return Err(format!("optional output {task}/{output} is awaiting producer settlement"));
                    }
                    if *optional && !legacy && producer.state != TaskState::Completed {
                        return Ok((name.clone(), TaskInputValue::Absent));
                    }
                    match producer
                        .result
                        .as_ref()
                        .and_then(|result| result.outputs.get(output))
                    {
                        Some(reference) => TaskInputValue::Artifact {
                            reference: reference.clone(),
                        },
                        None if *optional && producer.state.is_settled() => TaskInputValue::Absent,
                        None => return Err(format!("missing output {task}/{output}")),
                    }
                }
                TaskInputBinding::SettledOutcome { task } => {
                    let producer = workflow
                        .tasks
                        .get(task)
                        .ok_or_else(|| format!("producer {task} is absent"))?;
                    if !producer.state.is_settled() {
                        return Err(format!("producer {task} is not settled"));
                    }
                    TaskInputValue::Json {
                        value: serde_json::json!({"task":task,"state":producer.state,"result":producer.result}),
                    }
                }
            };
            Ok((name.clone(), value))
        })
        .collect()
}

fn supplied_input(
    value: Option<&TaskInputValue>,
    name: &str,
    optional: bool,
) -> Result<TaskInputValue, String> {
    match value {
        None | Some(TaskInputValue::Absent) if !optional => {
            Err(format!("missing supplied input {name}"))
        }
        Some(value) => Ok(value.clone()),
        None => Ok(TaskInputValue::Absent),
    }
}

#[cfg(test)]
mod tests {
    use super::supplied_input;
    use milkdrift_blueprint::TaskInputValue;

    #[test]
    fn required_absence_is_missing_but_empty_text_and_json_null_are_values() {
        for absent in [None, Some(TaskInputValue::Absent)] {
            assert_eq!(
                supplied_input(absent.as_ref(), "brief", false),
                Err("missing supplied input brief".to_owned())
            );
            assert_eq!(
                supplied_input(absent.as_ref(), "brief", true),
                Ok(TaskInputValue::Absent)
            );
        }
        for value in [
            TaskInputValue::Text {
                text: String::new(),
            },
            TaskInputValue::Json {
                value: serde_json::Value::Null,
            },
        ] {
            for optional in [false, true] {
                assert_eq!(
                    supplied_input(Some(&value), "brief", optional),
                    Ok(value.clone())
                );
            }
        }
    }
}
