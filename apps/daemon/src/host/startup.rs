//! Recover active work before allowing a request or worker to start new execution.
//!
//! Configuration becomes concrete storage, authority, runtime, adapters, and workers here.
//! The startup channel returns readiness only after recovery and registration succeed; an
//! error returns to the launcher with admission closed. Continuous-controller lifecycle
//! installation remains gated separately from the existing control service.
use super::{
    DaemonHost, HostError, LEGACY_SIDECAR_FILE, Owner, OwnerRequest, PeerRuntime,
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

    pub(super) fn start_with_clock(
        config: DaemonPlan,
        clock: Arc<dyn DaemonClockSource>,
    ) -> Result<Self, HostError> {
        let DaemonPlanParts {
            storage,
            authentication,
            runtime,
            adapters,
            peers,
            shutdown,
        } = config.into_parts();
        let auth = AuthRegistry::from_plan(&authentication)
            .map_err(|error| HostError::Configuration(error.to_string()))?;
        let queue_capacity = runtime.request_queue;
        let shutdown_deadline = Duration::from_millis(shutdown.deadline_ms);
        let queue_size = usize::try_from(queue_capacity)
            .map_err(|_| HostError::Configuration("request queue exceeds platform".to_owned()))?;
        let (sender, receiver) = sync_channel(queue_size);
        let sender = Arc::new(sender);
        let (startup_sender, startup_receiver) = sync_channel(1);
        let health = Arc::new(SharedHealth::new(queue_capacity, &storage, &peers));
        let thread_health = health.clone();
        let thread_auth = auth.clone();
        let owner_sender = Arc::downgrade(&sender);
        let owner_clock = clock.clone();
        let maintenance = Duration::from_millis(runtime.maintenance_interval_ms);
        let owner_plan = OwnerPlan {
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
                thread_health.set_lifecycle(Lifecycle::Ready);
                let _ = startup_sender.send(Ok(startup));
                info!(phase = "ready", "runtime owner ready after recovery");
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
    storage: StoragePlan,
    runtime: RuntimeHostConfig,
    adapters: AdapterConfig,
    peers: PeerHostConfig,
    shutdown: ShutdownConfig,
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
            storage,
            runtime: runtime_plan,
            adapters,
            peers,
            shutdown,
        } = plan;
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
        let startup_now = clock
            .now()
            .map(TimestampMillis::get)
            .map_err(|_| "daemon clock unavailable during startup".to_owned())?;
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
        let data = Arc::new(
            StoreInvocationDataAccess::new(
                store.clone(),
                storage.data_root.join("execution"),
                ArtifactReadAuthority::Authorized {
                    actor: ActorRef::new("service:daemon-runtime")
                        .map_err(|error| error.to_string())?,
                    evidence: EvidenceId::new("daemon-materialization")
                        .map_err(|error| error.to_string())?,
                },
            )
            .map_err(|error| error.to_string())?,
        );
        runtime
            .recover_startup_closed()
            .map_err(|error| error.to_string())?;
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
        capabilities::register_control(
            &capability_host,
            control.clone(),
            data.clone(),
            startup_now,
        )?;
        capabilities::register_configured(
            &adapters,
            &capability_host,
            data,
            auth.resolver(),
            startup_now,
        )?;
        let peer_runtime = build_peer_runtime(
            &peers,
            runtime_plan.lease_duration_ms,
            &capability_host,
            store.clone(),
            auth.resolver(),
            owner_queue,
            clock.clone(),
        )?;
        if let Some(service) = &peer_runtime.service {
            service.recover(1_024).map_err(|error| error.to_string())?;
            health.peer_status(
                service
                    .execution_status()
                    .map_err(|error| error.to_string())?,
            );
        }
        let effect_workers = EffectWorkerHost::start(
            runtime.clone(),
            capability_host.clone(),
            EffectWorkerConfig {
                execution_threads: runtime_plan.effect_threads,
                execution_queue: runtime_plan.effect_queue,
                cancellation_queue: runtime_plan.cancellation_queue,
                maximum_claim_page: runtime_plan.maximum_effect_claim,
            },
        )
        .map_err(|error| error.to_string())?;
        runtime
            .resume_admission()
            .map_err(|error| error.to_string())?;
        health.set_active_effects(0);
        Ok((
            Self {
                request_panicked: false,
                shutdown,
                store,
                runtime,
                control,
                capability_host,
                authority,
                effect_workers: Some(effect_workers),
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
