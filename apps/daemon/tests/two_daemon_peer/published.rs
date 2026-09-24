//! The peer adapter consumes a publication through its ordinary prepared allowance and output path.
use super::*;
#[path = "../support/published.rs"]
mod fixture;
use milkdrift_authority::{ActorRef, GrantDigest, GrantId};
use milkdrift_capability::{
    AdmissionConstraints, CapabilityCategory, CapabilityId, DescriptorBuilder, InvocationCounts,
    Locality,
};
use milkdrift_persistence::published::{
    PublishedMethod, PublishedOutput, PublishedServiceIdentity,
};
use milkdrift_workspace::{ValueKey, WorkspaceBudget};
use serde_json::json;

fn configure(
    root: &TempDir,
    local: &str,
    remote: &str,
    endpoint: &Url,
) -> TestResult<DaemonConfig> {
    let mut config = configuration_document(root, local, remote, endpoint)?;
    config.role = milkdrift_control_protocol::HostRole::WorkflowEnabled;
    config.runtime.effect_threads = 1;
    config.runtime.effect_queue = 1;
    config.serving.worker_threads = 1;
    config.runtime.controller_activation = milkdrift_daemon::ControllerActivation::Enabled;
    if local == "peer-a" {
        let path = config
            .adapters
            .process_profiles
            .first()
            .ok_or("process absent")?;
        let mut profile: serde_json::Value = serde_json::from_slice(&fs::read(path)?)?;
        profile["profile"]["stdout"] = json!({"max_capture_bytes":1024,"stream_progress":false,
            "max_progress_events":0,"overflow_action":"terminate","artifact_name":"result"});
        fs::write(path, serde_json::to_vec(&profile)?)?;
        config.runtime.publication_services.insert(
            CapabilityId::new("method:echo")?,
            GrantId::new("grant:peer-a-operator")?,
        );
    }
    let PeerHostConfig::Enabled { relationships } = &mut config.peers else {
        return Err("peers absent".into());
    };
    for relationship in relationships {
        relationship
            .capability_allow
            .insert("method:echo".to_owned());
        relationship
            .operation_allow
            .insert("method.invoke".to_owned());
        relationship.maximum_artifact_bytes = 2_097_152;
        relationship.maximum_input_units = Some(1000);
        relationship.maximum_output_units = Some(1000);
        relationship.nested_invocations = Some(InvocationCounts::new(4, 0));
    }
    Ok(config)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn peer_publication_creates_one_run_and_transfers_only_the_accepted_result() -> TestResult {
    let root_a = TempDir::new()?;
    let root_b = TempDir::new()?;
    let listener_a = tokio::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await?;
    let listener_b = tokio::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await?;
    let endpoint_a = Url::parse(&format!("http://{}/", listener_a.local_addr()?))?;
    let endpoint_b = Url::parse(&format!("http://{}/", listener_b.local_addr()?))?;
    let config_a = configure(&root_a, "peer-a", "peer-b", &endpoint_b)?;
    let config_b = configure(&root_b, "peer-b", "peer-a", &endpoint_a)?;
    let daemon_a = start_plan(config_a.validate(root_a.path())?, listener_a).await?;
    let daemon_b = start_plan(config_b.validate(root_b.path())?, listener_b).await?;
    let (base, revision) = fixture::definition()?;
    for (label, revision) in [("base", &base), ("governed", &revision)] {
        daemon_a
            .client
            .submit(&command(
                label,
                Command::ImportBlueprint {
                    document: serde_json::from_slice(
                        &BlueprintRevisionDocument::new(revision).to_canonical_json()?,
                    )?,
                },
            ))
            .await?;
    }
    let discovery = daemon_a.client.execution_discovery().await?;
    let process = discovery
        .catalog
        .entries
        .iter()
        .find(|entry| entry.descriptor.identity().as_str() == "golden-local-process")
        .ok_or("ordinary process absent")?;
    let descriptor = DescriptorBuilder::new(
        CapabilityId::new("method:echo")?,
        1,
        CapabilityCategory::Tool,
        AdmissionConstraints::new(2, 2)?,
        Locality::Local,
    )
    .operations(BTreeMap::from([(
        OperationId::new("method.invoke")?,
        process
            .descriptor
            .operation(&OperationId::new("process.execute")?)
            .ok_or("process operation absent")?
            .clone(),
    )]))
    .execution_trust(process.descriptor.execution_trust())
    .build()?;
    let authority = daemon_a.client.authority().await?;
    let method = PublishedMethod {
        schema_version: 1,
        descriptor,
        documentation: "Produce the governed process result at its owning host.".to_owned(),
        revision: revision.id().clone(),
        agreement: revision
            .semantic()
            .agreement()
            .ok_or("agreement absent")?
            .digest()
            .to_owned(),
        service: PublishedServiceIdentity {
            actor: ActorRef::new(authority.actor)?,
            grant: GrantId::new(authority.grant_id)?,
            grant_revision: authority.grant_revision,
            grant_digest: GrantDigest::new(authority.grant_digest)?,
            revocation_generation: 0,
        },
        inputs: BTreeMap::new(),
        outputs: BTreeMap::from([(
            "result".to_owned(),
            PublishedOutput {
                field: ValueKey::new("result")?,
                media_type: "application/octet-stream".to_owned(),
                maximum_bytes: 1024,
            },
        )]),
        workspace_budget: WorkspaceBudget::new(128, 65536, 1048576, 128, 1048576, 16777216)?,
        allowance: milkdrift_persistence::ControllerResourceBudget::new(
            0, None, 1000, 1000, 1048576, 4, 0,
        )?,
        maximum_outstanding: 2,
        maximum_depth: 4,
        maximum_duration_ms: 30_000,
    };
    daemon_a
        .client
        .submit(&command(
            "publish",
            Command::PublishMethod {
                document: serde_json::to_value(method)?,
                expected_previous_version: None,
            },
        ))
        .await?;
    daemon_b.client.peer_action("peer-a", "connect").await?;
    let mut outer = process_blueprint()?;
    // Re-author through the normal mutation/definition writer, preserving its workflow identity.
    let original = BlueprintRevisionDocument::from_json(&serde_json::to_vec(&outer)?)?.1;
    let node = Node::new(
        NodeId::new("process")?,
        NodeKind::task_direct_inputs(
            CapabilityRequirement::new(OperationId::new("method.invoke")?).with_placement(
                milkdrift_capability::PlacementRequirement::new(
                    Some(BTreeSet::from([Locality::Peer])),
                    Some(BTreeSet::from([PeerId::new("peer-a")?])),
                )?,
            ),
        )?,
    )?
    .with_control_output(PortId::new("next")?)?
    .with_data_output(
        PortId::new("result")?,
        milkdrift_blueprint::DataPort::output(milkdrift_blueprint::SchemaRef::new(
            milkdrift_capability::SchemaId::new("milkdrift.artifact-reference")?,
            1,
        )?),
    )?;
    let revision = original.revise(
        original.id(),
        MutationBatch::new(vec![Mutation::ReplaceNode { node }])?,
        AuthorRef::new("human:peer-b-operator")?,
        "invoke an exact remote service implementation",
    )?;
    for (label, revision) in [("outer-base", &original), ("outer", &revision)] {
        outer =
            serde_json::from_slice(&BlueprintRevisionDocument::new(revision).to_canonical_json()?)?;
        daemon_b
            .client
            .submit(&command(
                label,
                Command::ImportBlueprint {
                    document: outer.clone(),
                },
            ))
            .await?;
    }
    daemon_b
        .client
        .submit(&command(
            "start",
            Command::StartRun {
                run_id: "peer-published".to_owned(),
                workflow_id: revision.semantic().workflow().to_string(),
                revision_id: revision.id().to_string(),
            },
        ))
        .await?;
    let mut result = daemon_b.client.run("peer-published").await?;
    for _ in 0..200 {
        if result.terminal.is_some() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
        result = daemon_b.client.run("peer-published").await?;
    }
    assert_eq!(result.terminal.as_deref(), Some("succeeded"));
    let timeline = daemon_b
        .client
        .timeline(
            "peer-published",
            &PageRequest {
                cursor: None,
                limit: 128,
            },
        )
        .await?;
    let attempt_id = timeline
        .items
        .iter()
        .find_map(|item| item.attempt_id.as_ref())
        .ok_or("remote attempt absent")?;
    let attempt = daemon_b
        .client
        .attempt("peer-published", attempt_id)
        .await?;
    let nested = attempt
        .usage
        .as_ref()
        .and_then(|usage| usage.nested_work.as_ref())
        .ok_or("nested usage absent")?;
    assert_eq!(nested.process_admissions, 1);
    assert_eq!(nested.model_admissions, 0);
    assert!(nested.artifact_bytes >= 6);
    let output = attempt
        .outputs
        .iter()
        .find(|output| output.name == "result")
        .ok_or("accepted public output absent")?;
    let bytes = daemon_b
        .client
        .artifact_range(&output.artifact.artifact_id, 0, output.artifact.size - 1)
        .await?;
    assert_eq!(bytes.bytes, b"golden\n");
    assert_eq!(
        blake3::hash(&bytes.bytes).to_hex().as_str(),
        output.artifact.digest
    );
    let metadata = daemon_b
        .client
        .artifact_metadata(&output.artifact.artifact_id)
        .await?;
    assert_eq!(metadata.sensitivity, "restricted");
    let runs = daemon_a
        .client
        .runs(
            None,
            None,
            &PageRequest {
                limit: 8,
                cursor: None,
            },
        )
        .await?;
    assert_eq!(runs.items.len(), 1);
    let internal = daemon_a.client.run(&runs.items[0].run_id).await?;
    let source = internal
        .published_source
        .ok_or("internal link not inspectable")?;
    assert_eq!(source["type"], "serving");
    assert_eq!(source["caller"]["principal"]["peer"], "peer-b");
    assert_eq!(internal.terminal.as_deref(), Some("succeeded"));
    daemon_b.stop().await?;
    daemon_a.stop().await?;
    Ok(())
}
