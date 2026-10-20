//! Current operator inputs refuse an earlier accepted binding without changing that acceptance.
use super::*;
use milkdrift_blueprint::{
    ProjectEnvironment, ProjectSettings, TaskPathAccess, TaskProjectAccess, TaskProjectBinding,
    TaskProjectFolder,
};
use milkdrift_capability_host::{
    AdapterExecutionContext, DirectInputSelection, capture_native_project_root,
};
use milkdrift_redb_store::RedbStore;

#[test]
fn retained_project_binding_is_refused_by_current_operator_protection_before_adapter_entry()
-> TestResult {
    let directory = tempfile::tempdir()?;
    let project = directory.path().join("project");
    std::fs::create_dir(&project)?;
    let root = capture_native_project_root(&project, false)?;
    let folder = TaskProjectFolder {
        requested: root.requested.to_string_lossy().into_owned(),
        physical: root.physical.to_string_lossy().into_owned(),
        identity: root.identity,
    };
    let binding = TaskProjectBinding {
        project_id: "project:retained".into(),
        generation: 1,
        paths: vec![TaskPathAccess {
            path: folder.requested.clone(),
            write: false,
        }],
        scopes: vec![folder.clone()],
        folder,
        settings: ProjectSettings {
            environment: ProjectEnvironment::Host,
            allow_changes: false,
        },
        access: TaskProjectAccess::Read,
        tools: Vec::new(),
    };
    let accepted_bytes = serde_json::to_vec(&binding)?;
    let descriptor = descriptor("protected-project", 1, "fixture", 1)?;
    let snapshot = ResolvedCapabilitySnapshot::from_descriptor(
        &descriptor,
        &OperationId::new("model.generate")?,
    )?;
    let request = request(&snapshot, "accepted-before-config-change")?;
    let context = AdapterExecutionContext::direct(DirectInputSelection::new(&request, 1024)?)
        .with_project_binding_for_test(binding.clone());
    let store = Arc::new(RedbStore::open(directory.path().join("store"))?);
    let earlier = host(BTreeMap::new(), 1)?;
    earlier.install_native_path_store(store.clone(), Vec::new())?;
    let old_adapter = Arc::new(FakeAdapter::new(descriptor.identity().clone()));
    earlier.register(
        descriptor.clone(),
        old_adapter.clone(),
        Some(observation(&descriptor, 100, true)?),
    )?;
    earlier.execute_exact_with_context(
        &snapshot,
        &request,
        &context,
        &CountingReporter::default(),
    )?;
    assert_eq!(old_adapter.execute_count.load(Ordering::SeqCst), 1);
    drop(earlier);

    let operator_file = project.join("new-operator.token");
    std::fs::write(&operator_file, b"protected bytes")?;
    let current = host(BTreeMap::new(), 1)?;
    current.install_native_path_store(store, vec![operator_file.clone()])?;
    let current_adapter = Arc::new(FakeAdapter::new(descriptor.identity().clone()));
    current.register(
        descriptor.clone(),
        current_adapter.clone(),
        Some(observation(&descriptor, 100, true)?),
    )?;
    let reporter = CountingReporter::default();
    let Err(error) = current.execute_exact_with_context(&snapshot, &request, &context, &reporter)
    else {
        return Err("current protected inputs must refuse the retained binding".into());
    };
    assert!(
        error
            .to_string()
            .contains("current protected operator files")
    );
    assert_eq!(current_adapter.execute_count.load(Ordering::SeqCst), 0);
    assert_eq!(reporter.0.load(Ordering::SeqCst), 0);
    assert_eq!(serde_json::to_vec(&binding)?, accepted_bytes);
    assert_eq!(
        capture_native_project_root(&project, false)?.identity,
        binding.folder.identity
    );
    assert_eq!(std::fs::read(operator_file)?, b"protected bytes");
    Ok(())
}
