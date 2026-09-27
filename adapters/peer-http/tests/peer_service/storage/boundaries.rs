//! Independent entry deadlines and combined public/internal usage are transactional bounds.
use super::*;
use milkdrift_peer_protocol::CatalogSnapshot;

#[test]
fn entry_checks_lease_and_request_deadline_independently_at_their_boundaries() -> TestResult {
    for (lease_offset, entry_offset, accepted) in [
        (-10_i64, -10_i64, true),
        (-10, -9, false),
        (10, 0, true),
        (10, 1, false),
    ] {
        let root = tempfile::tempdir()?;
        let store = RedbStore::open(root.path())?;
        let peer = PeerId::new("peer:entry-boundary")?;
        let target = PeerId::new("host:entry-boundary")?;
        let catalog = CatalogSnapshot::new(1, 1, now() + 600_000, Vec::new())?;
        let request = request(
            &peer,
            &target,
            &descriptor()?,
            1,
            catalog.digest.clone(),
            "request:entry",
            "invocation:entry",
        )?;
        configure_store(&store, &target, &peer, &catalog.digest, 4)?;
        let execution = PeerExecutionId::new("execution:entry")?;
        admit(&store, &peer, &request, &execution, 4)?;
        let worker = WorkerId::new("worker:entry")?;
        let claimed = store.claim_peer_dispatch(&PeerDispatchClaimRequest {
            worker: &worker,
            claimed_at_unix_ms: now(),
            lease_expires_at_unix_ms: request
                .deadline_unix_ms
                .checked_add_signed(lease_offset)
                .ok_or("lease overflow")?,
        })?;
        let PeerClaimOutcome::Claimed(before) = claimed else {
            return Err("missing claim".into());
        };
        let result = store.mark_peer_entered(&PeerEntryRequest {
            owner: &before.caller,
            execution: &execution,
            worker: &worker,
            claim_generation: before.phase.claim().ok_or("claim absent")?.generation,
            relationship_generation: 1,
            authority: &before.authority,
            entered_at_unix_ms: request
                .deadline_unix_ms
                .checked_add_signed(entry_offset)
                .ok_or("entry overflow")?,
            published_invocation: None,
        });
        if accepted {
            assert!(matches!(result, Ok(PeerEntryOutcome::Entered(_))));
        } else {
            assert!(matches!(
                result,
                Err(milkdrift_persistence::PersistenceError::ImmutableConflict {
                    entity: "peer_dispatch_claim",
                    ..
                })
            ));
            assert_eq!(
                store.peer_execution(&before.caller, &execution)?,
                Some(PeerExecutionSnapshot::Hot(Box::new(before)))
            );
        }
    }
    Ok(())
}

#[test]
fn nested_terminal_usage_counts_input_bytes_and_refuses_each_excess_independently() -> TestResult {
    for (nested_bytes, process_count, accepted) in
        [(6, 1, true), (5, 1, true), (7, 1, false), (6, 2, false)]
    {
        let root = tempfile::tempdir()?;
        let store = RedbStore::open(root.path())?;
        let peer = PeerId::new("peer:usage-boundary")?;
        let target = PeerId::new("host:usage-boundary")?;
        let catalog = CatalogSnapshot::new(1, 1, now() + 600_000, Vec::new())?;
        let original = request_with_input_artifact(
            &peer,
            &target,
            &descriptor()?,
            1,
            catalog.digest.clone(),
            "request:usage",
            "invocation:usage",
            4,
        )?;
        let mut limits = original.limits.clone();
        limits.artifact_bytes = 10;
        limits.nested_invocations = Some(milkdrift_capability::InvocationCounts::new(1, 0));
        let milkdrift_peer_protocol::ServingAuthorization::Peer(mut authorization) =
            original.authorization
        else {
            return Err("peer expected".into());
        };
        authorization.limits = limits.clone();
        let request = ServingInvocationRequest::new(
            original.request_id,
            1,
            catalog.digest.clone(),
            original.selection,
            original.request,
            limits,
            original.deadline_unix_ms,
            *authorization,
        )?;
        configure_store(&store, &target, &peer, &catalog.digest, 4)?;
        let execution = PeerExecutionId::new("execution:usage")?;
        admit(&store, &peer, &request, &execution, 4)?;
        let worker = WorkerId::new("worker:usage")?;
        let claimed = claim(&store, &worker)?;
        enter(
            &store,
            &target,
            &peer,
            &execution,
            &worker,
            claimed.phase.claim().ok_or("claim absent")?.generation,
        )?;
        let owner = request.authorization.caller();
        let before = store.peer_execution(&owner, &execution)?;
        let usage = milkdrift_capability::UsageObservation::new(
            None,
            None,
            None,
            None,
            None,
            BTreeMap::new(),
        )?
        .with_nested_work(milkdrift_capability::NestedWorkUsage::new(
            milkdrift_capability::InvocationCounts::new(process_count, 0),
            nested_bytes,
        ));
        let observation = PeerObservation {
            execution: execution.clone(),
            sequence: 1,
            category: ObservationCategory::Terminal,
            event: InvocationEvent::new(
                request.request.invocation().clone(),
                1,
                InvocationEventKind::Terminal {
                    terminal: InvocationTerminal::new(
                        TerminalStatus::Success,
                        Vec::new(),
                        None,
                        Some(usage),
                        SideEffectClass::ReadOnly,
                    )?,
                },
            )?,
            observed_at_unix_ms: now(),
        };
        let result = store.append_peer_observation(&owner, &execution, &observation);
        if accepted {
            assert!(result.is_ok(), "{result:?}");
        } else {
            assert!(
                matches!(result, Err(milkdrift_persistence::PersistenceError::InvalidDocument(reason)) if reason.contains("serving allowance"))
            );
            assert_eq!(store.peer_execution(&owner, &execution)?, before);
        }
    }
    Ok(())
}
