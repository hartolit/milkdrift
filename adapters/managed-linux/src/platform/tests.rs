use super::{start_owned, verify_controllers, verify_podman_version, verify_subordinate_ids};
use crate::{InferenceBackend, LinuxRecipe, ModelService, recipe::Deployment, units};
use milkdrift_capability::{BoundedJson, managed::ManagedName};
use milkdrift_persistence::managed::ApprovedSetup;
use std::cell::Cell;

type Result<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

#[test]
fn current_podman_and_required_delegation_are_checked_before_effects() -> Result {
    for version in ["5.4.0", "5.9.2", "6.0.0", "6.1.2"] {
        verify_podman_version(version)?;
    }
    for version in ["4.9.0", "5.3.2", "6", "6.bad", "7.0.0"] {
        assert!(verify_podman_version(version).is_err(), "{version}");
    }
    verify_controllers(&serde_json::json!({"host":{"cgroupControllers":["cpu","memory","pids"]}}))?;
    for controllers in [
        serde_json::json!(["memory", "pids"]),
        serde_json::json!(["cpu", "pids"]),
        serde_json::json!(["cpu", "memory"]),
        serde_json::json!(null),
    ] {
        assert!(
            verify_controllers(&serde_json::json!({"host":{"cgroupControllers":controllers}}))
                .is_err()
        );
    }
    Ok(())
}

fn owned_setup() -> Result<ApprovedSetup> {
    let mut recipe = LinuxRecipe::from_json(include_bytes!(
        "../../../../examples/managed-linux/slotbook.json"
    ))?;
    recipe.model_service = ModelService::Owned {
        image: format!("sha256:{}", "a".repeat(64)),
        executable: "/usr/bin/llama-server".to_owned(),
        model: "/models/approved.gguf".into(),
        model_digest: format!("b3_{}", "b".repeat(64)),
        model_bytes: 1024,
        port: 8080,
        context_tokens: 4096,
        token_limits: milkdrift_model_provider::ModelTokenLimits::ByteBpe {
            template_tokens_per_message: 8,
            template_tokens_per_request: 8,
            maximum_input_tokens: 4096,
            maximum_output_tokens: 128,
            output_control: milkdrift_model_provider::OutputTokenControl::MaxTokens,
            source: "deterministic unit-generation fixture".to_owned(),
        },
        threads: 1,
        backend: InferenceBackend::Cpu {},
    };
    let ownership = format!("b3_{}", "c".repeat(64));
    let prefix = units::volume_prefix("fixture", &ownership);
    let reference = recipe.reference()?;
    let deployment = Deployment {
        recipe,
        installation: ManagedName::new("slotbook")?,
        generation: 1,
        unit: format!("{prefix}g1"),
        volume_prefix: prefix,
        manager_root: "/manager".into(),
        systemd_directory: "/quadlets".into(),
        quadlet_directory: "/quadlets".into(),
    };
    Ok(ApprovedSetup {
        recipe: reference,
        mechanism: "linux-quadlet-v2".to_owned(),
        configuration: BoundedJson::new(serde_json::to_value(deployment)?)?,
        platform_owner: "fixture".to_owned(),
        ownership,
        resources: Vec::new(),
        capabilities: Vec::new(),
    })
}

#[test]
fn service_name_collision_cannot_reach_the_supervisor() -> Result {
    let setup = owned_setup()?;
    let starts = Cell::new(0);
    let owned = serde_json::json!({"Config":{"Labels":{
        "org.milkdrift.owner": setup.ownership,
        "org.milkdrift.platform": setup.platform_owner,
        "org.milkdrift.recipe": setup.recipe.digest
    }}});
    for field in [
        "org.milkdrift.owner",
        "org.milkdrift.platform",
        "org.milkdrift.recipe",
    ] {
        let mut foreign = owned.clone();
        foreign["Config"]["Labels"][field] = serde_json::json!("foreign");
        assert!(
            start_owned(&setup, Some(&foreign), || {
                starts.set(starts.get() + 1);
                Ok(())
            })
            .is_err()
        );
    }
    assert_eq!(starts.get(), 0);
    for container in [None, Some(&owned)] {
        start_owned(&setup, container, || {
            starts.set(starts.get() + 1);
            Ok(())
        })?;
    }
    assert_eq!(starts.get(), 2);
    // The engine must also refuse a container appearing after the identity check.
    let definition = units::unit_text(&setup, true)?.ok_or("owned unit missing")?;
    assert!(definition.contains("PodmanArgs=--replace=false "));
    Ok(())
}

#[test]
fn owned_model_leaves_a_private_user_namespace_for_the_worker() -> Result {
    let setup = owned_setup()?;
    let deployment = units::deployment(&setup)?;
    let mut info = serde_json::json!({"host":{"idMappings":{
        "uidmap":[{"container_id":1,"host_id":100000,"size":65536}],
        "gidmap":[{"container_id":1,"host_id":100000,"size":65536}]
    }}});
    verify_subordinate_ids(&info, &ModelService::Disabled {})?;
    assert!(verify_subordinate_ids(&info, &deployment.recipe.model_service).is_err());
    info["host"]["idMappings"]["uidmap"][0]["size"] = serde_json::json!(131072);
    assert!(verify_subordinate_ids(&info, &deployment.recipe.model_service).is_err());
    info["host"]["idMappings"]["gidmap"][0]["size"] = serde_json::json!(131072);
    verify_subordinate_ids(&info, &deployment.recipe.model_service)?;
    for key in ["uidmap", "gidmap"] {
        info["host"]["idMappings"][key] = serde_json::json!([
            {"container_id":0,"host_id":1000,"size":1},
            {"container_id":1,"host_id":100000,"size":65536},
            {"container_id":65537,"host_id":165536,"size":65536}
        ]);
    }
    verify_subordinate_ids(&info, &deployment.recipe.model_service)?;
    Ok(())
}

#[test]
#[ignore = "requires real rootless Podman, systemd user search directories and a preloaded worker image"]
fn real_quadlet_failed_start_preserves_a_foreign_container() -> Result {
    use milkdrift_capability_host::managed::ManagedPlatform;
    use std::{fs, path::PathBuf};
    let input = LinuxRecipe::from_json(&fs::read(std::env::var("MILKDRIFT_LINUX_RECIPE")?)?)?;
    let root = tempfile::tempdir()?;
    let quadlet = tempfile::tempdir_in(std::env::var("MILKDRIFT_QUADLET_TEST_PARENT")?)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        for path in [root.path(), quadlet.path()] {
            fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
        }
    }
    let mut recipe = units::deployment(&owned_setup()?)?.recipe;
    recipe.worker_image = input.worker_image.clone();
    if let ModelService::Owned {
        image,
        executable,
        model,
        ..
    } = &mut recipe.model_service
    {
        *image = input.worker_image;
        *executable = "/bin/false".to_owned();
        *model = root.path().join("model.gguf");
        fs::write(model, b"fixture")?;
    }
    let recipe_path = root.path().join("recipe.json");
    fs::write(&recipe_path, serde_json::to_vec(&recipe)?)?;
    let platform = super::LinuxManagedPlatform::new(crate::LinuxManagerConfig {
        state_root: root.path().to_owned(),
        quadlet_directory: quadlet.path().to_owned(),
        systemd_directory: PathBuf::from(std::env::var("MILKDRIFT_SYSTEMD_TEST_DIRECTORY")?),
        recipes: vec![recipe_path],
    })?;
    let setup = platform.plan(
        &ManagedName::new("collision")?,
        &recipe.reference()?,
        &format!("b3_{}", "d".repeat(64)),
        1,
    )?;
    let d = units::deployment(&setup)?;
    platform.write_unit(&setup, false)?;
    struct Cleanup<'a> {
        platform: &'a super::LinuxManagedPlatform,
        setup: &'a ApprovedSetup,
        unit: String,
        container_id: Option<String>,
    }
    impl Drop for Cleanup<'_> {
        fn drop(&mut self) {
            let _ = super::LinuxManagedPlatform::systemctl(&[
                "stop".to_owned(),
                format!("{}.service", self.unit),
            ]);
            if let Some(id) = &self.container_id {
                let _ = super::LinuxManagedPlatform::podman(&[
                    "rm".to_owned(),
                    "--force".to_owned(),
                    id.clone(),
                ]);
            }
            let _ = self.platform.remove_configuration(self.setup);
        }
    }
    let mut cleanup = Cleanup {
        platform: &platform,
        setup: &setup,
        unit: d.unit.clone(),
        container_id: None,
    };
    let id = super::LinuxManagedPlatform::podman(&[
        "run".to_owned(),
        "--detach".to_owned(),
        "--name".to_owned(),
        d.unit.clone(),
        "--pull=never".to_owned(),
        "--label=org.milkdrift.owner=foreign-test-sentinel".to_owned(),
        "--read-only".to_owned(),
        "--cap-drop=all".to_owned(),
        "--security-opt=no-new-privileges".to_owned(),
        "--network=none".to_owned(),
        "--memory=268435456".to_owned(),
        "--pids-limit=16".to_owned(),
        "--entrypoint=/bin/sleep".to_owned(),
        recipe.worker_image.clone(),
        "120".to_owned(),
    ])?;
    let id = String::from_utf8(id)?.trim().to_owned();
    cleanup.container_id = Some(id.clone());
    // Start without the manager's precheck, as systemd can after reboot or supervisor restart.
    assert!(
        super::LinuxManagedPlatform::systemctl(&[
            "start".to_owned(),
            format!("{}.service", d.unit)
        ])
        .is_err()
    );
    super::LinuxManagedPlatform::systemctl(&["stop".to_owned(), format!("{}.service", d.unit)])?;
    let observed = super::LinuxManagedPlatform::inspect("container", &d.unit)?
        .ok_or("failed start deleted the foreign container")?;
    assert_eq!(observed["Id"].as_str(), Some(id.as_str()));
    assert_eq!(
        observed.pointer("/State/Running").and_then(|v| v.as_bool()),
        Some(true)
    );
    Ok(())
}
