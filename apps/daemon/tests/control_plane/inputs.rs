//! Supplied briefs traverse upload, run workspace, causal selection, and real model HTTP.
use super::support::*;
use milkdrift_control_protocol::{
    BlueprintDraft, BlueprintEdit, InputUploadRequest, ModelInputSource, RunInput,
};
use serde_json::{Value, json};

pub(super) async fn workflow(client: &ControlClient) -> TestResult<String> {
    workflow_with_draft(client, "draft").await
}

pub(super) async fn workflow_with_draft(
    client: &ControlClient,
    draft_step: &str,
) -> TestResult<String> {
    workflow_with_names(client, draft_step, "brief").await
}

pub(super) async fn workflow_with_review_input(
    client: &ControlClient,
    input: &str,
) -> TestResult<String> {
    workflow_with_names(client, "draft", input).await
}

async fn workflow_with_names(
    client: &ControlClient,
    draft_step: &str,
    review_input: &str,
) -> TestResult<String> {
    let mut draft = BlueprintDraft {
        workflow_id: "release-notes".into(),
        base_revision: None,
        mutations: vec![],
    };
    let edits = [
        BlueprintEdit::Input {
            name: "brief".into(),
        },
        BlueprintEdit::AddModel {
            step: draft_step.into(),
            capability: "writing-model".into(),
            prompt: "Draft release notes from the brief.".into(),
            maximum_output_units: 512,
        },
        BlueprintEdit::AddModel {
            step: "review".into(),
            capability: "writing-model".into(),
            prompt: "Review the draft against the original brief.".into(),
            maximum_output_units: 512,
        },
        BlueprintEdit::Connect {
            step: draft_step.into(),
            input: "brief".into(),
            source: ModelInputSource::RunInput {
                name: "brief".into(),
            },
        },
        BlueprintEdit::Connect {
            step: "review".into(),
            input: review_input.into(),
            source: ModelInputSource::RunInput {
                name: "brief".into(),
            },
        },
        BlueprintEdit::Connect {
            step: "review".into(),
            input: "draft".into(),
            source: ModelInputSource::Step {
                step: draft_step.into(),
            },
        },
        BlueprintEdit::Output {
            step: "review".into(),
            name: "notes".into(),
        },
    ];
    for (index, edit) in edits.into_iter().enumerate() {
        let reply = client
            .submit(&request(
                &format!("author-{index}"),
                None,
                Command::AuthorBlueprint {
                    draft,
                    edit: Some(edit),
                    save: false,
                },
            ))
            .await?;
        draft = serde_json::from_value(
            reply
                .value
                .pointer("/draft")
                .ok_or("fixture field /draft absent")?
                .clone(),
        )?;
    }
    let reply = client
        .submit(&request(
            "save-notes",
            None,
            Command::AuthorBlueprint {
                draft,
                edit: None,
                save: true,
            },
        ))
        .await?;
    Ok(reply
        .value
        .pointer("/revision_id")
        .ok_or("fixture field /revision_id absent")?
        .as_str()
        .ok_or("revision missing")?
        .into())
}

pub(super) struct ModelFixture {
    pub(super) address: SocketAddr,
    pub(super) requests: Arc<std::sync::Mutex<Vec<Value>>>,
    task: JoinHandle<std::io::Result<()>>,
}

impl ModelFixture {
    pub(super) fn request_count(&self) -> Option<usize> {
        self.requests.try_lock().ok().map(|requests| requests.len())
    }

    pub(super) async fn checked(self, work: impl AsyncFnOnce(&Self) -> TestResult) -> TestResult {
        use futures_util::FutureExt as _;
        let result = std::panic::AssertUnwindSafe(work(&self))
            .catch_unwind()
            .await;
        self.task.abort();
        let mut fixture = self;
        let cleanup = (&mut fixture.task).await;
        match result {
            Ok(result) => {
                if let Err(error) = result {
                    return Err(format!("{error}; model cleanup: {cleanup:?}").into());
                }
                match cleanup {
                    Ok(result) => result.map_err(Into::into),
                    Err(error) if error.is_cancelled() => Ok(()),
                    Err(error) => Err(error.into()),
                }
            }
            Err(panic) => std::panic::resume_unwind(panic),
        }
    }

    pub(super) async fn start() -> TestResult<Self> {
        let requests = Arc::new(std::sync::Mutex::new(Vec::new()));
        let app = axum::Router::new().route("/v1/chat/completions", axum::routing::post(
            |axum::extract::State(requests): axum::extract::State<Arc<std::sync::Mutex<Vec<Value>>>>, axum::Json(body): axum::Json<Value>| async move {
                let text = body.to_string();
                let product = if text.contains("Harbor Host 1.4") { "Harbor" } else { "Lantern" };
                let stage = if text.contains("Review the draft") { "revised" } else { "draft" };
                requests.lock().map_err(|_| axum::http::StatusCode::INTERNAL_SERVER_ERROR)?.push(body);
                Ok::<_, axum::http::StatusCode>(axum::Json(json!({"id":"controlled", "model":"controlled-writer", "choices":[{"index":0,"message":{"role":"assistant","content":format!("{product} {stage} release notes")},"finish_reason":"stop"}], "usage":{"prompt_tokens":20,"completion_tokens":10,"total_tokens":30}})))
            }
        )).with_state(requests.clone());
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
impl Drop for ModelFixture {
    fn drop(&mut self) {
        self.task.abort();
    }
}

pub(super) async fn upload(client: &ControlClient, id: &str, bytes: &[u8]) -> TestResult<RunInput> {
    let artifact = client
        .upload_input(&InputUploadRequest::from_content(
            "host:local".into(),
            id.into(),
            "text/plain".into(),
            "restricted".into(),
            bytes,
        )?)
        .await?;
    Ok(RunInput {
        name: "brief".into(),
        artifact_id: artifact.artifact_id,
    })
}

pub(super) fn start_request(run: &str, revision: &str, inputs: Vec<RunInput>) -> CommandRequest {
    let mut command = request(
        &format!("start-{run}"),
        Some(0),
        Command::StartRun {
            run_id: run.into(),
            workflow_id: "release-notes".into(),
            revision_id: revision.into(),
            inputs,
        },
    );
    command.expected_revision = Some(revision.into());
    command
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn supplied_input_types_and_run_budgets_refuse_before_creation() -> TestResult {
    use milkdrift_blueprint::{FieldId, InterfaceField, SchemaRef, WorkflowInterface};
    let directory = tempfile::tempdir()?;
    let model = ModelFixture::start().await?;
    let mut config = super::authoring::model_configuration_document(&directory, model.address)?;
    config
        .actors
        .get_mut(1)
        .ok_or("fixture actor absent")?
        .authority
        .budget
        .artifact_bytes = Some(8);
    let daemon = start(config.validate(directory.path())?, CONTROLLER_TOKEN).await?;
    let limited = client(&daemon.endpoint, OBSERVER_TOKEN)?;
    let input = upload(
        &daemon.client,
        "oversized-for-run",
        b"more than eight bytes",
    )
    .await?;
    for (name, version, caller) in [("type", 2, &daemon.client), ("budget", 1, &limited)] {
        let definition = BlueprintRevision::genesis(
            WorkflowId::new(name)?,
            MutationBatch::new(vec![
                Mutation::AddNode {
                    node: Node::new(
                        NodeId::new("done")?,
                        NodeKind::Terminal {
                            outcome: TerminalOutcome::Success,
                        },
                    )?,
                },
                Mutation::SetInterface {
                    interface: WorkflowInterface::new(
                        [(
                            FieldId::new("brief")?,
                            InterfaceField::required(SchemaRef::new(
                                milkdrift_capability::SchemaId::new(
                                    "milkdrift.artifact-reference",
                                )?,
                                version,
                            )?),
                        )],
                        [],
                    )?,
                },
            ])?,
            AuthorRef::new("human:test")?,
            "input contract refusal",
        )?;
        daemon
            .client
            .submit(&request(
                &format!("import-{name}"),
                None,
                Command::ImportBlueprint {
                    document: serde_json::from_slice(
                        &BlueprintRevisionDocument::new(&definition).to_canonical_json()?,
                    )?,
                },
            ))
            .await?;
        let command = request(
            &format!("start-{name}"),
            Some(0),
            Command::StartRun {
                run_id: name.into(),
                workflow_id: name.into(),
                revision_id: definition.id().to_string(),
                inputs: vec![input.clone()],
            },
        );
        assert!(
            matches!(caller.submit(&command).await, Err(ClientError::Api(error)) if error.code == ErrorCode::InvalidInput),
            "{name}"
        );
        assert!(daemon.client.run(name).await.is_err());
    }
    assert!(
        model
            .requests
            .lock()
            .map_err(|_| "fixture lock")?
            .is_empty()
    );
    daemon.stop().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn supplied_inputs_are_validated_frozen_isolated_and_materialized() -> TestResult {
    let directory = tempfile::tempdir()?;
    let model = ModelFixture::start().await?;
    let mut config = super::authoring::model_configuration_document(&directory, model.address)?;
    config
        .actors
        .get_mut(1)
        .ok_or("fixture actor absent")?
        .authority
        .resources
        .artifacts = ArtifactAuthorityScope::none();
    let plan = config.validate(directory.path())?;
    let daemon = start(plan.clone(), CONTROLLER_TOKEN).await?;
    let revision = workflow(&daemon.client).await?;
    let harbor = b"Harbor Host 1.4: add preview environments; no billing change.";
    let lantern = b"Lantern Rally 0.8: improve checkpoint hints; no new vehicles.";
    let first = upload(&daemon.client, "harbor", harbor).await?;
    let second = upload(&daemon.client, "lantern", lantern).await?;
    let _unrelated = upload(&daemon.client, "unrelated", b"UNRELATED-PRIVATE-CONTENT").await?;
    let hidden = client(&daemon.endpoint, OBSERVER_TOKEN)?;
    assert!(
        matches!(hidden.submit(&start_request("private", &revision, vec![first.clone()])).await,
        Err(ClientError::Api(error)) if error.code == ErrorCode::Unauthorized)
    );
    assert!(daemon.client.run("private").await.is_err());
    let mut corrupt = InputUploadRequest::from_content(
        "host:local".into(),
        "corrupt".into(),
        "text/plain".into(),
        "restricted".into(),
        harbor,
    )?;
    corrupt.digest = "0".repeat(64);
    assert!(daemon.client.upload_input(&corrupt).await.is_err());
    corrupt.content_base64 =
        "A".repeat(milkdrift_control_protocol::MAX_INPUT_UPLOAD_BYTES.div_ceil(3) * 4 + 4);
    assert!(daemon.client.upload_input(&corrupt).await.is_err());
    let mut unexpected = first.clone();
    unexpected.name = "other".into();
    let mut missing = first.clone();
    missing.artifact_id = "missing".into();
    for (run, inputs) in [
        ("absent", vec![]),
        ("duplicate", vec![first.clone(), first.clone()]),
        ("unexpected", vec![unexpected]),
        ("missing", vec![missing]),
    ] {
        assert!(
            daemon
                .client
                .submit(&start_request(run, &revision, inputs))
                .await
                .is_err(),
            "{run}"
        );
        assert!(daemon.client.run(run).await.is_err());
    }
    assert!(
        model
            .requests
            .lock()
            .map_err(|_| "fixture lock")?
            .is_empty()
    );
    for (run, input) in [("harbor", first.clone()), ("lantern", second)] {
        let command = start_request(run, &revision, vec![input]);
        daemon.client.submit(&command).await?;
        let state = wait_for_run(&daemon.client, run, Duration::from_secs(45), |state| {
            state.terminal.is_some()
        })
        .await?;
        if state.terminal.as_deref() != Some("succeeded") {
            daemon.stop().await?;
            use milkdrift_persistence::RunQueryStore as _;
            let store = RedbStore::open(directory.path().join("data"))?;
            let events = store.events(&milkdrift_persistence::EventPageQuery::new(
                RunId::new(run)?,
                None,
                PageSize::new(100)?,
            )?)?;
            return Err(serde_json::to_string_pretty(&events.events)?.into());
        }
        assert_eq!(
            state.terminal.as_deref(),
            Some("succeeded"),
            "{}",
            serde_json::to_string_pretty(&state)?
        );
        assert_eq!(state.revision_id.as_deref(), Some(revision.as_str()));
        let review_id = attempt_id_for_node(&daemon.client, run, "review").await?;
        let review = daemon.client.attempt(run, &review_id).await?;
        let output = review
            .outputs
            .iter()
            .find(|output| output.name == "final_text")
            .ok_or("review final text missing")?;
        let bytes = daemon
            .client
            .artifact_range(&output.artifact.artifact_id, 0, output.artifact.size - 1)
            .await?
            .bytes;
        assert_eq!(
            std::str::from_utf8(&bytes)?,
            if run == "harbor" {
                "Harbor revised release notes"
            } else {
                "Lantern revised release notes"
            }
        );
        assert!(daemon.client.submit(&command).await?.replayed);
    }
    let requests = model.requests.lock().map_err(|_| "fixture lock")?.clone();
    assert_eq!(requests.len(), 4);
    for (index, body) in requests.iter().enumerate() {
        let text = body.to_string();
        let (brief, absent) = if index < 2 {
            (harbor.as_slice(), "Lantern Rally")
        } else {
            (lantern.as_slice(), "Harbor Host")
        };
        assert!(text.contains(std::str::from_utf8(brief)?), "{body}");
        assert!(!text.contains(absent));
        assert!(!text.contains("UNRELATED-PRIVATE-CONTENT"));
        if index % 2 == 1 {
            assert!(text.contains(if index < 2 {
                "Harbor draft release notes"
            } else {
                "Lantern draft release notes"
            }));
        } else {
            assert!(!text.contains("draft release notes"));
        }
    }
    let original = start_request("harbor", &revision, vec![first]);
    let saved = daemon.client.prepare_run(original.clone()).await?;
    assert_eq!(saved.authority.host, "host:local");
    let mut wrong_host = saved.clone();
    wrong_host.authority.host = "host:other".into();
    assert!(matches!(
        daemon.client.submit_saved_run(&wrong_host).await,
        Err(ClientError::Configuration(_))
    ));
    assert!(matches!(
        hidden.submit_saved_run(&saved).await,
        Err(ClientError::Configuration(_))
    ));
    assert!(daemon.client.submit_saved_run(&saved).await?.replayed);
    let mut changed = original.clone();
    changed.reason = "different exact request".into();
    assert!(
        matches!(daemon.client.submit(&changed).await, Err(ClientError::Api(error)) if error.code == ErrorCode::Conflict)
    );
    daemon.stop().await?;
    let restarted = start(plan, CONTROLLER_TOKEN).await?;
    assert!(restarted.client.submit(&original).await?.replayed);
    assert!(restarted.client.submit_saved_run(&saved).await?.replayed);
    assert_eq!(model.requests.lock().map_err(|_| "fixture lock")?.len(), 4);
    restarted.stop().await
}
