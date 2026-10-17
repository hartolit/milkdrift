//! Real public naming operations preserve identity, authority and unrelated workflow state.
use super::support::*;
use milkdrift_control_protocol::{HostRole, RenameDaemonRequest};

fn rename(key: &str, generation: u64, name: &str) -> RenameDaemonRequest {
    RenameDaemonRequest {
        request_id: key.into(),
        expected_generation: generation,
        name: name.into(),
    }
}

fn refused<T: std::fmt::Debug>(result: Result<T, ClientError>, code: ErrorCode) {
    assert!(
        matches!(&result, Err(ClientError::Api(error)) if error.code == code),
        "expected {code:?}, observed {result:?}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn independent_clients_share_names_while_observer_workflow_permissions_remain_separate()
-> TestResult {
    let directory = TempDir::new()?;
    let profile = configured_process_profile(&directory)?;
    let mut config = configuration_document_with_process_profiles(&directory, 32, vec![profile])?;
    config.name = "Studio A".into();
    let daemon = start(config.validate(directory.path())?, CONTROLLER_TOKEN).await?;
    let initial = daemon.client.task_session().await?;
    assert_eq!(initial.daemon_name, "Studio A");
    assert!(initial.can_rename_daemon);
    let controller =
        client(&daemon.endpoint, CONTROLLER_TOKEN)?.with_daemon_identity(&initial.daemon_id)?;
    let observer =
        client(&daemon.endpoint, OBSERVER_TOKEN)?.with_daemon_identity(&initial.daemon_id)?;
    let observer_session = observer.task_session().await?;
    assert_eq!(observer_session.daemon_name, initial.daemon_name);
    assert!(!observer_session.can_rename_daemon);
    let work = controller
        .create_workflow(&create_request("name-independent-work", false))
        .await?;
    assert!(observer.workflows("", None, 10).await?.items.is_empty());
    refused(
        observer.workflow(&work.workflow_id).await,
        ErrorCode::Unauthorized,
    );
    refused(
        observer
            .rename_daemon(&rename(
                "observer-rename",
                initial.daemon_name_generation,
                "Denied",
            ))
            .await,
        ErrorCode::Unauthorized,
    );
    let accepted = controller
        .rename_daemon(&rename(
            "shared-name",
            initial.daemon_name_generation,
            " Shared studio ",
        ))
        .await?;
    assert_eq!(accepted.name, "Shared studio");
    for reader in [&controller, &observer] {
        let session = reader.task_session().await?;
        assert_eq!(session.daemon_id, initial.daemon_id);
        assert_eq!(session.daemon_name, accepted.name);
        assert_eq!(session.daemon_name_generation, accepted.generation);
        assert_eq!(reader.daemon_metadata().await?.name, accepted.name);
    }
    let fresh = client(&daemon.endpoint, CONTROLLER_TOKEN)?
        .task_session()
        .await?;
    assert_eq!(fresh.daemon_name, accepted.name);
    assert_eq!(
        controller.workflow_receipt("name-independent-work").await?,
        work
    );
    assert!(observer.workflows("", None, 10).await?.items.is_empty());
    daemon.stop().await
}

fn retained_workflow_bytes(
    directory: &TempDir,
    workflow: &str,
    request: &str,
) -> TestResult<(Vec<u8>, Vec<u8>)> {
    let store = RedbStore::open(directory.path().join("data"))?;
    let snapshot = store
        .workflow(&WorkflowId::new(workflow)?)?
        .ok_or("workflow missing")?;
    let receipt = store
        .workflow_receipt("human:integration-controller", request)?
        .ok_or("workflow receipt missing")?;
    Ok((
        serde_json::to_vec(&snapshot)?,
        serde_json::to_vec(&receipt)?,
    ))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn rename_receipt_and_queued_work_survive_restart_without_seed_overwrite() -> TestResult {
    let directory = TempDir::new()?;
    let profile = configured_process_profile(&directory)?;
    let mut config = configuration_document_with_process_profiles(&directory, 32, vec![profile])?;
    config.name = "Initial operator seed".into();
    let daemon = start(config.clone().validate(directory.path())?, CONTROLLER_TOKEN).await?;
    let metadata = daemon.client.daemon_metadata().await?;
    let request = create_request("name-preserved-work", false);
    let work = daemon.client.create_workflow(&request).await?;
    let task = work.aliases.get("inspect").ok_or("task alias missing")?;
    wait_task(
        &daemon.client,
        &work.workflow_id,
        task,
        TaskStateDto::Queued,
    )
    .await?;
    daemon.stop().await?;
    let before = retained_workflow_bytes(&directory, &work.workflow_id, &request.request_id)?;

    let daemon = start(config.clone().validate(directory.path())?, CONTROLLER_TOKEN).await?;
    let exact = rename("lost-rename", metadata.generation, "  Team studio  ");
    let accepted = daemon.client.rename_daemon(&exact).await?;
    assert_eq!(daemon.client.create_workflow(&request).await?, work);
    daemon.stop().await?;
    assert_eq!(
        retained_workflow_bytes(&directory, &work.workflow_id, &request.request_id)?,
        before
    );

    config.name = "A later config seed must not win".into();
    let daemon = start(config.validate(directory.path())?, CONTROLLER_TOKEN).await?;
    let restored = daemon.client.task_session().await?;
    assert_eq!(restored.daemon_id, metadata.daemon_id);
    assert_eq!(restored.daemon_name, accepted.name);
    assert_eq!(restored.daemon_name_generation, accepted.generation);
    assert_eq!(
        daemon
            .client
            .daemon_rename_receipt(&exact.request_id)
            .await?,
        accepted
    );
    assert_eq!(daemon.client.rename_daemon(&exact).await?, accepted);
    assert_eq!(daemon.client.create_workflow(&request).await?, work);
    let mut changed = request.clone();
    changed.title.push_str(" changed");
    refused(
        daemon.client.create_workflow(&changed).await,
        ErrorCode::Conflict,
    );
    wait_task(
        &daemon.client,
        &work.workflow_id,
        task,
        TaskStateDto::Queued,
    )
    .await?;
    daemon.stop().await?;
    assert_eq!(
        retained_workflow_bytes(&directory, &work.workflow_id, &request.request_id)?,
        before
    );
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn rename_validates_unicode_names_and_retains_untrimmed_exact_requests() -> TestResult {
    let directory = TempDir::new()?;
    let daemon = start(configuration(&directory, 32)?, CONTROLLER_TOKEN).await?;
    let initial = daemon.client.daemon_metadata().await?;
    assert_eq!(initial.name, "Milkdrift");
    for (index, invalid) in [
        "".to_owned(),
        "   ".into(),
        "雪".repeat(81),
        "Studio\nNext".into(),
        "\tStudio".into(),
        "Studio\u{007f}".into(),
    ]
    .into_iter()
    .enumerate()
    {
        refused(
            daemon
                .client
                .rename_daemon(&rename(
                    &format!("invalid-{index}"),
                    initial.generation,
                    &invalid,
                ))
                .await,
            ErrorCode::InvalidInput,
        );
        assert_eq!(daemon.client.daemon_metadata().await?, initial);
    }
    let long = "雪".repeat(80);
    let exact = rename("unicode-exact", initial.generation, &format!("  {long}  "));
    let accepted = daemon.client.rename_daemon(&exact).await?;
    assert_eq!(accepted.name, long);
    assert_eq!(daemon.client.rename_daemon(&exact).await?, accepted);
    let mut normalized_but_changed = exact.clone();
    normalized_but_changed.name = long;
    refused(
        daemon.client.rename_daemon(&normalized_but_changed).await,
        ErrorCode::Conflict,
    );
    let mut changed_guard = exact.clone();
    changed_guard.expected_generation = accepted.generation;
    refused(
        daemon.client.rename_daemon(&changed_guard).await,
        ErrorCode::Conflict,
    );
    refused(
        daemon
            .client
            .rename_daemon(&rename("new-stale", initial.generation, "Stale draft"))
            .await,
        ErrorCode::Conflict,
    );
    let current = daemon.client.daemon_metadata().await?;
    assert_eq!(current.name, accepted.name);
    assert_eq!(current.generation, accepted.generation);
    daemon.stop().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn simultaneous_clients_cannot_both_rename_one_observed_generation() -> TestResult {
    let directory = TempDir::new()?;
    let daemon = start(configuration(&directory, 32)?, CONTROLLER_TOKEN).await?;
    let initial = daemon.client.daemon_metadata().await?;
    let other =
        client(&daemon.endpoint, CONTROLLER_TOKEN)?.with_daemon_identity(&initial.daemon_id)?;
    let first = rename("competing-a", initial.generation, "Team A");
    let second = rename("competing-b", initial.generation, "Team B");
    let (first_result, second_result) = tokio::join!(
        daemon.client.rename_daemon(&first),
        other.rename_daemon(&second)
    );
    let mut winner = None;
    let mut conflicts = 0;
    for outcome in [first_result, second_result] {
        match outcome {
            Ok(receipt) => {
                assert!(winner.replace(receipt).is_none());
            }
            Err(ClientError::Api(error)) if error.code == ErrorCode::Conflict => conflicts += 1,
            Err(error) => return Err(error.into()),
        }
    }
    assert_eq!(conflicts, 1);
    let winner = winner.ok_or("neither rename accepted")?;
    let current = daemon.client.daemon_metadata().await?;
    assert_eq!(current.name, winner.name);
    assert_eq!(current.generation, winner.generation);
    assert_eq!(
        other.daemon_rename_receipt(&winner.request_id).await?,
        winner
    );
    daemon.stop().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn same_request_key_belongs_to_each_authenticated_actor_separately() -> TestResult {
    let directory = TempDir::new()?;
    let mut config = configuration_document_with_process_profiles(&directory, 32, vec![])?;
    let second = config.actors.get_mut(1).ok_or("second actor missing")?;
    second.preset = AuthorityPresetConfig::Controller;
    second.grant_revision = 2;
    second.authority = ActorGrantConfig::dangerous_administrator();
    let daemon = start(config.validate(directory.path())?, CONTROLLER_TOKEN).await?;
    let initial = daemon.client.daemon_metadata().await?;
    let second =
        client(&daemon.endpoint, OBSERVER_TOKEN)?.with_daemon_identity(&initial.daemon_id)?;
    let first_request = rename("actor-shared-key", initial.generation, "First actor");
    let first_receipt = daemon.client.rename_daemon(&first_request).await?;
    refused(
        second
            .daemon_rename_receipt(&first_request.request_id)
            .await,
        ErrorCode::NotFound,
    );
    let second_request = rename("actor-shared-key", first_receipt.generation, "Second actor");
    let second_receipt = second.rename_daemon(&second_request).await?;
    assert_ne!(first_receipt, second_receipt);
    assert_eq!(
        daemon
            .client
            .daemon_rename_receipt("actor-shared-key")
            .await?,
        first_receipt
    );
    assert_eq!(
        second.daemon_rename_receipt("actor-shared-key").await?,
        second_receipt
    );
    assert_eq!(
        daemon.client.rename_daemon(&first_request).await?,
        first_receipt
    );
    assert_eq!(
        daemon.client.daemon_metadata().await?.name,
        second_receipt.name
    );
    daemon.stop().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn duplicate_names_do_not_replace_the_daemon_identity_pin() -> TestResult {
    let first_root = TempDir::new()?;
    let second_root = TempDir::new()?;
    let mut first_config = configuration_document_with_process_profiles(&first_root, 32, vec![])?;
    first_config.host_id = "host:studio-a".into();
    first_config.name = "Same studio".into();
    let mut second_config = configuration_document_with_process_profiles(&second_root, 32, vec![])?;
    second_config.host_id = "host:studio-b".into();
    second_config.name = first_config.name.clone();
    let first = start(first_config.validate(first_root.path())?, CONTROLLER_TOKEN).await?;
    let second = start(
        second_config.validate(second_root.path())?,
        CONTROLLER_TOKEN,
    )
    .await?;
    let a = first.client.task_session().await?;
    let b = second.client.task_session().await?;
    assert_eq!(a.daemon_name, b.daemon_name);
    assert_ne!(a.daemon_id, b.daemon_id);
    assert_ne!(first.endpoint, second.endpoint);
    let wrong = client(&second.endpoint, CONTROLLER_TOKEN)?.with_daemon_identity(&a.daemon_id)?;
    refused(
        wrong.task_session().await,
        ErrorCode::DaemonIdentityMismatch,
    );
    refused(
        wrong.daemon_metadata().await,
        ErrorCode::DaemonIdentityMismatch,
    );
    refused(
        wrong
            .rename_daemon(&rename(
                "wrong-daemon",
                b.daemon_name_generation,
                "Unwanted",
            ))
            .await,
        ErrorCode::DaemonIdentityMismatch,
    );
    refused(
        wrong.daemon_rename_receipt("wrong-daemon").await,
        ErrorCode::DaemonIdentityMismatch,
    );
    refused(
        wrong.workflows("", None, 10).await,
        ErrorCode::DaemonIdentityMismatch,
    );
    assert_eq!(second.client.daemon_metadata().await?.name, b.daemon_name);
    assert_eq!(first.client.daemon_metadata().await?.name, a.daemon_name);
    second.stop().await?;
    first.stop().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn both_daemon_roles_require_controller_operation_and_explicit_configuration_scope()
-> TestResult {
    for role in [HostRole::WorkflowEnabled, HostRole::ExecutionOnly] {
        for (preset, configuration_scope, allowed) in [
            (AuthorityPresetConfig::Controller, true, true),
            (AuthorityPresetConfig::Controller, false, false),
            (AuthorityPresetConfig::Supervisor, true, false),
        ] {
            let directory = TempDir::new()?;
            let mut config = configuration_document_with_process_profiles(&directory, 32, vec![])?;
            config.role = role;
            let actor = config.actors.first_mut().ok_or("controller missing")?;
            actor.preset = preset;
            actor.authority.resources.daemon.configuration = configuration_scope;
            let daemon = start(config.validate(directory.path())?, CONTROLLER_TOKEN).await?;
            let metadata = daemon.client.daemon_metadata().await?;
            assert_eq!(metadata.can_rename, allowed);
            let result = daemon
                .client
                .rename_daemon(&rename("scoped", metadata.generation, "Shared name"))
                .await;
            if allowed {
                assert_eq!(result?.name, "Shared name");
            } else {
                refused(result, ErrorCode::Unauthorized);
                assert_eq!(daemon.client.daemon_metadata().await?, metadata);
            }
            daemon.stop().await?;
        }
    }
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn actual_cli_names_and_recovers_the_same_shared_public_rename() -> TestResult {
    let directory = TempDir::new()?;
    let daemon = start(configuration(&directory, 32)?, CONTROLLER_TOKEN).await?;
    let initial = cli_ok(&daemon, &directory, "name-read", &["daemon", "name"])?;
    assert_eq!(initial.get("name"), Some(&serde_json::json!("Milkdrift")));
    let generation = initial
        .get("generation")
        .and_then(serde_json::Value::as_u64)
        .ok_or("generation missing")?
        .to_string();
    let arguments = [
        "daemon",
        "rename",
        "Shared CLI name",
        "--expected-generation",
        generation.as_str(),
        "--request-id",
        "cli shared rename 雪 / %? #",
    ];
    let accepted = cli_ok(&daemon, &directory, "name-rename", &arguments)?;
    assert_eq!(
        accepted.get("name"),
        Some(&serde_json::json!("Shared CLI name"))
    );
    assert_eq!(
        daemon.client.task_session().await?.daemon_name,
        "Shared CLI name"
    );
    let receipt = cli_ok(
        &daemon,
        &directory,
        "name-receipt",
        &["daemon", "rename-receipt", "cli shared rename 雪 / %? #"],
    )?;
    assert_eq!(receipt, accepted);
    assert_eq!(
        cli_ok(&daemon, &directory, "name-replay", &arguments)?,
        accepted
    );
    daemon.stop().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn accepted_512_byte_rename_keys_remain_recoverable_and_dot_keys_refuse_before_effect()
-> TestResult {
    let directory = TempDir::new()?;
    let daemon = start(configuration(&directory, 32)?, CONTROLLER_TOKEN).await?;
    let initial = daemon.client.daemon_metadata().await?;
    for key in [".", ".."] {
        let request = rename(key, initial.generation, "Must not be saved");
        refused(
            daemon.client.rename_daemon(&request).await,
            ErrorCode::InvalidInput,
        );
        assert_eq!(daemon.client.daemon_metadata().await?, initial);
    }
    let request_id = format!("{}ab", "雪".repeat(170));
    assert_eq!(request_id.len(), 512);
    let accepted = daemon
        .client
        .rename_daemon(&rename(&request_id, initial.generation, "Long key"))
        .await?;
    assert_eq!(
        daemon.client.daemon_rename_receipt(&request_id).await?,
        accepted
    );
    let cli_receipt = cli_ok(
        &daemon,
        &directory,
        "long-name-receipt",
        &["daemon", "rename-receipt", &request_id],
    )?;
    assert_eq!(cli_receipt, serde_json::to_value(&accepted)?);
    daemon.stop().await
}
