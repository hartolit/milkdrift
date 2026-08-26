//! Provider-neutral model contract fixtures and hostile-input tests.

use std::collections::BTreeMap;

use milkdrift_blueprint::{NodeId, RevisionId, TaskContextPolicy};
use milkdrift_capability::{BoundedJson, ExtensionKey};
use milkdrift_model::{
    ContentPart, ContextManifest, ContextManifestDocument, ContextTotals, Message, MessageRole,
    ModelResponse, ModelResponseDocument, ModelTaskRequest, ModelTaskRequestDocument,
    SessionSelection, ToolCall, ToolDefinition, Usage,
};
use milkdrift_persistence::{AttemptId, NodeExecutionId};
use milkdrift_workspace::RunId;
use serde_json::json;

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

fn request() -> TestResult<ModelTaskRequest> {
    Ok(ModelTaskRequest::new(
        vec![
            Message::new(
                MessageRole::System,
                vec![ContentPart::Text {
                    text: "answer using evidence".to_owned(),
                }],
                None,
            )?,
            Message::new(
                MessageRole::User,
                vec![ContentPart::Text {
                    text: "summarize".to_owned(),
                }],
                None,
            )?,
        ],
        vec![ToolDefinition::new(
            "lookup",
            "look up one exact item",
            BoundedJson::new(
                json!({"type":"object","properties":{"id":{"type":"string"}},"required":["id"]}),
            )?,
        )?],
        None,
        SessionSelection::Fresh,
        None,
        512,
        true,
        BTreeMap::new(),
    )?)
}

#[test]
fn model_task_document_is_canonical_and_rejects_hostile_shape() -> TestResult {
    let document = ModelTaskRequestDocument::new(request()?);
    let bytes = document.to_canonical_json()?;
    assert_eq!(
        bytes,
        include_bytes!("fixtures/model-task-request-v1.json").trim_ascii_end()
    );
    assert_eq!(ModelTaskRequestDocument::from_json(&bytes)?, document);
    assert!(
        ModelTaskRequestDocument::from_json(br#"{"schema_version":1,"schema_version":1}"#).is_err()
    );
    assert!(ModelTaskRequestDocument::from_json(br#"{"schema_version":2,"request":{}}"#).is_err());
    let deep = format!(
        "{{\"schema_version\":1,\"request\":{}}}",
        "[".repeat(60) + &"]".repeat(60)
    );
    assert!(ModelTaskRequestDocument::from_json(deep.as_bytes()).is_err());
    Ok(())
}

#[test]
fn context_manifest_has_exact_golden_bytes_and_verified_digest() -> TestResult {
    let policy = TaskContextPolicy::default();
    let revision: RevisionId = serde_json::from_value(json!(format!("rev_{}", "0".repeat(64))))?;
    let manifest = ContextManifest::new(
        RunId::new("run-model-contract")?,
        revision,
        NodeId::new("model")?,
        NodeExecutionId::new("execution-model")?,
        AttemptId::new("attempt-model")?,
        1,
        policy.digest()?,
        Vec::new(),
        Vec::new(),
        ContextTotals::default(),
        policy.budget(),
    )?;
    let document = ContextManifestDocument::new(manifest);
    let bytes = document.to_canonical_json()?;
    assert_eq!(
        bytes,
        include_bytes!("fixtures/context-manifest-v1.json").trim_ascii_end()
    );
    assert_eq!(ContextManifestDocument::from_json(&bytes)?, document);
    let mut tampered: serde_json::Value = serde_json::from_slice(&bytes)?;
    tampered["manifest"]["digest"] =
        json!("b3_0000000000000000000000000000000000000000000000000000000000000000");
    assert!(ContextManifestDocument::from_json(&serde_json::to_vec(&tampered)?).is_err());
    Ok(())
}

#[test]
fn tool_results_and_sessions_are_explicit() -> TestResult {
    assert!(
        Message::new(
            MessageRole::ToolResult,
            vec![ContentPart::Text {
                text: "result".to_owned()
            }],
            None,
        )
        .is_err()
    );
    assert!(
        Message::new(
            MessageRole::User,
            vec![ContentPart::Text {
                text: "result".to_owned()
            }],
            Some("call-1".to_owned()),
        )
        .is_err()
    );
    let mut value = serde_json::to_value(ModelTaskRequestDocument::new(request()?))?;
    value["request"]["session"] = json!({"type":"provider_managed","session_id":"bad session"});
    assert!(ModelTaskRequestDocument::from_json(&serde_json::to_vec(&value)?).is_err());
    Ok(())
}

#[test]
fn response_preserves_structured_tool_and_provider_data() -> TestResult {
    let call = ToolCall::new("call-1", "lookup", BoundedJson::new(json!({"id":"x"}))?)?;
    let response = ModelResponse::new(
        "{\"answer\":42}".to_owned(),
        Some(BoundedJson::new(json!({"answer":42}))?),
        vec![call],
        milkdrift_model::FinishReason::ToolCalls,
        Usage {
            input_units: Some(10),
            output_units: Some(5),
            cached_input_units: Some(2),
            cost_micros: None,
            currency: None,
        },
        BTreeMap::from([(
            ExtensionKey::new("org.example.provider/response")?,
            BoundedJson::new(json!({"request_id":"safe"}))?,
        )]),
    )?;
    let document = ModelResponseDocument::new(response);
    assert_eq!(
        ModelResponseDocument::from_json(&document.to_canonical_json()?)?,
        document
    );
    Ok(())
}
