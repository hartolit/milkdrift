//! Startup composition and preservation of workflow obligations across role changes.
use super::support::*;
use milkdrift_control_protocol::HostRole;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn removing_workflow_role_preserves_closed_history_for_offline_inspection() -> TestResult {
    use milkdrift_persistence::{EventPageQuery, RunQueryStore};
    let directory = tempfile::tempdir()?;
    let mut config = configuration_document_with_process_profiles(&directory, 16, Vec::new())?;
    let daemon = start(config.clone().validate(directory.path())?, CONTROLLER_TOKEN).await?;
    assert!(
        daemon
            .client
            .execution_discovery()
            .await?
            .catalog
            .entries
            .is_empty(),
        "workflow-only adapters were advertised as independently invocable"
    );
    let revision = BlueprintRevision::genesis(
        WorkflowId::new("golden")?,
        MutationBatch::new(vec![Mutation::AddNode {
            node: Node::new(
                NodeId::new("done")?,
                NodeKind::Terminal {
                    outcome: TerminalOutcome::Success,
                },
            )?,
        }])?,
        AuthorRef::new("human:role-test")?,
        "closed role-removal evidence",
    )?;
    daemon
        .client
        .submit(&request(
            "closed-import",
            None,
            Command::ImportBlueprint {
                document: serde_json::from_slice(
                    &BlueprintRevisionDocument::new(&revision).to_canonical_json()?,
                )?,
            },
        ))
        .await?;
    daemon
        .client
        .submit(&request(
            "closed-start",
            None,
            Command::StartRun {
                run_id: "closed-run".to_owned(),
                workflow_id: "golden".to_owned(),
                revision_id: revision.id().to_string(),
            },
        ))
        .await?;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    while daemon.client.run("closed-run").await?.terminal.as_deref() != Some("succeeded") {
        if tokio::time::Instant::now() > deadline {
            return Err("terminal fixture failed to close".into());
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    daemon.stop().await?;
    let run = RunId::new("closed-run")?;
    let query = EventPageQuery {
        run: run.clone(),
        cursor: None,
        limit: PageSize::new(128)?,
    };
    let (before, journal) = {
        let store = RedbStore::open(directory.path().join("data"))?;
        (store.run_summary(&run)?, store.events(&query)?)
    };
    config.role = HostRole::ExecutionOnly;
    let daemon = start(config.validate(directory.path())?, CONTROLLER_TOKEN).await?;
    assert_eq!(
        daemon.client.readiness().await?.role,
        HostRole::ExecutionOnly
    );
    daemon.stop().await?;
    let store = RedbStore::open(directory.path().join("data"))?;
    assert_eq!(store.run_summary(&run)?, before);
    assert_eq!(store.events(&query)?, journal);
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn serving_installation_identity_cannot_change_on_restart() -> TestResult {
    let directory = tempfile::tempdir()?;
    let mut config = configuration_document_with_process_profiles(&directory, 16, Vec::new())?;
    config.role = HostRole::ExecutionOnly;
    let daemon = start(config.clone().validate(directory.path())?, CONTROLLER_TOKEN).await?;
    daemon.stop().await?;
    let original = config.clone();
    config.host_id = "different-installation".to_owned();
    let error = DaemonHost::start(config.validate(directory.path())?)
        .err()
        .ok_or("installation identity was rewritten")?;
    assert!(error.to_string().contains("configured host_id differs"));
    let daemon = start(original.validate(directory.path())?, CONTROLLER_TOKEN).await?;
    daemon.stop().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn execution_only_exposes_its_role_and_refuses_workflow_commands() -> TestResult {
    let directory = tempfile::tempdir()?;
    let mut config = configuration_document_with_process_profiles(&directory, 16, Vec::new())?;
    config.role = HostRole::ExecutionOnly;
    let daemon = start(config.validate(directory.path())?, CONTROLLER_TOKEN).await?;
    assert_eq!(
        daemon.client.readiness().await?.role,
        HostRole::ExecutionOnly
    );
    assert!(daemon.client.capabilities().await?.is_empty());
    assert!(matches!(
        daemon.client.submit(&request("role-import", None, Command::ImportBlueprint {
            document: blueprint()?,
        })).await,
        Err(ClientError::Api(error)) if error.code == ErrorCode::Unavailable
            && !error.retryable && error.message.contains("workflow_enabled")
    ));
    daemon.stop().await?;
    let store = RedbStore::open(directory.path().join("data"))?;
    use milkdrift_persistence::{RunQueryStore, RunSummaryFilter, RunSummaryPageQuery};
    assert!(
        store
            .run_summaries(&RunSummaryPageQuery {
                filter: RunSummaryFilter::default(),
                cursor: None,
                limit: PageSize::new(1)?,
            })?
            .runs
            .is_empty()
    );
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn execution_only_refuses_live_workflow_obligations_without_rewriting_history() -> TestResult
{
    let directory = tempfile::tempdir()?;
    let config = configuration_document_with_process_profiles(&directory, 16, Vec::new())?;
    let daemon = start(config.clone().validate(directory.path())?, CONTROLLER_TOKEN).await?;
    let revision = import_blueprint(&daemon.client, "role-import").await?;
    daemon
        .client
        .submit(&request(
            "role-start",
            None,
            Command::StartRun {
                run_id: "role-live-run".to_owned(),
                workflow_id: "golden".to_owned(),
                revision_id: revision,
            },
        ))
        .await?;
    daemon.stop().await?;
    use milkdrift_persistence::RunQueryStore;
    let run = RunId::new("role-live-run")?;
    let before = {
        let store = RedbStore::open(directory.path().join("data"))?;
        store.run_summary(&run)?.ok_or("live run missing")?
    };
    let mut execution = config.clone();
    execution.role = HostRole::ExecutionOnly;
    let error = DaemonHost::start(execution.validate(directory.path())?)
        .err()
        .ok_or("execution-only startup abandoned workflow obligations")?;
    assert!(
        error
            .to_string()
            .contains("active or unresolved workflow obligations")
    );
    {
        let store = RedbStore::open(directory.path().join("data"))?;
        assert_eq!(store.run_summary(&run)?, Some(before));
    }
    let resumed = start(config.validate(directory.path())?, CONTROLLER_TOKEN).await?;
    assert_eq!(
        resumed.client.readiness().await?.role,
        HostRole::WorkflowEnabled
    );
    resumed.stop().await
}
