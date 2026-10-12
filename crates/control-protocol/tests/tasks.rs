//! Shared browser/native wire fixtures, independent of task scheduling decisions.
use milkdrift_control_protocol::{
    ResponseEnvelope, decode_json, encode_json,
    tasks::{CreateWorkflowRequest, TaskSessionDto},
};

#[test]
fn browser_session_fixture_decodes_and_reencodes_exactly() -> Result<(), Box<dyn std::error::Error>>
{
    let bytes = include_bytes!("fixtures/task-session.json");
    let envelope: ResponseEnvelope<TaskSessionDto> = decode_json(bytes)?;
    envelope.protocol.negotiate()?;
    assert_eq!(envelope.value.daemon_id, "fixture-daemon-a");
    assert_eq!(envelope.value.limits.concurrent_tasks, 4);
    let expected: serde_json::Value = serde_json::from_slice(bytes)?;
    let actual: serde_json::Value = serde_json::from_slice(&encode_json(&envelope)?)?;
    assert_eq!(actual, expected);
    Ok(())
}

#[test]
fn task_transport_rejects_duplicate_and_unknown_request_fields() {
    let unknown = br#"{"request_id":"one","title":"Report","description":"","tasks":[],"inputs":{},"finish_when_settled":true,"limits":null,"actor":"forged"}"#;
    assert!(decode_json::<CreateWorkflowRequest>(unknown).is_err());
    let duplicate = br#"{"request_id":"one","request_id":"two","title":"Report","description":"","tasks":[],"inputs":{},"finish_when_settled":true,"limits":null}"#;
    assert!(decode_json::<CreateWorkflowRequest>(duplicate).is_err());
}

#[test]
fn tasks_are_required_unless_the_request_explicitly_waives_the_obligation()
-> Result<(), Box<dyn std::error::Error>> {
    let mut value = serde_json::json!({
        "title":"Analyze supplied project", "description":"", "instructions":"Inspect dependencies",
        "capability":{"id":"cargo-report", "operation":"process.execute"},
        "inputs":{}, "dependencies":[], "corrective_of":null, "paths":[]
    });
    let definition: milkdrift_control_protocol::tasks::TaskDefinitionDto =
        decode_json(&serde_json::to_vec(&value)?)?;
    assert!(definition.required);
    value["required"] = serde_json::Value::Bool(false);
    let optional: milkdrift_control_protocol::tasks::TaskDefinitionDto =
        decode_json(&serde_json::to_vec(&value)?)?;
    assert!(!optional.required);
    Ok(())
}
