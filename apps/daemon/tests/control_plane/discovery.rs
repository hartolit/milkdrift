//! Revision discovery uses current inspect authority on every bounded page.
use super::support::*;
use milkdrift_authority::WorkflowSet;
use milkdrift_control_protocol::Cursor;
use std::collections::BTreeSet;

fn scope(names: &[&str]) -> TestResult<WorkflowRunScope> {
    Ok(WorkflowRunScope::Workflows {
        workflows: WorkflowSet::new(
            names
                .iter()
                .map(|name| WorkflowId::new(*name))
                .collect::<Result<Vec<_>, _>>()?,
        )?,
    })
}

async fn save(
    client: &ControlClient,
    workflow: &str,
    base: Option<&BlueprintRevision>,
) -> TestResult<BlueprintRevision> {
    let author = AuthorRef::new("human:discovery")?;
    let revision = match base {
        None => BlueprintRevision::genesis(
            WorkflowId::new(workflow)?,
            MutationBatch::new(vec![Mutation::AddNode {
                node: Node::new(
                    NodeId::new("done")?,
                    NodeKind::Terminal {
                        outcome: TerminalOutcome::Success,
                    },
                )?,
            }])?,
            author,
            "discovery fixture",
        )?,
        Some(base) => base.revise(
            base.id(),
            MutationBatch::new(vec![Mutation::SetMetadata {
                metadata: BlueprintMetadata::new(
                    format!("revision {}", base.sequence()),
                    "",
                    BTreeSet::new(),
                    BTreeMap::new(),
                )?,
            }])?,
            author,
            "discovery child",
        )?,
    };
    client
        .submit(&request(
            revision.id().as_str(),
            None,
            Command::ImportBlueprint {
                document: serde_json::from_slice(
                    &BlueprintRevisionDocument::new(&revision).to_canonical_json()?,
                )?,
            },
        ))
        .await?;
    Ok(revision)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn scoped_discovery_pages_only_permitted_revisions_and_rechecks_grants() -> TestResult {
    let directory = TempDir::new()?;
    let mut config = configuration_document_with_process_profiles(&directory, 64, vec![])?;
    let observer = config.actors.get_mut(1).ok_or("observer absent")?;
    observer.authority.resources.workflow_run = scope(&["allowed-a", "allowed-b"])?;
    for (name, workflow_run, preset) in [
        (
            "empty",
            scope(&["empty-a", "empty-b"])?,
            AuthorityPresetConfig::Observer,
        ),
        (
            "single",
            WorkflowRunScope::Workflow {
                workflow: WorkflowId::new("allowed-a")?,
            },
            AuthorityPresetConfig::Observer,
        ),
        (
            "run",
            WorkflowRunScope::Run {
                run: RunId::new("run")?,
                workflow: Some(WorkflowId::new("allowed-a")?),
            },
            AuthorityPresetConfig::Observer,
        ),
        (
            "denied",
            scope(&["allowed-a", "allowed-b"])?,
            AuthorityPresetConfig::Invoker,
        ),
    ] {
        let mut actor = config.actors.get(1).ok_or("observer absent")?.clone();
        actor.actor = format!("human:discovery-{name}");
        actor.grant_id = format!("grant:discovery-{name}");
        actor.credential_ref = format!("credential:discovery-{name}");
        actor.preset = preset;
        actor.authority.resources.workflow_run = workflow_run;
        let path = directory.path().join(format!("{name}.token"));
        write_secret(&path, &format!("discovery-fixture-{name}"))?;
        config.secret_sources.insert(
            actor.credential_ref.clone(),
            SecretSourceConfig::File { path },
        );
        config.actors.push(actor);
    }
    let daemon = start(config.clone().validate(directory.path())?, CONTROLLER_TOKEN).await?;
    let mut expected = Vec::new();
    for workflow in ["allowed-a", "allowed-b"] {
        let mut base = None;
        for _ in 0..3 {
            let revision = save(&daemon.client, workflow, base.as_ref()).await?;
            expected.push(revision.id().to_string());
            base = Some(revision);
        }
    }
    // Hidden rows greatly outnumber a page and can sort before or between permitted identities.
    for index in 0..40 {
        save(&daemon.client, &format!("hidden-{index}"), None).await?;
    }
    expected.sort();
    let restricted = client(&daemon.endpoint, OBSERVER_TOKEN)?;
    let mut page = PageRequest {
        limit: 2,
        cursor: None,
    };
    let mut found = Vec::new();
    let mut first_cursor = None;
    for _ in 0..5 {
        let result = restricted.revisions(None, &page).await?;
        assert!(result.items.len() <= 2);
        for item in &result.items {
            assert!(matches!(
                item.workflow_id.as_str(),
                "allowed-a" | "allowed-b"
            ));
        }
        found.extend(result.items.into_iter().map(|item| item.revision_id));
        if first_cursor.is_none() {
            first_cursor = result.next_cursor.clone();
        }
        if result.next_cursor.is_none() {
            break;
        }
        assert_ne!(result.next_cursor, page.cursor);
        page.cursor = result.next_cursor;
    }
    assert_eq!(found, expected);
    let cursor = first_cursor.ok_or("cursor absent")?;
    assert!(cursor.as_str().len() < 2048);
    let continuation = PageRequest {
        limit: 2,
        cursor: Some(cursor.clone()),
    };
    assert!(
        restricted
            .revisions(Some("allowed-a"), &continuation)
            .await
            .is_err()
    );
    assert!(daemon.client.revisions(None, &continuation).await.is_err());
    let mut tampered: serde_json::Value = serde_json::to_value(&cursor)?;
    let mut bytes = tampered
        .as_str()
        .ok_or("opaque cursor absent")?
        .as_bytes()
        .to_vec();
    let last = bytes.last_mut().ok_or("cursor empty")?;
    *last = if *last == b'A' { b'B' } else { b'A' };
    tampered = serde_json::Value::String(String::from_utf8(bytes)?);
    let tampered: Cursor = serde_json::from_value(tampered)?;
    assert!(
        restricted
            .revisions(
                None,
                &PageRequest {
                    limit: 2,
                    cursor: Some(tampered)
                }
            )
            .await
            .is_err()
    );
    assert!(
        restricted
            .revisions(
                Some("hidden-0"),
                &PageRequest {
                    limit: 2,
                    cursor: None
                }
            )
            .await
            .is_err()
    );
    for limit in [0, milkdrift_control_protocol::MAX_PAGE_ITEMS + 1] {
        assert!(
            restricted
                .revisions(
                    None,
                    &PageRequest {
                        limit,
                        cursor: None
                    }
                )
                .await
                .is_err()
        );
    }
    let empty = client(&daemon.endpoint, "discovery-fixture-empty")?
        .revisions(
            None,
            &PageRequest {
                limit: 1,
                cursor: None,
            },
        )
        .await?;
    assert!(empty.items.is_empty());
    assert!(empty.next_cursor.is_none());
    let single = client(&daemon.endpoint, "discovery-fixture-single")?
        .revisions(
            None,
            &PageRequest {
                limit: 10,
                cursor: None,
            },
        )
        .await?;
    assert_eq!(single.items.len(), 3);
    assert!(
        single
            .items
            .iter()
            .all(|item| item.workflow_id == "allowed-a")
    );
    for name in ["run", "denied"] {
        let denied = client(&daemon.endpoint, &format!("discovery-fixture-{name}"))?;
        for filter in [None, Some("allowed-a")] {
            assert!(
                denied
                    .revisions(
                        filter,
                        &PageRequest {
                            limit: 2,
                            cursor: None
                        }
                    )
                    .await
                    .is_err()
            );
        }
    }
    daemon.stop().await?;
    let observer = config.actors.get_mut(1).ok_or("observer absent")?;
    observer.grant_revision += 1;
    observer.revocation_generation += 1;
    observer.authority.resources.workflow_run = scope(&["allowed-a"])?;
    let daemon = start(config.clone().validate(directory.path())?, OBSERVER_TOKEN).await?;
    assert!(daemon.client.revisions(None, &continuation).await.is_err());
    assert_eq!(
        daemon
            .client
            .revisions(
                None,
                &PageRequest {
                    limit: 10,
                    cursor: None
                }
            )
            .await?
            .items
            .len(),
        3
    );
    daemon.stop().await?;
    config.actors.get_mut(1).ok_or("observer absent")?.enabled = false;
    let daemon = start(config.validate(directory.path())?, CONTROLLER_TOKEN).await?;
    assert!(
        client(&daemon.endpoint, OBSERVER_TOKEN)?
            .revisions(None, &continuation)
            .await
            .is_err()
    );
    daemon.stop().await?;
    Ok(())
}
