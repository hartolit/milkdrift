//! Overload, credential rotation, process execution, and startup refusal behavior.

use super::support::*;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn daemon_bounded_overload_returns_stable_error() -> TestResult {
    let directory = tempfile::tempdir()?;
    let daemon = start(configuration(&directory, 1)?, CONTROLLER_TOKEN).await?;
    let raw = reqwest::Client::new();
    let target = daemon.endpoint.join("v1/revisions?limit=100")?;
    let mut overload = None;

    for _round in 0..4 {
        let barrier = Arc::new(tokio::sync::Barrier::new(129));
        let mut requests = Vec::new();
        for _ in 0..128 {
            let barrier = barrier.clone();
            let raw = raw.clone();
            let target = target.clone();
            requests.push(tokio::spawn(async move {
                barrier.wait().await;
                raw.get(target).bearer_auth(CONTROLLER_TOKEN).send().await
            }));
        }
        barrier.wait().await;
        for request in requests {
            let response = request.await??;
            if response.status() == reqwest::StatusCode::TOO_MANY_REQUESTS {
                overload = Some(response.bytes().await?);
            }
        }
        if overload.is_some() {
            break;
        }
    }

    let bytes = overload.ok_or("concurrent requests did not exercise the queue bound")?;
    let error: milkdrift_control_protocol::ErrorEnvelope = decode_json(&bytes)?;
    assert_eq!(error.code, ErrorCode::Overload);
    assert!(error.retryable);
    daemon.stop().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn daemon_stream_reconnect_auth_rotation_and_shutdown() -> TestResult {
    let directory = tempfile::tempdir()?;
    let daemon = start(configuration(&directory, 16)?, CONTROLLER_TOKEN).await?;
    let revision = import_blueprint(&daemon.client, "stream-import").await?;
    daemon
        .client
        .submit(&request(
            "stream-start",
            None,
            Command::StartRun {
                run_id: "run-stream".to_owned(),
                workflow_id: "golden".to_owned(),
                revision_id: revision,
            },
        ))
        .await?;

    let mut stream = daemon.client.subscribe("v1/runs/run-stream/stream", None);
    let first = tokio::time::timeout(Duration::from_secs(3), stream.next())
        .await?
        .ok_or("run stream closed before its first observation")??;
    assert_eq!(first.feed, "run:run-stream");
    let first_position = first.cursor.position_for("run:run-stream")?;
    drop(stream);

    let mut resumed = daemon
        .client
        .subscribe("v1/runs/run-stream/stream", Some(first.cursor));
    let second = tokio::time::timeout(Duration::from_secs(3), resumed.next())
        .await?
        .ok_or("resumed run stream closed")??;
    assert!(second.cursor.position_for("run:run-stream")? > first_position);
    drop(resumed);

    let mut capabilities = daemon.client.subscribe("v1/stream/capabilities", None);
    let capability = tokio::time::timeout(Duration::from_secs(3), capabilities.next())
        .await?
        .ok_or("capability stream closed before its first observation")??;
    assert_eq!(capability.feed, "capability-health");
    assert!(matches!(capability.observation, Observation::Capability(_)));
    drop(capabilities);

    let mut health = daemon.client.subscribe("v1/stream/health", None);
    let initial_health = tokio::time::timeout(Duration::from_secs(3), health.next())
        .await?
        .ok_or("health stream closed before its first observation")??;
    assert!(matches!(
        initial_health.observation,
        Observation::DaemonHealth(_)
    ));
    let initial_health_position = initial_health.cursor.position_for("daemon-health")?;
    daemon
        .client
        .submit(&request(
            "health-stream-operational-change",
            None,
            Command::ValidateBlueprint {
                document: blueprint()?,
            },
        ))
        .await?;
    let changed_health = tokio::time::timeout(Duration::from_secs(3), health.next())
        .await?
        .ok_or("health stream did not publish an operational change")??;
    assert!(matches!(
        changed_health.observation,
        Observation::DaemonHealth(_)
    ));
    assert!(
        changed_health.cursor.position_for("daemon-health")? > initial_health_position,
        "queue/receipt health changes must advance the coherent feed generation"
    );
    write_secret(
        &directory.path().join("controller.token"),
        "rotated-controller-token",
    )?;
    let closing = tokio::time::timeout(Duration::from_secs(3), health.next())
        .await?
        .ok_or("health stream closed without a rotation observation")??;
    assert!(matches!(
        closing.observation,
        Observation::StreamClosing { .. }
    ));
    assert!(
        matches!(daemon.client.health().await, Err(ClientError::Api(error)) if error.code == ErrorCode::Unauthenticated)
    );
    assert!(
        client(&daemon.endpoint, "rotated-controller-token")?
            .readiness()
            .await?
            .ready
    );
    daemon.stop().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn daemon_graceful_shutdown_and_restart() -> TestResult {
    let directory = tempfile::tempdir()?;
    let config = configuration(&directory, 16)?;
    let daemon = start(config.clone(), CONTROLLER_TOKEN).await?;
    daemon.stop().await?;
    let restarted = start(config, CONTROLLER_TOKEN).await?;
    assert!(restarted.client.readiness().await?.ready);
    restarted.stop().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn daemon_configured_process_adapter_executes_to_terminal() -> TestResult {
    let directory = tempfile::tempdir()?;
    let profile = configured_process_profile(&directory)?;
    let config = configuration_with_process_profiles(&directory, 16, vec![profile])?;
    let (artifact_id, artifact_bytes) =
        publish_restricted_test_artifact(&directory.path().join("data"))?;
    let daemon = start(config.clone(), CONTROLLER_TOKEN).await?;
    assert!(
        daemon
            .client
            .capabilities()
            .await?
            .iter()
            .any(|capability| capability.capability_id == "golden-local-process")
    );
    let imported = daemon
        .client
        .submit(&request(
            "process-import",
            None,
            Command::ImportBlueprint {
                document: process_blueprint()?,
            },
        ))
        .await?;
    let revision = imported
        .value
        .get("revision_id")
        .and_then(serde_json::Value::as_str)
        .ok_or("process import response omitted revision identity")?;
    daemon
        .client
        .submit(&request(
            "process-start",
            None,
            Command::StartRun {
                run_id: "run-process".to_owned(),
                workflow_id: "daemon-process".to_owned(),
                revision_id: revision.to_owned(),
            },
        ))
        .await?;

    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        let run = daemon.client.run("run-process").await?;
        if run.lifecycle == "terminal" {
            assert_eq!(run.terminal.as_deref(), Some("succeeded"));
            break;
        }
        if tokio::time::Instant::now() >= deadline {
            let health = daemon.client.health().await?;
            let capabilities = daemon.client.capabilities().await?;
            let timeline = daemon
                .client
                .timeline(
                    "run-process",
                    &PageRequest {
                        cursor: None,
                        limit: 100,
                    },
                )
                .await?;
            return Err(format!(
                "configured process invocation did not reach terminal state: run={run:?}, health={health:?}, capabilities={capabilities:?}, timeline={:?}",
                timeline.items
            )
            .into());
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let attempt_id = daemon
        .client
        .timeline(
            "run-process",
            &PageRequest {
                cursor: None,
                limit: 100,
            },
        )
        .await?
        .items
        .into_iter()
        .find(|entry| entry.node_id.as_deref() == Some("process") && entry.attempt_id.is_some())
        .and_then(|entry| entry.attempt_id)
        .ok_or("process run omitted its exact attempt")?;
    let attempt = daemon.client.attempt("run-process", &attempt_id).await?;
    assert_eq!(attempt.context_access, "authorized");
    assert_eq!(
        attempt.capability_id.as_deref(),
        Some("golden-local-process")
    );
    assert_eq!(attempt.descriptor_revision, Some(2));
    let provenance = attempt
        .capability_provenance
        .as_ref()
        .ok_or("process attempt omitted exact capability provenance")?;
    assert_eq!(provenance.execution_trust, "trusted_host_process");
    assert_eq!(provenance.snapshot_digest.len(), 64);
    assert!(
        provenance
            .snapshot_digest
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    );
    assert!(
        provenance
            .implementation_identity
            .as_deref()
            .is_some_and(|digest| digest.starts_with("b3_"))
    );
    assert!(
        provenance
            .implementation_content_digest
            .as_deref()
            .is_some_and(|digest| digest.starts_with("b3_"))
    );
    assert!(
        provenance
            .implementation_size_bytes
            .is_some_and(|size| size > 0)
    );
    assert!(
        provenance
            .process_profile_digest
            .as_deref()
            .is_some_and(|digest| digest.starts_with("b3_"))
    );
    assert!(
        provenance
            .execution_policy_digest
            .as_deref()
            .is_some_and(|digest| digest.starts_with("b3_"))
    );
    let manifest_metadata = attempt
        .context_manifest
        .as_ref()
        .ok_or("process attempt omitted context manifest metadata")?;
    assert_eq!(
        manifest_metadata.content_type,
        "application/vnd.milkdrift.context-manifest.v2+json"
    );
    let inspected_context = attempt
        .context
        .clone()
        .ok_or("authorized process attempt omitted context details")?;
    assert_eq!(inspected_context.schema_version, 2);
    assert!(inspected_context.digest.starts_with("b3_"));
    assert!(inspected_context.entries.is_empty());
    assert!(!inspected_context.truncated);
    let metadata = daemon.client.artifact_metadata(&artifact_id).await?;
    assert_eq!(metadata.artifact_id, artifact_id);
    let range = daemon
        .client
        .artifact_range(&artifact_id, 0, metadata.size.saturating_sub(1))
        .await?;
    assert_eq!(range.bytes, artifact_bytes);
    let observer = client(&daemon.endpoint, OBSERVER_TOKEN)?;
    assert!(matches!(
        observer.artifact_metadata(&artifact_id).await,
        Err(ClientError::Api(error)) if error.code == ErrorCode::Unauthorized
    ));
    assert!(matches!(
        observer.artifact_range(&artifact_id, 0, 0).await,
        Err(ClientError::Api(error)) if error.code == ErrorCode::Unauthorized
    ));
    assert!(!directory.path().join("data/control-state-v1.json").exists());
    daemon.stop().await?;
    let store = RedbStore::open(directory.path().join("data"))?;
    let audit = store.security_audit(&ApplicationPageQuery {
        after: None,
        limit: PageSize::new(1_000)?,
    })?;
    assert!(audit.items.iter().any(|record| {
        record.entry.operation == "read_artifact_content"
            && record.entry.grant_revision == 1
            && record.entry.decision_digest.starts_with("b3_")
    }));
    drop(store);
    let restarted = start(config, CONTROLLER_TOKEN).await?;
    let reopened = restarted.client.attempt("run-process", &attempt_id).await?;
    assert_eq!(reopened.context_access, "authorized");
    assert_eq!(reopened.context, Some(inspected_context));
    assert_eq!(reopened.context_manifest, attempt.context_manifest);
    restarted.stop().await
}

#[test]
fn daemon_startup_refuses_legacy_sidecar_and_peer_prototype_authority() -> TestResult {
    let directory = tempfile::tempdir()?;
    let config = configuration(&directory, 16)?;
    let data_root = directory.path().join("data");
    fs::create_dir_all(&data_root)?;
    fs::write(
        data_root.join("control-state-v1.json"),
        br#"{"schema_version":1,"layouts":{},"commands":{"broken":true}}"#,
    )?;
    let result = DaemonHost::start(config);
    assert!(result.is_err());
    for prototype in ["peer-executions-v1", "peer-artifacts-v1"] {
        let directory = tempfile::tempdir()?;
        let config = configuration(&directory, 16)?;
        fs::create_dir_all(directory.path().join("data").join(prototype))?;
        assert!(DaemonHost::start(config).is_err());
    }
    Ok(())
}

#[test]
fn page_requests_remain_explicitly_bounded() -> TestResult {
    PageRequest {
        cursor: None,
        limit: 100,
    }
    .validate()?;
    Ok(())
}
