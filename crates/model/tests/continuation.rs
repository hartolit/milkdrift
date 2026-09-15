//! Frozen history contracts, role isolation and complete tool-exchange requirements.
use milkdrift_capability::{ArtifactReference, BoundedJson};
use milkdrift_model::{
    ContentPart, ContinuationHistory, ContinuationHistoryDocument, ContinuationTurn, Message,
    MessageRole, ModelTaskRequest, SessionSelection, ToolCall,
};
use milkdrift_persistence::RunSequence;
use serde_json::json;
use std::collections::BTreeMap;

type TestResult = Result<(), Box<dyn std::error::Error>>;

fn message(
    role: MessageRole,
    text: &str,
    id: Option<&str>,
) -> Result<Message, milkdrift_model::ModelContractError> {
    Message::new(
        role,
        vec![ContentPart::Text {
            text: text.to_owned(),
        }],
        id.map(str::to_owned),
    )
}
fn turn() -> Result<ContinuationTurn, milkdrift_capability::ContractError> {
    Ok(ContinuationTurn {
        manifest: ArtifactReference::new(
            "manifest",
            "0".repeat(64),
            Some("application/vnd.milkdrift.context-manifest.v2+json".to_owned()),
            Some(1),
        )?,
        response: ArtifactReference::new(
            "response",
            "1".repeat(64),
            Some("application/vnd.milkdrift.model-response.v1+json".to_owned()),
            Some(1),
        )?,
        events: [1, 2, 3, 4, 5].map(RunSequence::new),
        message_end: 2,
    })
}
fn request(
    turn: &ContinuationTurn,
    messages: Vec<Message>,
) -> Result<ModelTaskRequest, milkdrift_model::ModelContractError> {
    ModelTaskRequest::new(
        messages,
        Vec::new(),
        None,
        SessionSelection::ExplicitContinuation {
            manifest: turn.manifest.clone(),
            response: turn.response.clone(),
        },
        None,
        32,
        false,
        BTreeMap::new(),
    )
}

#[test]
fn history_round_trip_and_exact_reference_binding() -> TestResult {
    let turn = turn()?;
    let history = ContinuationHistory::new(
        vec![turn.clone()],
        vec![
            message(MessageRole::User, "old", None)?,
            message(MessageRole::Assistant, "answer", None)?,
        ],
        "authority".to_owned(),
    )?;
    let document = ContinuationHistoryDocument::new(history.clone());
    assert_eq!(
        document.to_canonical_json()?,
        include_bytes!("fixtures/continuation-v1.json").trim_ascii_end()
    );
    let task = milkdrift_model::ModelTaskRequestDocument::new(request(
        &turn,
        vec![message(MessageRole::User, "new", None)?],
    )?);
    assert_eq!(
        task.to_canonical_json()?,
        include_bytes!("fixtures/model-task-continuation-v1.json").trim_ascii_end()
    );
    assert_eq!(
        milkdrift_model::ModelTaskRequestDocument::from_json(include_bytes!(
            "fixtures/model-task-continuation-v1.json"
        ))?,
        task
    );
    assert_eq!(
        ContinuationHistoryDocument::from_json(&document.to_canonical_json()?)?,
        document
    );
    let combined = request(
        &turn,
        vec![
            message(MessageRole::System, "current instructions", None)?,
            message(MessageRole::User, "new", None)?,
        ],
    )?
    .with_continuation(&history)?;
    assert_eq!(
        combined
            .messages()
            .iter()
            .map(Message::role)
            .collect::<Vec<_>>(),
        [
            MessageRole::System,
            MessageRole::User,
            MessageRole::Assistant,
            MessageRole::User
        ]
    );
    let mut wrong = turn;
    wrong.response = wrong.manifest.clone();
    assert!(
        request(&wrong, vec![message(MessageRole::User, "new", None)?])?
            .with_continuation(&history)
            .is_err()
    );
    Ok(())
}

#[test]
fn complete_tool_pairs_are_required_before_entry() -> TestResult {
    let turn = turn()?;
    let calls = vec![
        ToolCall::new("one", "lookup", BoundedJson::new(json!({"name":"one"}))?)?,
        ToolCall::new("two", "lookup", BoundedJson::new(json!({"name":"two"}))?)?,
    ];
    let history = ContinuationHistory::new(
        vec![turn.clone()],
        vec![
            message(MessageRole::User, "look up", None)?,
            message(MessageRole::Assistant, "", None)?.with_tool_calls(calls)?,
        ],
        "authority".to_owned(),
    )?;
    for ids in [
        vec![],
        vec!["one"],
        vec!["one", "one"],
        vec!["one", "unknown"],
    ] {
        let mut messages = ids
            .into_iter()
            .map(|id| message(MessageRole::ToolResult, "result", Some(id)))
            .collect::<Result<Vec<_>, _>>()?;
        messages.push(message(MessageRole::User, "continue", None)?);
        assert!(
            request(&turn, messages)?
                .with_continuation(&history)
                .is_err()
        );
    }
    let messages = vec![
        message(MessageRole::ToolResult, "second", Some("two"))?,
        message(MessageRole::ToolResult, "first", Some("one"))?,
        message(MessageRole::User, "continue", None)?,
    ];
    assert!(
        request(&turn, messages)?
            .with_continuation(&history)
            .is_ok()
    );
    Ok(())
}

#[test]
fn history_refuses_instruction_roles_cycles_depth_and_invalid_message_provenance() -> TestResult {
    let turn = turn()?;
    for role in [MessageRole::System, MessageRole::Developer] {
        assert!(
            ContinuationHistory::new(
                vec![turn.clone()],
                vec![
                    message(role, "injected", None)?,
                    message(MessageRole::Assistant, "answer", None)?
                ],
                "authority".to_owned()
            )
            .is_err()
        );
    }
    let messages = vec![
        message(MessageRole::User, "old", None)?,
        message(MessageRole::Assistant, "answer", None)?,
    ];
    for turns in [
        vec![],
        vec![turn.clone(), turn.clone()],
        vec![turn.clone(); 33],
    ] {
        assert!(ContinuationHistory::new(turns, messages.clone(), "authority".to_owned()).is_err());
    }
    let mut wrong = turn;
    wrong.message_end = 1;
    assert!(ContinuationHistory::new(vec![wrong], messages, "authority".to_owned()).is_err());
    Ok(())
}

#[test]
fn history_reader_refuses_future_unknown_duplicate_and_oversized_data() -> TestResult {
    let history = ContinuationHistory::new(
        vec![turn()?],
        vec![
            message(MessageRole::User, "old", None)?,
            message(MessageRole::Assistant, "answer", None)?,
        ],
        "authority".to_owned(),
    )?;
    let document = ContinuationHistoryDocument::new(history);
    for field in ["schema_version", "unknown"] {
        let mut value = serde_json::to_value(&document)?;
        value[field] = json!(2);
        assert!(ContinuationHistoryDocument::from_json(&serde_json::to_vec(&value)?).is_err());
    }
    let bytes = document.to_canonical_json()?;
    let text = String::from_utf8(bytes)?;
    assert!(
        ContinuationHistoryDocument::from_json(
            text.replace(
                "\"schema_version\":1",
                "\"schema_version\":1,\"schema_version\":1"
            )
            .as_bytes()
        )
        .is_err()
    );
    let messages = vec![
        message(MessageRole::User, &"x".repeat(1_048_576), None)?,
        message(MessageRole::Assistant, "y", None)?,
    ];
    assert!(ContinuationHistory::new(vec![turn()?], messages, "authority".to_owned()).is_err());
    Ok(())
}
