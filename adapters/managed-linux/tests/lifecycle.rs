//! Deterministic durable-boundary evidence; these tests make no OS-isolation claim.
use milkdrift_authority::{
    ActorRef, AuthorityBudget, AuthorityDecisionSnapshot, AuthorityError, AuthorityEvaluator,
    AuthorityOperation, AuthorityRequest, BoundaryTimeMillis, CapabilityExecutionRequirements,
    DecisionId, DecisionReasonCode, GrantDigest, GrantId, PolicyId, RequestedResourceFacts,
};
use milkdrift_capability::{BoundedJson, SideEffectClass, managed::*};
use milkdrift_capability_host::managed::{ManagedError, ManagedPlatform, ManagedResources};
use milkdrift_persistence::{PageSize, PersistenceError, managed::*};
use milkdrift_redb_store::{
    FaultInjector, FaultPoint, RedbStore, RedbStoreConfig, injected_failure,
};
use std::{
    collections::BTreeSet,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
};
type Result<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

struct Authority(bool);
impl AuthorityEvaluator for Authority {
    fn evaluate(
        &self,
        request: &AuthorityRequest,
    ) -> std::result::Result<AuthorityDecisionSnapshot, AuthorityError> {
        AuthorityDecisionSnapshot::from_evaluation(
            PolicyId::new("managed-tests")?,
            1,
            request.clone(),
            vec![if self.0 {
                DecisionReasonCode::Allowed
            } else {
                DecisionReasonCode::WrongActor
            }],
            AuthorityBudget::default(),
            SideEffectClass::Unknown,
        )
    }
}
fn caller() -> Result<AuthorityRequest> {
    Ok(AuthorityRequest {
        decision: DecisionId::new("test-managed")?,
        actor: ActorRef::new("human:managed")?,
        grant: GrantId::new("grant:managed")?,
        grant_revision: 1,
        grant_digest: GrantDigest::new(format!("b3_{}", "1".repeat(64)))?,
        revocation_generation: 0,
        operation: AuthorityOperation::AdministerCapabilities,
        resources: RequestedResourceFacts::empty(),
        budget: Default::default(),
        evaluated_at: BoundaryTimeMillis::new(1000),
        provenance: Default::default(),
    })
}
fn reference(ch: char) -> Result<RecipeReference> {
    Ok(RecipeReference {
        name: ManagedName::new("slotbook")?,
        digest: format!("b3_{}", ch.to_string().repeat(64)),
    })
}
fn request(key: &str, version: u64, action: ManagedAction) -> Result<ManagedRequest> {
    Ok(ManagedRequest {
        schema_version: 1,
        installation: ManagedName::new("slotbook")?,
        command: ManagedName::new(key)?,
        expected_version: version,
        action,
    })
}
fn failure(value: impl std::fmt::Display) -> ManagedError {
    ManagedError::Platform(value.to_string())
}

#[derive(Default)]
struct Physical {
    volumes: BTreeSet<String>,
    running: bool,
    starts: usize,
    content: String,
    interrupt: Option<(ManagedStep, bool)>,
    calls: usize,
}
#[derive(Default)]
struct Platform(
    Mutex<Physical>,
    Mutex<Option<(std::sync::mpsc::Sender<()>, std::sync::mpsc::Receiver<()>)>>,
);
impl ManagedPlatform for Platform {
    fn plan(
        &self,
        _: &ManagedName,
        recipe: &RecipeReference,
        ownership: &str,
        _: u64,
    ) -> std::result::Result<ApprovedSetup, ManagedError> {
        let gate = self.1.lock().map_err(failure)?.take();
        if let Some((ready, resume)) = gate {
            ready.send(()).map_err(failure)?;
            resume
                .recv_timeout(std::time::Duration::from_secs(10))
                .map_err(failure)?;
        }
        let resources = [
            (
                "working",
                ResourceOwnership::Owned,
                ManagedResourceKind::WorkingArea,
            ),
            ("data", ResourceOwnership::Owned, ManagedResourceKind::Data),
            (
                "model",
                ResourceOwnership::Owned,
                ManagedResourceKind::Service,
            ),
            (
                "attached",
                ResourceOwnership::Attached,
                ManagedResourceKind::Service,
            ),
            (
                "toolchain",
                ResourceOwnership::Shared,
                ManagedResourceKind::Prerequisite,
            ),
        ]
        .into_iter()
        .map(|(name, class, kind)| {
            Ok(ManagedResourceView {
                name: ManagedName::new(name).map_err(failure)?,
                kind,
                ownership: class,
                identity: format!("{ownership}-{name}"),
                disposition: DataDisposition::Preserve,
            })
        })
        .collect::<std::result::Result<Vec<_>, ManagedError>>()?;
        Ok(ApprovedSetup {
            recipe: recipe.clone(),
            mechanism: "deterministic-test".to_owned(),
            configuration: BoundedJson::new(serde_json::json!({"exact":recipe.digest}))
                .map_err(failure)?,
            platform_owner: "test-platform".to_owned(),
            ownership: ownership.to_owned(),
            resources,
            capabilities: Vec::new(),
        })
    }
    fn diagnose(&self, _: &ApprovedSetup) -> std::result::Result<Vec<String>, ManagedError> {
        Ok(vec!["deterministic test, no OS claim".to_owned()])
    }
    fn reconcile(
        &self,
        record: &InstallationRecord,
        step: ManagedStep,
    ) -> std::result::Result<ManagedObservation, ManagedError> {
        let pending = record
            .pending
            .as_ref()
            .ok_or_else(|| failure("no intent before effect"))?;
        let mut physical = self.0.lock().map_err(failure)?;
        physical.calls += 1;
        let interrupt = physical.interrupt.is_some_and(|(at, _)| at == step);
        if interrupt && physical.interrupt == Some((step, false)) {
            physical.interrupt = None;
            return Err(failure("injected before platform action"));
        }
        match step {
            ManagedStep::PrepareStorage => {
                for r in &pending.change.candidate.resources {
                    physical.volumes.insert(r.identity.clone());
                }
            }
            ManagedStep::StartService => {
                if !physical.running {
                    physical.starts += 1;
                }
                physical.running = true;
            }
            ManagedStep::StopService => physical.running = false,
            ManagedStep::RemoveStorage => {
                for r in &pending.change.candidate.resources {
                    if r.ownership == ResourceOwnership::Owned
                        && r.disposition == DataDisposition::DeleteOnRemoval
                    {
                        physical.volumes.remove(&r.identity);
                    }
                }
            }
            _ => {}
        }
        if interrupt {
            physical.interrupt = None;
            return Err(failure("injected lost platform response"));
        }
        Ok(observation(physical.running))
    }
    fn observe(&self, _: &ApprovedSetup) -> std::result::Result<ManagedObservation, ManagedError> {
        Ok(observation(self.0.lock().map_err(failure)?.running))
    }
    fn requirements(
        &self,
        setup: &ApprovedSetup,
    ) -> std::result::Result<CapabilityExecutionRequirements, ManagedError> {
        Ok(CapabilityExecutionRequirements {
            network_destinations: BTreeSet::from([if setup.recipe.digest.ends_with('1') {
                "old.example".to_owned()
            } else {
                "new.example".to_owned()
            }]),
            ..Default::default()
        })
    }
    fn fence(
        &self,
        _: &ApprovedSetup,
        _: &ManagedUse,
    ) -> std::result::Result<QuiescenceEvidence, ManagedError> {
        Err(failure("not part of this platform fixture"))
    }
}
fn observation(running: bool) -> ManagedObservation {
    ManagedObservation {
        digest: format!("b3_{}", "a".repeat(64)),
        summary: "verified deterministic boundary".to_owned(),
        running,
    }
}
fn owner(store: Arc<RedbStore>, platform: Arc<Platform>) -> ManagedResources {
    ManagedResources::new(
        store,
        platform,
        Arc::new(Authority(true)),
        Arc::new(milkdrift_runtime::SystemBoundaryClock),
    )
}
fn current(store: &RedbStore) -> Result<InstallationRecord> {
    store
        .managed_installation(&ManagedName::new("slotbook")?)?
        .ok_or_else(|| "missing inventory".into())
}

#[test]
fn concurrent_command_replay_cannot_drive_the_same_platform_transition_twice() -> Result {
    let root = tempfile::tempdir()?;
    let store = Arc::new(RedbStore::open(root.path())?);
    let platform = Arc::new(Platform::default());
    let manager = Arc::new(owner(store.clone(), platform.clone()));
    let apply = request(
        "install",
        0,
        ManagedAction::Apply {
            recipe: reference('1')?,
        },
    )?;
    let (ready_tx, ready_rx) = std::sync::mpsc::channel();
    let (resume_tx, resume_rx) = std::sync::mpsc::channel();
    *platform.1.lock().map_err(|e| e.to_string())? = Some((ready_tx, resume_rx));
    let task_manager = manager.clone();
    let task_request = apply.clone();
    let actor = caller()?;
    let first = std::thread::spawn(move || task_manager.execute(&actor, &task_request));
    ready_rx.recv_timeout(std::time::Duration::from_secs(10))?;
    let competing = manager.execute(&caller()?, &apply);
    resume_tx.send(())?;
    let receipt = first.join().map_err(|_| "driver panicked")??;
    assert!(matches!(competing, Err(ManagedError::Conflict(_))));
    assert_eq!(platform.0.lock().map_err(|e| e.to_string())?.calls, 5);
    assert_eq!(manager.execute(&caller()?, &apply)?, receipt);
    assert!(current(&store)?.admission_open);
    store.verify_managed_integrity()?;
    Ok(())
}

#[test]
fn reapply_update_remove_preserve_content_and_exact_receipts() -> Result {
    let root = tempfile::tempdir()?;
    let store = Arc::new(RedbStore::open(root.path())?);
    let platform = Arc::new(Platform::default());
    let manager = owner(store.clone(), platform.clone());
    let apply = request(
        "apply-one",
        0,
        ManagedAction::Apply {
            recipe: reference('1')?,
        },
    )?;
    let accepted = manager.execute(&caller()?, &apply)?;
    assert_eq!(accepted.state, "pending");
    assert!(current(&store)?.admission_open);
    platform.0.lock().map_err(|e| e.to_string())?.content =
        "source edits, output, documentation and database".to_owned();
    let resources = current(&store)?.current.ok_or("setup absent")?.resources;
    manager.execute(
        &caller()?,
        &request("reapply", current(&store)?.version, apply.action.clone())?,
    )?;
    assert_eq!(platform.0.lock().map_err(|e| e.to_string())?.starts, 1);
    assert_eq!(
        platform.0.lock().map_err(|e| e.to_string())?.content,
        "source edits, output, documentation and database"
    );
    assert_eq!(manager.execute(&caller()?, &apply)?, accepted);
    let mut other_scope = caller()?;
    other_scope.resources.run = Some(serde_json::from_str(r#""other-run""#)?);
    assert!(matches!(
        manager.execute(&other_scope, &apply),
        Err(ManagedError::Conflict(_))
    ));

    assert!(
        manager
            .execute(&caller()?, &request("stale", 1, ManagedAction::Stop {})?)
            .is_err()
    );
    assert!(
        manager
            .execute(
                &caller()?,
                &request(
                    "changed",
                    current(&store)?.version,
                    ManagedAction::Apply {
                        recipe: reference('2')?
                    }
                )?
            )
            .is_err()
    );
    assert!(
        manager
            .execute(
                &caller()?,
                &request(
                    "no-interruption",
                    current(&store)?.version,
                    ManagedAction::Update {
                        recipe: reference('2')?,
                        allow_interruption: false
                    }
                )?
            )
            .is_err()
    );
    platform.0.lock().map_err(|e| e.to_string())?.interrupt = Some((ManagedStep::Verify, false));
    manager.execute(
        &caller()?,
        &request(
            "update",
            current(&store)?.version,
            ManagedAction::Update {
                recipe: reference('2')?,
                allow_interruption: true,
            },
        )?,
    )?;
    let failed = current(&store)?;
    assert_eq!(failed.generation, 1);
    assert!(!failed.admission_open);
    assert_eq!(
        failed.current.ok_or("old setup absent")?.recipe,
        reference('1')?
    );
    manager.execute(
        &caller()?,
        &request(
            "recover-update",
            current(&store)?.version,
            ManagedAction::Recover {},
        )?,
    )?;
    assert_eq!(current(&store)?.generation, 2);
    manager.execute(
        &caller()?,
        &request(
            "delete-owned",
            current(&store)?.version,
            ManagedAction::Preserve {
                disposition: DataDisposition::DeleteOnRemoval,
            },
        )?,
    )?;
    manager.execute(
        &caller()?,
        &request("remove", current(&store)?.version, ManagedAction::Remove {})?,
    )?;
    assert!(current(&store)?.removed);
    let physical = platform.0.lock().map_err(|e| e.to_string())?;
    for resource in resources {
        assert_eq!(
            physical.volumes.contains(&resource.identity),
            resource.ownership != ResourceOwnership::Owned
        );
    }
    assert_eq!(
        physical.content,
        "source edits, output, documentation and database"
    );
    drop(physical);
    store.verify_managed_integrity()?;
    assert_eq!(
        store.managed_installations(None, PageSize::new(1)?)?.len(),
        1
    );
    Ok(())
}

#[test]
fn every_platform_boundary_recovers_the_original_intent_without_duplicate_service() -> Result {
    for step in [
        ManagedStep::Prerequisites,
        ManagedStep::PrepareStorage,
        ManagedStep::Configure,
        ManagedStep::StartService,
        ManagedStep::Verify,
    ] {
        for after in [false, true] {
            let root = tempfile::tempdir()?;
            let store = Arc::new(RedbStore::open(root.path())?);
            let platform = Arc::new(Platform::default());
            platform.0.lock().map_err(|e| e.to_string())?.interrupt = Some((step, after));
            let apply = request(
                "install",
                0,
                ManagedAction::Apply {
                    recipe: reference('1')?,
                },
            )?;
            let manager = owner(store.clone(), platform.clone());
            let receipt = manager.execute(&caller()?, &apply)?;
            let interrupted = current(&store)?;
            assert!(interrupted.pending.is_some());
            assert!(!interrupted.admission_open);
            assert!(
                store
                    .advance_managed_change(
                        &apply.installation,
                        &"f".repeat(64),
                        0,
                        observation(true)
                    )
                    .is_err()
            );
            drop(manager);
            drop(store);
            let reopened = Arc::new(RedbStore::open(root.path())?);
            let manager = owner(reopened.clone(), platform.clone());
            manager.recover_startup()?;
            assert!(
                current(&reopened)?.pending.is_some(),
                "uncertain work requires explicit recovery"
            );
            manager.execute(
                &caller()?,
                &request(
                    "recover",
                    current(&reopened)?.version,
                    ManagedAction::Recover {},
                )?,
            )?;
            assert_eq!(manager.execute(&caller()?, &apply)?, receipt);
            assert!(current(&reopened)?.admission_open);
            assert_eq!(platform.0.lock().map_err(|e| e.to_string())?.starts, 1);
        }
    }
    Ok(())
}

struct FailCommit {
    point: FaultPoint,
    index: usize,
    calls: AtomicUsize,
}
impl FaultInjector for FailCommit {
    fn check(&self, point: FaultPoint) -> std::result::Result<(), PersistenceError> {
        if self.point == point && self.calls.fetch_add(1, Ordering::SeqCst) == self.index {
            Err(injected_failure(point))
        } else {
            Ok(())
        }
    }
}

#[test]
fn intent_and_each_result_commit_survive_before_after_commit_faults() -> Result {
    for point in [
        FaultPoint::BeforeManagedCommit,
        FaultPoint::AfterManagedCommit,
    ] {
        for index in 0..6 {
            let root = tempfile::tempdir()?;
            let fault = Arc::new(FailCommit {
                point,
                index,
                calls: AtomicUsize::new(0),
            });
            let store = Arc::new(RedbStore::open_with_config(
                RedbStoreConfig::new(root.path()).with_fault_injector(fault),
            )?);
            let platform = Arc::new(Platform::default());
            let apply = request(
                "install",
                0,
                ManagedAction::Apply {
                    recipe: reference('1')?,
                },
            )?;
            let manager = owner(store.clone(), platform.clone());
            assert!(manager.execute(&caller()?, &apply).is_err());
            drop(manager);
            drop(store);
            let store = Arc::new(RedbStore::open(root.path())?);
            let manager = owner(store.clone(), platform.clone());
            manager.recover_startup()?;
            manager.execute(&caller()?, &apply)?;
            assert!(current(&store)?.admission_open);
            assert_eq!(platform.0.lock().map_err(|e| e.to_string())?.starts, 1);
            store.verify_managed_integrity()?;
        }
    }
    Ok(())
}

#[test]
fn denied_authority_never_commits_intent_or_touches_platform() -> Result {
    let root = tempfile::tempdir()?;
    let store = Arc::new(RedbStore::open(root.path())?);
    let platform = Arc::new(Platform::default());
    let manager = ManagedResources::new(
        store.clone(),
        platform.clone(),
        Arc::new(Authority(false)),
        Arc::new(milkdrift_runtime::SystemBoundaryClock),
    );
    assert!(matches!(
        manager.execute(
            &caller()?,
            &request(
                "install",
                0,
                ManagedAction::Apply {
                    recipe: reference('1')?
                }
            )?
        ),
        Err(ManagedError::Unauthorized)
    ));
    assert!(
        store
            .managed_installations(None, PageSize::new(1)?)?
            .is_empty()
    );
    assert_eq!(platform.0.lock().map_err(|e| e.to_string())?.calls, 0);
    Ok(())
}

#[test]
fn recovery_authorizes_pending_replacement_instead_of_old_approved_resources() -> Result {
    struct OldSetupOnly;
    impl AuthorityEvaluator for OldSetupOnly {
        fn evaluate(
            &self,
            request: &AuthorityRequest,
        ) -> std::result::Result<AuthorityDecisionSnapshot, AuthorityError> {
            Authority(
                !request
                    .resources
                    .network_destinations
                    .contains("new.example"),
            )
            .evaluate(request)
        }
    }
    let root = tempfile::tempdir()?;
    let store = Arc::new(RedbStore::open(root.path())?);
    let platform = Arc::new(Platform::default());
    let manager = owner(store.clone(), platform.clone());
    manager.execute(
        &caller()?,
        &request(
            "install",
            0,
            ManagedAction::Apply {
                recipe: reference('1')?,
            },
        )?,
    )?;
    platform.0.lock().map_err(|e| e.to_string())?.interrupt = Some((ManagedStep::Configure, true));
    manager.execute(
        &caller()?,
        &request(
            "update",
            current(&store)?.version,
            ManagedAction::Update {
                recipe: reference('2')?,
                allow_interruption: true,
            },
        )?,
    )?;
    let pending = current(&store)?;
    assert_eq!(
        pending.current.as_ref().ok_or("old setup missing")?.recipe,
        reference('1')?
    );
    let calls = platform.0.lock().map_err(|e| e.to_string())?.calls;
    let limited = ManagedResources::new(
        store.clone(),
        platform.clone(),
        Arc::new(OldSetupOnly),
        Arc::new(milkdrift_runtime::SystemBoundaryClock),
    );
    limited.execute(
        &caller()?,
        &request("inspect-old", 0, ManagedAction::Inspect {})?,
    )?;
    let recover = request("recover", pending.version, ManagedAction::Recover {})?;
    assert!(matches!(
        limited.execute(&caller()?, &recover),
        Err(ManagedError::Unauthorized)
    ));
    assert_eq!(current(&store)?, pending);
    assert_eq!(platform.0.lock().map_err(|e| e.to_string())?.calls, calls);
    assert!(
        store
            .managed_receipt(caller()?.actor.as_str(), &recover.command)?
            .is_none()
    );
    manager.execute(&caller()?, &recover)?;
    assert_eq!(
        current(&store)?.current.ok_or("new setup missing")?.recipe,
        reference('2')?
    );
    assert!(current(&store)?.admission_open);
    Ok(())
}

mod support;
#[test]
fn lifecycle_adapter_passes_shared_conformance_through_entered_serving_context() -> Result {
    use milkdrift_capability_host::{StoreInvocationDataAccess, conformance::*, managed::*};
    use milkdrift_persistence::ArtifactReadAuthority;
    run_adapter_conformance(|_| -> Result<AdapterConformanceCase> {
        let directory = tempfile::tempdir()?;
        let store = Arc::new(RedbStore::open(directory.path())?);
        let platform = Arc::new(Platform::default());
        let manager = Arc::new(owner(store.clone(), platform));
        let descriptor = managed_lifecycle_descriptor()?;
        let (invocation, context, accepted) = support::entered(
            &store,
            &descriptor,
            "lifecycle-conformance",
            "request",
            serde_json::to_value(request(
                "prepare",
                0,
                ManagedAction::Prepare {
                    recipe: reference('1')?,
                },
            )?)?,
        )?;
        let data = Arc::new(StoreInvocationDataAccess::new(
            store,
            directory.path().join("temporary"),
            ArtifactReadAuthority::PublicOnly,
        )?);
        Ok(AdapterConformanceCase::new(
            Arc::new(ManagedLifecycleAdapter::new(manager, data)),
            descriptor,
            invocation,
            context,
            AdapterConformanceExpectations {
                start_replay: StartReplayExpectation::Idempotent,
                available_while_draining: true,
                available_after_shutdown: true,
                unknown_cancellation: UnknownCancellationExpectation::NegativeAcknowledgement,
            },
        )?
        .with_serving_allowance(accepted.request.limits.clone())
        .with_keepalive(directory))
    })?;
    Ok(())
}

#[test]
fn offline_backup_restore_preserves_inventory_and_refuses_duplicate_activation() -> Result {
    use milkdrift_redb_store::offline::{BackupProducer, InspectionFamily, OfflineStore};
    let source = tempfile::tempdir()?;
    let scratch = tempfile::tempdir()?;
    let copies = tempfile::tempdir()?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        for p in [source.path(), scratch.path(), copies.path()] {
            std::fs::set_permissions(p, std::fs::Permissions::from_mode(0o700))?;
        }
    }
    let store = Arc::new(RedbStore::open(source.path())?);
    let manager = owner(store.clone(), Arc::new(Platform::default()));
    manager.execute(
        &caller()?,
        &request(
            "apply",
            0,
            ManagedAction::Apply {
                recipe: reference('1')?,
            },
        )?,
    )?;
    let expected = current(&store)?.view();
    drop(manager);
    drop(store);
    let offline = OfflineStore::open(source.path(), scratch.path())?;
    let page = offline.inspect(InspectionFamily::Managed, PageSize::new(1)?, None, false)?;
    assert_eq!(page.records.len(), 1);
    let backup = copies.path().join("backup");
    let manifest = offline.backup(
        &backup,
        scratch.path(),
        BackupProducer {
            version: "test".to_owned(),
            binary_digest: "0".repeat(64),
            source_revision: "test".to_owned(),
        },
    )?;
    assert_eq!(manifest.integrity_failures, 0);
    let restored = copies.path().join("restored");
    OfflineStore::restore(&backup, &restored, scratch.path())?;
    assert!(
        RedbStore::open(&restored).is_err(),
        "clone must not become a second platform owner"
    );
    let clone = OfflineStore::open(&restored, scratch.path())?;
    assert_eq!(
        serde_json::to_value(
            clone
                .inspect(InspectionFamily::Managed, PageSize::new(1)?, None, false)?
                .records
        )?,
        serde_json::to_value(&page.records)?
    );
    assert!(
        serde_json::to_string(&page.records)?
            .contains(&expected.recipe.ok_or("recipe missing")?.digest)
    );
    Ok(())
}

#[test]
fn interrupted_capability_publication_rebuilds_without_repeating_platform_changes() -> Result {
    struct Publisher(std::sync::atomic::AtomicBool, AtomicUsize);
    impl milkdrift_capability_host::managed::ManagedGenerationPublisher for Publisher {
        fn synchronize(
            &self,
            record: &InstallationRecord,
        ) -> std::result::Result<(), ManagedError> {
            if record.admission_open {
                if self.0.load(Ordering::SeqCst) {
                    return Err(failure("publication interrupted"));
                }
                self.1.fetch_add(1, Ordering::SeqCst);
            }
            Ok(())
        }
    }
    let root = tempfile::tempdir()?;
    let store = Arc::new(RedbStore::open(root.path())?);
    let platform = Arc::new(Platform::default());
    let publisher = Arc::new(Publisher(
        std::sync::atomic::AtomicBool::new(true),
        AtomicUsize::new(0),
    ));
    let erased: Arc<dyn milkdrift_capability_host::managed::ManagedGenerationPublisher> =
        publisher.clone();
    let manager = owner(store.clone(), platform.clone()).with_publisher(Arc::downgrade(&erased));
    let apply = request(
        "apply",
        0,
        ManagedAction::Apply {
            recipe: reference('1')?,
        },
    )?;
    assert!(manager.execute(&caller()?, &apply).is_err());
    let verified = current(&store)?;
    assert!(verified.admission_open && verified.pending.is_none());
    assert_eq!(publisher.1.load(Ordering::SeqCst), 0);
    let calls = platform.0.lock().map_err(|e| e.to_string())?.calls;
    drop(manager);
    publisher.0.store(false, Ordering::SeqCst);
    let reopened = owner(store.clone(), platform.clone()).with_publisher(Arc::downgrade(&erased));
    reopened.recover_startup()?;
    assert_eq!(publisher.1.load(Ordering::SeqCst), 1);
    assert_eq!(platform.0.lock().map_err(|e| e.to_string())?.calls, calls);
    assert_eq!(current(&store)?, verified);
    assert_eq!(reopened.execute(&caller()?, &apply)?.state, "pending");
    Ok(())
}
