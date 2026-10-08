//! The private owner transition drains a real generation; all editing uses HTTP and the CLI.
use super::*;
use milkdrift_control_client::{BearerCredential, ClientConfig, ControlClient};
use milkdrift_control_protocol::{BlueprintDraft, Command, CommandRequest, ProtocolVersion};
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
        BillingTerms::Unknown,
        ModelTokenLimits::Unknown,
    )?)
}

fn command(id: &str, command: Command) -> CommandRequest {
    CommandRequest {
        protocol: ProtocolVersion::CURRENT,
        command_id: id.into(),
        expected_sequence: None,
        expected_revision: None,
        reason: "drained model repair regression".into(),
        evidence: vec![],
        command,
    }
}

fn cli(endpoint: &url::Url, root: &Path, id: &str, args: &[&str]) -> TestResult<Value> {
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
        .args([
            "--json",
            "--yes",
            "--command-id",
            id,
            "--timeout-secs",
            "20",
            "workflow",
        ])
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
    let bytes = include_bytes!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/authoring/legacy-workflow.json"
    ));
    let (_, definition) = milkdrift_blueprint::BlueprintRevisionDocument::from_json(bytes)?;
    client
        .submit(&command(
            "drain-import",
            Command::ImportBlueprint {
                document: serde_json::from_slice(bytes)?,
            },
        ))
        .await?;
    let base = BlueprintDraft {
        workflow_id: "release-notes".into(),
        base_revision: Some(definition.id().to_string()),
        mutations: vec![],
    };
    let mut save = command(
        "drain-save",
        Command::AuthorBlueprint {
            draft: base,
            edit: None,
            save: true,
        },
    );
    save.expected_revision = Some(definition.id().to_string());
    client.submit(&save).await?;
    host.dispatch(false, |owner| {
        owner
            .capability_host
            .begin_drain(
                &milkdrift_capability::CapabilityId::new("writing-model")
                    .map_err(|error| crate::host::invalid(&error.to_string()))?,
                1,
            )
            .map_err(|error| crate::host::invalid(&error.to_string()))
    })
    .await
    .map_err(|error| error.message)?;
    for (id, args, unresolved) in [
        (
            "drain-open",
            vec!["open", definition.id().as_str(), "--file", "repair.json"],
            2,
        ),
        (
            "drain-first",
            vec!["model", "repair.json", "draft", "replacement-model"],
            1,
        ),
        ("drain-inspect", vec!["inspect", "repair.json"], 1),
        (
            "drain-second",
            vec!["model", "repair.json", "review", "replacement-model"],
            0,
        ),
    ] {
        let value = cli(&endpoint, root.path(), id, &args)?;
        assert_eq!(
            value
                .pointer("/workflow/selection_diagnostics")
                .and_then(Value::as_array)
                .ok_or("diagnostics absent")?
                .len(),
            unresolved
        );
    }
    let saved = cli(
        &endpoint,
        root.path(),
        "drain-final-save",
        &["save", "repair.json"],
    )?;
    let revision = saved
        .get("revision_id")
        .and_then(Value::as_str)
        .ok_or("revision absent")?;
    let upload = client
        .upload_input(
            &milkdrift_control_protocol::InputUploadRequest::from_content(
                "host:local".into(),
                "drained-brief".into(),
                "text/plain".into(),
                "restricted".into(),
                b"brief",
            )?,
        )
        .await?;
    let mut start = command(
        "drain-start",
        Command::StartRun {
            run_id: "drain-repaired".into(),
            workflow_id: "release-notes".into(),
            revision_id: revision.into(),
            inputs: vec![milkdrift_control_protocol::RunInput {
                name: "brief".into(),
                artifact_id: upload.artifact_id,
            }],
        },
    );
    start.expected_revision = Some(revision.into());
    client.submit(&start).await?;
    let completed = tokio::time::timeout(std::time::Duration::from_secs(30), async {
        loop {
            let run = client.run("drain-repaired").await?;
            if run.terminal.is_some() {
                return Ok::<_, milkdrift_control_client::ClientError>(run);
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    })
    .await??;
    assert_eq!(completed.terminal.as_deref(), Some("succeeded"));
    assert_eq!(calls.load(Ordering::SeqCst), 2);
    assert!(client.submit(&start).await?.replayed);
    assert_eq!(
        client.revision(definition.id().as_str()).await?.document,
        Some(serde_json::from_slice(bytes)?)
    );
    shutdown.send(()).map_err(|()| "shutdown channel closed")?;
    serving.await??;
    model.abort();
    Ok(())
}
