//! Recover active work before allowing a request or worker to start new execution.
//!
//! Configuration becomes concrete storage, authority, runtime, adapters, and workers here.
//! Normal readiness follows recovery and registration; explicit recovery composition returns
//! only authenticated controls with execution permanently closed. An error returns to the
//! launcher with admission closed. Controller lifecycle installation remains separately gated.
use super::{
    DaemonHost, HostError, LEGACY_SIDECAR_FILE, Owner, OwnerRequest, PeerRuntime, WorkflowServices,
    build_peer_runtime, capabilities, clock::DaemonClockSource, clock::DurableClock,
    clock::StoreClockAdapter, clock::SystemDaemonClock, health::Lifecycle, health::SharedHealth,
    queue::OwnerQueue,
};
use crate::{
    auth::AuthRegistry, config::AdapterConfig, config::DaemonPlan, config::DaemonPlanParts,
    config::PeerHostConfig, config::RuntimeHostConfig, config::ShutdownConfig, config::StoragePlan,
};
use milkdrift_authority::{ActorRef, GrantSetEvaluator, PolicyId};
use milkdrift_capability::ErrorClass;
use milkdrift_capability_host::{
    CapabilityHost, CapabilitySelectionPolicy, EffectWorkerConfig, EffectWorkerHost, HostConfig,
    StoreInvocationDataAccess,
};
use milkdrift_control::ControlService;
use milkdrift_persistence::{
    ApplicationCommandStore, ApplicationPageQuery, ArtifactReadAuthority, EvidenceId, PageSize,
    TimestampMillis, WorkerId,
};
use milkdrift_redb_store::{RedbStore, RedbStoreConfig};
use milkdrift_runtime::{
    RetryPolicy, RuntimeConfig, RuntimeService, SchedulerLimits, SequentialIdGenerator,
};
use std::{
    collections::BTreeMap, collections::BTreeSet, fs, sync::Arc, sync::Mutex, sync::Weak,
    sync::atomic::AtomicBool, sync::mpsc::SyncSender, sync::mpsc::sync_channel, thread,
    time::Duration,
};
use tracing::{info, warn};

impl DaemonHost {
    /// Starts the dedicated owner, completes recovery/adapters/workers, then returns ready.
    pub fn start(config: DaemonPlan) -> Result<Self, HostError> {
        Self::start_with_clock(config, Arc::new(SystemDaemonClock))
    }

    /// Opens authenticated recovery controls without automatic recovery or effect workers.
    ///
    /// This mode cannot become execution-ready. After prospective reconciliation, shut down
    /// this host and start normally to validate all active state before dispatch resumes.
    pub fn start_recovery(config: DaemonPlan) -> Result<Self, HostError> {
        Self::start_mode(config, Arc::new(SystemDaemonClock), true)
    }

    pub(super) fn start_with_clock(
        config: DaemonPlan,
        clock: Arc<dyn DaemonClockSource>,
    ) -> Result<Self, HostError> {
        Self::start_mode(config, clock, false)
    }

    fn start_mode(
        config: DaemonPlan,
        clock: Arc<dyn DaemonClockSource>,
        recovery_controls: bool,
    ) -> Result<Self, HostError> {
        let DaemonPlanParts {
            role,
            host_id,
            serving,
            storage,
            authentication,
            runtime,
            adapters,
            mut peers,
            shutdown,
        } = config.into_parts();
        if recovery_controls {
            if role == milkdrift_control_protocol::HostRole::ExecutionOnly {
                return Err(HostError::Configuration(
                    "recovery controls require the workflow_enabled role".to_owned(),
                ));
            }
            peers = PeerHostConfig::Disabled;
        }
        let auth = AuthRegistry::from_plan(&authentication)
            .map_err(|error| HostError::Configuration(error.to_string()))?;
        let queue_capacity = runtime.request_queue;
        let shutdown_deadline = Duration::from_millis(shutdown.deadline_ms);
        let queue_size = usize::try_from(queue_capacity)
            .map_err(|_| HostError::Configuration("request queue exceeds platform".to_owned()))?;
        let (sender, receiver) = sync_channel(queue_size);
        let sender = Arc::new(sender);
        let (startup_sender, startup_receiver) = sync_channel(1);
        let health = Arc::new(SharedHealth::new(
            queue_capacity,
            &storage,
            if recovery_controls {
                None
            } else {
                Some(&serving)
            },
            role,
        ));
        let thread_health = health.clone();
        let thread_auth = auth.clone();
        let owner_sender = Arc::downgrade(&sender);
        let owner_clock = clock.clone();
        let maintenance = Duration::from_millis(runtime.maintenance_interval_ms);
        let owner_plan = OwnerPlan {
            role,
            host_id,
            serving,
            recovery_controls,
            storage,
            runtime,
            adapters,
            peers,
            shutdown,
        };
        let join = thread::Builder::new()
            .name("milkdrift-runtime-owner".to_owned())
            .spawn(move || {
                info!(phase = "startup", "runtime owner starting");
                let (mut owner, startup) = match Owner::open(
                    owner_plan,
                    thread_auth,
                    thread_health.clone(),
                    owner_sender,
                    owner_clock,
                ) {
                    Ok(opened) => opened,
                    Err(failure) => {
                        warn!(
                            phase = "startup",
                            outcome = "failed",
                            code = "initialization",
                            "runtime owner failed before readiness"
                        );
                        thread_health.failure("daemon startup initialization failed");
                        thread_health.set_lifecycle(Lifecycle::Failed);
                        let _ = startup_sender.send(Err(failure));
                        return;
                    }
                };
                thread_health.set_lifecycle(if recovery_controls {
                    Lifecycle::Recovery
                } else {
                    Lifecycle::Ready
                });
                let _ = startup_sender.send(Ok(startup));
                info!(
                    recovery_controls,
                    "runtime owner accepting control requests"
                );
                owner.run(receiver, maintenance, &thread_health);
            })
            .map_err(|error| HostError::Startup(error.to_string()))?;
        match startup_receiver.recv() {
            Ok(Ok(peer_runtime)) => Ok(Self {
                sender,
                health,
                auth,
                mutating_admission: Arc::new(AtomicBool::new(true)),
                join: Arc::new(Mutex::new(Some(join))),
                shutdown_deadline,
                peer_service: peer_runtime.service,
                peer_registries: Arc::new(peer_runtime.registries),
                revoked_peers: Arc::new(Mutex::new(BTreeSet::new())),
                clock: peer_runtime.clock,
            }),
            Ok(Err(error)) => {
                let _ = join.join();
                Err(HostError::Startup(error))
            }
            Err(_) => {
                let _ = join.join();
                Err(HostError::Startup(
                    "runtime owner ended before startup result".to_owned(),
                ))
            }
        }
    }
}

pub(super) struct OwnerPlan {
    role: milkdrift_control_protocol::HostRole,
    host_id: String,
    serving: crate::config::ServingHostConfig,
    recovery_controls: bool,
    storage: StoragePlan,
    runtime: RuntimeHostConfig,
    adapters: AdapterConfig,
    peers: PeerHostConfig,
    shutdown: ShutdownConfig,
}

/// Startup has no dispatched work. Keep each created owner alive until its idle workers
/// are joined on failure; successful composition transfers all three owners together.
struct StartupCleanup {
    host: Option<CapabilityHost>,
    service: Option<Arc<milkdrift_capability_host::PeerService>>,
    effects: Option<EffectWorkerHost>,
    deadline: Duration,
}

impl Drop for StartupCleanup {
    fn drop(&mut self) {
        let started = std::time::Instant::now();
        if let Some(service) = &self.service {
            let report = service.shutdown_workers(self.deadline);
            if !report.clean {
                warn!(
                    phase = "startup",
                    retained_workers = report.retained_workers,
                    "failed startup could not join every serving worker"
                );
            }
        }
        if let Some(effects) = &self.effects {
            let _ = effects.shutdown(
                milkdrift_capability_host::EffectShutdownMode::Retain,
                self.deadline.saturating_sub(started.elapsed()),
            );
        }
        if let Some(host) = &self.host {
            let _ =
                host.shutdown_with_deadline(false, self.deadline.saturating_sub(started.elapsed()));
        }
    }
}

impl Owner {
    pub(super) fn open(
        plan: OwnerPlan,
        auth: AuthRegistry,
        health: Arc<SharedHealth>,
        sender: Weak<SyncSender<OwnerRequest>>,
        clock_source: Arc<dyn DaemonClockSource>,
    ) -> Result<(Self, PeerRuntime), String> {
        let OwnerPlan {
            role,
            host_id,
            serving,
            recovery_controls,
            storage,
            runtime: runtime_plan,
            adapters,
            peers,
            shutdown,
        } = plan;
        let host_identity = milkdrift_capability::PeerId::new(host_id.clone())
            .map_err(|error| error.to_string())?;
        let input_budget = milkdrift_workspace::WorkspaceBudget::new(
            0,
            0,
            0,
            serving.clients.maximum_uploaded_artifacts,
            milkdrift_control_protocol::MAX_INPUT_UPLOAD_BYTES as u64,
            serving.clients.maximum_uploaded_bytes,
        )
        .map_err(|error| error.to_string())?;
        fs::create_dir_all(&storage.data_root)
            .map_err(|error| format!("data root creation failed: {:?}", error.kind()))?;
        if storage.data_root.join(LEGACY_SIDECAR_FILE).exists() {
            return Err(
                "legacy control-state-v1.json is unsupported; this release refuses sidecar state instead of silently importing or ignoring idempotency truth"
                    .to_owned(),
            );
        }
        for prototype in ["peer-executions-v1", "peer-artifacts-v1"] {
            if storage.data_root.join(prototype).exists() {
                return Err(format!(
                    "prototype {prototype} storage is unsupported; this release refuses parallel peer authorities instead of partially importing them"
                ));
            }
        }
        let store = Arc::new(
            RedbStore::open_with_config(
                RedbStoreConfig::new(&storage.data_root)
                    .with_application_receipt_lifecycle(
                        storage.application_receipts.hot_receipt_bound,
                        storage.application_receipts.archive_batch_size,
                    )
                    .with_security_audit_limit(storage.security_audit_record_bound)
                    .with_clock(Arc::new(StoreClockAdapter(clock_source.clone()))),
            )
            .map_err(|error| error.to_string())?,
        );
        if role == milkdrift_control_protocol::HostRole::ExecutionOnly {
            refuse_workflow_obligations(store.as_ref())?;
        }
        milkdrift_persistence::PeerExecutionStore::bind_serving_host(
            store.as_ref(),
            &host_identity,
        )
        .map_err(|error| error.to_string())?;
        let owner_queue = OwnerQueue::new(sender, health.clone(), thread::current().id());
        let clock = DurableClock::new(owner_queue.clone(), Arc::downgrade(&store), health.clone());
        let authority = Arc::new(
            GrantSetEvaluator::new(
                PolicyId::new("daemon.authority.v1").map_err(|error| error.to_string())?,
                1,
                auth.grants(),
                auth.revocations(),
            )
            .map_err(|error| error.to_string())?,
        );
        let capability_host = CapabilityHost::new(
            HostConfig {
                max_registrations: 1_024,
                max_generations_per_capability: 16,
                max_concurrent_per_generation: runtime_plan.global_concurrency,
                observation_stale_after_ms:
                    super::capabilities::CAPABILITY_OBSERVATION_STALE_AFTER_MS,
            },
            CapabilitySelectionPolicy::priorities(BTreeMap::new()),
        )
        .map_err(|error| error.to_string())?;
        let mut startup_cleanup = StartupCleanup {
            host: Some(capability_host.clone()),
            service: None,
            effects: None,
            deadline: Duration::from_millis(shutdown.deadline_ms),
        };
        let startup_now = clock
            .now()
            .map(TimestampMillis::get)
            .map_err(|_| "daemon clock unavailable during startup".to_owned())?;
        if !recovery_controls {
            recover_input_uploads(store.as_ref(), startup_now)?;
        }
        let workflow = if role == milkdrift_control_protocol::HostRole::WorkflowEnabled {
            let scheduler = SchedulerLimits::new(
                runtime_plan.global_concurrency,
                runtime_plan.per_run_concurrency,
                runtime_plan.per_branch_concurrency,
                runtime_plan.per_capability_concurrency,
            )
            .map_err(|error| error.to_string())?;
            let retry = RetryPolicy::new(
                3,
                vec![
                    ErrorClass::RateLimit,
                    ErrorClass::Transport,
                    ErrorClass::Provider,
                ],
                250,
                30_000,
                500,
            )
            .map_err(|error| error.to_string())?;
            let runtime_config = RuntimeConfig::new(
                WorkerId::new("daemon-worker").map_err(|error| error.to_string())?,
                ActorRef::new("service:daemon-runtime").map_err(|error| error.to_string())?,
                runtime_plan.lease_duration_ms,
                runtime_plan.maximum_tick_items,
                scheduler,
                retry,
            )
            .map_err(|error| error.to_string())?;
            let runtime = Arc::new(
                RuntimeService::open_closed_with_authority(
                    store.clone(),
                    Arc::new(capability_host.clone()),
                    authority.clone(),
                    clock.runtime_adapter(),
                    Arc::new(
                        SequentialIdGenerator::new("daemon", startup_now)
                            .map_err(|error| error.to_string())?,
                    ),
                    runtime_config,
                )
                .map_err(|error| error.to_string())?,
            );
            let control = Arc::new(ControlService::new(
                store.clone(),
                runtime.clone(),
                authority.clone(),
            ));
            if matches!(
                runtime_plan.controller_activation,
                crate::config::ControllerActivation::Qualification
                    | crate::config::ControllerActivation::Enabled
            ) {
                runtime
                    .install_controller_lifecycle(control.controller_lifecycle_owner())
                    .map_err(|error| error.to_string())?;
            }
            Some(WorkflowServices { runtime, control })
        } else {
            None
        };
        if recovery_controls {
            workflow
                .as_ref()
                .ok_or_else(|| "workflow role is absent".to_owned())?
                .runtime
                .enable_recovery_controls()
                .map_err(|error| error.to_string())?;
            health.receipt_status(
                store
                    .application_receipt_status()
                    .map_err(|error| error.to_string())?,
            );
            startup_cleanup.host.take();
            return Ok((
                Self {
                    host_id: host_identity,
                    input_budget,
                    recovery_controls,
                    request_panicked: false,
                    shutdown,
                    store,
                    workflow,
                    capability_host,
                    authority,
                    effect_workers: None,
                    peer_service: None,
                    _peer_artifacts: None,
                    peer_registries: BTreeMap::new(),
                    clock: clock.clone(),
                },
                PeerRuntime {
                    service: None,
                    artifacts: None,
                    registries: BTreeMap::new(),
                    clock,
                },
            ));
        }
        let data = Arc::new(
            StoreInvocationDataAccess::new(
                store.clone(),
                storage
                    .data_root
                    .join(milkdrift_redb_store::offline::EXECUTION_DIRECTORY),
                ArtifactReadAuthority::Authorized {
                    actor: ActorRef::new("service:daemon-runtime")
                        .map_err(|error| error.to_string())?,
                    evidence: EvidenceId::new("daemon-materialization")
                        .map_err(|error| error.to_string())?,
                },
            )
            .map_err(|error| error.to_string())?,
        );
        if let Some(workflow) = &workflow {
            workflow
                .runtime
                .recover_startup_closed()
                .map_err(|error| error.to_string())?;
        }
        store
            .application_command_receipts(&ApplicationPageQuery {
                after: None,
                limit: PageSize::new(1).map_err(|error| error.to_string())?,
            })
            .map_err(|error| error.to_string())?;
        health.receipt_status(
            store
                .application_receipt_status()
                .map_err(|error| error.to_string())?,
        );
        store
            .application_layouts(&ApplicationPageQuery {
                after: None,
                limit: PageSize::new(1).map_err(|error| error.to_string())?,
            })
            .map_err(|error| error.to_string())?;
        if let Some(workflow) = &workflow {
            capabilities::register_control(
                &capability_host,
                workflow.control.clone(),
                data.clone(),
                store.clone(),
                store.clone(),
                authority.clone(),
                startup_now,
            )?;
        }
        capabilities::register_configured(
            &adapters,
            &capability_host,
            data,
            auth.resolver(),
            startup_now,
        )?;
        let peer_runtime = build_peer_runtime(
            &host_id,
            &serving,
            &auth,
            &peers,
            runtime_plan.lease_duration_ms,
            &capability_host,
            store.clone(),
            auth.resolver(),
            owner_queue,
            clock.clone(),
        )?;
        startup_cleanup.service = peer_runtime.service.clone();
        if let Some(service) = &peer_runtime.service {
            health.peer_status(
                service
                    .execution_status()
                    .map_err(|error| error.to_string())?,
            );
        }
        if let Some(workflow) = &workflow {
            let workers = EffectWorkerHost::start(
                workflow.runtime.clone(),
                capability_host.clone(),
                EffectWorkerConfig {
                    execution_threads: runtime_plan.effect_threads,
                    execution_queue: runtime_plan.effect_queue,
                    cancellation_queue: runtime_plan.cancellation_queue,
                    maximum_claim_page: runtime_plan.maximum_effect_claim,
                },
            )
            .map_err(|error| error.to_string())?;
            startup_cleanup.effects = Some(workers);
            workflow
                .runtime
                .resume_admission()
                .map_err(|error| error.to_string())?;
        }
        // This is the last fallible startup operation. Until it succeeds, serving workers
        // cannot claim and workflow workers have never been polled by the owner loop.
        if let Some(service) = &peer_runtime.service {
            service.recover(1_024).map_err(|error| error.to_string())?;
        }
        let effect_workers = startup_cleanup.effects.take();
        startup_cleanup.service.take();
        startup_cleanup.host.take();
        health.set_active_effects(0);
        Ok((
            Self {
                host_id: host_identity,
                input_budget,
                recovery_controls,
                request_panicked: false,
                shutdown,
                store,
                workflow,
                capability_host,
                authority,
                effect_workers,
                peer_service: peer_runtime.service.as_ref().map(Arc::downgrade),
                _peer_artifacts: peer_runtime.artifacts.clone(),
                peer_registries: peer_runtime.registries.clone(),
                clock,
            },
            peer_runtime,
        ))
    }
}

use milkdrift_persistence::ApplicationLayoutStore as _;
use milkdrift_persistence::RunQueryStore as _;

fn recover_input_uploads(store: &RedbStore, now: u64) -> Result<(), String> {
    let observed_at = TimestampMillis::new(now);
    let mut cursor = None;
    for _ in 0..1_024 {
        let result = store
            .recover_interrupted_client_inputs(milkdrift_persistence::OrphanCleanupRequest {
                observed_at,
                created_before: observed_at,
                limit: PageSize::new(128).map_err(|error| error.to_string())?,
                cursor,
            })
            .map_err(|error| error.to_string())?;
        cursor = result.next_cursor;
        if cursor.is_none() {
            return Ok(());
        }
    }
    Err("input upload recovery exceeded its bounded startup scan; admission remains closed; restart to continue cleanup".to_owned())
}

fn refuse_workflow_obligations(store: &RedbStore) -> Result<(), String> {
    use milkdrift_persistence::{ControllerAccountStore, RunSummaryFilter, RunSummaryPageQuery};
    let mut cursor = None;
    loop {
        let page = store
            .nonterminal_run_page(
                cursor.as_ref(),
                PageSize::new(128).map_err(|error| error.to_string())?,
            )
            .map_err(|error| error.to_string())?;
        if !page.runs.is_empty() {
            return Err("execution_only role refused: this store retains active or unresolved workflow obligations; start workflow_enabled to settle them, or use storage-admin for offline inspection".to_owned());
        }
        match page.next {
            Some(next) => cursor = Some(next),
            None => break,
        }
    }
    if !store
        .active_leases(PageSize::new(1).map_err(|error| error.to_string())?)
        .map_err(|error| error.to_string())?
        .entries
        .is_empty()
    {
        return Err("execution_only role refused: workflow execution leases remain; start workflow_enabled to recover them".to_owned());
    }
    // A terminal workflow can still retain unknown controller usage. Read its ordinary
    // durable account without constructing a runtime or changing historical evidence.
    let mut cursor = None;
    loop {
        let page = store
            .run_summaries(&RunSummaryPageQuery {
                filter: RunSummaryFilter::default(),
                cursor,
                limit: PageSize::new(128).map_err(|error| error.to_string())?,
            })
            .map_err(|error| error.to_string())?;
        for run in &page.runs {
            if let Some(binding) = store
                .controller_account_binding(&run.run)
                .map_err(|error| error.to_string())?
            {
                let account = store
                    .controller_account(&binding)
                    .map_err(|error| error.to_string())?
                    .ok_or_else(|| "controller account binding has no account".to_owned())?;
                if !account.reservations().is_empty() || account.blocked().is_some() {
                    return Err("execution_only role refused: workflow controller usage remains unresolved; start workflow_enabled to inspect and reconcile it".to_owned());
                }
            }
        }
        match page.next {
            Some(next) => cursor = Some(next),
            None => break,
        }
    }
    Ok(())
}
