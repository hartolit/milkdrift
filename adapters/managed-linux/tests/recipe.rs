//! Strict typed input, private manager ownership, and immutable descriptor evidence without Podman.
use milkdrift_capability::managed::{DataDisposition, MANAGED_BINDING_EXTENSION, ManagedName};
use milkdrift_capability_host::managed::ManagedPlatform;
use milkdrift_managed_linux::{
    LinuxManagedPlatform, LinuxManagerConfig, LinuxRecipe, ModelService, WorkerNetwork,
};
use std::{fs, path::Path};
type Result<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

fn recipe() -> Result<LinuxRecipe> {
    Ok(LinuxRecipe {
        schema_version: 1,
        name: ManagedName::new("slotbook")?,
        family: "slotbook-v1".to_owned(),
        worker_image: format!("sha256:{}", "a".repeat(64)),
        worker_network: WorkerNetwork::None,
        memory_bytes: 536_870_912,
        cpu_percent: 100,
        pids: 64,
        task_timeout_ms: 30000,
        output_bytes: 65536,
        minimum_free_bytes: 1_073_741_824,
        data_disposition: DataDisposition::Preserve,
        model_service: ModelService::Disabled {},
    })
}
fn write_private(path: &Path, bytes: &[u8]) -> Result {
    fs::write(path, bytes)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}

#[test]
fn typed_recipe_refuses_raw_engine_unit_injection_and_floating_inputs() -> Result {
    let valid = serde_json::to_value(recipe()?)?;
    for (key, bad) in [
        ("worker_image", serde_json::json!("rust:latest")),
        (
            "worker_image",
            serde_json::json!(format!("--option@sha256:{}", "0".repeat(64))),
        ),
        (
            "worker_image",
            serde_json::json!(format!("image@@sha256:{}", "0".repeat(64))),
        ),
        ("engine_args", serde_json::json!(["--privileged"])),
        ("unit", serde_json::json!("[Service]\nExecStart=/bin/sh")),
        ("memory_bytes", serde_json::json!(0)),
        ("worker_network", serde_json::json!("host")),
        ("family", serde_json::json!("arbitrary")),
        (
            "model_service",
            serde_json::json!({"type":"disabled","engine_args":["--privileged"]}),
        ),
    ] {
        let mut value = valid.clone();
        value[key] = bad;
        assert!(
            LinuxRecipe::from_json(&serde_json::to_vec(&value)?).is_err(),
            "{key}"
        );
    }
    assert!(LinuxRecipe::from_json(br#"{"schema_version":1,"schema_version":1}"#).is_err());
    assert!(LinuxRecipe::from_json(&vec![b' '; 65537]).is_err());
    let original = recipe()?;
    let reformatted = LinuxRecipe::from_json(&serde_json::to_vec_pretty(&original)?)?;
    assert_eq!(original.reference()?, reformatted.reference()?);
    let mut changed = original;
    changed.worker_network = WorkerNetwork::Outbound;
    assert_ne!(changed.reference()?, reformatted.reference()?);
    Ok(())
}

#[test]
fn manager_namespace_is_exclusive_and_changed_generation_is_exact() -> Result {
    if !cfg!(target_os = "linux") {
        return Ok(());
    }
    let root = tempfile::tempdir()?;
    let quadlets = tempfile::tempdir()?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(root.path(), fs::Permissions::from_mode(0o700))?;
        fs::set_permissions(quadlets.path(), fs::Permissions::from_mode(0o700))?;
    }
    let path = root.path().join("recipe.json");
    let recipe = recipe()?;
    write_private(&path, &serde_json::to_vec(&recipe)?)?;
    let config = LinuxManagerConfig {
        state_root: root.path().to_path_buf(),
        systemd_directory: quadlets.path().to_path_buf(),
        quadlet_directory: quadlets.path().to_path_buf(),
        recipes: vec![path],
    };
    let platform = LinuxManagedPlatform::new(config.clone())?;
    assert!(LinuxManagedPlatform::new(config.clone()).is_err());
    let setup = platform.plan(
        &ManagedName::new("slotbook")?,
        &recipe.reference()?,
        &format!("b3_{}", "1".repeat(64)),
        1,
    )?;
    assert_eq!(setup.capabilities.len(), 1);
    for modification in 0..3 {
        let mut inconsistent = setup.clone();
        match modification {
            0 => inconsistent.resources[0].identity.push_str("-foreign"),
            1 => {
                inconsistent.resources[0].ownership =
                    milkdrift_capability::managed::ResourceOwnership::Shared
            }
            _ => inconsistent.capabilities.clear(),
        }
        assert!(platform.requirements(&inconsistent).is_err());
    }

    assert!(
        setup.capabilities[0]
            .extensions()
            .keys()
            .any(|k| k.as_str() == MANAGED_BINDING_EXTENSION)
    );
    assert!(
        setup
            .resources
            .iter()
            .all(|r| r.identity != root.path().display().to_string())
    );
    let next = platform.plan(
        &ManagedName::new("slotbook")?,
        &recipe.reference()?,
        &setup.ownership,
        2,
    )?;
    assert_eq!(setup.resources, next.resources);
    assert_ne!(setup.capabilities, next.capabilities);
    // A second store can receive the same actor/grant/command digest. Its different manager root
    // must still select distinct platform resources, never adopt the first store's working data.
    let second_root = tempfile::tempdir()?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(second_root.path(), fs::Permissions::from_mode(0o700))?;
    }
    let second_config = LinuxManagerConfig {
        state_root: second_root.path().to_path_buf(),
        ..config.clone()
    };
    let second_platform = LinuxManagedPlatform::new(second_config)?;
    let second = second_platform.plan(
        &ManagedName::new("slotbook")?,
        &recipe.reference()?,
        &setup.ownership,
        1,
    )?;
    assert_ne!(setup.platform_owner, second.platform_owner);
    for (first, second) in setup.resources.iter().zip(&second.resources) {
        if first.ownership == milkdrift_capability::managed::ResourceOwnership::Owned {
            assert_ne!(first.identity, second.identity);
        }
    }
    assert!(
        platform
            .plan(
                &ManagedName::new("slotbook")?,
                &milkdrift_capability::managed::RecipeReference {
                    name: recipe.name.clone(),
                    digest: format!("b3_{}", "2".repeat(64))
                },
                &setup.ownership,
                1
            )
            .is_err()
    );
    drop(platform);
    LinuxManagedPlatform::new(config)?;
    Ok(())
}

#[test]
fn attached_model_profile_retains_exact_service_dependency_and_never_owns_endpoint() -> Result {
    if !cfg!(target_os = "linux") {
        return Ok(());
    }
    let root = tempfile::tempdir()?;
    let quadlets = tempfile::tempdir()?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(root.path(), fs::Permissions::from_mode(0o700))?;
        fs::set_permissions(quadlets.path(), fs::Permissions::from_mode(0o700))?;
    }
    let mut recipe = recipe()?;
    recipe.model_service = ModelService::Attached {
        api_base: "http://127.0.0.1:1234/v1".to_owned(),
        model_alias: "google/gemma-4-12b-qat".to_owned(),
        billing: milkdrift_model_provider::BillingTerms::Unbilled {
            source: "local deterministic fixture, no provider charge".to_owned(),
        },
        token_limits: milkdrift_model_provider::ModelTokenLimits::ByteBpe {
            template_tokens_per_message: 0,
            template_tokens_per_request: 0,
            maximum_input_tokens: 8192,
            maximum_output_tokens: 4096,
            output_control: milkdrift_model_provider::OutputTokenControl::MaxTokens,
            source: "deterministic bounded fixture contract; no real-model qualification"
                .to_owned(),
        },
    };
    for field in ["token_limits", "billing"] {
        let mut unknown = serde_json::to_value(&recipe)?;
        unknown["model_service"][field] = serde_json::json!({"type":"unknown"});
        assert!(LinuxRecipe::from_json(&serde_json::to_vec(&unknown)?).is_err());
    }
    let path = root.path().join("recipe.json");
    write_private(&path, &serde_json::to_vec(&recipe)?)?;
    let platform = LinuxManagedPlatform::new(LinuxManagerConfig {
        state_root: root.path().to_path_buf(),
        systemd_directory: quadlets.path().to_path_buf(),
        quadlet_directory: quadlets.path().to_path_buf(),
        recipes: vec![path],
    })?;
    let setup = platform.plan(
        &ManagedName::new("slotbook")?,
        &recipe.reference()?,
        &format!("b3_{}", "1".repeat(64)),
        1,
    )?;
    assert_eq!(setup.capabilities.len(), 2);
    let model = setup
        .resources
        .iter()
        .find(|r| r.name.as_str() == "model")
        .ok_or("model resource absent")?;
    assert_eq!(
        model.ownership,
        milkdrift_capability::managed::ResourceOwnership::Attached
    );
    for descriptor in setup.capabilities {
        assert!(
            descriptor
                .extensions()
                .keys()
                .any(|k| k.as_str() == MANAGED_BINDING_EXTENSION)
        );
    }
    Ok(())
}
