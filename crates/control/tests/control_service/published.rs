#[path = "published/inputs.rs"]
mod inputs;
use super::*;
#[path = "published/constraints.rs"]
mod constraints;
#[path = "published/entry.rs"]
mod entry;
#[path = "published/managed.rs"]
mod managed;
#[path = "published/recovery.rs"]
mod recovery;
use milkdrift_authority::{
    AuthorityEvaluator, AuthorityExecutionProvenance, AuthorityGrant, AuthorityGrantBuilder,
    AuthorityOperation, AuthorityRequest, BoundaryTimeMillis, DecisionId, RequestedResourceFacts,
};
use milkdrift_capability_host::{PublishedWorkflowContinuation, StoreInvocationDataAccess};
use milkdrift_control::PublishedWorkflowService;
use milkdrift_persistence::{
    ArtifactReadAuthority, EvidenceId,
    published::{
        PublishedInvocationStore, PublishedMethod, PublishedMethodStore, PublishedServiceIdentity,
    },
};

fn publication_grant(actor: &str, name: &str) -> TestResult<AuthorityGrant> {
    let base = AuthorityPreset::Autonomous
        .template(
            GrantId::new(name)?,
            1,
            ActorRef::new(actor)?,
            WorkflowRunScope::Any,
            CapabilityAuthorityScope::allow_any(SideEffectClass::ReadOnly),
            AuthorityBudget {
                artifact_bytes: Some(16_777_216),
                duration_ms: Some(3_600_000),
                invocations: Some(1_000),
                units: Some(1_000_000),
                concurrency: Some(8),
                cost_minor: Some(1_000_000),
            },
        )
        .build()?;
    let mut operations = base.operations().clone();
    operations.insert(AuthorityOperation::AdministerCapabilities);
    Ok(
        AuthorityGrantBuilder::new(base.identity().clone(), 1, base.actor().clone())
            .operations(operations)
            .resources(base.resources().clone())
            .budget(base.budget())
            .build()?,
    )
}

struct Fixture {
    store: Arc<RedbStore>,
    runtime: Arc<RuntimeService>,
    host: CapabilityHost,
    published: Arc<PublishedWorkflowService>,
    control: Arc<ControlService>,
    clock: Arc<ManualClock>,
    context: ActorAuthorityContext,
    process: Arc<CountingProcessAdapter>,
    method: PublishedMethod,
    decision: milkdrift_authority::AuthorityDecisionSnapshot,
    uncertain_entry: Arc<std::sync::atomic::AtomicBool>,
}

fn fixture(directory: &std::path::Path, prefix: &str) -> TestResult<Fixture> {
    fixture_with_resources(directory, prefix, false)
}
fn fixture_with_resources(
    directory: &std::path::Path,
    prefix: &str,
    managed: bool,
) -> TestResult<Fixture> {
    fixture_with_faults(directory, prefix, managed, None)
}
fn fixture_with_faults(
    directory: &std::path::Path,
    prefix: &str,
    managed: bool,
    faults: Option<Arc<dyn milkdrift_redb_store::FaultInjector>>,
) -> TestResult<Fixture> {
    fixture_with_service_scope(directory, prefix, managed, faults, None)
}

fn fixture_with_service_scope(
    directory: &std::path::Path,
    prefix: &str,
    managed: bool,
    faults: Option<Arc<dyn milkdrift_redb_store::FaultInjector>>,
    service_scope: Option<CapabilityAuthorityScope>,
) -> TestResult<Fixture> {
    let mut config = milkdrift_redb_store::RedbStoreConfig::new(directory.join("store.redb"));
    if let Some(faults) = faults {
        config = config.with_fault_injector(faults);
    }
    let store = Arc::new(RedbStore::open_with_config(config)?);
    let caller = publication_grant("human:caller", "grant:caller")?;
    let mut service = publication_grant("service:deployment", "grant:deployment")?;
    if let Some(scope) = service_scope {
        let mut resources = service.resources().clone();
        resources.capability = scope;
        service =
            AuthorityGrantBuilder::new(service.identity().clone(), 1, service.actor().clone())
                .operations(service.operations().clone())
                .resources(resources)
                .budget(service.budget())
                .build()?;
    }
    let evaluator = Arc::new(GrantSetEvaluator::new(
        PolicyId::new("test.publication")?,
        1,
        [
            caller.clone(),
            service.clone(),
            publication_grant("human:other", "grant:other")?,
        ],
        BTreeMap::new(),
    )?);
    let clock = Arc::new(ManualClock::new(NOW));
    let host = CapabilityHost::new(
        HostConfig {
            max_registrations: 4,
            max_generations_per_capability: 2,
            max_concurrent_per_generation: 4,
            observation_stale_after_ms: 60_000,
        },
        CapabilitySelectionPolicy::priorities(BTreeMap::new()),
    )?;
    let descriptor = admission::process_descriptor()?;
    let descriptor = if managed {
        managed::install(store.as_ref(), descriptor)?
    } else {
        descriptor
    };
    let process = Arc::new(CountingProcessAdapter::default());
    let uncertain_entry = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let adapter: Arc<dyn CapabilityAdapter> = if managed {
        Arc::new(managed::ManagedProcessAdapter {
            store: store.clone(),
            process: process.clone(),
            uncertain_entry: uncertain_entry.clone(),
        })
    } else {
        process.clone()
    };
    host.register(
        descriptor.clone(),
        adapter,
        Some(CapabilityObservation::new(
            descriptor.identity().clone(),
            NOW,
            true,
            0,
            "fixture ready",
        )?),
    )?;
    let runtime = Arc::new(RuntimeService::open_closed_with_authority(
        store.clone(),
        Arc::new(host.clone()),
        evaluator.clone(),
        clock.clone(),
        Arc::new(SequentialIdGenerator::new(prefix, 1)?),
        RuntimeConfig::new(
            WorkerId::new(format!("worker:{prefix}"))?,
            ActorRef::new("system:published")?,
            30_000,
            16,
            SchedulerLimits::new(1, 1, 1, 1)?,
            RetryPolicy::new(1, vec![], 1, 1, 0)?,
        )?,
    )?);
    let control = Arc::new(ControlService::new(
        store.clone(),
        runtime.clone(),
        evaluator.clone(),
    ));
    runtime.install_controller_lifecycle(control.controller_lifecycle_owner())?;
    let (base, revision) = agreements::governed()?;
    store.put_revision(&base)?;
    store.put_revision(&revision)?;
    let operation = descriptor
        .operations()
        .values()
        .next()
        .ok_or("fixture operation absent")?
        .clone();
    let operation = milkdrift_capability::OperationContract::new(
        operation.input().clone(),
        operation.output().clone(),
        operation.streaming().clone(),
        operation.cancellation(),
        operation.idempotency(),
        SideEffectClass::ReadOnly,
        operation.features().clone(),
    )?;
    let public_descriptor = DescriptorBuilder::new(
        CapabilityId::new("method:deployment")?,
        1,
        CapabilityCategory::Tool,
        AdmissionConstraints::new(2, 2)?,
        milkdrift_capability::Locality::Local,
    )
    .operations(BTreeMap::from([(
        OperationId::new("method.invoke")?,
        operation,
    )]))
    .execution_trust(descriptor.execution_trust())
    .extensions(descriptor.extensions().clone())
    .build()?;
    let service_identity = PublishedServiceIdentity {
        actor: service.actor().clone(),
        grant: service.identity().clone(),
        grant_revision: 1,
        grant_digest: service.digest()?,
        revocation_generation: 0,
    };
    let method = PublishedMethod {
        documentation: "Run the fixed governed fixture and return only its accepted result."
            .to_owned(),
        schema_version: 1,
        descriptor: public_descriptor,
        revision: revision.id().clone(),
        agreement: revision
            .semantic()
            .agreement()
            .ok_or("agreement absent")?
            .digest()
            .to_owned(),
        service: service_identity.clone(),
        inputs: BTreeMap::new(),
        outputs: BTreeMap::new(),
        workspace_budget: WorkspaceBudget::new(128, 65_536, 1_048_576, 64, 1_048_576, 16_777_216)?,
        allowance: milkdrift_persistence::ControllerResourceBudget::new(
            0, None, 10000, 10000, 1048576, 10, 10,
        )?,
        maximum_outstanding: 2,
        maximum_depth: 4,
        maximum_duration_ms: 60_000,
    };
    let data = Arc::new(StoreInvocationDataAccess::new(
        store.clone(),
        directory.join("temporary"),
        ArtifactReadAuthority::Authorized {
            actor: service.actor().clone(),
            evidence: EvidenceId::new("evidence:published-test")?,
        },
    )?);
    let published = PublishedWorkflowService::new(
        host.clone(),
        store.clone(),
        runtime.clone(),
        evaluator.clone(),
        clock.clone(),
        data,
        BTreeMap::from([
            (
                method.descriptor.identity().clone(),
                service_identity.clone(),
            ),
            (CapabilityId::new("method:inner")?, service_identity),
        ]),
    );
    let port: Arc<dyn PublishedWorkflowContinuation> = published.clone();
    host.install_published_continuation(&port)?;
    let mut resources = RequestedResourceFacts::empty();
    resources.capability = Some(method.descriptor.identity().clone());
    resources.capability_operation = Some(OperationId::new("method.publish")?);
    let decision = evaluator.evaluate(&AuthorityRequest {
        decision: DecisionId::new("decision:publish")?,
        actor: caller.actor().clone(),
        grant: caller.identity().clone(),
        grant_revision: 1,
        grant_digest: caller.digest()?,
        revocation_generation: 0,
        operation: AuthorityOperation::AdministerCapabilities,
        resources,
        budget: AuthorityBudget::default(),
        evaluated_at: BoundaryTimeMillis::new(NOW),
        provenance: AuthorityExecutionProvenance::default(),
    })?;
    assert!(decision.is_allowed());
    published.publish(
        method.clone(),
        None,
        &decision,
        &milkdrift_persistence::IntegrityDigest::hash(&serde_json::to_vec(&method)?),
    )?;
    published.restore()?;
    runtime.initialize_startup()?;
    let context = ActorAuthorityContext::new(
        caller.actor().clone(),
        CommandAuthorityClaim::new(caller.identity().clone(), 1, caller.digest()?, 0)?,
    );
    Ok(Fixture {
        store,
        runtime,
        host,
        published,
        control,
        clock,
        context,
        process,
        method,
        decision,
        uncertain_entry,
    })
}

fn start_outer(fixture: &Fixture, name: &str) -> TestResult<RunId> {
    let base = base_revision("outer-published")?;
    let outer = base.revise(
        base.id(),
        MutationBatch::new(vec![Mutation::ReplaceNode {
            node: task_node("work", "method.invoke")?,
        }])?,
        AuthorRef::new("human:caller")?,
        "invoke the published service",
    )?;
    fixture.store.put_revision(&base)?;
    fixture.store.put_revision(&outer)?;
    let run = RunId::new(name)?;
    revision_and_lifecycle::create_and_start(
        &fixture.control,
        &fixture.runtime,
        &fixture.context,
        &run,
        &outer,
    )?;
    Ok(run)
}

#[test]
fn publication_releases_single_scheduler_slot_and_creates_one_internal_run() -> TestResult {
    let directory = TempDir::new()?;
    let fixture = fixture(directory.path(), "first")?;
    let run = start_outer(&fixture, "run:outer")?;
    runtime_tick(&fixture.runtime)?;
    let (plans, _) = fixture
        .store
        .published_local_page(None, PageSize::new(8)?)?;
    assert_eq!(
        plans.len(),
        1,
        "the outer attempt must commit its child before creating it"
    );
    let plan = &plans[0];
    assert_eq!(
        fixture.runtime.projection(&plan.child_run)?.lifecycle(),
        RunLifecycle::Uncreated
    );
    assert!(
        fixture
            .runtime
            .projection(&run)?
            .leases()
            .values()
            .all(|lease| !lease.is_active())
    );
    fixture
        .runtime
        .arrange_published_run(plan, fixture.store.as_ref())?;
    fixture
        .runtime
        .arrange_published_run(plan, fixture.store.as_ref())?;
    for _ in 0..48 {
        fixture.clock.advance(1)?;
        runtime_tick(&fixture.runtime)?;
        if fixture.runtime.projection(&run)?.lifecycle().is_completed() {
            break;
        }
    }
    assert_eq!(
        fixture.runtime.projection(&run)?.lifecycle(),
        RunLifecycle::Terminal(RunOutcome::Succeeded)
    );
    assert_eq!(fixture.process.entries(), 2);
    assert_eq!(
        fixture
            .runtime
            .history(&plan.child_run)?
            .iter()
            .filter(|event| matches!(event.kind(), RunEventKind::RunCreated { .. }))
            .count(),
        1
    );
    assert_eq!(
        fixture.store.published_invocation(&plan.source)?.as_ref(),
        Some(plan)
    );
    assert!(
        fixture
            .store
            .published_local_page(None, PageSize::new(8)?)?
            .0
            .is_empty()
    );
    fixture
        .host
        .begin_drain(&plan.capability, plan.generation)?;
    fixture
        .host
        .finish_drain(&plan.capability, plan.generation)?;
    Ok(())
}

#[test]
fn published_association_recovers_before_creation_and_after_start_with_another_worker() -> TestResult
{
    for arrange in [false, true] {
        let directory = TempDir::new()?;
        let run;
        let plan;
        {
            let fixture = fixture(directory.path(), "before-crash")?;
            run = start_outer(&fixture, "run:recovered-outer")?;
            runtime_tick(&fixture.runtime)?;
            plan = fixture
                .store
                .published_local_page(None, PageSize::new(8)?)?
                .0
                .pop()
                .ok_or("pending call absent")?;
            if arrange {
                fixture
                    .runtime
                    .arrange_published_run(&plan, fixture.store.as_ref())?;
            }
        }
        let fixture = fixture(directory.path(), "after-crash")?;
        assert!(
            fixture
                .host
                .finish_drain(&plan.capability, plan.generation)
                .is_err()
        );
        for _ in 0..48 {
            fixture.clock.advance(1)?;
            runtime_tick(&fixture.runtime)?;
            if fixture.runtime.projection(&run)?.lifecycle().is_completed() {
                break;
            }
        }
        assert_eq!(
            fixture.runtime.projection(&run)?.lifecycle(),
            RunLifecycle::Terminal(RunOutcome::Succeeded)
        );
        assert_eq!(fixture.process.entries(), 2);
        assert_eq!(
            fixture
                .runtime
                .history(&plan.child_run)?
                .iter()
                .filter(|event| matches!(event.kind(), RunEventKind::RunCreated { .. }))
                .count(),
            1
        );
        assert_eq!(
            fixture.store.published_invocation(&plan.source)?.as_ref(),
            Some(&plan)
        );
    }
    Ok(())
}

#[test]
fn retirement_preserves_accepted_work_and_definition_conflicts_are_exact() -> TestResult {
    let directory = TempDir::new()?;
    let fixture = fixture(directory.path(), "retirement")?;
    let run = start_outer(&fixture, "run:retirement-outer")?;
    runtime_tick(&fixture.runtime)?;
    let mut replacement = fixture.method.clone();
    replacement.maximum_duration_ms += 1;
    assert!(
        fixture
            .published
            .publish(
                replacement,
                None,
                &fixture.decision,
                &milkdrift_persistence::IntegrityDigest::hash(b"replacement")
            )
            .is_err()
    );
    let caller = publication_grant("human:caller", "grant:caller")?;
    let evaluator = GrantSetEvaluator::new(
        PolicyId::new("test.publication")?,
        1,
        [caller],
        BTreeMap::new(),
    )?;
    let mut request = fixture.decision.request().clone();
    request.resources.capability_operation = Some(OperationId::new("method.retire")?);
    let retirement = evaluator.evaluate(&request)?;
    fixture.published.retire(
        fixture.method.descriptor.identity(),
        1,
        1,
        &retirement,
        &milkdrift_persistence::IntegrityDigest::hash(b"retire"),
    )?;
    assert!(matches!(
        fixture
            .host
            .finish_drain(fixture.method.descriptor.identity(), 1),
        Err(milkdrift_capability_host::HostError::InFlight(1))
    ));
    assert!(
        fixture
            .store
            .published_method(fixture.method.descriptor.identity(), 1)?
            .ok_or("publication absent")?
            .retired
    );
    for _ in 0..48 {
        fixture.clock.advance(1)?;
        runtime_tick(&fixture.runtime)?;
        if fixture.runtime.projection(&run)?.lifecycle().is_completed() {
            break;
        }
    }
    assert_eq!(
        fixture.runtime.projection(&run)?.lifecycle(),
        RunLifecycle::Terminal(RunOutcome::Succeeded)
    );
    fixture
        .host
        .finish_drain(fixture.method.descriptor.identity(), 1)?;
    Ok(())
}

#[test]
fn cancellation_before_internal_creation_never_enters_the_method() -> TestResult {
    let directory = TempDir::new()?;
    let fixture = fixture(directory.path(), "cancel")?;
    let run = start_outer(&fixture, "run:cancel-outer")?;
    runtime_tick(&fixture.runtime)?;
    let plan = fixture
        .store
        .published_local_page(None, PageSize::new(8)?)?
        .0
        .pop()
        .ok_or("pending call absent")?;
    let projection = fixture.runtime.projection(&run)?;
    let command = milkdrift_runtime::RunCommandDocument::new(
        milkdrift_persistence::CommandId::new("command:cancel")?,
        run.clone(),
        fixture.context.actor().clone(),
        projection.sequence(),
        TimestampMillis::new(NOW),
        Reason::new("cancel accepted public call")?,
        vec![],
        milkdrift_runtime::RunCommand::RequestCancellation,
    )?;
    fixture
        .runtime
        .handle_authorized_command(&command, fixture.context.authority())?;
    for _ in 0..16 {
        runtime_tick(&fixture.runtime)?;
    }
    assert_eq!(fixture.process.entries(), 0);
    assert_eq!(
        fixture.runtime.projection(&plan.child_run)?.lifecycle(),
        RunLifecycle::Uncreated
    );
    assert_eq!(
        fixture.runtime.projection(&run)?.lifecycle(),
        RunLifecycle::Terminal(RunOutcome::Cancelled)
    );
    assert!(
        fixture
            .store
            .published_local_page(None, PageSize::new(8)?)?
            .0
            .is_empty()
    );
    Ok(())
}

#[test]
fn one_execution_thread_progresses_internal_work_and_settles_its_allowance() -> TestResult {
    for managed in [false, true] {
        let directory = TempDir::new()?;
        let fixture = fixture_with_resources(directory.path(), "single-thread", managed)?;
        let run = start_outer(&fixture, "run:single-thread")?;
        let workers = milkdrift_capability_host::EffectWorkerHost::start(
            fixture.runtime.clone(),
            fixture.host.clone(),
            milkdrift_capability_host::EffectWorkerConfig {
                execution_threads: 1,
                execution_queue: 1,
                cancellation_queue: 1,
                maximum_claim_page: 1,
            },
        )?;
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while std::time::Instant::now() < deadline
            && !fixture.runtime.projection(&run)?.lifecycle().is_completed()
        {
            fixture.clock.advance(1)?;
            fixture.runtime.scheduler_tick()?;
            workers.poll()?;
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        assert_eq!(
            fixture.runtime.projection(&run)?.lifecycle(),
            RunLifecycle::Terminal(RunOutcome::Succeeded)
        );
        assert_eq!(fixture.process.entries(), 2);
        let shutdown = workers.shutdown(
            milkdrift_capability_host::EffectShutdownMode::Drain,
            std::time::Duration::from_secs(2),
        )?;
        assert!(shutdown.clean);
        assert!(shutdown.unresolved_invocations.is_empty());
        if managed {
            use milkdrift_persistence::managed::ManagedResourceStore;
            assert!(
                fixture
                    .store
                    .managed_installation(&milkdrift_capability::managed::ManagedName::new(
                        "publication-test"
                    )?)?
                    .ok_or("installation absent")?
                    .uses
                    .is_empty()
            );
            fixture.store.verify_managed_integrity()?;
        }
    }
    Ok(())
}
