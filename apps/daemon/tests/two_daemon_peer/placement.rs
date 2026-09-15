//! Two approved serving hosts, an origin with a local look-alike, and durable provenance.
use super::*;
use milkdrift_authority::CapabilityAuthorityScopeBuilder;
use milkdrift_blueprint::{DataPort, SchemaRef};
use milkdrift_capability::{
    CapabilityCategory, ExecutionTrustClass, Locality, PlacementRequirement, SchemaId,
    SideEffectClass,
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
    document["profile"]["arguments"] = json!(["placement", root.path(), host]);
    document["profile"]["side_effect"] = json!("non_idempotent_write");
    document["profile"]["stdout"] = json!({
        "max_capture_bytes":1024,"stream_progress":false,"max_progress_events":0,
        "overflow_action":"terminate","artifact_name":"host_result"
    });
    document["profile"]["limits"]["wall_timeout_ms"] = json!(5000);
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
    let PeerHostConfig::Enabled {
        relationships,
        serving,
        ..
    } = &mut config.peers
    else {
        return Err("peer mode disabled".into());
    };
    serving.observation_hot_retention_ms = 30_000;
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

fn placement_blueprint(workflow: &str, hosts: &[&str]) -> TestResult<serde_json::Value> {
    let mut mutations = Vec::new();
    for (index, host) in hosts.iter().enumerate() {
        let mut task = Node::new(
            NodeId::new(format!("repository-{index}"))?,
            NodeKind::task_direct_inputs(
                CapabilityRequirement::new(OperationId::new("process.execute")?)
                    .category(CapabilityCategory::Process)
                    .execution_trust(ExecutionTrustClass::TrustedHostProcess)
                    .maximum_side_effect(SideEffectClass::NonIdempotentWrite)
                    .with_placement(PlacementRequirement::new(
                        Some(BTreeSet::from([Locality::Peer])),
                        Some(BTreeSet::from([PeerId::new(*host)?])),
                    )?),
            )?,
        )?
        .with_control_output(PortId::new("next")?)?
        .with_data_output(
            PortId::new("host_result")?,
            DataPort::output(SchemaRef::new(SchemaId::new("milkdrift.value")?, 1)?),
        )?;
        if index > 0 {
            task = task.with_control_input(PortId::new("in")?)?;
            mutations.push(Mutation::AddEdge {
                edge: Edge::new(
                    EdgeId::new(format!("step-{index}"))?,
                    EdgeKind::Control,
                    NodeId::new(format!("repository-{}", index - 1))?,
                    PortId::new("next")?,
                    task.id().clone(),
                    PortId::new("in")?,
                ),
            });
        }
        mutations.push(Mutation::AddNode { node: task });
    }
    mutations.push(Mutation::AddNode {
        node: Node::new(
            NodeId::new("done")?,
            NodeKind::Terminal {
                outcome: TerminalOutcome::Success,
            },
        )?
        .with_control_input(PortId::new("in")?)?,
    });
    mutations.push(Mutation::AddEdge {
        edge: Edge::new(
            EdgeId::new("finished")?,
            EdgeKind::Control,
            NodeId::new(format!("repository-{}", hosts.len() - 1))?,
            PortId::new("next")?,
            NodeId::new("done")?,
            PortId::new("in")?,
        ),
    });
    let revision = BlueprintRevision::genesis(
        WorkflowId::new(workflow)?,
        MutationBatch::new(mutations)?,
        AuthorRef::new("human:placement")?,
        "pin each repository operation to its approved host",
    )?;
    Ok(serde_json::from_slice(
        &BlueprintRevisionDocument::new(&revision).to_canonical_json()?,
    )?)
}

async fn import_and_start(
    client: &ControlClient,
    workflow: &str,
    hosts: &[&str],
) -> TestResult<milkdrift_control_protocol::CommandAccepted> {
    let imported = client
        .submit(&command(
            &format!("import-{workflow}"),
            Command::ImportBlueprint {
                document: if workflow == "placed-repositories" {
                    serde_json::from_slice(include_bytes!(
                        "../../../../examples/operator/peer-placement.json"
                    ))?
                } else {
                    placement_blueprint(workflow, hosts)?
                },
            },
        ))
        .await?;
    let revision = imported.value["revision_id"]
        .as_str()
        .ok_or("import omitted revision")?;
    Ok(client
        .submit(&command(
            &format!("start-{workflow}"),
            Command::StartRun {
                run_id: format!("run-{workflow}"),
                workflow_id: workflow.to_owned(),
                revision_id: revision.to_owned(),
            },
        ))
        .await?)
}

async fn completed_attempts(
    client: &ControlClient,
    run: &str,
) -> TestResult<Vec<milkdrift_control_protocol::AttemptRead>> {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(15);
    loop {
        let view = client.run(run).await?;
        if view.uncertainty_count > 0 {
            let mut detail = Vec::new();
            for node in &view.nodes {
                if let Some(id) = &node.latest_attempt_id {
                    detail.push(client.attempt(run, id).await?.terminal_detail);
                }
            }
            return Err(format!("placement uncertain: {detail:?}").into());
        }
        if view.lifecycle == "terminal" {
            if view.terminal.as_deref() != Some("succeeded") {
                let mut detail = Vec::new();
                for node in &view.nodes {
                    if let Some(id) = &node.latest_attempt_id {
                        detail.push(client.attempt(run, id).await?);
                    }
                }
                return Err(format!("placement failed: {view:?}; attempts={detail:?}").into());
            }
            break;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "placement run did not finish: {view:?}"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let timeline = client
        .timeline(
            run,
            &PageRequest {
                cursor: None,
                limit: 100,
            },
        )
        .await?;
    let ids: BTreeSet<_> = timeline
        .items
        .iter()
        .filter_map(|item| item.attempt_id.as_ref())
        .collect();
    let mut attempts = Vec::new();
    for id in ids {
        attempts.push(client.attempt(run, id).await?);
    }
    attempts.sort_by(|a, b| a.peer_id.cmp(&b.peer_id));
    Ok(attempts)
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
        PeerHostConfig::Enabled { relationships, .. } => relationships[0].clone(),
        _ => return Err("peers disabled".into()),
    };
    second.peer_id = "peer-b".to_owned();
    second.endpoint = b_url.to_string();
    if let PeerHostConfig::Enabled { relationships, .. } = &mut origin_config.peers {
        relationships.push(second);
    }
    origin_config.actors[0].authority.resources.network = NetworkScope::new(
        BTreeSet::from([
            NetworkProfileRef::new("peer:peer-a")?,
            NetworkProfileRef::new("peer:peer-b")?,
        ]),
        BTreeSet::from([a_address.to_string(), b_address.to_string()]),
    )?;
    origin_config.actors[0].authority.resources.capability =
        CapabilityAuthorityScopeBuilder::new(SideEffectClass::NonIdempotentWrite)
            .only_categories(BTreeSet::from([CapabilityCategory::Process]))?
            .only_operations(BTreeSet::from([OperationId::new("process.execute")?]))?
            .only_execution_trust_classes(BTreeSet::from([
                ExecutionTrustClass::TrustedHostProcess,
            ]))?
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
    // A grant narrowed to these exact hosts admits the constrained revision.
    import_and_start(&origin.client, "placed-repositories", &["peer-a", "peer-b"]).await?;
    let attempts = completed_attempts(&origin.client, "run-placed-repositories").await?;
    assert_eq!(attempts.len(), 2);
    for (attempt, (host, repository, root)) in attempts
        .iter()
        .zip([("peer-a", "repo-a", &a_root), ("peer-b", "repo-b", &b_root)])
    {
        let expected = format!("host={host} repository={repository}\n");
        assert_eq!(fs::read_to_string(root.path().join("entries"))?, expected);
        assert_eq!(attempt.peer_id.as_deref(), Some(host));
        assert_eq!(
            attempt.requirement.as_ref().ok_or("missing requirement")?["placement"]["peers"],
            json!([host])
        );
        let provenance = attempt
            .capability_provenance
            .as_ref()
            .ok_or("missing provenance")?;
        assert_eq!(provenance.locality, "peer");
        let peer = provenance
            .peer
            .as_ref()
            .ok_or("missing catalog provenance")?;
        assert_eq!(peer.peer_id, host);
        assert_eq!(peer.remote_capability_id, "golden-local-process");
        assert_eq!(peer.remote_descriptor_revision, 2);
        assert!(peer.catalog_generation > 0);
        assert!(
            attempt
                .entry_authorization
                .as_ref()
                .is_some_and(|decision| decision.allowed)
        );
        let output = attempt
            .outputs
            .iter()
            .find(|output| output.name == "host_result")
            .ok_or("missing host result")?;
        let artifact = &output.artifact;
        let content = origin
            .client
            .artifact_range(&artifact.artifact_id, 0, artifact.size - 1)
            .await?;
        assert_eq!(content.bytes, expected.as_bytes());
        assert_eq!(
            blake3::hash(&content.bytes).to_hex().as_str(),
            artifact.digest
        );
    }
    assert!(!origin_root.path().join("entries").exists());
    // An unauthorized exact peer is refused at revision admission, before any entry.
    assert!(
        import_and_start(&origin.client, "forbidden-placement", &["peer-forbidden"])
            .await
            .is_err()
    );
    for peer in ["peer-a", "peer-b"] {
        origin.client.peer_action(peer, "disconnect").await?;
        origin.client.peer_action(peer, "connect").await?;
    }
    assert_eq!(
        completed_attempts(&origin.client, "run-placed-repositories").await?,
        attempts
    );
    origin.stop().await?;
    a.stop().await?;
    b.stop().await?;
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
        completed_attempts(&origin.client, "run-placed-repositories").await?,
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
    Ok(())
}
