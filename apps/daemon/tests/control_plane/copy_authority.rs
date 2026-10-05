//! The ordinary CLI copy needs exactly its named source and destination workflows.
use super::{inputs::ModelFixture, support::*};
use milkdrift_authority::WorkflowSet;

fn named(ids: &[&str]) -> TestResult<WorkflowRunScope> {
    Ok(WorkflowRunScope::Workflows {
        workflows: WorkflowSet::new(
            ids.iter()
                .map(|id| WorkflowId::new(*id))
                .collect::<Result<Vec<_>, _>>()?,
        )?,
    })
}

async fn import(client: &ControlClient, workflow: &str) -> TestResult<String> {
    let revision = BlueprintRevision::genesis(
        WorkflowId::new(workflow)?,
        MutationBatch::new(vec![Mutation::AddNode {
            node: Node::new(
                NodeId::new("done")?,
                NodeKind::Terminal {
                    outcome: TerminalOutcome::Success,
                },
            )?,
        }])?,
        AuthorRef::new("human:fixture")?,
        "fixture definition",
    )?;
    client
        .submit(&request(
            &format!("import-{workflow}"),
            None,
            Command::ImportBlueprint {
                document: serde_json::from_slice(
                    &BlueprintRevisionDocument::new(&revision).to_canonical_json()?,
                )?,
            },
        ))
        .await?;
    Ok(revision.id().to_string())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn named_copy_cli_refuses_missing_scopes_actions_and_hidden_workflows() -> TestResult {
    let directory = TempDir::new()?;
    let model = ModelFixture::start().await?;
    model.checked(async |model| {
        let mut config = configuration_document_with_process_profiles(&directory, 64, vec![])?;
        config.actors.first_mut().ok_or("controller absent")?.authority.resources.workflow_run = named(&["a", "b"])?;
        let unrelated = config.actors.get_mut(1).ok_or("second actor absent")?;
        unrelated.preset = AuthorityPresetConfig::Controller;
        unrelated.authority = ActorGrantConfig::dangerous_administrator();
        unrelated.authority.resources.workflow_run = named(&["c"])?;
        for (label, ids, preset) in [
            ("source", vec!["a"], AuthorityPresetConfig::Controller),
            ("destination", vec!["b"], AuthorityPresetConfig::Controller),
            ("observer", vec!["a", "b"], AuthorityPresetConfig::Observer),
            ("run-only", vec![], AuthorityPresetConfig::Controller),
        ] {
            let mut actor = config.actors.first().ok_or("controller absent")?.clone();
            actor.actor = format!("human:{label}");
            actor.grant_id = format!("grant:{label}");
            actor.credential_ref = format!("credential:copy-{label}");
            actor.preset = preset;
            actor.authority.resources.workflow_run = if ids.is_empty() {
                WorkflowRunScope::Run { run: RunId::new("run-a")?, workflow: Some(WorkflowId::new("a")?) }
            } else { named(&ids)? };
            let path = directory.path().join(format!("{label}-copy.token"));
            write_secret(&path, &format!("copy-fixture-{label}"))?;
            config.secret_sources.insert(actor.credential_ref.clone(), SecretSourceConfig::File { path });
            config.actors.push(actor);
        }
        // Roundtrip through the same TOML decoder used by a configured daemon.
        let config: DaemonConfig = toml::from_str(&toml::to_string(&config)?)?;
        let plan = config.validate(directory.path())?;
        let daemon = start(plan.clone(), CONTROLLER_TOKEN).await?;
        let mut copied = None;
        daemon.checked(model, async |daemon| {
            let source = import(&daemon.client, "a").await?;
            let hidden = client(&daemon.endpoint, OBSERVER_TOKEN)?;
            let secret_revision = import(&hidden, "c").await?;
            hidden.submit(&request("start-c", Some(0), Command::StartRun {
                run_id: "run-c".into(), workflow_id: "c".into(), revision_id: secret_revision.clone(), inputs: vec![],
            })).await?;
            assert!(matches!(daemon.client.run("run-c").await, Err(ClientError::Api(error)) if error.code == ErrorCode::NotFound));
            let page = PageRequest { limit: 10, cursor: None };
            let mut copy = request("copy-refused", None, Command::CopyBlueprint {
                source_revision: source.clone(), workflow_id: "b".into(), name: "Named copy".into()
            });
            copy.expected_revision = Some(source.clone());
            for label in ["source", "destination", "observer", "run-only"] {
                let restricted = client(&daemon.endpoint, &format!("copy-fixture-{label}"))?;
                let error = restricted.submit(&copy).await.err().ok_or("copy unexpectedly accepted")?;
                assert!(matches!(error, ClientError::Api(ref error) if matches!(error.code, ErrorCode::Unauthorized | ErrorCode::NotFound)));
                assert!(!error.to_string().contains(&secret_revision));
                assert!(daemon.client.revisions(Some("b"), &page).await?.items.is_empty());
            }
            assert_eq!(daemon.client.revisions(None, &page).await?.items.len(), 1);
            assert!(daemon.client.runs(None, None, &page).await.is_err());
            assert!(daemon.client.revisions(Some("c"), &page).await.is_err());
            assert!(daemon.client.revision(&secret_revision).await.is_err());
            let mut outside = copy.clone();
            outside.command_id = "copy-outside".into();
            if let Command::CopyBlueprint { workflow_id, .. } = &mut outside.command { *workflow_id = "c".into(); }
            assert!(daemon.client.submit(&outside).await.is_err());
            assert_eq!(hidden.revisions(Some("c"), &page).await?.items.len(), 1);
            let value = cli_ok(daemon, &directory, "copy-named-cli", &[
                "workflow", "copy", &source, "b", "--name", "Named copy", "--file", "named.json"
            ])?;
            let revision = value.get("revision_id").and_then(serde_json::Value::as_str).ok_or("copy absent")?.to_owned();
            assert_eq!(daemon.client.revisions(Some("b"), &page).await?.items.len(), 1);
            let discovered = cli_ok(daemon, &directory, "discover-named-cli", &[
                "workflow", "list", "--limit", "10"
            ])?;
            let items = discovered.get("items").and_then(serde_json::Value::as_array).ok_or("revision page absent")?;
            assert_eq!(items.len(), 2);
            assert!(items.iter().all(|item| matches!(item.get("workflow_id").and_then(serde_json::Value::as_str), Some("a" | "b"))));
            assert!(daemon.client.runs(None, Some("b"), &page).await?.items.is_empty());
            assert!(daemon.client.revision(&revision).await?.reason.contains(&source));
            copied = Some(revision);
            Ok(())
        }).await?;
        let daemon = start(plan, CONTROLLER_TOKEN).await?;
        daemon.checked(model, async |daemon| {
            assert_eq!(daemon.client.revision(copied.as_deref().ok_or("copy absent")?).await?.summary.workflow_id, "b");
            assert_eq!(model.request_count(), Some(0));
            Ok(())
        }).await
    }).await
}
