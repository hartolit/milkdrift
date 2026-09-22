//! The administrative client reaches the same configured resource owner with finite refused state.
use super::support::*;
use milkdrift_capability::managed::*;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn resource_client_roundtrip_requires_configuration_and_authentication() -> TestResult {
    let directory = tempfile::tempdir()?;
    let daemon = start(configuration(&directory, 16)?, CONTROLLER_TOKEN).await?;
    let request = ManagedRequest {
        schema_version: 1,
        command: ManagedName::new("inspect")?,
        installation: ManagedName::new("slotbook")?,
        expected_version: 0,
        action: ManagedAction::Inspect {},
    };
    let error = daemon
        .client
        .manage_resources(&request)
        .await
        .err()
        .ok_or("unconfigured owner accepted")?;
    assert!(matches!(error, ClientError::Api(ref error) if error.code == ErrorCode::InvalidInput));
    let unauthenticated = client(&daemon.endpoint, "wrong-credential")?;
    assert!(
        matches!(unauthenticated.manage_resources(&request).await, Err(ClientError::Api(ref error)) if error.code == ErrorCode::Unauthenticated)
    );
    daemon.stop().await?;
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn configured_resource_owner_retains_exact_failed_platform_intent_across_restart()
-> TestResult {
    if !cfg!(target_os = "linux") {
        return Ok(());
    }
    let directory = tempfile::tempdir()?;
    let manager = directory.path().join("manager");
    let quadlets = directory.path().join("quadlets");
    for path in [&manager, &quadlets] {
        fs::create_dir(path)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
        }
    }
    // The deliberately absent image prevents this HTTP contract test creating any OS resource.
    let bytes = include_bytes!("../../../../examples/managed-linux/slotbook.json");
    let recipe = milkdrift_managed_linux::LinuxRecipe::from_json(bytes)?;
    let path = manager.join("recipe.json");
    fs::write(&path, bytes)?;
    let mut config = configuration_document_with_process_profiles(&directory, 16, Vec::new())?;
    config.adapters.managed_linux = Some(milkdrift_managed_linux::LinuxManagerConfig {
        state_root: manager,
        quadlet_directory: quadlets,
        recipes: vec![path],
    });
    let daemon = start(config.clone().validate(directory.path())?, CONTROLLER_TOKEN).await?;
    let request = ManagedRequest {
        schema_version: 1,
        command: ManagedName::new("apply-exact")?,
        installation: ManagedName::new("slotbook")?,
        expected_version: 0,
        action: ManagedAction::Apply {
            recipe: recipe.reference()?,
        },
    };
    let accepted = daemon.client.manage_resources(&request).await?;
    assert!(accepted.pending.is_some());
    assert_eq!(daemon.client.manage_resources(&request).await?, accepted);
    let inspect = ManagedRequest {
        command: ManagedName::new("inspect")?,
        action: ManagedAction::Inspect {},
        ..request.clone()
    };
    let state = daemon.client.manage_resources(&inspect).await?;
    assert_eq!(state.state, "pending");
    assert!(!state.diagnostics.is_empty());
    assert!(state.capabilities.is_empty());
    let observer = client(&daemon.endpoint, OBSERVER_TOKEN)?;
    assert!(
        matches!(observer.manage_resources(&request).await, Err(ClientError::Api(ref e)) if e.code == ErrorCode::Unauthorized)
    );
    daemon.stop().await?;
    let reopened = start(config.validate(directory.path())?, CONTROLLER_TOKEN).await?;
    assert_eq!(reopened.client.manage_resources(&request).await?, accepted);
    assert_eq!(
        reopened.client.manage_resources(&inspect).await?.pending,
        state.pending
    );
    reopened.stop().await?;
    Ok(())
}
