use super::start_owned;
use crate::{LinuxRecipe, ModelService, recipe::Deployment, units};
use milkdrift_capability::{BoundedJson, managed::ManagedName};
use milkdrift_persistence::managed::ApprovedSetup;
use std::cell::Cell;

type Result<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

pub(super) fn owned_setup() -> Result<ApprovedSetup> {
    let recipe =
        LinuxRecipe::from_json(include_bytes!("../../tests/fixtures/owned-recipe-v2.json"))?;
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
        mechanism: "linux-quadlet-v3".to_owned(),
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

#[test]
fn generated_service_uses_the_owned_policy_and_model_identity() -> Result {
    let setup = owned_setup()?;
    let unit = units::unit_text(&setup, true)?.ok_or("unit absent")?;
    assert!(unit.contains("--alias research/model-v2\n"));
    assert!(
        unit.contains("--memory=1073741824 --memory-swap=1073741824 --cpus=2.50 --pids-limit=96")
    );
    assert!(unit.contains("size=134217728"));
    assert!(unit.contains("TimeoutStartSec=180000ms\nTimeoutStopSec=13s"));
    assert!(unit.contains("--n-gpu-layers 0"));
    let d = units::deployment(&setup)?;
    let profile =
        crate::model::profile_for_service(&d.recipe.model_service, &d.installation, d.generation)?
            .ok_or("profile absent")?;
    let profile = serde_json::to_value(profile)?;
    assert_eq!(profile["model"], "research/model-v2");
    assert_eq!(profile["limits"]["request_timeout_ms"], 600000);
    assert_eq!(profile["limits"]["max_response_bytes"], 65536);
    Ok(())
}
