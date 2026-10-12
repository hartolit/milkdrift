//! Generic lifecycle mechanics remain independent of finite task/serving admission.
use super::{Platform, Result, caller, current, owner, reference, request};
use milkdrift_authority::{
    ActorRef, AuthorityBudget, AuthorityGrantBuilder, AuthorityOperation,
    CapabilityAuthorityScopeBuilder, GrantId,
};
use milkdrift_capability::{
    BoundedJson, CapabilityObservation, InputReference, InvocationValueReference, OperationId,
    PeerId, SideEffectClass, TerminalStatus, managed::ManagedAction,
};
use milkdrift_capability_host::{
    AdapterInvocation, CapabilityAdapter, CapabilityHost, CapabilitySelectionPolicy, HostConfig,
    PeerClock, PeerServerConfig, PeerService, PeerWorkerConfig, ServingClientPolicy,
    StoreInvocationDataAccess, SystemPeerClock,
    conformance::disabled_artifact_store,
    managed::{ManagedLifecycleAdapter, managed_lifecycle_descriptor},
};
use milkdrift_peer_protocol::{
    DirectInvocationDraft, ExecutionLimits, HardLimits, HeartbeatLease, InvocationAcceptance,
    PeerRequestId, ProtocolVersionRange, SessionId,
};
use milkdrift_persistence::{
    ArtifactReadAuthority, PageSize, PeerExecutionSnapshot, managed::ManagedResourceStore,
};
use milkdrift_redb_store::RedbStore;
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
    time::{Duration, Instant},
};

fn serving(
    host: CapabilityHost,
    store: Arc<RedbStore>,
    actor: &ActorRef,
    clock: Arc<SystemPeerClock>,
) -> Result<Arc<PeerService>> {
    let descriptor = managed_lifecycle_descriptor()?;
    let base = AuthorityGrantBuilder::new(GrantId::new("grant:lifecycle")?, 1, actor.clone())
        .operations(BTreeSet::from([AuthorityOperation::Inspect]))
        .build()?;
    let mut resources = base.resources().clone();
    resources.capability = CapabilityAuthorityScopeBuilder::new(SideEffectClass::IdempotentWrite)
        .only_capabilities(BTreeSet::from([descriptor.identity().clone()]))?
        .only_operations(BTreeSet::from([OperationId::new("resource.manage")?]))?
        .build();
    let grant = AuthorityGrantBuilder::new(base.identity().clone(), 1, actor.clone())
        .resources(resources)
        .operations(BTreeSet::from([
            AuthorityOperation::Inspect,
            AuthorityOperation::ListCapabilities,
            AuthorityOperation::InspectCapabilityHealth,
            AuthorityOperation::InspectProviderProfile,
            AuthorityOperation::InvokeCapability,
        ]))
        .budget(AuthorityBudget {
            cost_minor: Some(0),
            duration_ms: Some(300_000),
            invocations: Some(1),
            artifact_bytes: Some(1_048_576),
            concurrency: Some(1),
            ..AuthorityBudget::default()
        })
        .build()?;
    Ok(PeerService::with_clients(
        PeerServerConfig {
            local_peer: PeerId::new("host-lifecycle")?,
            session: SessionId::new("session-lifecycle")?,
            versions: ProtocolVersionRange::default(),
            limits: HardLimits::default(),
            lease: HeartbeatLease {
                heartbeat_ms: 100,
                idle_timeout_ms: 1_000,
                execution_lease_ms: 5_000,
            },
            relationships: Vec::new(),
            workers: PeerWorkerConfig {
                threads: 1,
                poll_interval: Duration::from_millis(5),
                ..PeerWorkerConfig::default()
            },
        },
        host,
        store,
        disabled_artifact_store(),
        None,
        Some(ServingClientPolicy {
            grants: vec![grant],
            revocations: BTreeMap::new(),
            execution_limits: ExecutionLimits {
                nested_invocations: None,
                duration_ms: 300_000,
                artifact_bytes: 1_048_576,
                cost_micros: 0,
                cost_currency: None,
                input_units: None,
                output_units: None,
                observations: 32,
            },
            maximum_concurrent: 1,
            maximum_requests_per_minute: 10_000,
            catalog_ttl_ms: 60_000,
        }),
        clock,
    )?)
}

#[test]
fn lifecycle_without_duration_bound_refuses_serving_entry_but_keeps_direct_administration() -> Result
{
    let directory = tempfile::tempdir()?;
    let store = Arc::new(RedbStore::open(directory.path())?);
    let platform = Arc::new(Platform::default());
    let manager = Arc::new(owner(store.clone(), platform.clone()));
    let descriptor = managed_lifecycle_descriptor()?;
    let clock = Arc::new(SystemPeerClock::default());
    let host = CapabilityHost::new(
        HostConfig {
            max_registrations: 1,
            max_generations_per_capability: 1,
            max_concurrent_per_generation: 1,
            observation_stale_after_ms: 60_000,
        },
        CapabilitySelectionPolicy::priorities(BTreeMap::new()),
    )?;
    let data = Arc::new(StoreInvocationDataAccess::new(
        store.clone(),
        directory.path().join("temporary"),
        ArtifactReadAuthority::PublicOnly,
    )?);
    let adapter = Arc::new(ManagedLifecycleAdapter::new(manager.clone(), data));
    host.register(
        descriptor.clone(),
        adapter.clone(),
        Some(CapabilityObservation::new(
            descriptor.identity().clone(),
            clock.now_unix_ms()?,
            true,
            0,
            "ready",
        )?),
    )?;
    let actor = ActorRef::new("human:lifecycle")?;
    let service = serving(host.clone(), store.clone(), &actor, clock)?;
    service.recover(32)?;
    let command = request(
        "install",
        0,
        ManagedAction::Apply {
            recipe: reference('1')?,
        },
    )?;
    let submission = service
        .prepare_client_invocation(
            &actor,
            &DirectInvocationDraft {
                host: PeerId::new("host-lifecycle")?,
                request_id: PeerRequestId::new("unbounded-lifecycle")?,
                capability: descriptor.identity().clone(),
                operation: OperationId::new("resource.manage")?,
                inputs: vec![InputReference::new(
                    "request",
                    InvocationValueReference::Inline {
                        value: BoundedJson::new(serde_json::to_value(&command)?)?,
                    },
                )?],
                limits: None,
            },
        )
        .map_err(|error| format!("prepare lifecycle client invocation: {error}"))?;
    let envelope = adapter.admission_envelope(&AdapterInvocation::new(
        &submission.selection,
        &submission.request,
    ))?;
    assert!(envelope.execution_duration_ms().is_unknown());
    assert!(!submission.limits.permits_prepared(&envelope, 0));
    let InvocationAcceptance::Accepted { execution, .. } = service
        .invoke_client(&actor, &submission)
        .map_err(|error| format!("submit lifecycle client invocation: {error}"))?
    else {
        return Err("valid lifecycle request was not accepted for preparation".into());
    };
    let deadline = Instant::now() + Duration::from_secs(5);
    let terminal = loop {
        let page = service
            .client_observations(&actor, &execution, 0, 8)
            .map_err(|error| format!("observe lifecycle client invocation: {error}"))?;
        if let Some(terminal) = page
            .observations
            .last()
            .and_then(|item| item.event.kind().terminal())
        {
            break terminal.clone();
        }
        if Instant::now() >= deadline {
            return Err("bounded admission did not settle".into());
        }
        std::thread::sleep(Duration::from_millis(5));
    };
    assert_eq!(terminal.status(), TerminalStatus::Failure);
    assert!(terminal.failure().is_some_and(|failure| {
        failure
            .message()
            .contains("preparation refused before adapter entry")
    }));
    assert_eq!(terminal.side_effect(), SideEffectClass::None);
    let PeerExecutionSnapshot::Hot(record) = service
        .client_inspect(&actor, &execution)
        .map_err(|error| format!("inspect lifecycle client invocation: {error}"))?
    else {
        return Err("fresh refusal was unexpectedly archived".into());
    };
    assert!(record.phase.entry_evidence().is_none());
    assert!(
        store
            .managed_installations(None, PageSize::new(1)?)?
            .is_empty()
    );
    assert!(
        store
            .managed_receipt(actor.as_str(), &command.command)?
            .is_none()
    );
    assert_eq!(
        platform.0.lock().map_err(|error| error.to_string())?.calls,
        0
    );
    assert!(service.shutdown_workers(Duration::from_secs(5)).clean);
    host.shutdown()?;

    let receipt = manager.execute(&caller()?, &command)?;
    assert_eq!(manager.execute(&caller()?, &command)?, receipt);
    assert!(current(&store)?.admission_open);
    assert_eq!(
        platform.0.lock().map_err(|error| error.to_string())?.starts,
        1
    );
    store.verify_managed_integrity()?;
    Ok(())
}
