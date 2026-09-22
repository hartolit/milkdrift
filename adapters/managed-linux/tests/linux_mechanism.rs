//! Opt-in physical lane. This invokes real Podman/systemd and never substitutes fake isolation.
mod support;
use milkdrift_authority::*;
use milkdrift_capability::managed::*;
use milkdrift_capability_host::{conformance::*, managed::*, *};
use milkdrift_managed_linux::*;
use milkdrift_persistence::{managed::*, *};
use milkdrift_redb_store::RedbStore;
use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};
use support::Result;

struct Authority;
impl AuthorityEvaluator for Authority {
    fn evaluate(
        &self,
        request: &AuthorityRequest,
    ) -> std::result::Result<AuthorityDecisionSnapshot, AuthorityError> {
        AuthorityDecisionSnapshot::from_evaluation(
            PolicyId::new("physical-lane")?,
            1,
            request.clone(),
            vec![DecisionReasonCode::Allowed],
            AuthorityBudget::default(),
            milkdrift_capability::SideEffectClass::Unknown,
        )
    }
}
fn owner(store: Arc<RedbStore>, platform: Arc<LinuxManagedPlatform>) -> ManagedResources {
    ManagedResources::new(
        store,
        platform,
        Arc::new(Authority),
        Arc::new(milkdrift_runtime::SystemBoundaryClock),
    )
}
fn operation(
    manager: &ManagedResources,
    store: &RedbStore,
    name: &ManagedName,
    key: &str,
    action: ManagedAction,
) -> Result<ManagedResponse> {
    Ok(manager.execute(
        &support::caller()?,
        &ManagedRequest {
            schema_version: 1,
            command: ManagedName::new(key)?,
            installation: name.clone(),
            expected_version: store.managed_installation(name)?.map_or(0, |r| r.version),
            action,
        },
    )?)
}

fn check_retained(
    manager: &ManagedResources,
    store: Arc<RedbStore>,
    platform: Arc<LinuxManagedPlatform>,
    setup: &ApprovedSetup,
    root: &std::path::Path,
    key: &str,
) -> Result {
    let descriptor = setup
        .capabilities
        .iter()
        .find(|d| *d.category() == milkdrift_capability::CapabilityCategory::Process)
        .ok_or("worker missing")?;
    let (request, context, accepted) = support::entered(
        &store,
        descriptor,
        key,
        "command",
        serde_json::json!({"argv":["/bin/sh","-c","set -eu; test $(cat /workspace/retained) = retained; test $(/workspace/home/bin/tool) = tool-ok; test -s /workspace/docs/slotbook.md"]}),
    )?;
    let data = Arc::new(StoreInvocationDataAccess::new(
        store.clone(),
        root.join("temporary"),
        ArtifactReadAuthority::PublicOnly,
    )?);
    let adapter = Arc::new(ManagedWorkerAdapter::new(
        platform,
        store.clone(),
        setup.clone(),
        data,
    )?);
    let snapshot = milkdrift_capability::ResolvedCapabilitySnapshot::from_descriptor(
        descriptor,
        request.operation(),
    )?;
    let invocation = AdapterInvocation::with_context(&snapshot, &request, &context);
    let reporter = RecordingReporter::default();
    adapter.execute(&invocation, &reporter)?;
    assert!(
        reporter
            .events()?
            .iter()
            .any(|e| e.kind().terminal().is_some_and(
                |terminal| terminal.status() == milkdrift_capability::TerminalStatus::Success
            )),
        "retained files/tool were not readable through the reopened worker"
    );
    let id = managed_use_id(&accepted.managed_invocation()?);
    let usage = store.managed_use(&id)?.ok_or("hold missing")?;
    operation(
        manager,
        &store,
        &usage.binding.installation,
        &format!("resolve-{key}"),
        ManagedAction::Resolve {
            use_id: id,
            expected_claim: usage.claim,
        },
    )?;
    Ok(())
}

#[test]
#[ignore = "requires explicitly supplied preloaded recipe, rootless Podman 5.4..5.x, cgroup v2 and systemd user session"]
fn real_linux_worker_conformance_reapply_restart_and_preserved_removal() -> Result {
    if !cfg!(target_os = "linux") {
        return Err("this lane requires Linux".into());
    }
    let recipe_file = PathBuf::from(std::env::var("MILKDRIFT_LINUX_RECIPE")?);
    let recipe = LinuxRecipe::from_json(&std::fs::read(&recipe_file)?)?;
    let root = tempfile::tempdir()?;
    let parent = PathBuf::from(std::env::var("MILKDRIFT_QUADLET_TEST_PARENT")?);
    let quadlet = tempfile::tempdir_in(parent)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(root.path(), std::fs::Permissions::from_mode(0o700))?;
        std::fs::set_permissions(quadlet.path(), std::fs::Permissions::from_mode(0o700))?;
    }
    let config = LinuxManagerConfig {
        state_root: root.path().to_path_buf(),
        quadlet_directory: quadlet.path().to_path_buf(),
        recipes: vec![recipe_file],
    };
    let platform = Arc::new(LinuxManagedPlatform::new(config.clone())?);
    let store = Arc::new(RedbStore::open(root.path().join("store"))?);
    let manager = owner(store.clone(), platform.clone());
    let installation = ManagedName::new("physical-test")?;
    operation(
        &manager,
        &store,
        &installation,
        "prepare",
        ManagedAction::Prepare {
            recipe: recipe.reference()?,
        },
    )?;
    operation(
        &manager,
        &store,
        &installation,
        "apply",
        ManagedAction::Apply {
            recipe: recipe.reference()?,
        },
    )?;
    let record = store
        .managed_installation(&installation)?
        .ok_or("inventory missing")?;
    assert!(
        record.pending.is_none(),
        "physical verification failed: {:?}",
        record.view()
    );
    let setup = record.current.ok_or("setup missing")?;
    let descriptor = setup
        .capabilities
        .iter()
        .find(|d| *d.category() == milkdrift_capability::CapabilityCategory::Process)
        .ok_or("worker missing")?
        .clone();
    let data = Arc::new(StoreInvocationDataAccess::new(
        store.clone(),
        root.path().join("temporary"),
        ArtifactReadAuthority::PublicOnly,
    )?);
    let sequence = AtomicUsize::new(0);
    run_adapter_conformance(|scenario| -> Result<AdapterConformanceCase> {
        let key = format!("physical-{}", sequence.fetch_add(1, Ordering::SeqCst));
        // The scoped working directory permits tool installation; manager files/sockets/devices
        // and rootfs writes must remain denied by the effective mounts/namespaces verified at apply.
        let script = "set -eu; mkdir -p /workspace/home/bin; printf '#!/bin/sh\nprintf tool-ok' > /workspace/home/bin/tool; chmod +x /workspace/home/bin/tool; /workspace/home/bin/tool; printf retained > /workspace/retained; test -s /workspace/docs/slotbook.md; test ! -e /run/podman/podman.sock; test ! -e /dev/dri; if touch /etc/forbidden 2>/dev/null; then exit 71; fi; git --version; rustc --version; cargo --version";
        let (request, context, record) = support::entered(
            &store,
            &descriptor,
            &key,
            "command",
            serde_json::json!({"argv":["/bin/sh","-c",script]}),
        )?;
        let adapter = Arc::new(ManagedWorkerAdapter::new(
            platform.clone(),
            store.clone(),
            setup.clone(),
            data.clone(),
        )?);
        let cleanup_store = store.clone();
        let cleanup_platform = platform.clone();
        let name = installation.clone();
        Ok(AdapterConformanceCase::new(
            adapter,
            descriptor.clone(),
            request,
            context,
            AdapterConformanceExpectations {
                start_replay: StartReplayExpectation::Idempotent,
                available_while_draining: true,
                available_after_shutdown: true,
                unknown_cancellation: UnknownCancellationExpectation::NegativeAcknowledgement,
            },
        )?
        .with_serving_allowance(record.request.limits.clone())
        .with_cleanup(move || {
            let cleanup = || -> Result {
                let id = managed_use_id(&record.managed_invocation()?);
                let usage = cleanup_store.managed_use(&id)?.ok_or("hold missing")?;
                if scenario.executes() && !matches!(usage.phase, ManagedUsePhase::Quiescent { .. })
                {
                    return Err("worker returned without physical stop evidence".into());
                }
                let manager = owner(cleanup_store.clone(), cleanup_platform);
                operation(
                    &manager,
                    &cleanup_store,
                    &name,
                    &format!("resolve-{key}"),
                    ManagedAction::Resolve {
                        use_id: id,
                        expected_claim: usage.claim,
                    },
                )?;
                cleanup_store.verify_managed_integrity()?;
                Ok(())
            };
            cleanup().map_err(|e| e.to_string())
        }))
    })?;
    let before = platform.observe(&setup)?;
    operation(
        &manager,
        &store,
        &installation,
        "reapply",
        ManagedAction::Apply {
            recipe: recipe.reference()?,
        },
    )?;
    assert!(
        store
            .managed_installation(&installation)?
            .ok_or("inventory missing")?
            .pending
            .is_none()
    );
    assert_eq!(platform.observe(&setup)?.running, before.running);
    check_retained(
        &manager,
        store.clone(),
        platform.clone(),
        &setup,
        root.path(),
        "retained-reapply",
    )?;
    // Drop every Milkdrift manager and reopen the same store. Systemd owns any persistent service.
    drop(data);
    drop(manager);
    drop(platform);
    drop(store);
    let platform = Arc::new(LinuxManagedPlatform::new(config)?);
    let store = Arc::new(RedbStore::open(root.path().join("store"))?);
    let manager = owner(store.clone(), platform.clone());
    manager.recover_startup()?;
    check_retained(
        &manager,
        store.clone(),
        platform.clone(),
        &setup,
        root.path(),
        "retained-restart",
    )?;
    assert_eq!(platform.observe(&setup)?.running, before.running);
    assert_eq!(
        store
            .managed_installation(&installation)?
            .ok_or("inventory missing")?
            .generation,
        1
    );
    operation(
        &manager,
        &store,
        &installation,
        "preserve",
        ManagedAction::Preserve {
            disposition: DataDisposition::Preserve,
        },
    )?;
    operation(
        &manager,
        &store,
        &installation,
        "remove",
        ManagedAction::Remove {},
    )?;
    let removed = store
        .managed_installation(&installation)?
        .ok_or("tombstone missing")?;
    assert!(
        removed.removed,
        "removal did not finish: {:?}",
        removed.view()
    );
    for resource in &removed
        .current
        .ok_or("removal identities missing")?
        .resources
    {
        if resource.ownership == ResourceOwnership::Owned
            && resource.kind != ManagedResourceKind::Service
        {
            let output = std::process::Command::new("/usr/bin/podman")
                .args(["volume", "inspect", &resource.identity])
                .output()?;
            assert!(
                output.status.success(),
                "preserved owned volume disappeared"
            );
            // This test explicitly preserved the volume. Print its exact identity for operator cleanup.
            eprintln!("preserved test volume: {}", resource.identity);
        }
    }
    Ok(())
}
