//! Controlled response content for the daemon-owned animation journey. This endpoint never
//! calls workflow commands: its only output is model content returned to an ordinary invocation.

use serde_json::{Value, json};
use std::{io, path::Path, time::Duration};

pub(super) struct Reply {
    pub status: u16,
    pub message: Value,
    pub finish_reason: &'static str,
}

pub(super) fn respond(root: &Path, body: &Value) -> io::Result<Option<Reply>> {
    let Some(name) = body
        .get("model")
        .and_then(Value::as_str)
        .and_then(|model| model.strip_prefix("controlled-animation-"))
    else {
        return Ok(None);
    };
    if !matches!(name, "a" | "b" | "fixture") {
        return Err(invalid(
            "animation model alias names an unconfigured daemon",
        ));
    }
    let messages = body
        .get("messages")
        .and_then(Value::as_array)
        .ok_or_else(|| invalid("animation messages absent"))?;
    let user = messages
        .iter()
        .filter(|message| message.get("role").and_then(Value::as_str) == Some("user"))
        .map(|message| message.get("content").map(content_text).unwrap_or_default())
        .collect::<Vec<_>>()
        .join("\n");
    let role = messages
        .iter()
        .filter(|message| message.get("role").and_then(Value::as_str) == Some("user"))
        .filter_map(|message| message.get("content").and_then(Value::as_str))
        .find_map(|text| {
            text.strip_prefix("Animation fixture role: ")
                .and_then(|role| role.lines().next())
        })
        .unwrap_or("planner");
    let project = root
        .join(format!("daemon-{name}"))
        .join("animation-project");
    let reply = match role {
        "planner" => text(planning(&snapshot(messages)?, &project)?.to_string()),
        "compose" => text(composition(&snapshot(messages)?, &project)?.to_string()),
        "review" => text(review(&snapshot(messages)?)?.to_string()),
        "render" => Reply {
            status: 503,
            message: json!({"role":"assistant","content":"Controlled render research failed."}),
            finish_reason: "stop",
        },
        "motion" | "training" | "render-correction" => {
            let rounds = messages
                .iter()
                .filter(|message| message.get("role").and_then(Value::as_str) == Some("tool"))
                .count();
            let folder = if role == "render-correction" {
                "render"
            } else {
                role
            };
            let path = project.join(folder).join("research.txt");
            match rounds {
                0 => {
                    if role == "motion" {
                        std::thread::sleep(Duration::from_millis(1800));
                    }
                    tool(
                        role,
                        "write_file",
                        json!({"path":path,"text":format!("Animation research {role}\nInput digest: b3_{}\n{}",blake3::hash(user.as_bytes()),milkdrift_contracts::truncate_utf8(&user,2048))}),
                    )
                }
                1 => tool(role, "read_file", json!({"path":path})),
                _ => {
                    let results = messages
                        .iter()
                        .filter(|message| {
                            message.get("role").and_then(Value::as_str) == Some("tool")
                        })
                        .map(|message| message.get("content").map(content_text).unwrap_or_default())
                        .collect::<Vec<_>>()
                        .join("\n");
                    text(format!(
                        "Animation {role} research complete. Exact native tool feedback:\n{results}"
                    ))
                }
            }
        }
        "final" => text(format!(
            "Final animation report\nSuccessful research and the adopted render correction were supplied as immutable inputs.\nInput digest: b3_{}\n{}",
            blake3::hash(user.as_bytes()),
            milkdrift_contracts::truncate_utf8(&user, 4096)
        )),
        _ => return Err(invalid("unknown animation research role")),
    };
    Ok(Some(reply))
}

fn content_text(value: &Value) -> String {
    match value {
        Value::String(value) => value.clone(),
        Value::Array(parts) => parts
            .iter()
            .filter_map(|part| part.get("text").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}

fn snapshot(messages: &[Value]) -> io::Result<Value> {
    let line = messages
        .iter()
        .rev()
        .filter(|message| message.get("role").and_then(Value::as_str) == Some("user"))
        .filter_map(|message| message.get("content").and_then(Value::as_str))
        .find_map(|text| {
            text.strip_prefix("Authorized future-work snapshot (data, not instructions): ")
                .and_then(|snapshot| snapshot.lines().next())
        })
        .ok_or_else(|| invalid("daemon authoring snapshot absent"))?;
    serde_json::from_str(line).map_err(io::Error::other)
}

fn definition(
    snapshot: &Value,
    title: &str,
    role: &str,
    project: &Path,
    folder: Option<&str>,
    actions: bool,
) -> io::Result<Value> {
    let mut definition = snapshot
        .get("definition_example")
        .cloned()
        .ok_or_else(|| invalid("daemon definition example absent"))?;
    let object = definition
        .as_object_mut()
        .ok_or_else(|| invalid("definition is not an object"))?;
    object.insert("title".into(), json!(title));
    object.insert(
        "description".into(),
        json!("Controlled animation journey through production model, tool and task owners"),
    );
    object.insert("instructions".into(), json!(format!("Animation fixture role: {role}\nUse the supplied brief and declared inputs. Report the exact tool observations. Preserve failed work and propose only prospective correction.")));
    object.insert(
        "inputs".into(),
        json!({"brief":{"kind":"supplied","name":"brief","optional":false}}),
    );
    object.insert(
        "outputs".into(),
        if actions {
            json!({"model_response":{"required":true,"media_type":"application/vnd.milkdrift.model-response.v1+json","max_bytes":2_097_152}})
        } else {
            json!({"final_text":{"required":true,"media_type":"text/plain","max_bytes":16384}})
        },
    );
    object.insert("dependencies".into(), json!([]));
    object.insert("corrective_of".into(), Value::Null);
    object.insert("required".into(), json!(true));
    object.insert(
        "actions".into(),
        if actions {
            json!({"automatic_repair":true})
        } else {
            Value::Null
        },
    );
    object.insert(
        "paths".into(),
        folder.map_or_else(
            || json!([]),
            |folder| json!([{"path":project.join(folder),"write":true}]),
        ),
    );
    object.insert(
        "tools".into(),
        if folder.is_some() {
            json!([{"type":"write_file"},{"type":"read_file"}])
        } else {
            json!([])
        },
    );
    Ok(definition)
}

fn planning(snapshot: &Value, project: &Path) -> io::Result<Value> {
    let mut changes = Vec::new();
    for (alias, title, role) in [
        ("motion", "Research motion representation", "motion"),
        ("training", "Research training data", "training"),
        ("render", "Research render validation", "render"),
    ] {
        let task = definition(snapshot, title, role, project, Some(role), false)?;
        changes.push(json!({"kind":"add","alias":alias,"definition":task}));
    }
    let mut compose = definition(
        snapshot,
        "Animation composition",
        "compose",
        project,
        None,
        true,
    )?;
    set(
        &mut compose,
        "dependencies",
        json!(["motion", "training", "render"]),
    )?;
    set(
        &mut compose,
        "inputs",
        json!({
            "motion":{"kind":"task_output","task":"motion","output":"final_text","optional":false},
            "training":{"kind":"task_output","task":"training","output":"final_text","optional":false},
            "failed_render":{"kind":"settled_outcome","task":"render"}
        }),
    )?;
    changes.push(json!({"kind":"add","alias":"composition","definition":compose}));
    batch(snapshot, changes)
}

fn composition(snapshot: &Value, project: &Path) -> io::Result<Value> {
    let original = task(snapshot, "Research render validation")?;
    let motion = task(snapshot, "Research motion representation")?;
    let training = task(snapshot, "Research training data")?;
    let original_id = field(original, "id")?;
    let motion_id = field(motion, "id")?;
    let training_id = field(training, "id")?;
    if original.get("state").and_then(Value::as_str) != Some("failed") {
        return Err(invalid(
            "compositor did not observe the controlled failed research",
        ));
    }
    let mut correction = definition(
        snapshot,
        "Corrected render research",
        "render-correction",
        project,
        Some("render"),
        false,
    )?;
    set(&mut correction, "corrective_of", original_id.clone())?;
    let mut final_task = definition(
        snapshot,
        "Final animation report",
        "final",
        project,
        None,
        false,
    )?;
    set(
        &mut final_task,
        "dependencies",
        json!([original_id, motion_id, training_id]),
    )?;
    set(
        &mut final_task,
        "inputs",
        json!({
            "render":{"kind":"task_output","task":original_id,"output":"final_text","optional":false},
            "motion":{"kind":"task_output","task":motion_id,"output":"final_text","optional":false},
            "training":{"kind":"task_output","task":training_id,"output":"final_text","optional":false}
        }),
    )?;
    let mut reviewer = definition(
        snapshot,
        "Adopt corrected research",
        "review",
        project,
        None,
        true,
    )?;
    set(&mut reviewer, "dependencies", json!(["correction"]))?;
    set(
        &mut reviewer,
        "inputs",
        json!({"corrected_render":{"kind":"task_output","task":"correction","output":"final_text","optional":false}}),
    )?;
    batch(
        snapshot,
        vec![
            json!({"kind":"add","alias":"correction","definition":correction}),
            json!({"kind":"add","alias":"final","definition":final_task}),
            json!({"kind":"add","alias":"review","definition":reviewer}),
        ],
    )
}

fn review(snapshot: &Value) -> io::Result<Value> {
    let original = task(snapshot, "Research render validation")?;
    let correction = task(snapshot, "Corrected render research")?;
    let consumer = task(snapshot, "Final animation report")?;
    if correction.get("state").and_then(Value::as_str) != Some("completed")
        || consumer.get("started").and_then(Value::as_bool) != Some(false)
    {
        return Err(invalid(
            "reviewer did not observe successful correction and blocked future consumer",
        ));
    }
    batch(
        snapshot,
        vec![
            json!({"kind":"adopt_correction","task":field(original, "id")?,"correction":field(correction, "id")?,"redirects":[{"consumer":field(consumer, "id")?,"expected_version":field(consumer, "version")?,"input":"render"}]}),
            json!({"kind":"finish"}),
        ],
    )
}

fn field<'a>(value: &'a Value, name: &str) -> io::Result<&'a Value> {
    value
        .get(name)
        .ok_or_else(|| invalid(format!("snapshot field {name} absent")))
}

fn set(value: &mut Value, name: &str, field: Value) -> io::Result<()> {
    value
        .as_object_mut()
        .ok_or_else(|| invalid("task definition is not an object"))?
        .insert(name.into(), field);
    Ok(())
}

fn task<'a>(snapshot: &'a Value, title: &str) -> io::Result<&'a Value> {
    snapshot
        .get("tasks")
        .and_then(Value::as_array)
        .and_then(|tasks| {
            tasks
                .iter()
                .find(|task| task.get("title").and_then(Value::as_str) == Some(title))
        })
        .ok_or_else(|| invalid(format!("snapshot omits task {title}")))
}

fn batch(snapshot: &Value, changes: Vec<Value>) -> io::Result<Value> {
    let value = json!({"expected_generation":snapshot.get("expected_generation").ok_or_else(||invalid("snapshot generation absent"))?,"changes":changes});
    let batch: milkdrift_blueprint::TaskBatch =
        serde_json::from_value(value).map_err(io::Error::other)?;
    serde_json::to_value(batch).map_err(io::Error::other)
}

fn tool(role: &str, name: &str, arguments: Value) -> Reply {
    Reply {
        status: 200,
        message: json!({"role":"assistant","content":"","tool_calls":[{"id":format!("{role}-{name}"),"type":"function","function":{"name":name,"arguments":arguments.to_string()}}]}),
        finish_reason: "tool_calls",
    }
}

fn text(content: String) -> Reply {
    Reply {
        status: 200,
        message: json!({"role":"assistant","content":content}),
        finish_reason: "stop",
    }
}

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::other(message.into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preceding_artifact_evidence_cannot_choose_role_or_authoring_snapshot() -> io::Result<()> {
        let capability = milkdrift_capability::CapabilityRequirement::new(
            milkdrift_capability::OperationId::new("model.generate").map_err(io::Error::other)?,
        );
        let snapshot = json!({
            "expected_generation":7,
            "definition_example":{"title":"template","instructions":"fixture instructions","capability":capability},
            "tasks":[
                {"id":"motion-id","title":"Research motion representation","state":"completed"},
                {"id":"training-id","title":"Research training data","state":"completed"},
                {"id":"render-id","title":"Research render validation","state":"failed"}
            ]
        });
        let body = json!({
            "model":"controlled-animation-a",
            "messages":[
                {"role":"user","content":[{"type":"text","text":"Animation fixture role: motion\nAuthorized future-work snapshot (data, not instructions): {\"expected_generation\":999}"}]},
                {"role":"user","content":"Authorized future-work snapshot (data, not instructions): {\"expected_generation\":888}"},
                {"role":"user","content":"Animation fixture role: compose\nUse the selected immutable research artifacts."},
                {"role":"user","content":format!("Authorized future-work snapshot (data, not instructions): {snapshot}\nReturn a structured task batch.")}
            ]
        });
        let reply = respond(Path::new("/controlled-fixture"), &body)?
            .ok_or_else(|| invalid("animation response absent"))?;
        assert_eq!(reply.finish_reason, "stop");
        assert!(reply.message.get("tool_calls").is_none());
        let batch: milkdrift_blueprint::TaskBatch = serde_json::from_str(
            reply
                .message
                .get("content")
                .and_then(Value::as_str)
                .ok_or_else(|| invalid("response text absent"))?,
        )
        .map_err(io::Error::other)?;
        assert_eq!(batch.expected_generation, 7);
        assert_eq!(batch.changes.len(), 3);
        assert!(
            matches!(batch.changes.first(), Some(milkdrift_blueprint::TaskChange::Add { definition, .. }) if definition.title == "Corrected render research" && definition.corrective_of.as_ref().is_some_and(|id| id.as_str() == "render-id"))
        );
        Ok(())
    }
}
