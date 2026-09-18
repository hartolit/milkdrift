//! Useful independent execution through authenticated public APIs and production adapters.
use super::support::*;
use milkdrift_capability::{
    InputReference, InvocationId, InvocationRequest, InvocationValueReference,
    ResolvedCapabilitySnapshot, TerminalStatus,
};
use milkdrift_control_protocol::{HostRole, InputUploadRequest};
use milkdrift_peer_protocol::{
    DirectInvocationRequest, InvocationAcceptance, InvocationLookup, InvocationOrigin,
    PeerRequestId,
};
use milkdrift_persistence::{RunQueryStore, RunSummaryFilter, RunSummaryPageQuery};
use serde_json::json;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn direct_process_upload_replay_and_restart_have_no_workflow_records() -> TestResult {
    let directory = tempfile::tempdir()?;
    let profile_path = configured_process_profile(&directory)?;
    let marker = directory.path().join("external-entries");
    let mut profile: serde_json::Value = serde_json::from_slice(&fs::read(&profile_path)?)?;
    profile["profile"]["capability"] = json!("operator-process");
    profile["profile"]["arguments"] = json!(["append-stdin", marker]);
    profile["profile"]["side_effect"] = json!("non_idempotent_write");
    profile["profile"]["inputs"] = json!([{"input":"source","relative_path":"source.txt"}]);
    profile["profile"]["stdin"] = json!({"type":"input","input":"source","max_bytes":1024});
    profile["profile"]["stdout"] = json!({"max_capture_bytes":1024,"stream_progress":false,"max_progress_events":0,"overflow_action":"terminate","artifact_name":"result"});
    fs::write(&profile_path, serde_json::to_vec(&profile)?)?;
    let mut config =
        configuration_document_with_process_profiles(&directory, 32, vec![profile_path])?;
    config.role = HostRole::ExecutionOnly;
    // Exercise the maintained recipe's grant, including the sensitivity needed by default
    // uploads and adapter outputs. Only the fixture's executable/filesystem paths differ.
    let mut example: DaemonConfig = toml::from_str(include_str!(
        "../../../../examples/operator/execution-only.toml"
    ))?;
    config.actors[0].authority = example.actors.remove(0).authority;
    config.actors[0].authority.resources.filesystem = vec![
        milkdrift_authority::FilesystemScope::from_canonical_host_path(
            super::process::executable()?
                .parent()
                .ok_or("executable parent missing")?,
            std::collections::BTreeSet::from([milkdrift_authority::AccessMode::Execute]),
        )?,
        milkdrift_authority::FilesystemScope::from_canonical_host_path(
            &std::env::temp_dir().canonicalize()?,
            std::collections::BTreeSet::from([
                milkdrift_authority::AccessMode::Read,
                milkdrift_authority::AccessMode::Write,
            ]),
        )?,
    ];
    config.actors[1].authority = ActorGrantConfig::dangerous_administrator();
    config.serving.clients.maximum_uploaded_artifacts = 1;
    let plan = config.validate(directory.path())?;
    let daemon = start(plan.clone(), CONTROLLER_TOKEN).await?;
    assert_eq!(daemon.client.health().await?.role, HostRole::ExecutionOnly);
    let discovery = daemon.client.execution_discovery().await?;
    assert_eq!(discovery.host.as_str(), "host:local");
    let entry = discovery
        .catalog
        .entries
        .first()
        .ok_or("direct process missing from catalog")?;
    let bytes = b"independent process input\n";
    let upload = InputUploadRequest::from_content(
        discovery.host.to_string(),
        "file-1".to_owned(),
        "text/plain".to_owned(),
        "restricted".to_owned(),
        bytes,
    )?;
    let mut wrong_host = upload.clone();
    wrong_host.host = "another-host".to_owned();
    assert!(matches!(daemon.client.upload_input(&wrong_host).await,
        Err(ClientError::Api(error)) if error.code == ErrorCode::InvalidInput));
    let mut altered = upload.clone();
    altered.digest = "0".repeat(64);
    assert!(daemon.client.upload_input(&altered).await.is_err());
    let mut oversized = upload.clone();
    oversized.content_base64 =
        "A".repeat(milkdrift_control_protocol::MAX_INPUT_UPLOAD_BYTES.div_ceil(3) * 4 + 4);
    assert!(daemon.client.upload_input(&oversized).await.is_err());
    assert!(!marker.exists());
    let metadata = daemon.client.upload_input(&upload).await?;
    assert_eq!(daemon.client.upload_input(&upload).await?, metadata);
    let changed = InputUploadRequest::from_content(
        discovery.host.to_string(),
        "file-1".to_owned(),
        "text/plain".to_owned(),
        "restricted".to_owned(),
        b"different",
    )?;
    assert!(
        matches!(daemon.client.upload_input(&changed).await, Err(ClientError::Api(error)) if error.code == ErrorCode::Conflict)
    );
    let mut second = upload.clone();
    second.upload_id = "file-2".to_owned();
    assert!(
        daemon.client.upload_input(&second).await.is_err(),
        "actor upload quota was bypassed"
    );
    let observer = client(&daemon.endpoint, OBSERVER_TOKEN)?;
    assert!(
        matches!(observer.upload_input(&upload).await, Err(ClientError::Api(error)) if error.code == ErrorCode::Unauthorized)
    );
    let input = milkdrift_capability::ArtifactReference::new(
        &metadata.artifact_id,
        &metadata.digest,
        Some(metadata.content_type.clone()),
        Some(metadata.size),
    )?;
    let request = DirectInvocationRequest {
        host: discovery.host,
        request_id: PeerRequestId::new("process-request")?,
        catalog_generation: discovery.catalog.generation,
        catalog_digest: discovery.catalog.digest,
        selection: ResolvedCapabilitySnapshot::from_descriptor(
            &entry.descriptor,
            &OperationId::new("process.execute")?,
        )?,
        request: InvocationRequest::new(
            InvocationId::new("client-invocation")?,
            entry.descriptor.identity().clone(),
            OperationId::new("process.execute")?,
            None,
            None,
            vec![InputReference::new(
                "source",
                InvocationValueReference::Artifact { reference: input },
            )?],
            BTreeMap::new(),
        )?,
        limits: milkdrift_peer_protocol::ExecutionLimits {
            artifact_bytes: 4096,
            duration_ms: 3000,
            cost_micros: 0,
            cost_currency: None,
            input_units: None,
            output_units: None,
            observations: 32,
        },
        deadline_unix_ms: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_millis() as u64
            + 30_000,
    };
    let accepted = daemon.client.invoke(&request).await?;
    let InvocationAcceptance::Accepted { execution, .. } = accepted else {
        return Err(format!("not accepted: {accepted:?}").into());
    };
    assert!(matches!(
        observer.invocation_lookup(&request.request_id).await?,
        InvocationLookup::NotAccepted { .. }
    ));
    assert!(
        matches!(observer.invocation(&execution).await, Err(ClientError::Api(error)) if error.code == ErrorCode::NotFound)
    );
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    let page = loop {
        let page = daemon
            .client
            .invocation_observations(&execution, 0, 32)
            .await?;
        if page.closed {
            break page;
        }
        if tokio::time::Instant::now() > deadline {
            return Err(format!("direct process did not finish: {page:?}").into());
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    };
    let terminal = page
        .observations
        .iter()
        .find_map(|entry| entry.event.kind().terminal())
        .ok_or("terminal missing")?;
    assert_eq!(terminal.status(), TerminalStatus::Success, "{page:?}");
    let mut after = 0;
    let mut paged = Vec::new();
    loop {
        let next = daemon
            .client
            .invocation_observations(&execution, after, 1)
            .await?;
        assert!(next.next_sequence > after);
        after = next.next_sequence;
        paged.extend(next.observations);
        if next.closed {
            assert!(next.terminal);
            break;
        }
        assert!(!next.terminal);
    }
    assert_eq!(paged, page.observations);
    let (_, output) = page
        .observations
        .iter()
        .find_map(|entry| entry.event.kind().output())
        .ok_or("useful process output missing")?;
    assert_eq!(
        daemon
            .client
            .artifact_range(output.identity(), 0, bytes.len() as u64 - 1)
            .await?
            .bytes,
        bytes
    );
    assert_eq!(
        daemon.client.invocation(&execution).await?.origin,
        InvocationOrigin::Direct
    );
    assert!(matches!(
        daemon.client.invoke(&request).await?,
        InvocationAcceptance::Accepted { replayed: true, .. }
    ));
    assert_eq!(fs::read(&marker)?, bytes);
    daemon.stop().await?;
    {
        let store = RedbStore::open(directory.path().join("data"))?;
        assert!(
            store
                .run_summaries(&RunSummaryPageQuery {
                    filter: RunSummaryFilter::default(),
                    cursor: None,
                    limit: PageSize::new(1)?
                })?
                .runs
                .is_empty()
        );
        let metadata = store
            .metadata(&ArtifactId::new(metadata.artifact_id)?)?
            .ok_or("input metadata missing")?;
        assert!(
            matches!(metadata.provenance().producer(), CausalReference::ClientUpload { client, .. } if client.as_str() == "human:integration-controller")
        );
        let output = store
            .metadata(&ArtifactId::new(output.identity())?)?
            .ok_or("output metadata missing")?;
        assert!(matches!(
            output.provenance().producer(),
            CausalReference::HostInvocation { .. }
        ));
    }
    let reopened = start(plan, CONTROLLER_TOKEN).await?;
    assert!(matches!(
        reopened.client.invoke(&request).await?,
        InvocationAcceptance::Accepted { replayed: true, .. }
    ));
    assert_eq!(fs::read(&marker)?, bytes);
    reopened.stop().await
}
