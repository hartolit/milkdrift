//! Public comparisons locate every saved semantic field without exposing its contents.
use super::support::*;
use milkdrift_blueprint::{
    FieldId, GoverningAgreement, InterfaceField, SchemaRef, WorkflowInterface,
};
use milkdrift_capability::{BoundedJson, ExtensionKey, SchemaId};
use milkdrift_control_protocol::RevisionChange;
use std::collections::BTreeSet;

async fn import(client: &ControlClient, revision: &BlueprintRevision) -> TestResult {
    client
        .submit(&request(
            revision.id().as_str(),
            None,
            Command::ImportBlueprint {
                document: serde_json::from_slice(
                    &BlueprintRevisionDocument::new(revision).to_canonical_json()?,
                )?,
            },
        ))
        .await?;
    Ok(())
}

fn revision(workflow: &str, prefix: &str, count: usize) -> TestResult<BlueprintRevision> {
    let mut mutations = Vec::new();
    for index in 0..count {
        let id = NodeId::new(format!("{prefix}.{index:04}"))?;
        let mut node = Node::new(
            id.clone(),
            if index + 1 == count {
                NodeKind::Terminal {
                    outcome: TerminalOutcome::Success,
                }
            } else {
                NodeKind::Wait { duration_ms: 1 }
            },
        )?;
        if index > 0 {
            node = node.with_control_input(PortId::new("in")?)?;
            mutations.push(Mutation::AddEdge {
                edge: Edge::new(
                    EdgeId::new(format!("{prefix}.{index:04}"))?,
                    EdgeKind::Control,
                    NodeId::new(format!("{prefix}.{:04}", index - 1))?,
                    PortId::new("out")?,
                    id,
                    PortId::new("in")?,
                ),
            });
        }
        if index + 1 < count {
            node = node.with_control_output(PortId::new("out")?)?;
        }
        mutations.push(Mutation::AddNode { node });
    }
    Ok(BlueprintRevision::genesis(
        WorkflowId::new(workflow)?,
        MutationBatch::new(mutations)?,
        AuthorRef::new("human:comparison")?,
        "comparison fixture",
    )?)
}

fn summary(subject: &str, identity: Option<&str>, change: &str) -> RevisionChange {
    RevisionChange {
        subject: subject.into(),
        identity: identity.map(str::to_owned),
        change: change.into(),
        detail: serde_json::Value::Null,
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn comparison_reports_modified_ports_extensions_nodes_and_edges() -> TestResult {
    let directory = TempDir::new()?;
    let daemon = start(
        configuration_document_with_process_profiles(&directory, 64, vec![])?
            .validate(directory.path())?,
        CONTROLLER_TOKEN,
    )
    .await?;
    let (base, _) = super::published::fixture::definition()?;
    import(&daemon.client, &base).await?;
    let schema = SchemaRef::new(SchemaId::new("milkdrift.artifact-reference")?, 1)?;
    let metadata = |value| -> TestResult<BlueprintMetadata> {
        Ok(BlueprintMetadata::new(
            "comparison",
            "",
            BTreeSet::new(),
            BTreeMap::from([(
                ExtensionKey::new("org.example/value")?,
                BoundedJson::new(serde_json::json!(value))?,
            )]),
        )?)
    };
    let before = base.revise(
        base.id(),
        MutationBatch::new(vec![
            Mutation::SetMetadata {
                metadata: metadata(1)?,
            },
            Mutation::SetInterface {
                interface: WorkflowInterface::new(
                    [(
                        FieldId::new("brief")?,
                        InterfaceField::required(schema.clone()),
                    )],
                    [(
                        FieldId::new("result")?,
                        InterfaceField::required(schema.clone()),
                    )],
                )?,
            },
        ])?,
        AuthorRef::new("human:a")?,
        "before",
    )?;
    import(&daemon.client, &before).await?;
    let begin = before
        .semantic()
        .edges()
        .get(&EdgeId::new("begin")?)
        .ok_or("begin absent")?;
    let finish = before
        .semantic()
        .edges()
        .get(&EdgeId::new("finish")?)
        .ok_or("finish absent")?;
    let after = before.revise(
        before.id(),
        MutationBatch::new(vec![
            Mutation::SetMetadata {
                metadata: metadata(2)?,
            },
            Mutation::SetInterface {
                interface: WorkflowInterface::new(
                    [(
                        FieldId::new("brief")?,
                        InterfaceField::optional(schema.clone()),
                    )],
                    [(FieldId::new("result")?, InterfaceField::optional(schema))],
                )?,
            },
            Mutation::ReplaceNode {
                node: Node::new(NodeId::new("start")?, NodeKind::Wait { duration_ms: 500 })?
                    .with_control_output(PortId::new("out")?)?,
            },
            Mutation::ReplaceEdge {
                edge: Edge::new(
                    begin.id().clone(),
                    finish.kind(),
                    finish.source_node().clone(),
                    finish.source_port().clone(),
                    finish.target_node().clone(),
                    finish.target_port().clone(),
                ),
            },
            Mutation::ReplaceEdge {
                edge: Edge::new(
                    finish.id().clone(),
                    begin.kind(),
                    begin.source_node().clone(),
                    begin.source_port().clone(),
                    begin.target_node().clone(),
                    begin.target_port().clone(),
                ),
            },
        ])?,
        AuthorRef::new("human:a")?,
        "after",
    )?;
    import(&daemon.client, &after).await?;
    assert_eq!(
        daemon
            .client
            .revision_diff(before.id().as_str(), after.id().as_str())
            .await?
            .changes,
        vec![
            summary("extension", Some("org.example/value"), "changed"),
            summary("input", Some("brief"), "changed"),
            summary("output", Some("result"), "changed"),
            summary("node", Some("start"), "changed"),
            summary("edge", Some("begin"), "changed"),
            summary("edge", Some("finish"), "changed")
        ]
    );
    daemon.stop().await?;
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn comparison_covers_metadata_interfaces_agreements_and_ignores_provenance() -> TestResult {
    let directory = TempDir::new()?;
    let mut config = configuration_document_with_process_profiles(&directory, 64, vec![])?;
    config
        .actors
        .get_mut(1)
        .ok_or("observer absent")?
        .authority
        .resources
        .workflow_run = WorkflowRunScope::Workflow {
        workflow: WorkflowId::new("hidden")?,
    };
    let daemon = start(config.validate(directory.path())?, CONTROLLER_TOKEN).await?;
    let base = revision("comparison", "done", 1)?;
    import(&daemon.client, &base).await?;
    let schema = SchemaRef::new(SchemaId::new("milkdrift.artifact-reference")?, 1)?;
    let mut metadata = serde_json::to_value(base.semantic().metadata())?;
    let mut cases = Vec::new();
    for (key, value, expected) in [
        (
            "name",
            serde_json::json!("Changed name"),
            summary("metadata", Some("name"), "changed"),
        ),
        (
            "description",
            serde_json::json!("Private description"),
            summary("metadata", Some("description"), "changed"),
        ),
        (
            "labels",
            serde_json::json!(["new-label"]),
            summary("metadata", Some("labels"), "changed"),
        ),
    ] {
        *metadata.get_mut(key).ok_or("metadata field absent")? = value;
        cases.push((
            Mutation::SetMetadata {
                metadata: serde_json::from_value(metadata.clone())?,
            },
            expected,
        ));
        metadata = serde_json::to_value(base.semantic().metadata())?;
    }
    cases.push((
        Mutation::SetMetadata {
            metadata: BlueprintMetadata::new(
                base.semantic().metadata().name(),
                "",
                BTreeSet::new(),
                BTreeMap::from([(
                    ExtensionKey::new("org.example/private")?,
                    BoundedJson::new(serde_json::json!({"prompt":"do not copy me"}))?,
                )]),
            )?,
        },
        summary("extension", Some("org.example/private"), "added"),
    ));
    cases.push((
        Mutation::SetInterface {
            interface: WorkflowInterface::new(
                [(
                    FieldId::new("brief")?,
                    InterfaceField::required(schema.clone()),
                )],
                [],
            )?,
        },
        summary("input", Some("brief"), "added"),
    ));
    cases.push((
        Mutation::SetInterface {
            interface: WorkflowInterface::new(
                [],
                [(FieldId::new("result")?, InterfaceField::optional(schema))],
            )?,
        },
        summary("output", Some("result"), "added"),
    ));
    for (mutation, expected) in cases {
        let changed = base.revise(
            base.id(),
            MutationBatch::new(vec![mutation])?,
            AuthorRef::new("human:other-author")?,
            "other reason",
        )?;
        import(&daemon.client, &changed).await?;
        let read = daemon
            .client
            .revision_diff(base.id().as_str(), changed.id().as_str())
            .await?;
        assert_eq!(read.changes, vec![expected.clone()]);
        assert!(!read.truncated);
        assert_eq!(
            read,
            daemon
                .client
                .revision_diff(base.id().as_str(), changed.id().as_str())
                .await?
        );
        let reverse = daemon
            .client
            .revision_diff(changed.id().as_str(), base.id().as_str())
            .await?;
        assert_eq!(
            reverse.changes,
            vec![RevisionChange {
                change: if expected.change == "added" {
                    "removed".into()
                } else {
                    "changed".into()
                },
                ..expected
            }]
        );
    }
    let (ungoverned, governed) = super::published::fixture::definition()?;
    import(&daemon.client, &ungoverned).await?;
    import(&daemon.client, &governed).await?;
    assert_eq!(
        daemon
            .client
            .revision_diff(ungoverned.id().as_str(), governed.id().as_str())
            .await?
            .changes,
        vec![summary("agreement", None, "added")]
    );
    assert_eq!(
        daemon
            .client
            .revision_diff(governed.id().as_str(), ungoverned.id().as_str())
            .await?
            .changes,
        vec![summary("agreement", None, "removed")]
    );
    let replacement = GoverningAgreement::seal(
        NodeId::new("contract")?,
        &ungoverned,
        governed
            .semantic()
            .agreement()
            .ok_or("agreement absent")?
            .scope()
            .clone(),
        format!("b3_{}", "1".repeat(64)),
    )?;
    let changed = governed.revise(
        governed.id(),
        MutationBatch::new(vec![Mutation::SetAgreement {
            agreement: Some(replacement),
        }])?,
        AuthorRef::new("human:a")?,
        "agreement change",
    )?;
    import(&daemon.client, &changed).await?;
    assert_eq!(
        daemon
            .client
            .revision_diff(governed.id().as_str(), changed.id().as_str())
            .await?
            .changes,
        vec![summary("agreement", None, "changed")]
    );
    let same = base.revise(
        base.id(),
        MutationBatch::new(vec![Mutation::SetMetadata {
            metadata: base.semantic().metadata().clone(),
        }])?,
        AuthorRef::new("human:new-author")?,
        "different provenance",
    )?;
    assert_ne!(same.id(), base.id());
    import(&daemon.client, &same).await?;
    let unchanged = daemon
        .client
        .revision_diff(base.id().as_str(), same.id().as_str())
        .await?;
    assert!(unchanged.changes.is_empty());
    assert!(!unchanged.truncated);
    let other = revision("other-workflow", "done", 1)?;
    import(&daemon.client, &other).await?;
    assert!(
        daemon
            .client
            .revision_diff(base.id().as_str(), other.id().as_str())
            .await
            .is_err()
    );
    assert!(
        client(&daemon.endpoint, OBSERVER_TOKEN)?
            .revision_diff(base.id().as_str(), changed.id().as_str())
            .await
            .is_err()
    );
    daemon.stop().await?;
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn comparison_limit_is_exact_and_node_edge_changes_are_ordered() -> TestResult {
    let directory = TempDir::new()?;
    let daemon = start(
        configuration_document_with_process_profiles(&directory, 64, vec![])?
            .validate(directory.path())?,
        CONTROLLER_TOKEN,
    )
    .await?;
    let left = revision("comparison-limit", "a", 256)?;
    let right = revision("comparison-limit", "b", 256)?;
    import(&daemon.client, &left).await?;
    import(&daemon.client, &right).await?;
    let expected: Vec<_> = [("node", 0), ("edge", 1)]
        .into_iter()
        .flat_map(|(subject, start)| {
            [("a", "removed"), ("b", "added")]
                .into_iter()
                .flat_map(move |(prefix, action)| {
                    (start..256).map(move |index| {
                        summary(subject, Some(&format!("{prefix}.{index:04}")), action)
                    })
                })
        })
        .collect();
    let read = daemon
        .client
        .revision_diff(left.id().as_str(), right.id().as_str())
        .await?;
    assert_eq!(read.changes, expected);
    assert!(!read.truncated);
    for (labels, truncated) in [
        (BTreeSet::new(), false),
        (BTreeSet::from(["extra".into()]), true),
    ] {
        let changed = right.revise(
            right.id(),
            MutationBatch::new(vec![Mutation::SetMetadata {
                metadata: BlueprintMetadata::new(
                    "changed name",
                    "changed description",
                    labels,
                    BTreeMap::new(),
                )?,
            }])?,
            AuthorRef::new("human:a")?,
            "extra changes",
        )?;
        import(&daemon.client, &changed).await?;
        let read = daemon
            .client
            .revision_diff(left.id().as_str(), changed.id().as_str())
            .await?;
        assert_eq!(read.changes.len(), 1024);
        assert_eq!(read.truncated, truncated);
        let mut wanted = vec![
            summary("metadata", Some("name"), "changed"),
            summary("metadata", Some("description"), "changed"),
        ];
        if truncated {
            wanted.push(summary("metadata", Some("labels"), "changed"));
        }
        wanted.extend(expected.clone());
        wanted.truncate(1024);
        assert_eq!(read.changes, wanted);
    }
    daemon.stop().await?;
    Ok(())
}
