//! Bounded queues, version negotiation and fail-closed startup.
use super::support::*;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn unsupported_protocol_and_retired_command_routes_refuse_before_dispatch() -> TestResult {
    let directory = TempDir::new()?;
    let daemon = start(configuration(&directory, 16)?, CONTROLLER_TOKEN).await?;
    assert_eq!(
        daemon.client.negotiate().await?.protocol,
        ProtocolVersion::CURRENT
    );
    let raw = reqwest::Client::new();
    for protocol in [
        ProtocolVersion { major: 2, minor: 0 },
        ProtocolVersion {
            major: ProtocolVersion::CURRENT.major,
            minor: ProtocolVersion::CURRENT.minor + 1,
        },
    ] {
        let response = raw
            .post(daemon.endpoint.join("v1/version")?)
            .bearer_auth(CONTROLLER_TOKEN)
            .json(&serde_json::json!({"protocol":protocol}))
            .send()
            .await?;
        assert_eq!(response.status(), reqwest::StatusCode::UPGRADE_REQUIRED);
        let error: milkdrift_control_protocol::ErrorEnvelope =
            decode_json(&response.bytes().await?)?;
        assert_eq!(error.code, ErrorCode::UnsupportedVersion);
        assert!(!error.retryable);
    }
    for retired in ["v1/commands", "v1/revisions", "v1/runs"] {
        let response = raw
            .post(daemon.endpoint.join(retired)?)
            .bearer_auth(CONTROLLER_TOKEN)
            .json(&serde_json::json!({}))
            .send()
            .await?;
        assert_eq!(
            response.status(),
            reqwest::StatusCode::NOT_FOUND,
            "{retired}"
        );
    }
    daemon.stop().await
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn daemon_bounded_overload_returns_stable_error() -> TestResult {
    let directory = tempfile::tempdir()?;
    let daemon = start(configuration(&directory, 1)?, CONTROLLER_TOKEN).await?;
    let raw = reqwest::Client::new();
    let target = daemon.endpoint.join("v1/workflows?limit=100")?;
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
