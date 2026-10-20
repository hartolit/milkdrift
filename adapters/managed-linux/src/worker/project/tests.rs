use super::{PROJECT_ID, PinnedProject, project_id_mappings};
use crate::{LinuxRecipe, ModelService, recipe::Deployment, units};
use milkdrift_capability::{BoundedJson, managed::ManagedName};
use milkdrift_persistence::managed::ApprovedSetup;
use std::path::Path;

type Result<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

fn setup(image: Option<String>, generation: u64) -> Result<ApprovedSetup> {
    let mut recipe = LinuxRecipe::from_json(include_bytes!(
        "../../../tests/fixtures/owned-recipe-v2.json"
    ))?;
    recipe.model_service = ModelService::Disabled {};
    if let Some(image) = image {
        recipe.worker_image = image;
    }
    let ownership = crate::digest("attached-project-test");
    let prefix = units::volume_prefix("project-fixture", &ownership);
    let reference = recipe.reference()?;
    let deployment = Deployment {
        recipe,
        installation: ManagedName::new("attachment-test")?,
        generation,
        unit: format!("{prefix}g{generation}"),
        volume_prefix: prefix,
        manager_root: "/manager".into(),
        systemd_directory: "/quadlets".into(),
        quadlet_directory: "/quadlets".into(),
    };
    Ok(ApprovedSetup {
        protection: None,
        recipe: reference,
        mechanism: "linux-quadlet-v4".to_owned(),
        configuration: BoundedJson::new(serde_json::to_value(deployment)?)?,
        platform_owner: "project-fixture".to_owned(),
        ownership,
        resources: Vec::new(),
        capabilities: Vec::new(),
    })
}

#[cfg(target_os = "linux")]
fn pinned(path: &Path, write: bool) -> Result<PinnedProject> {
    use std::os::unix::fs::MetadataExt;
    let physical = path.canonicalize()?;
    let metadata = physical.metadata()?;
    Ok(PinnedProject::open(
        path.into(),
        physical,
        format!("unix:{}:{}", metadata.dev(), metadata.ino()),
        write,
    )?)
}

fn maps() -> serde_json::Value {
    serde_json::json!({
        "UidMap": ["0:1:65534", "65534:0:1", "65535:65535:1"],
        "GidMap": ["0:65537:65534", "65534:0:1", "65535:131071:1"]
    })
}

#[test]
#[cfg(target_os = "linux")]
fn project_cannot_contain_the_operator_pin_executable() -> Result {
    let root = tempfile::tempdir()?;
    let project = root.path().join("project");
    std::fs::create_dir(&project)?;
    let inside = project.join("daemon");
    std::fs::write(&inside, b"operator executable")?;
    let outside = root.path().join("daemon");
    std::fs::write(&outside, b"operator executable")?;
    let pinned = pinned(&project, true)?;
    assert!(pinned.verify_helper_path(&inside).is_err());
    pinned.verify_helper_path(&outside)?;
    Ok(())
}

#[test]
fn attachment_refuses_empty_access_narrow_claims_policy_and_generation_drift() -> Result {
    use milkdrift_blueprint::*;
    use milkdrift_capability::*;
    use milkdrift_capability_host::*;
    let project = tempfile::tempdir()?;
    let native = capture_native_project_root(project.path(), false)?;
    let folder = TaskProjectFolder {
        requested: native.requested.to_string_lossy().into_owned(),
        physical: native.physical.to_string_lossy().into_owned(),
        identity: native.identity,
    };
    let binding = TaskProjectBinding {
        project_id: "project:test".into(),
        generation: 1,
        folder: folder.clone(),
        paths: vec![TaskPathAccess {
            path: folder.requested.clone(),
            write: true,
        }],
        scopes: vec![folder],
        settings: ProjectSettings {
            allow_changes: true,
            environment: ProjectEnvironment::Podman {
                capability: "managed.attachment-test.worker".into(),
                operation: "workspace.execute".into(),
                generation: 1,
            },
        },
        access: TaskProjectAccess::Write,
        tools: Vec::new(),
    };
    let request = InvocationRequest::new(
        InvocationId::new("project-view")?,
        CapabilityId::new("managed.attachment-test.worker")?,
        OperationId::new("workspace.execute")?,
        None,
        None,
        Vec::new(),
        std::collections::BTreeMap::new(),
    )?;
    let context = AdapterExecutionContext::direct(DirectInputSelection::new(&request, 1024)?);
    assert!(
        context
            .clone()
            .with_project_binding_for_test(binding.clone())
            .project_process(
                ProjectProcessKind::Podman,
                "managed.attachment-test.worker",
                "workspace.execute",
                1
            )?
            .is_some_and(|access| matches!(
                access,
                ProjectProcessAccess::Directory { write: true, .. }
            ))
    );
    for change in 0..5 {
        let mut changed = binding.clone();
        match change {
            0 => changed.access = TaskProjectAccess::None,
            1 => changed.settings.allow_changes = false,
            2 => changed.paths.clear(),
            3 => {
                changed.settings.environment = ProjectEnvironment::Podman {
                    capability: "managed.attachment-test.worker".into(),
                    operation: "workspace.execute".into(),
                    generation: 2,
                }
            }
            _ => {
                changed.access = TaskProjectAccess::None;
                changed.paths.clear();
                changed.scopes.clear();
            }
        }
        let changed_context = context.clone().with_project_binding_for_test(changed);
        assert!(PinnedProject::capture(&changed_context, &setup(None, 1)?).is_err());
    }
    Ok(())
}

#[test]
fn attached_maps_expose_only_one_explicit_nonroot_user_and_group() -> Result {
    assert!(project_id_mappings(Some(&maps())));
    let reordered = serde_json::json!({
        "UidMap": ["0:65537:65534", "65535:131071:1", "65534:0:1"],
        "GidMap": ["0:65537:65534", "65535:131071:1", "65534:0:1"]
    });
    assert!(project_id_mappings(Some(&reordered)));
    for invalid in [
        vec!["0:0:65536"],
        vec!["0:1:65536"],
        vec!["0:1:65534", "65534:0:2"],
        vec!["0:1:65534", "65534:0:1"],
        vec!["0:1:65535", "65535:0:1"],
        vec!["0:1:65534", "65534:0:1", "65535:18446744073709551615:1"],
    ] {
        for key in ["UidMap", "GidMap"] {
            let mut changed = maps();
            *changed.get_mut(key).ok_or("map missing")? = serde_json::json!(invalid);
            assert!(!project_id_mappings(Some(&changed)), "{key} {invalid:?}");
        }
    }
    Ok(())
}

#[cfg(target_os = "linux")]
#[test]
fn attachment_refuses_missing_helper_and_custom_engine_root_overlap() -> Result {
    let parent = tempfile::tempdir()?;
    let root = parent.path().join("project");
    std::fs::create_dir(&root)?;
    let mut project = pinned(&root, true)?;
    assert!(project.activate(None, 1000).is_err());
    let outside = tempfile::tempdir()?;
    project.verify_engine_paths(&serde_json::json!({"store":{
        "graphRoot":outside.path(), "runRoot":outside.path()
    }}))?;
    for overlap in [&root, parent.path()] {
        assert!(
            project
                .verify_engine_paths(&serde_json::json!({"store":{
                    "graphRoot":overlap, "runRoot":outside.path()
                }}))
                .is_err()
        );
    }
    assert!(project.verify_engine_paths(&serde_json::json!({})).is_err());
    Ok(())
}

#[cfg(target_os = "linux")]
#[test]
fn attachment_pins_original_directory_and_refuses_path_replacement() -> Result {
    let parent = tempfile::tempdir()?;
    let path = parent.path().join("project");
    std::fs::create_dir(&path)?;
    std::fs::write(path.join("retained"), b"original")?;
    let project = pinned(&path, true)?;
    std::fs::rename(&path, parent.path().join("original"))?;
    std::fs::create_dir(&path)?;
    std::fs::write(path.join("retained"), b"replacement")?;
    assert!(project.revalidate().is_err());
    assert_eq!(
        std::fs::read(Path::new(&project.source).join("retained"))?,
        b"original"
    );
    assert_eq!(std::fs::read(path.join("retained"))?, b"replacement");
    Ok(())
}

#[cfg(target_os = "linux")]
#[test]
fn attachment_arguments_preserve_folder_ownership_without_relabel_or_owned_volume() -> Result {
    let root = tempfile::tempdir()?;
    for write in [false, true] {
        let project = pinned(root.path(), write)?;
        let args = super::super::task_arguments(
            &setup(None, 1)?,
            "mdtask-test",
            &["/usr/bin/cargo".into(), "check".into()],
            Some(&project),
        )?;
        assert!(args.iter().any(|arg| arg == "--workdir=/project"));
        assert!(
            args.iter()
                .any(|arg| arg == &format!("--user={PROJECT_ID}:{PROJECT_ID}"))
        );
        assert!(args.iter().any(|arg| arg.contains(&format!(
            "dst=/project,{},",
            if write { "rw" } else { "ro" }
        ))));
        assert!(!args.iter().any(|arg| arg.contains("/workspace")
            || arg.contains(":U")
            || arg.contains("relabel")));
        assert!(!args.iter().any(|arg| arg == "--privileged"));
    }
    Ok(())
}

#[cfg(target_os = "linux")]
#[test]
fn actual_attachment_requires_exact_source_access_cwd_user_and_no_extra_mounts() -> Result {
    let root = tempfile::tempdir()?;
    for write in [false, true] {
        let project = pinned(root.path(), write)?;
        let value = serde_json::json!({
            "Config": { "WorkingDir": "/project", "User": format!("{PROJECT_ID}:{PROJECT_ID}") },
            "HostConfig": { "IDMappings": maps() },
            "Mounts": [
                {"Type":"bind", "Destination":"/project", "Source":project.source, "RW":write, "Propagation":"rprivate"},
                {"Type":"tmpfs", "Destination":"/tmp"}
            ]
        });
        project.verify(&value)?;
        for (pointer, replacement) in [
            ("/Config/WorkingDir", serde_json::json!("/workspace")),
            ("/Config/User", serde_json::json!("0:0")),
            ("/Mounts/0/Source", serde_json::json!(root.path())),
            ("/Mounts/0/RW", serde_json::json!(!write)),
            ("/Mounts/0/Propagation", serde_json::json!("shared")),
            ("/Mounts/0/Type", serde_json::json!("volume")),
        ] {
            let mut changed = value.clone();
            *changed.pointer_mut(pointer).ok_or("field missing")? = replacement;
            assert!(project.verify(&changed).is_err(), "{pointer}");
        }
        let mut changed = value.clone();
        changed.get_mut("Mounts").and_then(serde_json::Value::as_array_mut).ok_or("mounts missing")?
            .push(serde_json::json!({"Type":"bind","Source":"/run/podman/podman.sock","Destination":"/socket","RW":true}));
        assert!(project.verify(&changed).is_err());
    }
    Ok(())
}

#[cfg(target_os = "linux")]
#[test]
#[ignore = "requires an explicitly approved local exact image and disposable rootless Podman resources"]
fn physical_attachment_preserves_host_files_modes_and_denies_read_only_changes() -> Result {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    let image = std::env::var("MILKDRIFT_PROJECT_WORKER_IMAGE")?;
    let helper = std::path::PathBuf::from(std::env::var("MILKDRIFT_PROJECT_PIN_HELPER")?);
    let root = tempfile::tempdir()?.keep();
    #[expect(
        clippy::print_stderr,
        reason = "The explicit physical lane reports retained evidence even on failure"
    )]
    {
        eprintln!("attached project evidence: {}", root.display());
    }
    let project_path = root.join("project");
    std::fs::create_dir(&project_path)?;
    std::fs::set_permissions(&project_path, std::fs::Permissions::from_mode(0o750))?;
    std::fs::write(project_path.join("retained"), b"retained")?;
    std::fs::set_permissions(
        project_path.join("retained"),
        std::fs::Permissions::from_mode(0o640),
    )?;
    let before = project_path.join("retained").metadata()?;
    let run = |generation, write, command: &str| -> Result {
        let setup = setup(Some(image.clone()), generation)?;
        let mut pinned = pinned(&project_path, write)?;
        pinned.activate(Some(&helper), 900_000)?;
        let name = format!("mdtask-project-{}-{generation}", std::process::id());
        let stopped = super::super::run_and_remove_task(
            || {
                super::super::run_task(
                    &setup,
                    &name,
                    &["/bin/sh".into(), "-c".into(), command.into()],
                    Some(&pinned),
                    None,
                    std::time::Duration::from_secs(30),
                    65_536,
                )
            },
            || super::super::cleanup_task(&setup, &name),
        );
        pinned.stop_pin()?;
        let stopped = stopped?;
        let output = stopped.output?;
        let retained = project_path.join("retained").metadata()?;
        std::fs::write(
            root.join(format!("attempt-{generation}.json")),
            serde_json::to_vec_pretty(&serde_json::json!({
                "image": image, "write":write, "success":output.success,
                "exit_code":output.exit_code, "stderr":String::from_utf8_lossy(&output.stderr),
                "container_absence_verified":true, "uid":retained.uid(), "gid":retained.gid(), "mode":retained.mode(),
                "retained_bytes":std::fs::read_to_string(project_path.join("retained"))?
            }))?,
        )?;
        if !output.success {
            return Err(format!(
                "physical attachment workload failed: {}",
                String::from_utf8_lossy(&output.stderr)
            )
            .into());
        }
        Ok(())
    };
    run(
        1,
        true,
        "set -eu; test \"$PWD\" = /project; test \"$(cat retained)\" = retained; printf created > created; test ! -e /workspace; test ! -e /run/podman/podman.sock; if touch /etc/forbidden 2>/dev/null; then exit 1; fi",
    )?;
    assert_eq!(std::fs::read(project_path.join("created"))?, b"created");
    run(
        2,
        false,
        "set -eu; test \"$(cat created)\" = created; if touch forbidden 2>/dev/null; then exit 1; fi; if printf changed > retained 2>/dev/null; then exit 1; fi",
    )?;
    run(
        3,
        true,
        "set -eu; test \"$(cat retained)\" = retained; test \"$(cat created)\" = created; printf edited > created",
    )?;
    let after = project_path.join("retained").metadata()?;
    assert_eq!(
        (before.uid(), before.gid(), before.mode()),
        (after.uid(), after.gid(), after.mode())
    );
    assert_eq!(std::fs::read(project_path.join("retained"))?, b"retained");
    let created = project_path.join("created").metadata()?;
    assert_eq!((created.uid(), created.gid()), (before.uid(), before.gid()));
    assert_eq!(std::fs::read(project_path.join("created"))?, b"edited");
    assert!(!project_path.join("forbidden").exists());
    std::fs::write(
        root.join("evidence.json"),
        serde_json::to_vec_pretty(&serde_json::json!({
            "image":image,"project":project_path,"uid":after.uid(),"gid":after.gid(),
            "mode":after.mode(),"retained":"retained","created":"edited","read_only_denied":true
        }))?,
    )?;
    Ok(())
}
