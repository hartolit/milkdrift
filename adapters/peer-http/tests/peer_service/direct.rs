use super::support::*;
use milkdrift_authority::{AuthorityGrantBuilder, CapabilityAuthorityScopeBuilder};
use milkdrift_capability_host::ServingClientPolicy;
use milkdrift_peer_protocol::{
    DirectInvocationRequest, InvocationLookup, InvocationOrigin, ServingAuthorization,
    ServingPrincipal,
};

fn client_policy(actors: &[ActorRef]) -> TestResult<ServingClientPolicy> {
    let mut grants = Vec::new();
    for actor in actors {
        let base =
            AuthorityGrantBuilder::new(GrantId::new(format!("grant:{actor}"))?, 1, actor.clone())
                .operations(BTreeSet::from([AuthorityOperation::Inspect]))
                .build()?;
        let mut resources = base.resources().clone();
        resources.capability = CapabilityAuthorityScopeBuilder::new(SideEffectClass::ReadOnly)
            .only_capabilities(BTreeSet::from([CapabilityId::new("test-capability")?]))?
            .only_operations(BTreeSet::from([OperationId::new("test.execute")?]))?
            .build();
        grants.push(
            AuthorityGrantBuilder::new(base.identity().clone(), 1, actor.clone())
                .resources(resources)
                .operations(BTreeSet::from([
                    AuthorityOperation::Inspect,
                    AuthorityOperation::ListCapabilities,
                    AuthorityOperation::InspectCapabilityHealth,
                    AuthorityOperation::InspectProviderProfile,
                    AuthorityOperation::InvokeCapability,
                    AuthorityOperation::CancelCapability,
                ]))
                .budget(AuthorityBudget {
                    cost_minor: Some(0),
                    duration_ms: Some(30_000),
                    invocations: Some(1),
                    artifact_bytes: Some(1_048_576),
                    concurrency: Some(2),
                    ..AuthorityBudget::default()
                })
                .build()?,
        );
    }
    Ok(ServingClientPolicy {
        grants,
        revocations: BTreeMap::new(),
        execution_limits: ExecutionLimits {
            nested_invocations: None,
            artifact_bytes: 1_048_576,
            duration_ms: 30_000,
            cost_micros: 0,
            cost_currency: None,
            input_units: None,
            output_units: None,
            observations: 100,
        },
        maximum_concurrent: 2,
        maximum_requests_per_minute: 10_000,
        catalog_ttl_ms: 60_000,
    })
}

#[test]
fn client_realms_replay_archive_and_reopen_without_workflow_provenance() -> TestResult {
    let root = tempfile::tempdir()?;
    let store = Arc::new(RedbStore::open(root.path())?);
    let actors = [ActorRef::new("same-name")?, ActorRef::new("second-client")?];
    let target = PeerId::new("independent-host")?;
    let calls = Arc::new(AtomicUsize::new(0));
    let adapter = Arc::new(TerminalAdapter {
        capability: CapabilityId::new("test-capability")?,
        delay: Duration::ZERO,
        active: Arc::new(AtomicUsize::new(0)),
        maximum: Arc::new(AtomicUsize::new(0)),
        calls: calls.clone(),
        requirements: CapabilityExecutionRequirements::default(),
    });
    let (host, descriptor) = host_with_adapter(adapter)?;
    let mut config = server_config(PeerId::new("same-name")?, target.clone(), 2, 4)?;
    config.relationships.clear();
    let policy = client_policy(&actors)?;
    let service = PeerService::with_clients(
        config.clone(),
        host.clone(),
        store.clone(),
        milkdrift_capability_host::conformance::disabled_artifact_store(),
        None,
        Some(policy.clone()),
        system_peer_clock(),
    )?;
    service.recover(32)?;
    let catalog = service.client_catalog(&actors[0])?;
    let submission = DirectInvocationRequest {
        host: target.clone(),
        request_id: PeerRequestId::new("same-request")?,
        catalog_generation: catalog.generation,
        catalog_digest: catalog.digest,
        selection: ResolvedCapabilitySnapshot::from_descriptor(
            &descriptor,
            &OperationId::new("test.execute")?,
        )?,
        request: InvocationRequest::new(
            InvocationId::new("same-invocation")?,
            descriptor.identity().clone(),
            OperationId::new("test.execute")?,
            None,
            None,
            Vec::new(),
            BTreeMap::new(),
        )?,
        limits: policy.execution_limits.clone(),
        deadline_unix_ms: now().saturating_add(30_000),
    };
    let InvocationAcceptance::Accepted { execution, .. } =
        service.invoke_client(&actors[0], &submission)?
    else {
        return Err("client not accepted".into());
    };
    assert!(matches!(
        service.client_lookup(&actors[1], &submission.request_id)?,
        InvocationLookup::NotAccepted { .. }
    ));
    assert!(service.client_inspect(&actors[1], &execution).is_err());
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    loop {
        let page = service.client_observations(&actors[0], &execution, 0, 8)?;
        if page.terminal {
            break;
        }
        if std::time::Instant::now() > deadline {
            return Err("client result did not become terminal".into());
        }
        thread::sleep(Duration::from_millis(5));
    }
    let snapshot = service.client_inspect(&actors[0], &execution)?;
    let PeerExecutionSnapshot::Hot(record) = snapshot else {
        return Err("unexpected archive".into());
    };
    assert!(matches!(
        record.caller.principal,
        ServingPrincipal::Client { .. }
    ));
    assert!(matches!(
        record.request.authorization,
        ServingAuthorization::Client(_)
    ));
    assert_eq!(
        record.request.authorization.origin(),
        InvocationOrigin::Direct
    );
    assert!(record.authority.request().provenance.attempt.is_none());
    assert!(matches!(
        service.invoke_client(&actors[0], &submission)?,
        InvocationAcceptance::Accepted { replayed: true, .. }
    ));
    let mut conflict = submission.clone();
    conflict.deadline_unix_ms += 1;
    assert!(matches!(
        service.invoke_client(&actors[0], &conflict)?,
        InvocationAcceptance::Rejected { .. }
    ));
    let mut wrong_host = submission.clone();
    wrong_host.host = PeerId::new("another-host")?;
    assert!(service.invoke_client(&actors[0], &wrong_host).is_err());
    assert!(service.shutdown_workers(Duration::from_secs(5)).clean);
    drop(record);
    drop(service);
    store.archive_peer_executions(&PeerRetentionRequest {
        terminal_before_or_at: TimestampMillis::new(now()),
        archived_at: TimestampMillis::new(now()),
        limit: PageSize::new(8)?,
    })?;
    store.verify_peer_execution_integrity()?;
    drop(store);
    let store = Arc::new(RedbStore::open(root.path())?);
    let service = PeerService::with_clients(
        config,
        host.clone(),
        store.clone(),
        milkdrift_capability_host::conformance::disabled_artifact_store(),
        None,
        Some(policy),
        system_peer_clock(),
    )?;
    service.recover(32)?;
    assert!(matches!(
        service.invoke_client(&actors[0], &submission)?,
        InvocationAcceptance::Archived { .. }
    ));
    assert!(matches!(
        service.invoke_client(&actors[0], &conflict)?,
        InvocationAcceptance::Rejected { .. }
    ));
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert!(service.shutdown_workers(Duration::from_secs(5)).clean);
    host.shutdown()?;
    Ok(())
}
