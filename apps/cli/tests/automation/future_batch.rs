//! Exact future-batch preview observes current guards without submitting a mutation.
use super::{Server, TestResult, negotiation, response};
use serde_json::json;

#[test]
fn future_batch_preview_preserves_stale_guards_and_never_submits() -> TestResult {
    let snapshot = json!({
        "wait_question":null,"admitted_tasks":1,"admitted_model_batches":0,
        "rejected_responses":0,"active_execution_ms":1,"admissions":[],
        "id":"workflow-one", "title":"Research", "description":"", "generation":9,
        "state":"waiting", "created_at":1, "updated_at":2, "tasks":[], "task_count":1,
        "next_task_cursor":null,
        "limits":{"tasks":256,"concurrent_tasks":4,"model_batches":32,"rejected_responses":3,
            "corrective_failures":3,"active_execution_ms":3600000,"tool_calls_per_task":16}
    });
    let task = json!({
        "id":"consumer", "version":4,
        "definition":{"title":"Composition","description":"","instructions":"Compose notes",
            "capability":{"id":"model","operation":"model.generate"},"inputs":{},
            "outputs":{},"dependencies":[],"required":true,"corrective_of":null,"paths":[]},
        "state":"running","reason":null,"started_at":2,"ended_at":null,
        "stop_requested":false,"stop_request_id":"stop-consumer","frozen":null,"result":null,
        "result_restricted":false
    });
    let request = json!({
        "request_id":"reviewed-request", "expected_generation":7,
        "changes":[{"kind":"adopt_correction","task":"failed","correction":"fixed",
            "redirects":[{"consumer":"consumer","expected_version":3,"input":"research"}]}]
    });
    let server = Server::with_request_hook(
        vec![negotiation(), response(snapshot), response(task)],
        |index, request| {
            let expected = match index {
                0 => b"POST /v1/version HTTP/1.1\r\n".as_slice(),
                1 => b"GET /v1/workflows/workflow-one HTTP/1.1\r\n".as_slice(),
                2 => b"GET /v1/workflows/workflow-one/tasks/consumer HTTP/1.1\r\n".as_slice(),
                _ => return Err(std::io::Error::other("unexpected preview request")),
            };
            assert!(
                request.starts_with(expected),
                "preview must negotiate, then read current guards without submitting changes"
            );
            Ok(())
        },
    )?;
    let root = tempfile::tempdir()?;
    let file = root.path().join("changes.json");
    std::fs::write(&file, serde_json::to_vec(&request)?)?;
    let (exit, records, errors) = server.invoke(
        &[
            "workflow",
            "edit",
            "workflow-one",
            file.to_str().ok_or("non-UTF-8 fixture path")?,
            "--preview",
        ],
        false,
    )?;
    assert_eq!(exit, 0, "{records:?}; {errors}");
    let value = records
        .first()
        .and_then(|record| record.get("value"))
        .ok_or("missing preview")?;
    assert_eq!(value.get("request"), Some(&request));
    assert_eq!(value.get("generation_matches"), Some(&json!(false)));
    assert_eq!(
        value.pointer("/guarded_tasks/0/observed_version"),
        Some(&json!(4))
    );
    assert_eq!(
        value.pointer("/guarded_tasks/0/version_matches"),
        Some(&json!(false))
    );
    assert_eq!(
        value.pointer("/guarded_tasks/0/started_at"),
        Some(&json!(2))
    );
    Ok(())
}
