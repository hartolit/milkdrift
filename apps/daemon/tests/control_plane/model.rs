//! A controlled Rust HTTP provider observes actual supplied text and captured file bytes.
use super::support::*;
use serde_json::{Value, json};
struct ModelFixture {
    address: SocketAddr,
    requests: Arc<std::sync::Mutex<Vec<Value>>>,
    task: JoinHandle<std::io::Result<()>>,
}
impl ModelFixture {
    async fn start() -> TestResult<Self> {
        let requests = Arc::new(std::sync::Mutex::new(Vec::new()));
        let app=axum::Router::new().route("/v1/chat/completions",axum::routing::post(
            |axum::extract::State(requests):axum::extract::State<Arc<std::sync::Mutex<Vec<Value>>>>,axum::Json(body):axum::Json<Value>| async move {
                let text=body.to_string();
                let project=if text.contains("Harbor Host 1.4") {"Harbor"} else {"Lantern"};
                let report=json!({"project":project,"summary":format!("{project} release notes from the supplied brief and captured project file"),"captured_file":text.contains("captured Cargo evidence"),"request_bytes":text.len()}).to_string();
                requests.lock().map_err(|_|axum::http::StatusCode::INTERNAL_SERVER_ERROR)?.push(body);
                Ok::<_,axum::http::StatusCode>(axum::Json(json!({"id":"controlled","model":"controlled-writer","choices":[{"index":0,"message":{"role":"assistant","content":report},"finish_reason":"stop"}],"usage":{"prompt_tokens":20,"completion_tokens":10,"total_tokens":30}})))
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

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn controlled_model_receives_selected_text_and_uploaded_file_and_retains_its_response()
-> TestResult {
    let directory = TempDir::new()?;
    let model = ModelFixture::start().await?;
    let config = configuration_with_model(&directory, model.address, 2)?;
    let plan = config.validate(directory.path())?;
    let daemon = start(plan.clone(), CONTROLLER_TOKEN).await?;
    let bytes = "captured Cargo evidence\n[package]\nname = \"release-notes\"\n";
    let file = daemon
        .client
        .upload_task_input(&TaskUploadRequest {
            request_id: "model-file".into(),
            name: "Cargo.toml".into(),
            media_type: "text/plain".into(),
            text: bytes.into(),
        })
        .await?;
    let mut request = create_request("controlled-model", true);
    let task = request.tasks.first_mut().ok_or("model task missing")?;
    task.definition = task_definition("writing-model", "model.generate");
    task.definition.inputs.insert(
        "file".into(),
        InputBinding::Supplied {
            name: "file".into(),
            optional: false,
        },
    );
    request.inputs.insert(
        "brief".into(),
        InputValue::Text {
            text: "Write release notes for Harbor Host 1.4 using the captured file".into(),
        },
    );
    request.inputs.insert(
        "file".into(),
        InputValue::Uploaded {
            input_id: file.input_id,
        },
    );
    let accepted = daemon.client.create_workflow(&request).await?;
    let task = accepted.aliases.get("inspect").ok_or("alias absent")?;
    wait_task(
        &daemon.client,
        &accepted.workflow_id,
        task,
        TaskStateDto::Completed,
    )
    .await?;
    let result = daemon
        .client
        .task_result(&accepted.workflow_id, task)
        .await?;
    assert!(
        result.text.contains("Harbor") && result.text.contains("captured_file"),
        "{result:?}"
    );
    assert!(result.text.contains("true"));
    let captured = model
        .requests
        .lock()
        .map_err(|_| "fixture request lock poisoned")?
        .clone();
    assert_eq!(captured.len(), 1);
    let body = captured.first().ok_or("model request missing")?.to_string();
    assert!(body.contains("Harbor Host 1.4") && body.contains("captured Cargo evidence"));
    assert!(!result.artifacts.is_empty());
    daemon.stop().await?;
    let restarted = start(plan, CONTROLLER_TOKEN).await?;
    let fresh = client(&restarted.endpoint, CONTROLLER_TOKEN)?;
    let discovered = fresh.workflows("Harbor", None, 10).await?;
    assert!(
        discovered
            .items
            .iter()
            .any(|workflow| workflow.id == accepted.workflow_id)
    );
    assert_eq!(
        fresh.task_result(&accepted.workflow_id, task).await?,
        result
    );
    assert_eq!(
        model
            .requests
            .lock()
            .map_err(|_| "fixture request lock poisoned")?
            .len(),
        1
    );
    restarted.stop().await
}
pub(super) fn configuration_with_model(
    directory: &TempDir,
    address: std::net::SocketAddr,
    concurrency: u32,
) -> TestResult<DaemonConfig> {
    use milkdrift_model_provider::{
        AuthMode, BillingTerms, EndpointLimits, EndpointProfile, ModelFeature, ModelTokenLimits,
        ProviderProtocol, ProxyPolicy, RedirectPolicy, TlsPolicy,
    };
    let profile = EndpointProfile::new(
        milkdrift_capability::ProviderProfileRef::new("writing-profile")?,
        1,
        ProviderProtocol::OpenAiCompatible {
            path: "v1/chat/completions".into(),
        },
        format!("http://{address}"),
        "controlled-writer",
        AuthMode::NoAuth,
        EndpointLimits {
            connect_timeout_ms: 1000,
            request_timeout_ms: 5000,
            idle_timeout_ms: 5000,
            max_request_bytes: 262_144,
            max_response_bytes: 262_144,
            max_headers: 64,
            max_header_bytes: 65_536,
            max_stream_line_bytes: 4096,
            max_stream_event_bytes: 65_536,
            max_fragment_bytes: 4096,
        },
        RedirectPolicy::Deny,
        TlsPolicy::WebPkiRoots,
        ProxyPolicy::Disabled,
        std::collections::BTreeSet::from([ModelFeature::SystemRole]),
        concurrency,
        true,
        std::collections::BTreeSet::from(["127.0.0.1".into()]),
        std::collections::BTreeSet::from(["test".into()]),
        BTreeMap::new(),
        BillingTerms::Unbilled {
            source: "controlled loopback fixture".into(),
        },
        ModelTokenLimits::ByteBpe {
            template_tokens_per_message: 64,
            template_tokens_per_request: 64,
            maximum_input_tokens: 32768,
            maximum_output_tokens: 65536,
            output_control: milkdrift_model_provider::OutputTokenControl::MaxTokens,
            source: "controlled loopback fixture with bounded encoded input".into(),
        },
    )?;
    let path = directory.path().join("model.json");
    fs::write(&path, profile.to_canonical_json()?)?;
    let mut config = configuration_document_with_process_profiles(directory, 64, vec![])?;
    config
        .actors
        .get_mut(0)
        .ok_or("fixture actor absent")?
        .authority
        .resources
        .network = NetworkScope::new(
        std::collections::BTreeSet::from([milkdrift_authority::NetworkProfileRef::new(
            "writing-profile",
        )?]),
        std::collections::BTreeSet::from([address.to_string()]),
    )?;
    config
        .actors
        .get_mut(1)
        .ok_or("fixture actor absent")?
        .authority = ActorGrantConfig::dangerous_administrator();
    config
        .actors
        .get_mut(1)
        .ok_or("fixture actor absent")?
        .authority
        .resources
        .capability = CapabilityAuthorityScope::deny_all();
    config
        .adapters
        .model_profiles
        .push(milkdrift_daemon::ModelProfileConfig {
            capability_id: "writing-model".into(),
            profile: path,
        });
    Ok(config)
}
