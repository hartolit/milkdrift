//! Preserve independent roles and ordinary task obligations across restart.
use super::support::*;
use milkdrift_control_protocol::HostRole;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn execution_only_refuses_task_creation_without_creating_workflow_state() -> TestResult {
    let directory = TempDir::new()?;
    let mut config = configuration_document_with_process_profiles(&directory, 16, vec![])?;
    config.role = HostRole::ExecutionOnly;
    let daemon = start(config.validate(directory.path())?, CONTROLLER_TOKEN).await?;
    assert_eq!(
        daemon.client.readiness().await?.role,
        HostRole::ExecutionOnly
    );
    assert!(
        matches!(daemon.client.create_workflow(&create_request("refused-role", false)).await, Err(ClientError::Api(error)) if error.code == ErrorCode::Unavailable && !error.retryable)
    );
    daemon.stop().await?;
    assert!(
        RedbStore::open(directory.path().join("data"))?
            .workflows(None, 10)?
            .items
            .is_empty()
    );
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn role_removal_preserves_completed_history_and_refuses_live_obligations() -> TestResult {
    for supplied in [true, false] {
        let directory = TempDir::new()?;
        let profile = configured_process_profile(&directory)?;
        let mut config =
            configuration_document_with_process_profiles(&directory, 32, vec![profile])?;
        let daemon = start(config.clone().validate(directory.path())?, CONTROLLER_TOKEN).await?;
        let accepted = daemon
            .client
            .create_workflow(&create_request("role-change", supplied))
            .await?;
        if supplied {
            wait_task(
                &daemon.client,
                &accepted.workflow_id,
                accepted.aliases.get("inspect").ok_or("alias absent")?,
                TaskStateDto::Completed,
            )
            .await?;
        }
        daemon.stop().await?;
        let id = WorkflowId::new(&accepted.workflow_id)?;
        let before = {
            let store = RedbStore::open(directory.path().join("data"))?;
            serde_json::to_value(store.workflow(&id)?)?
        };
        config.role = HostRole::ExecutionOnly;
        if supplied {
            let daemon = start(config.validate(directory.path())?, CONTROLLER_TOKEN).await?;
            assert_eq!(
                daemon.client.readiness().await?.role,
                HostRole::ExecutionOnly
            );
            daemon.stop().await?;
        } else {
            assert!(
                DaemonHost::start(config.validate(directory.path())?).is_err(),
                "live workflow role was removed"
            );
        }
        let store = RedbStore::open(directory.path().join("data"))?;
        assert_eq!(serde_json::to_value(store.workflow(&id)?)?, before);
    }
    Ok(())
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn disabled_actor_restarts_fail_closed_and_requires_fresh_authority_to_reenable() -> TestResult
{
    let directory = tempfile::tempdir()?;
    let mut config = configuration_document_with_process_profiles(&directory, 16, Vec::new())?;
    let daemon = start(config.clone().validate(directory.path())?, CONTROLLER_TOKEN).await?;
    client(&daemon.endpoint, OBSERVER_TOKEN)?
        .readiness()
        .await?;
    daemon.stop().await?;

    config
        .actors
        .get_mut(1)
        .ok_or("fixture actor absent")?
        .enabled = false;
    // Repeating the disabled configuration must also be an exact, safe restart.
    for _ in 0..2 {
        let daemon = start(config.clone().validate(directory.path())?, CONTROLLER_TOKEN).await?;
        let refused = client(&daemon.endpoint, OBSERVER_TOKEN)?.readiness().await;
        assert!(
            matches!(refused, Err(ClientError::Api(error)) if error.code == ErrorCode::Unauthenticated)
        );
        daemon.stop().await?;
    }

    config
        .actors
        .get_mut(1)
        .ok_or("fixture actor absent")?
        .enabled = true;
    let error = DaemonHost::start(config.clone().validate(directory.path())?)
        .err()
        .ok_or("revoked authority was restored without a new grant")?;
    assert!(error.to_string().contains("peer_relationship_generation"));
    config
        .actors
        .get_mut(1)
        .ok_or("fixture actor absent")?
        .grant_revision += 1;
    config
        .actors
        .get_mut(1)
        .ok_or("fixture actor absent")?
        .revocation_generation += 1;
    let daemon = start(config.validate(directory.path())?, CONTROLLER_TOKEN).await?;
    client(&daemon.endpoint, OBSERVER_TOKEN)?
        .readiness()
        .await?;
    daemon.stop().await
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
