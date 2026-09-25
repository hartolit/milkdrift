//! These counter tests exercise the public managed owner and real durable store, not OS isolation.
use super::*;
use milkdrift_authority::ProtectedEffectPolicy;
use milkdrift_persistence::{ArtifactPublicationId, ArtifactStore, BeginArtifactPublication};
use milkdrift_workspace::{
    ArtifactId, ArtifactMetadata, ArtifactProvenance, ArtifactReference, ArtifactRetention,
    ArtifactSensitivity, CandidateCheck, CandidateEvaluation, CausalId, CausalReference,
    ContentDigest, MediaType, RunId, WorkspaceBudget, WorkspaceUsage,
};
use std::sync::atomic::{AtomicBool, AtomicU64};

struct Clock(AtomicU64);
impl milkdrift_runtime::BoundaryClock for Clock {
    fn now(
        &self,
    ) -> std::result::Result<milkdrift_persistence::TimestampMillis, milkdrift_runtime::RuntimeError>
    {
        Ok(milkdrift_persistence::TimestampMillis::new(
            self.0.load(Ordering::SeqCst),
        ))
    }
}
#[derive(Default)]
struct ProtectedPlatform {
    base: Platform,
    revoked: AtomicBool,
    evaluations: AtomicUsize,
    revoke_before_configuration: AtomicBool,
    prepare_gate: Mutex<Option<(std::sync::mpsc::Sender<()>, std::sync::mpsc::Receiver<()>)>>,
}
fn policy() -> Result<ProtectedEffectPolicy> {
    Ok(ProtectedEffectPolicy {
        schema_version: 1,
        required_checks: vec!["fixed-check".to_owned()],
        verifier: format!("b3_{}", "a".repeat(64)),
        producer: CausalId::new("trusted:verifier")?,
        maximum_candidate_bytes: 64,
        validity_ms: 1000,
    })
}
impl ManagedPlatform for ProtectedPlatform {
    fn plan(
        &self,
        name: &ManagedName,
        recipe: &RecipeReference,
        ownership: &str,
        generation: u64,
    ) -> std::result::Result<ApprovedSetup, ManagedError> {
        let mut setup = self.base.plan(name, recipe, ownership, generation)?;
        setup.protection = Some(ProtectedDeployment {
            agreement: format!("b3_{}", "b".repeat(64)),
            policy: policy().map_err(failure)?,
            evidence: None,
        });
        Ok(setup)
    }
    fn diagnose(
        &self,
        setup: &ApprovedSetup,
        current: Option<&ApprovedSetup>,
    ) -> std::result::Result<Vec<String>, ManagedError> {
        self.base.diagnose(setup, current)
    }
    fn requirements(
        &self,
        setup: &ApprovedSetup,
    ) -> std::result::Result<CapabilityExecutionRequirements, ManagedError> {
        self.base.requirements(setup)
    }
    fn observe(
        &self,
        setup: &ApprovedSetup,
    ) -> std::result::Result<ManagedObservation, ManagedError> {
        self.base.observe(setup)
    }
    fn fence(
        &self,
        setup: &ApprovedSetup,
        usage: &ManagedUse,
    ) -> std::result::Result<QuiescenceEvidence, ManagedError> {
        self.base.fence(setup, usage)
    }
    fn check_protected_policy(&self, _: &ApprovedSetup) -> std::result::Result<(), ManagedError> {
        if self.revoked.load(Ordering::SeqCst) {
            return Err(failure("policy revoked"));
        }
        Ok(())
    }
    fn evaluate_candidate(
        &self,
        _: &ApprovedSetup,
        _: &CandidateEvaluation,
        bytes: &[u8],
    ) -> std::result::Result<Vec<CandidateCheck>, ManagedError> {
        self.evaluations.fetch_add(1, Ordering::SeqCst);
        Ok(vec![CandidateCheck {
            name: "fixed-check".to_owned(),
            passed: Some(bytes == b"repaired"),
            diagnostic: "finite trusted fixture observation".to_owned(),
        }])
    }
    fn prepare_publication(
        &self,
        setup: &ApprovedSetup,
        evidence: &CandidateEvaluation,
        bytes: &[u8],
        _: u64,
    ) -> std::result::Result<ApprovedSetup, ManagedError> {
        if let Some((ready, resume)) = self.prepare_gate.lock().map_err(failure)?.take() {
            ready.send(()).map_err(failure)?;
            resume
                .recv_timeout(std::time::Duration::from_secs(10))
                .map_err(failure)?;
        }
        let mut setup = setup.clone();
        setup
            .protection
            .as_mut()
            .ok_or_else(|| failure("protection absent"))?
            .evidence = Some(evidence.clone());
        setup.configuration =
            BoundedJson::new(serde_json::json!({"bytes": String::from_utf8_lossy(bytes)}))
                .map_err(failure)?;
        Ok(setup)
    }
    fn reconcile(
        &self,
        record: &InstallationRecord,
        step: ManagedStep,
    ) -> std::result::Result<ManagedObservation, ManagedError> {
        let change = &record
            .pending
            .as_ref()
            .ok_or_else(|| failure("intent absent"))?
            .change;
        if step == ManagedStep::StartService && !change.running {
            return self.base.observe(&change.candidate);
        }
        if step == ManagedStep::StartService {
            self.base.0.lock().map_err(failure)?.content =
                change.candidate.configuration.value()["bytes"]
                    .as_str()
                    .unwrap_or("")
                    .to_owned();
        }
        let observation = self.base.reconcile(record, step)?;
        if step == ManagedStep::RemoveConfiguration
            && self.revoke_before_configuration.load(Ordering::SeqCst)
        {
            self.revoked.store(true, Ordering::SeqCst);
        }
        Ok(observation)
    }
}
fn publish_artifact(
    store: &RedbStore,
    name: &str,
    bytes: &[u8],
) -> Result<milkdrift_capability::ArtifactReference> {
    let durable = ArtifactReference::new(
        ArtifactId::new(name)?,
        ContentDigest::for_bytes(bytes),
        MediaType::new("text/plain")?,
        bytes.len() as u64,
    );
    let metadata = ArtifactMetadata::new(
        durable.clone(),
        ArtifactSensitivity::Restricted,
        ArtifactRetention::Indefinite,
        ArtifactProvenance::new(
            CausalReference::External {
                source: CausalId::new("untrusted:worker")?,
            },
            vec![],
        )?,
    )?;
    let publication = BeginArtifactPublication::new(
        ArtifactPublicationId::new(name)?,
        RunId::new(name)?,
        metadata,
        WorkspaceBudget::new(
            0,
            0,
            0,
            1,
            (bytes.len() as u64).max(64),
            (bytes.len() as u64).max(64),
        )?,
        WorkspaceUsage::EMPTY,
    )?;
    store.begin_publication(&publication)?;
    store.write_chunk(publication.publication(), 0, bytes)?;
    store.commit_publication(publication.publication())?;
    Ok(milkdrift_capability::ArtifactReference::new(
        name,
        durable.digest().to_string(),
        Some("text/plain".to_owned()),
        Some(bytes.len() as u64),
    )?)
}
fn manager(
    store: Arc<RedbStore>,
    platform: Arc<ProtectedPlatform>,
    clock: Arc<Clock>,
) -> ManagedResources {
    ManagedResources::new(store.clone(), platform, Arc::new(Authority(true)), clock)
        .with_artifacts(store)
}
fn evaluate(
    owner: &ManagedResources,
    store: &RedbStore,
    key: &str,
    candidate: milkdrift_capability::ArtifactReference,
) -> Result<String> {
    let request = request(
        key,
        current(store)?.version,
        ManagedAction::Evaluate { candidate },
    )?;
    let response = owner.execute(&caller()?, &request)?;
    let evidence: CandidateEvaluation = serde_json::from_value(
        response
            .evaluation
            .as_ref()
            .ok_or("evaluation absent")?
            .value()
            .clone(),
    )?;
    assert!(!evidence.complete);
    let retained = store
        .managed_evaluation(&evidence.identity)?
        .ok_or("evidence absent")?;
    assert!(retained.complete);
    assert_eq!(owner.execute(&caller()?, &request)?, response);
    Ok(evidence.identity)
}
#[test]
fn failed_forged_stale_and_revoked_evidence_cannot_publish_and_replay_is_exact() -> Result {
    let directory = tempfile::tempdir()?;
    let store = Arc::new(RedbStore::open(directory.path())?);
    let platform = Arc::new(ProtectedPlatform::default());
    let clock = Arc::new(Clock(AtomicU64::new(1000)));
    let owner = manager(store.clone(), platform.clone(), clock.clone());
    owner.execute(
        &caller()?,
        &request(
            "install",
            0,
            ManagedAction::Apply {
                recipe: reference('1')?,
            },
        )?,
    )?;
    assert_eq!(platform.base.0.lock().map_err(|e| e.to_string())?.starts, 0);
    assert!(
        owner
            .execute(
                &caller()?,
                &request(
                    "raw-start",
                    current(&store)?.version,
                    ManagedAction::Start {}
                )?
            )
            .is_err()
    );
    let failed = evaluate(
        &owner,
        &store,
        "seeded",
        publish_artifact(&store, "seeded", b"anonymous-memory")?,
    )?;
    assert!(
        owner
            .execute(
                &caller()?,
                &request(
                    "failed-publish",
                    current(&store)?.version,
                    ManagedAction::Publish {
                        evaluation: failed.clone()
                    }
                )?
            )
            .is_err()
    );
    let fabricated = format!("b3_{}", "f".repeat(64));
    assert!(
        owner
            .execute(
                &caller()?,
                &request(
                    "forged",
                    current(&store)?.version,
                    ManagedAction::Publish {
                        evaluation: fabricated
                    }
                )?
            )
            .is_err()
    );
    let candidate = publish_artifact(&store, "repair", b"repaired")?;
    let repaired = evaluate(&owner, &store, "repair", candidate.clone())?;
    assert_eq!(platform.evaluations.load(Ordering::SeqCst), 2);
    platform.revoked.store(true, Ordering::SeqCst);
    assert!(
        owner
            .execute(
                &caller()?,
                &request(
                    "revoked",
                    current(&store)?.version,
                    ManagedAction::Publish {
                        evaluation: repaired.clone()
                    }
                )?
            )
            .is_err()
    );
    platform.revoked.store(false, Ordering::SeqCst);
    clock.0.store(2000, Ordering::SeqCst);
    assert!(
        owner
            .execute(
                &caller()?,
                &request(
                    "expired",
                    current(&store)?.version,
                    ManagedAction::Publish {
                        evaluation: repaired.clone()
                    }
                )?
            )
            .is_err()
    );
    assert_eq!(platform.base.0.lock().map_err(|e| e.to_string())?.starts, 0);
    let expired = store
        .managed_evaluation(&repaired)?
        .ok_or("evidence absent")?;
    clock.0.store(2001, Ordering::SeqCst);
    let repaired = evaluate(&owner, &store, "renew-evidence", candidate)?;
    let renewed = store
        .managed_evaluation(&repaired)?
        .ok_or("renewal absent")?;
    assert_eq!(renewed.subject, expired.subject);
    assert_eq!(renewed.started_at, 2001);
    assert_eq!(renewed.expires_at, 3001);
    assert_ne!(renewed.identity, expired.identity);
    assert_eq!(
        store.managed_evaluation(&expired.identity)?.as_ref(),
        Some(&expired)
    );
    assert_eq!(platform.evaluations.load(Ordering::SeqCst), 3);
    let publication = request(
        "publish",
        current(&store)?.version,
        ManagedAction::Publish {
            evaluation: repaired.clone(),
        },
    )?;
    let receipt = owner.execute(&caller()?, &publication)?;
    assert_eq!(
        platform.base.0.lock().map_err(|e| e.to_string())?.content,
        "repaired"
    );
    assert_eq!(platform.base.0.lock().map_err(|e| e.to_string())?.starts, 1);
    assert!(
        store
            .managed_evaluation(&failed)?
            .ok_or("failure lost")?
            .checks[0]
            .passed
            == Some(false)
    );
    assert!(
        owner
            .execute(
                &caller()?,
                &request(
                    "reuse",
                    current(&store)?.version,
                    publication.action.clone()
                )?
            )
            .is_err()
    );
    drop(owner);
    drop(store);
    let store = Arc::new(RedbStore::open(directory.path())?);
    let owner = manager(store.clone(), platform.clone(), clock);
    assert_eq!(owner.execute(&caller()?, &publication)?, receipt);
    assert_eq!(platform.base.0.lock().map_err(|e| e.to_string())?.starts, 1);
    assert!(
        owner
            .execute(
                &caller()?,
                &request(
                    "remove-protection",
                    current(&store)?.version,
                    ManagedAction::Update {
                        recipe: reference('2')?,
                        allow_interruption: true
                    }
                )?
            )
            .is_err()
    );
    store.verify_managed_integrity()?;
    Ok(())
}

struct SelectiveAuthority {
    allow_read: bool,
    active: AtomicBool,
}
impl AuthorityEvaluator for SelectiveAuthority {
    fn evaluate(
        &self,
        request: &AuthorityRequest,
    ) -> std::result::Result<AuthorityDecisionSnapshot, AuthorityError> {
        Authority(
            self.active.load(Ordering::SeqCst)
                && (self.allow_read
                    || request.operation != AuthorityOperation::ReadArtifactContent),
        )
        .evaluate(request)
    }
}
#[test]
fn content_authority_and_concurrent_revocation_prevent_entry() -> Result {
    let directory = tempfile::tempdir()?;
    let store = Arc::new(RedbStore::open(directory.path())?);
    let platform = Arc::new(ProtectedPlatform::default());
    let clock = Arc::new(Clock(AtomicU64::new(1000)));
    let authority = Arc::new(SelectiveAuthority {
        allow_read: true,
        active: AtomicBool::new(true),
    });
    let owner = Arc::new(
        ManagedResources::new(
            store.clone(),
            platform.clone(),
            authority.clone(),
            clock.clone(),
        )
        .with_artifacts(store.clone()),
    );
    owner.execute(
        &caller()?,
        &request(
            "install",
            0,
            ManagedAction::Apply {
                recipe: reference('1')?,
            },
        )?,
    )?;
    let candidate = publish_artifact(&store, "repair", b"repaired")?;
    let denied = ManagedResources::new(
        store.clone(),
        platform.clone(),
        Arc::new(SelectiveAuthority {
            allow_read: false,
            active: AtomicBool::new(true),
        }),
        clock,
    )
    .with_artifacts(store.clone());
    assert!(
        denied
            .execute(
                &caller()?,
                &request(
                    "denied-read",
                    current(&store)?.version,
                    ManagedAction::Evaluate {
                        candidate: candidate.clone()
                    }
                )?
            )
            .is_err()
    );
    assert_eq!(platform.evaluations.load(Ordering::SeqCst), 0);
    let evaluation = evaluate(&owner, &store, "evaluate", candidate)?;
    let publication = request(
        "publish",
        current(&store)?.version,
        ManagedAction::Publish { evaluation },
    )?;
    let (ready_send, ready_recv) = std::sync::mpsc::channel();
    let (resume_send, resume_recv) = std::sync::mpsc::channel();
    *platform.prepare_gate.lock().map_err(|e| e.to_string())? = Some((ready_send, resume_recv));
    let worker = owner.clone();
    let principal = caller()?;
    let work = std::thread::spawn(move || worker.execute(&principal, &publication));
    ready_recv.recv_timeout(std::time::Duration::from_secs(10))?;
    authority.active.store(false, Ordering::SeqCst);
    resume_send.send(())?;
    assert!(
        work.join()
            .map_err(|_| "publication thread panic")?
            .is_err()
    );
    assert_eq!(platform.base.0.lock().map_err(|e| e.to_string())?.starts, 0);
    assert!(current(&store)?.pending.is_none());
    Ok(())
}
#[test]
fn policy_change_at_final_entry_and_lost_effect_response_survive_reopen() -> Result {
    for revoke in [false, true] {
        let directory = tempfile::tempdir()?;
        let store = Arc::new(RedbStore::open(directory.path())?);
        let platform = Arc::new(ProtectedPlatform::default());
        let clock = Arc::new(Clock(AtomicU64::new(1000)));
        let owner = manager(store.clone(), platform.clone(), clock.clone());
        owner.execute(
            &caller()?,
            &request(
                "install",
                0,
                ManagedAction::Apply {
                    recipe: reference('1')?,
                },
            )?,
        )?;
        let evaluation = evaluate(
            &owner,
            &store,
            "evaluate",
            publish_artifact(&store, "repair", b"repaired")?,
        )?;
        platform
            .revoke_before_configuration
            .store(revoke, Ordering::SeqCst);
        if !revoke {
            platform.base.0.lock().map_err(|e| e.to_string())?.interrupt =
                Some((ManagedStep::StartService, true));
        }
        let publication = request(
            "publish",
            current(&store)?.version,
            ManagedAction::Publish { evaluation },
        )?;
        let receipt = owner.execute(&caller()?, &publication)?;
        assert!(matches!(
            current(&store)?.pending.ok_or("intent lost")?.phase,
            ManagedChangePhase::Uncertain { .. }
        ));
        let expected_starts = usize::from(!revoke);
        assert_eq!(
            platform.base.0.lock().map_err(|e| e.to_string())?.starts,
            expected_starts
        );
        drop(owner);
        drop(store);
        let store = Arc::new(RedbStore::open(directory.path())?);
        let owner = manager(store.clone(), platform.clone(), clock);
        assert_eq!(owner.execute(&caller()?, &publication)?, receipt);
        assert_eq!(
            platform.base.0.lock().map_err(|e| e.to_string())?.starts,
            expected_starts
        );
        if revoke {
            assert!(
                owner
                    .execute(
                        &caller()?,
                        &request(
                            "recover-revoked",
                            current(&store)?.version,
                            ManagedAction::Recover {}
                        )?
                    )
                    .is_err()
            );
        } else {
            owner.execute(
                &caller()?,
                &request(
                    "recover",
                    current(&store)?.version,
                    ManagedAction::Recover {},
                )?,
            )?;
            assert!(current(&store)?.pending.is_none());
            assert_eq!(platform.base.0.lock().map_err(|e| e.to_string())?.starts, 1);
        }
    }
    Ok(())
}

#[test]
fn evaluation_commit_loss_retains_unknown_or_exact_result_without_rerunning() -> Result {
    for point in [
        FaultPoint::BeforeManagedCommit,
        FaultPoint::AfterManagedCommit,
    ] {
        for index in 0..2 {
            let root = tempfile::tempdir()?;
            let store = Arc::new(RedbStore::open(root.path())?);
            let platform = Arc::new(ProtectedPlatform::default());
            let clock = Arc::new(Clock(AtomicU64::new(1000)));
            let owner = manager(store.clone(), platform.clone(), clock.clone());
            owner.execute(
                &caller()?,
                &request(
                    "install",
                    0,
                    ManagedAction::Apply {
                        recipe: reference('1')?,
                    },
                )?,
            )?;
            let candidate = publish_artifact(&store, "repair", b"repaired")?;
            let evaluate = request(
                "evaluate",
                current(&store)?.version,
                ManagedAction::Evaluate { candidate },
            )?;
            drop(owner);
            drop(store);
            let store = Arc::new(RedbStore::open_with_config(
                RedbStoreConfig::new(root.path()).with_fault_injector(Arc::new(FailCommit {
                    point,
                    index,
                    calls: AtomicUsize::new(0),
                })),
            )?);
            let owner = manager(store.clone(), platform.clone(), clock.clone());
            assert!(owner.execute(&caller()?, &evaluate).is_err());
            drop(owner);
            drop(store);
            let store = Arc::new(RedbStore::open(root.path())?);
            let owner = manager(store.clone(), platform.clone(), clock);
            let receipt = owner.execute(&caller()?, &evaluate)?;
            let initial: CandidateEvaluation = serde_json::from_value(
                receipt
                    .evaluation
                    .ok_or("missing evaluation")?
                    .value()
                    .clone(),
            )?;
            let retained = store
                .managed_evaluation(&initial.identity)?
                .ok_or("missing retained evaluation")?;
            let complete = matches!(
                (point, index),
                (FaultPoint::BeforeManagedCommit, 0) | (FaultPoint::AfterManagedCommit, 1)
            );
            assert_eq!(retained.complete, complete);
            assert_eq!(
                platform.evaluations.load(Ordering::SeqCst),
                usize::from(index == 1 || complete)
            );
            assert_eq!(platform.base.0.lock().map_err(|e| e.to_string())?.starts, 0);
            if !complete {
                assert!(
                    owner
                        .execute(
                            &caller()?,
                            &request(
                                "publish",
                                current(&store)?.version,
                                ManagedAction::Publish {
                                    evaluation: initial.identity
                                }
                            )?
                        )
                        .is_err()
                );
            }
        }
    }
    Ok(())
}

#[test]
fn adapter_reports_known_refusal_but_preserves_uncertainty_after_an_accepted_effect() -> Result {
    use milkdrift_capability::{ResolvedCapabilitySnapshot, TerminalStatus};
    use milkdrift_capability_host::{
        AdapterInvocation, CapabilityAdapter, StoreInvocationDataAccess,
        conformance::RecordingReporter,
        managed::{
            ManagedGenerationPublisher, ManagedLifecycleAdapter, managed_lifecycle_descriptor,
        },
    };
    use milkdrift_persistence::ArtifactReadAuthority;
    struct Publisher;
    impl ManagedGenerationPublisher for Publisher {
        fn synchronize(&self, _: &InstallationRecord) -> std::result::Result<(), ManagedError> {
            Ok(())
        }
    }
    for accepted_effect in [false, true] {
        let root = tempfile::tempdir()?;
        let store = Arc::new(RedbStore::open(root.path())?);
        let platform = Arc::new(ProtectedPlatform::default());
        let clock = Arc::new(Clock(AtomicU64::new(1000)));
        let owner = manager(store.clone(), platform.clone(), clock);
        owner.execute(
            &caller()?,
            &request(
                "apply",
                0,
                ManagedAction::Apply {
                    recipe: reference('1')?,
                },
            )?,
        )?;
        let evaluation = evaluate(
            &owner,
            &store,
            "evaluate",
            publish_artifact(
                &store,
                "candidate",
                if accepted_effect {
                    b"repaired"
                } else {
                    b"failed"
                },
            )?,
        )?;
        let publisher: Arc<dyn ManagedGenerationPublisher> = Arc::new(Publisher);
        let owner = Arc::new(owner.with_publisher(Arc::downgrade(&publisher)));
        drop(publisher); // Publication succeeds physically, then its registry projection fails.
        let request = request(
            "publish",
            current(&store)?.version,
            ManagedAction::Publish { evaluation },
        )?;
        let descriptor = managed_lifecycle_descriptor()?;
        let (invocation, context, _) = super::support::entered(
            &store,
            &descriptor,
            "publish-invocation",
            "request",
            serde_json::to_value(&request)?,
        )?;
        let snapshot =
            ResolvedCapabilitySnapshot::from_descriptor(&descriptor, invocation.operation())?;
        let data = Arc::new(StoreInvocationDataAccess::new(
            store.clone(),
            root.path().join("temporary"),
            ArtifactReadAuthority::PublicOnly,
        )?);
        let adapter = ManagedLifecycleAdapter::new(owner, data);
        let reporter = RecordingReporter::default();
        let result = adapter.execute(
            &AdapterInvocation::with_context(&snapshot, &invocation, &context),
            &reporter,
        );
        let events = reporter.events()?;
        assert_eq!(
            platform.base.0.lock().map_err(|e| e.to_string())?.starts,
            usize::from(accepted_effect)
        );
        assert_eq!(
            store
                .managed_receipt(caller()?.actor.as_str(), &request.command)?
                .is_some(),
            accepted_effect
        );
        if accepted_effect {
            assert!(result.is_err());
            assert!(
                events.is_empty(),
                "an accepted effect cannot become a no-effect rejection"
            );
        } else {
            result?;
            assert_eq!(events.len(), 1);
            let terminal = events[0]
                .kind()
                .terminal()
                .ok_or("rejection terminal missing")?;
            assert_eq!(terminal.status(), TerminalStatus::Rejected);
            assert_eq!(terminal.side_effect(), SideEffectClass::None);
            assert!(serde_json::to_string(terminal)?.contains("verification is failed"));
        }
    }
    Ok(())
}

#[test]
fn referenced_managed_request_uses_bounded_authorized_input_reading() -> Result {
    use milkdrift_capability::{InvocationValueReference, ResolvedCapabilitySnapshot};
    use milkdrift_capability_host::{
        AdapterInvocation, CapabilityAdapter, StoreInvocationDataAccess,
        managed::{ManagedLifecycleAdapter, managed_lifecycle_descriptor},
    };
    use milkdrift_persistence::{ArtifactReadAuthority, EvidenceId};
    for case in ["valid", "denied", "malformed"] {
        let root = tempfile::tempdir()?;
        let store = Arc::new(RedbStore::open(root.path())?);
        let platform = Arc::new(ProtectedPlatform::default());
        let owner = Arc::new(manager(
            store.clone(),
            platform.clone(),
            Arc::new(Clock(AtomicU64::new(1000))),
        ));
        let bytes = if case == "malformed" {
            br#"{"schema_version":3,"schema_version":3}"#.to_vec()
        } else {
            serde_json::to_vec(&request(
                "prepare",
                0,
                ManagedAction::Prepare {
                    recipe: reference('1')?,
                },
            )?)?
        };
        let reference = publish_artifact(&store, "managed-request", &bytes)?;
        let descriptor = managed_lifecycle_descriptor()?;
        let (invocation, context, _) = super::support::entered_reference(
            &store,
            &descriptor,
            "referenced-request",
            "request",
            InvocationValueReference::Artifact { reference },
        )?;
        let read_authority = if case == "denied" {
            ArtifactReadAuthority::PublicOnly
        } else {
            ArtifactReadAuthority::Authorized {
                actor: caller()?.actor,
                evidence: EvidenceId::new("approved-request-read")?,
            }
        };
        let data = Arc::new(StoreInvocationDataAccess::new(
            store,
            root.path().join("temporary"),
            read_authority,
        )?);
        let adapter = ManagedLifecycleAdapter::new(owner, data);
        let snapshot =
            ResolvedCapabilitySnapshot::from_descriptor(&descriptor, invocation.operation())?;
        let result = adapter.admission_envelope(&AdapterInvocation::with_context(
            &snapshot,
            &invocation,
            &context,
        ));
        assert_eq!(result.is_ok(), case == "valid");
        assert_eq!(platform.base.0.lock().map_err(|e| e.to_string())?.starts, 0);
    }
    Ok(())
}
