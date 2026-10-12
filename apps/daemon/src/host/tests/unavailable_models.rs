//! The private owner transition drains a real generation; all editing uses HTTP and the CLI.
use super::*;
use milkdrift_control_client::{BearerCredential, ClientConfig, ControlClient};
use milkdrift_control_protocol::tasks::{
    ChangeWorkflowRequest, CreateWorkflowRequest, TaskChangeDto, TaskDefinitionDto, TaskStateDto,
};
use serde_json::{Value, json};
use std::{collections::BTreeSet, path::Path};

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

pub(super) fn profile(
    address: SocketAddr,
) -> TestResult<milkdrift_model_provider::EndpointProfile> {
    use milkdrift_model_provider::{
        AuthMode, BillingTerms, EndpointLimits, EndpointProfile, ModelFeature, ModelTokenLimits,
        ProviderProtocol, ProxyPolicy, RedirectPolicy, TlsPolicy,
    };
    Ok(EndpointProfile::new(
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
        BTreeSet::from([ModelFeature::SystemRole]),
        1,
        true,
        BTreeSet::from(["127.0.0.1".into()]),
        BTreeSet::from(["test".into()]),
        BTreeMap::new(),
        BillingTerms::Unbilled {
            source: "controlled fixture incurs no provider charge".into(),
        },
        ModelTokenLimits::ByteBpe {
            template_tokens_per_message: 16,
            template_tokens_per_request: 16,
            maximum_input_tokens: 262_144,
            maximum_output_tokens: 8192,
            output_control: milkdrift_model_provider::OutputTokenControl::MaxTokens,
            source: "controlled bounded text fixture".into(),
        },
    )?)
}

fn cli(endpoint: &url::Url, root: &Path, args: &[&str]) -> TestResult<Value> {
    let executable = std::env::current_exe()?
        .parent()
        .and_then(Path::parent)
        .ok_or("binary directory absent")?
        .join(format!("milkdrift{}", std::env::consts::EXE_SUFFIX));
    let output = std::process::Command::new(executable)
        .arg("--endpoint")
        .arg(endpoint.as_str())
        .arg("--token-file")
        .arg(root.join("token"))
        .args(["--json", "--yes", "--timeout-secs", "20", "workflow"])
        .args(args)
        .current_dir(root)
        .output()?;
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let value: Value = serde_json::from_slice(&output.stdout)?;
    Ok(value.get("value").ok_or("CLI value absent")?.clone())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn drained_models_can_be_replaced_individually_through_http_and_cli() -> TestResult {
    let root = tempfile::tempdir()?;
    let token = root.path().join("token");
    fs::write(&token, "drained-model-token")?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        fs::set_permissions(&token, fs::Permissions::from_mode(0o600))?;
    }
    let calls = Arc::new(AtomicU64::new(0));
    let observed = calls.clone();
    let model_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let model_address = model_listener.local_addr()?;
    let app = axum::Router::new().route("/v1/chat/completions", axum::routing::post(move || { observed.fetch_add(1, Ordering::SeqCst); async { axum::Json(json!({"id":"drain-fixture","model":"controlled-writer","choices":[{"index":0,"message":{"role":"assistant","content":"Complete repaired output"},"finish_reason":"stop"}]})) } }));
    let model = tokio::spawn(async move { axum::serve(model_listener, app).await });
    let path = root.path().join("model.json");
    fs::write(&path, profile(model_address)?.to_canonical_json()?)?;
    let mut config = owner_test_document(root.path(), &token, 32, 10);
    config
        .actors
        .first_mut()
        .ok_or("actor absent")?
        .authority
        .resources
        .network = milkdrift_authority::NetworkScope::new(
        BTreeSet::from([milkdrift_authority::NetworkProfileRef::new(
            "writing-profile",
        )?]),
        BTreeSet::from([model_address.to_string()]),
    )?;
    for capability in ["writing-model", "replacement-model"] {
        config
            .adapters
            .model_profiles
            .push(crate::ModelProfileConfig {
                capability_id: capability.into(),
                profile: path.clone(),
            });
    }
    let host = DaemonHost::start(config.validate(root.path())?)?;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let endpoint = url::Url::parse(&format!("http://{}/", listener.local_addr()?))?;
    let (shutdown, stop) = tokio::sync::oneshot::channel();
    let serving_host = host.clone();
    let serving = tokio::spawn(async move {
        crate::serve(listener, serving_host, async {
            let _closed = stop.await;
        })
        .await
    });
    let client = ControlClient::new(
        ClientConfig::new(endpoint.clone()),
        BearerCredential::new("drained-model-token")?,
    )?;
    let definition: TaskDefinitionDto = serde_json::from_value(json!({
        "title":"Writer","description":"","instructions":"Write the supplied brief",
        "capability":{"id":"writing-model","operation":"model.generate"},
        "inputs":{"brief":{"kind":"supplied","name":"brief","optional":false}},
        "dependencies":[],"required":true,"corrective_of":null,"paths":[]
    }))?;
    let request: CreateWorkflowRequest = serde_json::from_value(json!({
        "request_id":"drain-create","title":"Repair future work","description":"",
        "tasks":[{"alias":"draft","definition":definition},{"alias":"review","definition":definition}],
        "inputs":{},"finish_when_settled":true,"limits":null
    }))?;
    let receipt = client.create_workflow(&request).await?;
    let workflow = receipt.workflow_id.clone();
    let original = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            let read = client.workflow(&workflow).await?;
            if read.tasks.iter().all(|task| task.reason.is_some()) {
                return Ok::<_, milkdrift_control_client::ClientError>(read);
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await??;
    assert!(
        original
            .tasks
            .iter()
            .all(|task| task.state == TaskStateDto::Queued)
    );
    host.dispatch(false, |owner| {
        owner
            .capability_host
            .begin_drain(
                &milkdrift_capability::CapabilityId::new("writing-model")
                    .map_err(|error| crate::host::read_model::invalid(&error.to_string()))?,
                1,
            )
            .map_err(|error| crate::host::read_model::invalid(&error.to_string()))
    })
    .await
    .map_err(|error| error.message)?;
    let catalog = client.task_session().await?;
    let catalog_json = serde_json::to_value(catalog)?;
    assert!(catalog_json.to_string().contains("replacement-model"));
    let ids = original
        .tasks
        .iter()
        .map(|task| task.id.clone())
        .collect::<Vec<_>>();
    let first = ids.first().ok_or("first task absent")?;
    let second = ids.get(1).ok_or("second task absent")?;
    let mut replacement = definition.clone();
    replacement.capability.id = "replacement-model".into();
    client
        .change_workflow(
            &workflow,
            &ChangeWorkflowRequest {
                request_id: "repair-first".into(),
                expected_generation: original.generation,
                changes: vec![TaskChangeDto::Revise {
                    task: first.clone(),
                    expected_version: 1,
                    definition: replacement.clone(),
                }],
            },
        )
        .await?;
    let current = client.workflow(&workflow).await?;
    let second_change = ChangeWorkflowRequest {
        request_id: "repair-second".into(),
        expected_generation: current.generation,
        changes: vec![TaskChangeDto::Revise {
            task: second.clone(),
            expected_version: 1,
            definition: replacement,
        }],
    };
    fs::write(
        root.path().join("repair.json"),
        serde_json::to_vec(&second_change)?,
    )?;
    cli(&endpoint, root.path(), &["edit", &workflow, "repair.json"])?;
    for id in &ids {
        let task = client.task(&workflow, id).await?;
        assert_eq!(task.definition.capability.id, "replacement-model");
        assert_eq!(task.version, 2);
        assert_eq!(task.state, TaskStateDto::Queued);
        assert!(task.frozen.is_none());
    }
    let current = client.workflow(&workflow).await?;
    let supply = ChangeWorkflowRequest {
        request_id: "repair-supply".into(),
        expected_generation: current.generation,
        changes: vec![TaskChangeDto::BindInput {
            name: "brief".into(),
            value: milkdrift_control_protocol::tasks::InputValue::Text {
                text: "brief".into(),
            },
        }],
    };
    client.change_workflow(&workflow, &supply).await?;
    tokio::time::timeout(std::time::Duration::from_secs(30), async {
        loop {
            let read = client.workflow(&workflow).await?;
            if read.tasks.iter().any(|task| {
                matches!(
                    task.state,
                    TaskStateDto::Failed | TaskStateDto::Stopped | TaskStateDto::Uncertain
                )
            }) {
                let mut diagnostics = Vec::new();
                for task in &read.tasks {
                    diagnostics.push(client.task(&workflow, &task.id).await?);
                }
                return Err(format!(
                    "repaired model task failed before completion: {diagnostics:?}"
                )
                .into());
            }
            if read
                .tasks
                .iter()
                .all(|task| task.state == TaskStateDto::Completed)
            {
                assert!(
                    read.tasks
                        .iter()
                        .all(|task| task.state == TaskStateDto::Completed),
                    "{read:?}"
                );
                return Ok::<_, Box<dyn std::error::Error>>(());
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    })
    .await??;
    assert_eq!(calls.load(Ordering::SeqCst), 2);
    assert_eq!(
        client.change_workflow(&workflow, &supply).await?,
        client.workflow_receipt("repair-supply").await?
    );
    shutdown.send(()).map_err(|()| "shutdown channel closed")?;
    serving.await??;
    model.abort();
    Ok(())
}
