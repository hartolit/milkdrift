//! Two approved serving hosts, an origin with a local look-alike, and durable provenance.
use super::*;
use milkdrift_authority::CapabilityAuthorityScopeBuilder;
use milkdrift_capability::{
    CapabilityCategory, ExecutionTrustClass, Locality, OperationId, SideEffectClass,
};
use serde_json::json;

fn placement_profile(
    root: &TempDir,
    host: &str,
    repository: &str,
) -> TestResult<std::path::PathBuf> {
    let path = configured_process_profile(root)?;
    fs::write(root.path().join("repository.txt"), repository)?;
    let mut document: serde_json::Value = serde_json::from_slice(&fs::read(&path)?)?;
    *document
        .pointer_mut("/profile/arguments")
        .ok_or("fixture field /profile/arguments absent")? =
        json!(["placement", root.path(), host]);
    *document
        .pointer_mut("/profile/side_effect")
        .ok_or("fixture field /profile/side_effect absent")? = json!("non_idempotent_write");
    *document
        .pointer_mut("/profile/stdout")
        .ok_or("fixture field /profile/stdout absent")? = json!({
        "max_capture_bytes":1024,"stream_progress":false,"max_progress_events":0,
        "overflow_action":"terminate","artifact_name":"host_result"
    });
    *document
        .pointer_mut("/profile/limits/wall_timeout_ms")
        .ok_or("fixture field /profile/limits/wall_timeout_ms absent")? = json!(5000);
    fs::write(&path, serde_json::to_vec(&document)?)?;
    Ok(path)
}

fn placement_config(
    root: &TempDir,
    host: &str,
    remote: &str,
    endpoint: &Url,
    repository: &str,
) -> TestResult<DaemonConfig> {
    let mut config = configuration_document(root, host, remote, endpoint)?;
    config.adapters.process_profiles = vec![placement_profile(root, host, repository)?];
    let PeerHostConfig::Enabled { relationships, .. } = &mut config.peers else {
        return Err("peer mode disabled".into());
    };
    config.serving.observation_hot_retention_ms = 30_000;
    for relationship in relationships {
        relationship
            .actions
            .extend([PeerAction::ArtifactDownload, PeerAction::ArtifactUpload]);
        relationship.artifact_sensitivities =
            BTreeSet::from([milkdrift_workspace::ArtifactSensitivity::Restricted]);
        relationship.maximum_side_effect = PeerSideEffectConfig::NonIdempotentWrite;
    }
    Ok(config)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 6)]
async fn two_repository_hosts_enforce_placement_return_artifacts_and_retain_selection_after_restart()
-> TestResult {
    let origin_root = tempfile::tempdir()?;
    let a_root = tempfile::tempdir()?;
    let b_root = tempfile::tempdir()?;
    let origin_listener = tokio::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await?;
    let a_listener = tokio::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await?;
    let b_listener = tokio::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await?;
    let origin_address = origin_listener.local_addr()?;
    let a_address = a_listener.local_addr()?;
    let b_address = b_listener.local_addr()?;
    let origin_url = Url::parse(&format!("http://{origin_address}/"))?;
    let a_url = Url::parse(&format!("http://{a_address}/"))?;
    let b_url = Url::parse(&format!("http://{b_address}/"))?;
    let a_config = placement_config(&a_root, "peer-a", "peer-origin", &origin_url, "repo-a")?;
    let b_config = placement_config(&b_root, "peer-b", "peer-origin", &origin_url, "repo-b")?;
    let mut origin_config = placement_config(
        &origin_root,
        "peer-origin",
        "peer-a",
        &a_url,
        "local-lookalike",
    )?;
    let mut second = match &origin_config.peers {
        PeerHostConfig::Enabled { relationships, .. } => {
            relationships.first().ok_or("relationship absent")?.clone()
        }
        PeerHostConfig::Disabled => return Err("peers disabled".into()),
    };
    second.peer_id = "peer-b".to_owned();
    second.endpoint = b_url.to_string();
    if let PeerHostConfig::Enabled { relationships, .. } = &mut origin_config.peers {
        relationships.push(second);
    }
    origin_config
        .actors
        .get_mut(0)
        .ok_or("fixture actor absent")?
        .authority
        .resources
        .network = NetworkScope::new(
        BTreeSet::from([
            NetworkProfileRef::new("peer:peer-a")?,
            NetworkProfileRef::new("peer:peer-b")?,
        ]),
        BTreeSet::from([a_address.to_string(), b_address.to_string()]),
    )?;
    origin_config
        .actors
        .get_mut(0)
        .ok_or("fixture actor absent")?
        .authority
        .resources
        .capability = CapabilityAuthorityScopeBuilder::new(SideEffectClass::NonIdempotentWrite)
        .only_categories(BTreeSet::from([CapabilityCategory::Process]))?
        .only_operations(BTreeSet::from([OperationId::new("process.execute")?]))?
        .only_execution_trust_classes(BTreeSet::from([ExecutionTrustClass::TrustedHostProcess]))?
        .only_localities(BTreeSet::from([Locality::Peer]))?
        .only_peers(BTreeSet::from([
            PeerId::new("peer-a")?,
            PeerId::new("peer-b")?,
        ]))?
        .build();
    let a = start_plan(a_config.clone().validate(a_root.path())?, a_listener).await?;
    let b = start_plan(b_config.clone().validate(b_root.path())?, b_listener).await?;
    let origin = start_plan(
        origin_config.clone().validate(origin_root.path())?,
        origin_listener,
    )
    .await?;
    for peer in ["peer-a", "peer-b"] {
        assert!(origin.client.peer_action(peer, "connect").await?.connected);
    }
    let advertised = origin
        .client
        .capabilities()
        .await?
        .into_iter()
        .map(|capability| (capability.capability_id.clone(), capability))
        .collect::<BTreeMap<_, _>>();
    // A grant narrowed to these exact hosts admits their explicitly selected task capabilities.
    let receipt =
        submit_remote(&origin.client, "placed-repositories", &["peer-a", "peer-b"]).await?;
    let attempts = completed_tasks(&origin.client, &receipt.workflow_id).await?;
    assert_eq!(attempts.len(), 2);
    for (attempt, (host, repository, root)) in attempts
        .iter()
        .zip([("peer-a", "repo-a", &a_root), ("peer-b", "repo-b", &b_root)])
    {
        let expected = format!("host={host} repository={repository}\n");
        assert_eq!(fs::read_to_string(root.path().join("entries"))?, expected);
        assert_eq!(
            attempt.definition.title,
            format!("Inspect repository on {host}")
        );
        assert!(attempt.definition.capability.id.starts_with("peer:"));
        let frozen = attempt.frozen.as_ref().ok_or("frozen task start absent")?;
        let selected = advertised
            .get(&attempt.definition.capability.id)
            .ok_or("selected capability was not advertised")?;
        assert!(frozen.entered);
        assert_eq!(selected.peer_id.as_deref(), Some(host));
        assert_eq!(frozen.capability_generation, selected.generation);
        let result = attempt.result.as_ref().ok_or("peer result absent")?;
        // Process stdout has an opaque byte media type; verify the retained bytes through
        // the authorized artifact route instead of treating it as a text preview.
        assert!(result.text.is_empty());
        let artifact = result
            .artifacts
            .first()
            .ok_or("peer output artifact absent")?;
        let metadata = origin.client.artifact_metadata(artifact).await?;
        assert_eq!(metadata.content_type, "application/octet-stream");
        let content = origin
            .client
            .artifact_range(artifact, 0, metadata.size - 1)
            .await?;
        assert_eq!(content.bytes, expected.as_bytes());
        assert_eq!(
            blake3::hash(&content.bytes).to_hex().as_str(),
            metadata.digest
        );
    }
    assert!(!origin_root.path().join("entries").exists());
    // The constrained catalog cannot supply a capability for an unauthorized peer.
    assert!(
        submit_remote(&origin.client, "forbidden-placement", &["peer-forbidden"])
            .await
            .is_err()
    );
    for peer in ["peer-a", "peer-b"] {
        origin.client.peer_action(peer, "disconnect").await?;
        origin.client.peer_action(peer, "connect").await?;
    }
    assert_eq!(
        completed_tasks(&origin.client, &receipt.workflow_id).await?,
        attempts
    );
    origin.stop().await?;
    a.stop().await?;
    b.stop().await?;
    let retained_selections = {
        use milkdrift_persistence::WorkflowStore;
        let store = milkdrift_redb_store::RedbStore::open(origin_root.path().join("data"))?;
        let saved = store
            .workflow(&milkdrift_blueprint::WorkflowId::new(&receipt.workflow_id)?)?
            .ok_or("durable workflow absent")?;
        let record: milkdrift_runtime::WorkflowRecord = serde_json::from_value(saved.document)?;
        let mut peers = Vec::new();
        for task in record.tasks.values() {
            let dispatch = &task
                .frozen
                .as_ref()
                .ok_or("frozen selection absent")?
                .dispatch;
            assert_eq!(dispatch.resolution().locality(), Locality::Peer);
            let selected = advertised
                .get(dispatch.resolution().capability().as_str())
                .ok_or("durable selection was not advertised")?;
            assert_eq!(
                dispatch.resolution().capability().as_str(),
                task.definition
                    .capability
                    .exact_capability()
                    .ok_or("task lost exact capability")?
                    .as_str()
            );
            assert_eq!(
                dispatch.resolution().descriptor_revision(),
                selected.generation
            );
            assert_eq!(
                dispatch.resolution().peer().map(PeerId::as_str),
                selected.peer_id.as_deref()
            );
            assert!(dispatch.entry_authorization().is_allowed());
            peers.push(
                dispatch
                    .resolution()
                    .peer()
                    .ok_or("peer provenance absent")?
                    .to_string(),
            );
        }
        peers.sort();
        assert_eq!(peers, vec!["peer-a", "peer-b"]);
        record
            .tasks
            .iter()
            .map(|(id, task)| {
                Ok((
                    id.clone(),
                    task.frozen
                        .as_ref()
                        .ok_or("retained selection absent")?
                        .dispatch
                        .resolution()
                        .clone(),
                ))
            })
            .collect::<TestResult<BTreeMap<_, _>>>()?
    };
    let a = start_plan(
        a_config.validate(a_root.path())?,
        tokio::net::TcpListener::bind(a_address).await?,
    )
    .await?;
    let b = start_plan(
        b_config.validate(b_root.path())?,
        tokio::net::TcpListener::bind(b_address).await?,
    )
    .await?;
    let origin = start_plan(
        origin_config.validate(origin_root.path())?,
        tokio::net::TcpListener::bind(origin_address).await?,
    )
    .await?;
    assert_eq!(
        completed_tasks(&origin.client, &receipt.workflow_id).await?,
        attempts
    );
    for (host, repository, root) in [("peer-a", "repo-a", &a_root), ("peer-b", "repo-b", &b_root)] {
        assert_eq!(
            fs::read_to_string(root.path().join("entries"))?,
            format!("host={host} repository={repository}\n")
        );
    }
    assert!(!origin_root.path().join("entries").exists());
    origin.stop().await?;
    a.stop().await?;
    b.stop().await?;
    {
        use milkdrift_persistence::WorkflowStore;
        let store = milkdrift_redb_store::RedbStore::open(origin_root.path().join("data"))?;
        let snapshot = store
            .workflow(&milkdrift_blueprint::WorkflowId::new(&receipt.workflow_id)?)?
            .ok_or("restarted workflow absent")?;
        let record: milkdrift_runtime::WorkflowRecord = serde_json::from_value(snapshot.document)?;
        let after = record
            .tasks
            .iter()
            .map(|(id, task)| {
                Ok((
                    id.clone(),
                    task.frozen
                        .as_ref()
                        .ok_or("restarted selection absent")?
                        .dispatch
                        .resolution()
                        .clone(),
                ))
            })
            .collect::<TestResult<BTreeMap<_, _>>>()?;
        assert_eq!(after, retained_selections);
    }
    Ok(())
}
