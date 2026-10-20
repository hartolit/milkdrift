//! The production process adapter selects pinned projects without per-project profiles.
use super::*;
use milkdrift_blueprint::{
    ProjectEnvironment, ProjectSettings, TaskPathAccess, TaskProjectAccess, TaskProjectBinding,
    TaskProjectFolder,
};

fn binding(path: &Path, access: TaskProjectAccess) -> TestResult<TaskProjectBinding> {
    let native = milkdrift_capability_host::capture_native_project_root(
        path,
        access == TaskProjectAccess::Write,
    )?;
    let folder = TaskProjectFolder {
        requested: native.requested.to_string_lossy().into_owned(),
        physical: native.physical.to_string_lossy().into_owned(),
        identity: native.identity,
    };
    Ok(TaskProjectBinding {
        project_id: "project:test".into(),
        generation: 1,
        paths: vec![TaskPathAccess {
            path: folder.requested.clone(),
            write: access == TaskProjectAccess::Write,
        }],
        scopes: vec![folder.clone()],
        folder,
        settings: ProjectSettings {
            environment: ProjectEnvironment::Process {
                capability: "local-process-fixture".into(),
                operation: "process.execute".into(),
                generation: 1,
            },
            allow_changes: access == TaskProjectAccess::Write,
        },
        access,
        tools: Vec::new(),
    })
}

fn project_profile(data: &TestDataAccess) -> TestResult<milkdrift_local_process::ProcessProfile> {
    let mut value = profile_value(
        &data.root,
        vec![json!("mark"), json!("actual-project-file.txt")],
    )?;
    value
        .pointer_mut("/profile")
        .and_then(Value::as_object_mut)
        .ok_or("profile absent")?
        .insert("working_directory".into(), json!({"type":"project"}));
    parse_profile(&value)
}

#[test]
fn one_profile_writes_two_selected_projects_and_preserves_existing_files() -> TestResult {
    let data = Arc::new(TestDataAccess::new()?);
    let first = data.root.join("first");
    let second = data.root.join("second");
    fs::create_dir(&first)?;
    fs::create_dir(&second)?;
    fs::write(first.join("keep.txt"), b"user content")?;
    let profile = project_profile(&data)?;
    let (host, snapshot) = setup(
        profile.clone(),
        data,
        Arc::new(InMemorySecretResolver::new()),
    )?;
    for (index, path) in [(0, &first), (1, &second)] {
        let request = request(&profile, &format!("project-entry-{index}"), Vec::new())?;
        let context = context(&request)?
            .with_project_binding_for_test(binding(path, TaskProjectAccess::Write)?);
        let reporter = TestReporter::default();
        host.execute_exact_with_context(&snapshot, &request, &context, &reporter)?;
        assert_eq!(
            terminal_status(&reporter.events()?),
            Some(TerminalStatus::Success)
        );
        assert_eq!(fs::read(path.join("actual-project-file.txt"))?, b"entered");
    }
    assert_eq!(fs::read(first.join("keep.txt"))?, b"user content");
    Ok(())
}

#[test]
fn readonly_missing_binding_and_replaced_project_refuse_before_process_entry() -> TestResult {
    let data = Arc::new(TestDataAccess::new()?);
    let project = data.root.join("project");
    fs::create_dir(&project)?;
    let accepted = binding(&project, TaskProjectAccess::Write)?;
    let profile = project_profile(&data)?;
    let (host, snapshot) = setup(
        profile.clone(),
        data.clone(),
        Arc::new(InMemorySecretResolver::new()),
    )?;
    for (index, context_binding) in [
        (0, None),
        (1, Some(binding(&project, TaskProjectAccess::Read)?)),
    ] {
        let request = request(&profile, &format!("refused-project-{index}"), Vec::new())?;
        let context = match context_binding {
            Some(binding) => context(&request)?.with_project_binding_for_test(binding),
            None => context(&request)?,
        };
        let reporter = TestReporter::default();
        assert!(
            host.execute_exact_with_context(&snapshot, &request, &context, &reporter)
                .is_err()
        );
        assert!(reporter.events()?.is_empty());
        assert!(!project.join("actual-project-file.txt").exists());
    }
    fs::rename(&project, data.root.join("moved-project"))?;
    fs::create_dir(&project)?;
    let request = request(&profile, "replaced-project", Vec::new())?;
    let context = context(&request)?.with_project_binding_for_test(accepted);
    let reporter = TestReporter::default();
    assert!(
        host.execute_exact_with_context(&snapshot, &request, &context, &reporter)
            .is_err()
    );
    assert!(reporter.events()?.is_empty());
    assert!(!project.join("actual-project-file.txt").exists());
    Ok(())
}

#[test]
fn fixed_directory_profile_refuses_a_different_project_before_effects() -> TestResult {
    let data = Arc::new(TestDataAccess::new()?);
    let first = data.root.join("first");
    let second = data.root.join("second");
    fs::create_dir(&first)?;
    fs::create_dir(&second)?;
    let mut value = profile_value(&data.root, vec![json!("mark"), json!("unexpected.txt")])?;
    value
        .pointer_mut("/profile")
        .and_then(Value::as_object_mut)
        .ok_or("profile absent")?
        .insert(
            "working_directory".into(),
            json!({"type":"authorized_host_path","path":first}),
        );
    let profile = parse_profile(&value)?;
    let (host, snapshot) = setup(
        profile.clone(),
        data,
        Arc::new(InMemorySecretResolver::new()),
    )?;
    let request = request(&profile, "fixed-directory-conflict", Vec::new())?;
    let context = context(&request)?
        .with_project_binding_for_test(binding(&second, TaskProjectAccess::Write)?);
    let reporter = TestReporter::default();
    assert!(
        host.execute_exact_with_context(&snapshot, &request, &context, &reporter)
            .is_err()
    );
    assert!(!first.join("unexpected.txt").exists());
    assert!(!second.join("unexpected.txt").exists());
    Ok(())
}
