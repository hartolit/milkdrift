//! Controlled provider responses pass through real scheduling and acceptance before inspection.
use super::{
    inputs::{start_request, upload, workflow},
    support::*,
};
use serde_json::{Value, json};
use std::collections::VecDeque;

pub(super) struct Responses {
    pub(super) address: SocketAddr,
    pub(super) requests: Arc<std::sync::Mutex<Vec<Value>>>,
    task: JoinHandle<std::io::Result<()>>,
}
type ResponseState = (
    Arc<std::sync::Mutex<VecDeque<Value>>>,
    Arc<std::sync::Mutex<Vec<Value>>>,
);
impl Responses {
    pub(super) async fn start(responses: Vec<Value>) -> TestResult<Self> {
        let requests = Arc::new(std::sync::Mutex::new(Vec::new()));
        let state = (
            Arc::new(std::sync::Mutex::new(VecDeque::from(responses))),
            requests.clone(),
        );
        let app = axum::Router::new()
            .route(
                "/v1/chat/completions",
                axum::routing::post(
                    |axum::extract::State((responses, requests)): axum::extract::State<
                        ResponseState,
                    >,
                     axum::Json(body): axum::Json<Value>| async move {
                        use axum::response::IntoResponse as _;
                        let unavailable = axum::http::StatusCode::INTERNAL_SERVER_ERROR;
                        requests.lock().map_err(|_| unavailable)?.push(body);
                        let response = responses
                            .lock()
                            .map_err(|_| unavailable)?
                            .pop_front()
                            .ok_or(unavailable)?;
                        Ok::<_, axum::http::StatusCode>(if response == json!("refused") {
                            (axum::http::StatusCode::BAD_REQUEST, "request refused").into_response()
                        } else if response == json!("uncertain") {
                            (axum::http::StatusCode::OK, "{incomplete response").into_response()
                        } else {
                            axum::Json(response).into_response()
                        })
                    },
                ),
            )
            .with_state(state);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let address = listener.local_addr()?;
        let task = tokio::spawn(async move { axum::serve(listener, app).await });
        Ok(Self {
            address,
            requests,
            task,
        })
    }
}
impl Drop for Responses {
    fn drop(&mut self) {
        self.task.abort();
    }
}

pub(super) fn prose(text: &str, finish: &str) -> Value {
    json!({"id":"controlled", "model":"controlled-writer", "choices":[{"index":0,"message":{"role":"assistant","content":text},"finish_reason":finish}]})
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn result_view_preserves_complete_empty_truncated_refused_and_uncertain_truth() -> TestResult
{
    let directory = tempfile::tempdir()?;
    let model = Responses::start(vec![
        prose("draft", "stop"),
        prose("Complete notes\n\u{1b}[31m", "stop"),
        prose("draft", "stop"),
        prose("", "stop"),
        prose("draft", "stop"),
        prose("partial notes", "length"),
        json!("refused"),
        json!("uncertain"),
    ])
    .await?;
    let mut config = super::authoring::model_configuration_document(&directory, model.address)?;
    config.actors[1].preset = milkdrift_daemon::AuthorityPresetConfig::Invoker;
    let daemon = start(config.validate(directory.path())?, CONTROLLER_TOKEN).await?;
    let revision = workflow(&daemon.client).await?;
    let input = upload(&daemon.client, "brief", b"Harbor Host 1.4 release brief").await?;
    for case in ["complete", "empty", "truncated", "refused", "uncertain"] {
        daemon
            .client
            .submit(&start_request(case, &revision, vec![input.clone()]))
            .await?;
        wait_for_run(&daemon.client, case, Duration::from_secs(45), |state| {
            state.terminal.is_some()
                || state.uncertainty_count > 0
                || state
                    .nodes
                    .iter()
                    .any(|node| node.node_id == "author.review.hold")
        })
        .await?;
        let result = daemon.client.run_result(case).await?;
        assert_eq!(result.workflow_name.as_deref(), Some("release-notes"));
        assert!(result.version.is_some());
        let attempts = result
            .run
            .nodes
            .iter()
            .filter_map(|node| node.latest_attempt.as_ref())
            .collect::<Vec<_>>();
        match case {
            "complete" => {
                assert_eq!(result.run.terminal.as_deref(), Some("succeeded"));
                assert_eq!(result.outputs.len(), 1);
                assert_eq!(result.outputs[0].name, "notes");
                assert_eq!(
                    result.outputs[0].preview.as_deref(),
                    Some("Complete notes\n\u{1b}[31m")
                );
                assert!(
                    attempts
                        .iter()
                        .filter_map(|a| a.result_acceptance.as_ref())
                        .all(|a| a.accepted)
                );
                assert_eq!(result.actions, vec!["start_linked_run"]);
            }
            "empty" | "truncated" => {
                assert!(result.outputs.is_empty());
                assert!(result.run.terminal.is_none());
                assert!(
                    attempts
                        .iter()
                        .filter_map(|a| a.result_acceptance.as_ref())
                        .any(|a| !a.accepted)
                );
                assert!(result.actions.contains(&"pause".to_owned()));
            }
            "refused" => {
                assert!(result.outputs.is_empty());
                assert_ne!(result.run.terminal.as_deref(), Some("succeeded"));
            }
            "uncertain" => {
                assert!(result.outputs.is_empty());
                assert!(attempts.iter().any(|a| a.uncertain));
            }
            _ => unreachable!(),
        }
        assert!(
            attempts
                .iter()
                .filter(|a| a.model_generation.is_some())
                .all(|a| a
                    .usage
                    .as_ref()
                    .is_none_or(|u| u.input_units.is_none() && u.output_units.is_none()))
        );
        let hidden = client(&daemon.endpoint, OBSERVER_TOKEN)?;
        assert!(hidden.run_result(case).await.is_err());
        if matches!(case, "empty" | "truncated") {
            daemon
                .client
                .submit(&request(
                    &format!("end-{case}"),
                    Some(result.run.sequence),
                    Command::SignalRun {
                        run_id: case.into(),
                        signal_id: format!("end-{case}"),
                        signal_type: "workflow.reviewed".into(),
                        correlation: None,
                        broadcast: false,
                        payload: Value::Null,
                    },
                ))
                .await?;
            let ended = wait_for_run(&daemon.client, case, Duration::from_secs(45), |state| {
                state.terminal.is_some()
            })
            .await?;
            assert_eq!(ended.terminal.as_deref(), Some("failed"));
            assert!(
                daemon
                    .client
                    .submit(&request(
                        &format!("resume-{case}"),
                        Some(ended.sequence),
                        Command::ResumeRun {
                            run_id: case.into()
                        }
                    ))
                    .await
                    .is_err()
            );
            let ended_view = daemon.client.run_result(case).await?;
            assert!(ended_view.outputs.is_empty());
            assert_eq!(ended_view.actions, vec!["start_linked_run"]);
        }
    }
    assert_eq!(model.requests.lock().map_err(|_| "lock")?.len(), 8);
    daemon.stop().await?;
    Ok(())
}
