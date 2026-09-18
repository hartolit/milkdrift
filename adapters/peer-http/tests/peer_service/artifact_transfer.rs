//! Core peer artifact negotiation, resumability, and verification behavior.

use super::support::*;

#[test]
fn request_bound_input_staging_precedes_acceptance_and_preserves_foreign_causes() -> TestResult {
    let root = tempfile::tempdir()?;
    let core = Arc::new(RedbStore::open(root.path())?);
    let peer = PeerId::new("input-origin")?;
    let target = PeerId::new("input-host")?;
    let calls = Arc::new(AtomicUsize::new(0));
    let (host, descriptor) = host_with_adapter(Arc::new(TerminalAdapter {
        capability: CapabilityId::new("test-capability")?,
        delay: Duration::ZERO,
        active: Arc::new(AtomicUsize::new(0)),
        maximum: Arc::new(AtomicUsize::new(0)),
        calls: calls.clone(),
        requirements: CapabilityExecutionRequirements::default(),
    }))?;
    let clock = system_peer_clock();
    let artifacts = Arc::new(CorePeerArtifactStore::new(
        core.clone(),
        8,
        8,
        clock.clone(),
    )?);
    let service = PeerService::new_with_artifacts(
        server_config(peer.clone(), target.clone(), 1, 4)?,
        host,
        core.clone(),
        artifacts.clone(),
        clock,
    )?;
    service.recover(1024)?;
    let catalog = service.catalog(&peer)?;
    let base = request(
        &peer,
        &target,
        &descriptor,
        catalog.generation,
        catalog.digest,
        "staged-input",
        "staged-call",
    )?;
    let bytes = b"contents";
    let mut offer = input_offer(base, &peer, "staged-source", bytes)?;
    let consuming = match &offer.binding {
        milkdrift_peer_protocol::ArtifactTransferBinding::Input { request } => {
            request.as_ref().clone()
        }
        _ => return Err("input binding missing".into()),
    };
    assert!(
        core.peer_execution_by_request(
            &milkdrift_peer_protocol::ServingCaller::peer(&target, &peer),
            &consuming.request_id
        )?
        .is_none()
    );
    assert!(matches!(
        service.negotiate_artifact(&peer, &offer)?,
        ArtifactTransferDecision::Transfer { next_offset: 0, .. }
    ));
    assert!(core.metadata(offer.artifact.artifact())?.is_none());
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    assert!(
        service
            .negotiate_artifact(&PeerId::new("another-peer")?, &offer)
            .is_err()
    );
    assert_eq!(
        service.write_artifact_chunk(
            &peer,
            &ArtifactChunk {
                transfer: offer.transfer.clone(),
                offset: 0,
                bytes: bytes.to_vec(),
                final_chunk: true
            }
        )?,
        ArtifactTransferDecision::AlreadyPresent
    );
    let metadata = core
        .metadata(offer.artifact.artifact())?
        .ok_or("input was not committed")?;
    assert_eq!(
        metadata.provenance().producer(),
        &CausalReference::PeerClaim {
            peer: peer.clone(),
            reference: Box::new(offer.provenance.producer().clone())
        }
    );
    assert!(
        matches!(&metadata.provenance().causes()[0], CausalReference::PeerClaim { reference, .. } if matches!(reference.as_ref(), CausalReference::RunInput { .. }))
    );
    // Exact replay remains available even when this peer has consumed its entire byte quota.
    assert_eq!(
        service.negotiate_artifact(&peer, &offer)?,
        ArtifactTransferDecision::AlreadyPresent
    );
    assert!(matches!(
        service.invoke(&peer, consuming.clone())?,
        InvocationAcceptance::Accepted { .. }
    ));
    let deadline = std::time::Instant::now() + Duration::from_secs(2);
    while calls.load(Ordering::SeqCst) == 0 && std::time::Instant::now() < deadline {
        thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    let second = input_offer(consuming, &peer, "over-quota-source", b"x")?;
    assert!(service.negotiate_artifact(&peer, &second).is_err());
    assert!(core.metadata(second.artifact.artifact())?.is_none());
    offer.artifact = ArtifactReference::new(
        ArtifactId::new("forged-input")?,
        ContentDigest::for_bytes(bytes),
        MediaType::new("text/plain")?,
        bytes.len() as u64,
    );
    assert!(service.negotiate_artifact(&peer, &offer).is_err());
    assert!(service.shutdown_workers(Duration::from_secs(2)).clean);
    core.verify_peer_execution_integrity()?;
    Ok(())
}

fn input_offer(
    base: ServingInvocationRequest,
    peer: &PeerId,
    identity: &str,
    bytes: &[u8],
) -> TestResult<ArtifactMetadataOffer> {
    let artifact = ArtifactReference::new(
        ArtifactId::new(identity)?,
        ContentDigest::for_bytes(bytes),
        MediaType::new("text/plain")?,
        bytes.len() as u64,
    );
    let invocation = InvocationRequest::new(
        base.request.invocation().clone(),
        base.request.capability().clone(),
        base.request.operation().clone(),
        base.request.provider_profile().cloned(),
        base.request.idempotency_key().cloned(),
        vec![InputReference::new(
            "source",
            InvocationValueReference::Artifact {
                reference: InvocationArtifactReference::new(
                    identity,
                    artifact.digest().to_hex(),
                    Some("text/plain".to_owned()),
                    Some(bytes.len() as u64),
                )?,
            },
        )?],
        BTreeMap::new(),
    )?;
    let request = ServingInvocationRequest::new(
        base.request_id,
        base.catalog_generation,
        base.catalog_digest,
        base.selection,
        invocation,
        base.limits,
        base.deadline_unix_ms,
        base.authorization,
    )?;
    Ok(ArtifactMetadataOffer {
        transfer: TransferId::new(format!("transfer:{identity}"))?,
        direction: ArtifactTransferDirection::Upload,
        artifact,
        sensitivity: ArtifactSensitivity::Restricted,
        retention: ArtifactRetention::WhileReferenced,
        provenance: ArtifactProvenance::new(
            CausalReference::Invocation {
                invocation: InvocationId::new("foreign-invocation")?,
            },
            vec![CausalReference::RunInput {
                run: milkdrift_workspace::RunId::new("foreign-run")?,
                key: milkdrift_workspace::ValueKey::new("foreign-key")?,
            }],
        )?,
        source_peer: peer.clone(),
        expires_at_unix_ms: request.deadline_unix_ms,
        binding: milkdrift_peer_protocol::ArtifactTransferBinding::Input {
            request: Box::new(request),
        },
    })
}

#[test]
fn artifact_clock_failure_rejects_chunks_without_publishing_bytes() -> TestResult {
    let root = tempfile::tempdir()?;
    let peer = PeerId::new("peer-artifact-clock")?;
    let bytes = b"x".to_vec();
    let reference = ArtifactReference::new(
        ArtifactId::new("peer-artifact-clock-boundary")?,
        ContentDigest::for_bytes(&bytes),
        MediaType::new("application/octet-stream")?,
        1,
    );
    let offer = ArtifactMetadataOffer {
        transfer: TransferId::new("transfer-artifact-clock")?,
        direction: ArtifactTransferDirection::Upload,
        artifact: reference.clone(),
        sensitivity: ArtifactSensitivity::Internal,
        retention: ArtifactRetention::WhileReferenced,
        provenance: ArtifactProvenance::new(
            CausalReference::External {
                source: CausalId::new("remote-clock-source")?,
            },
            Vec::new(),
        )?,
        source_peer: peer.clone(),
        binding: milkdrift_peer_protocol::ArtifactTransferBinding::Execution {
            execution: PeerExecutionId::new("execution-artifact-clock")?,
        },
        expires_at_unix_ms: 1_000_000,
    };
    let clock = Arc::new(ControlledPeerClock::new(offer.expires_at_unix_ms));
    let core = Arc::new(RedbStore::open(root.path())?);
    let transfer = CorePeerArtifactStore::new(core.clone(), 1_048_576, 2_097_152, clock.clone())?;
    assert!(matches!(
        transfer.negotiate(&peer, &offer, 1_048_576, None)?,
        ArtifactTransferDecision::Transfer { next_offset: 0, .. }
    ));

    clock.set_available(false)?;
    let chunk = ArtifactChunk {
        transfer: offer.transfer.clone(),
        offset: 0,
        bytes,
        final_chunk: true,
    };
    assert!(matches!(
        transfer.write_chunk(&peer, &chunk, 1_048_576),
        Err(PeerArtifactError::Unavailable)
    ));
    assert!(core.metadata(reference.artifact())?.is_none());

    clock.set_available(true)?;
    clock.set(offer.expires_at_unix_ms.saturating_add(1))?;
    assert!(matches!(
        transfer.write_chunk(&peer, &chunk, 1_048_576),
        Err(PeerArtifactError::Rejected(_))
    ));
    assert!(core.metadata(reference.artifact())?.is_none());
    Ok(())
}

#[test]
fn core_artifact_transfer_preserves_metadata_provenance_resumes_and_reads_outbound() -> TestResult {
    let root = tempfile::tempdir()?;
    let peer = PeerId::new("peer-a")?;
    let serving = PeerId::new("peer-b")?;
    let execution = PeerExecutionId::new("execution-artifact")?;
    let bytes = b"verified ordinary core artifact".to_vec();
    let reference = ArtifactReference::new(
        ArtifactId::new("peer-imported-artifact")?,
        ContentDigest::for_bytes(&bytes),
        MediaType::new("application/octet-stream")?,
        u64::try_from(bytes.len())?,
    );
    let provenance = ArtifactProvenance::new(
        CausalReference::External {
            source: CausalId::new("remote-source")?,
        },
        Vec::new(),
    )?;
    let offer = ArtifactMetadataOffer {
        transfer: TransferId::new("transfer-core")?,
        direction: ArtifactTransferDirection::Upload,
        artifact: reference.clone(),
        sensitivity: ArtifactSensitivity::Internal,
        retention: ArtifactRetention::Indefinite,
        provenance: provenance.clone(),
        source_peer: peer.clone(),
        binding: milkdrift_peer_protocol::ArtifactTransferBinding::Execution {
            execution: execution.clone(),
        },
        expires_at_unix_ms: now().saturating_add(60_000),
    };

    let core = Arc::new(RedbStore::open(root.path())?);
    let transfer =
        CorePeerArtifactStore::new(core.clone(), 1_048_576, 2_097_152, system_peer_clock())?;
    assert!(
        transfer
            .negotiate(
                &peer,
                &offer,
                u64::try_from(bytes.len())?.saturating_sub(1),
                None
            )
            .is_err()
    );
    assert!(matches!(
        transfer.negotiate(&peer, &offer, 1_048_576, None)?,
        ArtifactTransferDecision::Transfer { next_offset: 0, .. }
    ));
    // Aborting an absent key in another caller's namespace is an idempotent no-op.
    // The original caller can still write and resume its independently owned upload.
    transfer.abort(&PeerId::new("peer-foreign")?, &offer.transfer)?;
    transfer.write_chunk(
        &peer,
        &ArtifactChunk {
            transfer: offer.transfer.clone(),
            offset: 0,
            bytes: bytes[..8].to_vec(),
            final_chunk: false,
        },
        1_048_576,
    )?;
    assert!(core.metadata(reference.artifact())?.is_none());
    drop(transfer);
    drop(core);

    let core = Arc::new(RedbStore::open(root.path())?);
    let transfer =
        CorePeerArtifactStore::new(core.clone(), 1_048_576, 2_097_152, system_peer_clock())?;
    assert!(matches!(
        transfer.negotiate(&peer, &offer, 1_048_576, None)?,
        ArtifactTransferDecision::Transfer { next_offset: 8, .. }
    ));
    assert_eq!(
        transfer.write_chunk(
            &peer,
            &ArtifactChunk {
                transfer: offer.transfer.clone(),
                offset: 8,
                bytes: bytes[8..].to_vec(),
                final_chunk: true,
            },
            1_048_576,
        )?,
        ArtifactTransferDecision::AlreadyPresent
    );
    let metadata = core
        .metadata(reference.artifact())?
        .ok_or("metadata missing")?;
    assert_eq!(metadata.sensitivity(), ArtifactSensitivity::Internal);
    assert_eq!(metadata.retention(), &ArtifactRetention::Indefinite);
    assert_eq!(
        metadata.provenance().producer(),
        &CausalReference::PeerClaim {
            peer: peer.clone(),
            reference: Box::new(provenance.producer().clone()),
        }
    );
    assert!(metadata.provenance().causes().is_empty());
    assert_eq!(
        transfer.negotiate(&peer, &offer, 1_048_576, None)?,
        ArtifactTransferDecision::AlreadyPresent
    );

    let download = ArtifactMetadataOffer {
        transfer: TransferId::new("transfer-download")?,
        direction: ArtifactTransferDirection::Download,
        artifact: reference,
        sensitivity: metadata.sensitivity(),
        retention: metadata.retention().clone(),
        provenance: metadata.provenance().clone(),
        source_peer: serving,
        binding: milkdrift_peer_protocol::ArtifactTransferBinding::Execution { execution },
        expires_at_unix_ms: now().saturating_add(60_000),
    };
    transfer.negotiate(&peer, &download, 1_048_576, None)?;
    let read = transfer.read_chunk(&peer, &download.transfer, 0, 1_048_576)?;
    assert_eq!(read.bytes, bytes);
    assert!(read.final_chunk);

    let corrupt_bytes = b"verified ordinary core artifacU".to_vec();
    assert_eq!(corrupt_bytes.len(), bytes.len());
    let corrupt_reference = ArtifactReference::new(
        ArtifactId::new("peer-corrupt-artifact")?,
        ContentDigest::for_bytes(&bytes),
        MediaType::new("application/octet-stream")?,
        u64::try_from(bytes.len())?,
    );
    let corrupt_offer = ArtifactMetadataOffer {
        transfer: TransferId::new("transfer-corrupt")?,
        direction: ArtifactTransferDirection::Upload,
        artifact: corrupt_reference.clone(),
        sensitivity: ArtifactSensitivity::Internal,
        retention: ArtifactRetention::Indefinite,
        provenance,
        source_peer: peer.clone(),
        binding: milkdrift_peer_protocol::ArtifactTransferBinding::Execution {
            execution: PeerExecutionId::new("execution-corrupt-artifact")?,
        },
        expires_at_unix_ms: now().saturating_add(60_000),
    };
    transfer.negotiate(&peer, &corrupt_offer, 1_048_576, None)?;
    assert!(
        transfer
            .write_chunk(
                &peer,
                &ArtifactChunk {
                    transfer: corrupt_offer.transfer.clone(),
                    offset: 0,
                    bytes: corrupt_bytes,
                    final_chunk: true,
                },
                1_048_576,
            )
            .is_err()
    );
    assert!(core.metadata(corrupt_reference.artifact())?.is_none());
    transfer.abort(&peer, &corrupt_offer.transfer)?;
    assert!(!root.path().join("peer-artifacts-v1").exists());
    assert!(!root.path().join("peer-executions-v1").exists());
    Ok(())
}

use super::faults::ControlledPeerClock;

#[test]
fn empty_peer_output_commits_exactly_and_rejects_a_false_empty_digest() -> TestResult {
    let root = tempfile::tempdir()?;
    let core = Arc::new(RedbStore::open(root.path())?);
    let transfer = CorePeerArtifactStore::new(core.clone(), 1024, 2048, system_peer_clock())?;
    let peer = PeerId::new("peer-empty-output")?;
    let mut offer = ArtifactMetadataOffer {
        transfer: TransferId::new("transfer-empty")?,
        direction: ArtifactTransferDirection::Upload,
        artifact: ArtifactReference::new(
            ArtifactId::new("empty-output")?,
            ContentDigest::for_bytes(b""),
            MediaType::new("text/plain")?,
            0,
        ),
        sensitivity: ArtifactSensitivity::Restricted,
        retention: ArtifactRetention::WhileReferenced,
        provenance: ArtifactProvenance::new(
            CausalReference::External {
                source: CausalId::new("peer-output-producer")?,
            },
            Vec::new(),
        )?,
        source_peer: peer.clone(),
        binding: milkdrift_peer_protocol::ArtifactTransferBinding::Execution {
            execution: PeerExecutionId::new("empty-execution")?,
        },
        expires_at_unix_ms: now() + 60_000,
    };
    for _ in 0..2 {
        assert_eq!(
            transfer.negotiate(&peer, &offer, 1024, None)?,
            ArtifactTransferDecision::AlreadyPresent
        );
    }
    assert_eq!(
        core.metadata(offer.artifact.artifact())?
            .ok_or("empty output missing")?
            .reference(),
        &offer.artifact
    );
    transfer.abort(&peer, &offer.transfer)?;
    offer.transfer = TransferId::new("false-empty-transfer")?;
    offer.artifact = ArtifactReference::new(
        ArtifactId::new("false-empty-output")?,
        ContentDigest::for_bytes(b"x"),
        MediaType::new("text/plain")?,
        0,
    );
    assert!(transfer.negotiate(&peer, &offer, 1024, None).is_err());
    assert!(core.metadata(offer.artifact.artifact())?.is_none());
    Ok(())
}

#[test]
fn entered_peer_execution_owns_output_budget_and_metadata_requires_live_download_authority()
-> TestResult {
    use milkdrift_capability_host::{
        AdapterExecutionContext, InvocationDataAccess, MaterializationLimits,
        StoreInvocationDataAccess,
    };
    use milkdrift_persistence::ArtifactReadAuthority;
    let root = tempfile::tempdir()?;
    let work = tempfile::tempdir()?;
    let core = Arc::new(RedbStore::open(root.path())?);
    let peer = PeerId::new("peer-output-owner")?;
    let target = PeerId::new("peer-output-host")?;
    let mut config = server_config(peer.clone(), target.clone(), 1, 4)?;
    let descriptor = descriptor()?;
    let catalog =
        milkdrift_peer_protocol::CatalogSnapshot::new(1, now(), now() + 60_000, Vec::new())?;
    core.set_peer_admission_open(true)?;
    core.configure_peer_relationship(&ServingCallerState {
        caller: milkdrift_peer_protocol::ServingCaller::peer(&target, &peer),
        generation: 1,
        enabled: true,
        expires_at_unix_ms: config.relationships[0].expires_at_unix_ms,
        maximum_active: u32::from(config.relationships[0].maximum_concurrent),
    })?;
    core.publish_peer_catalog(&ServingCatalogState {
        caller: milkdrift_peer_protocol::ServingCaller::peer(&target, &peer),
        relationship_generation: 1,
        generation: 1,
        digest: catalog.digest.as_str().to_owned(),
        expires_at_unix_ms: catalog.expires_at_unix_ms,
    })?;
    let mut request = request_with_input_artifact(
        &peer,
        &target,
        &descriptor,
        1,
        catalog.digest,
        "bounded-output-request",
        "bounded-output-invocation",
        1,
    )?;
    request.limits.artifact_bytes = 4;
    let request = ServingInvocationRequest::new(
        request.request_id,
        request.catalog_generation,
        request.catalog_digest,
        request.selection,
        request.request,
        request.limits,
        request.deadline_unix_ms,
        request.authorization,
    )?;
    let execution = PeerExecutionId::new("bounded-output-execution")?;
    admit(&core, &peer, &request, &execution, 1)?;
    let worker = WorkerId::new("output-worker")?;
    let claimed = claim(&core, &worker)?;
    let accepted_origin = request.authorization.origin();
    let origin = accepted_origin
        .workflow()
        .ok_or("workflow origin missing")?;
    let context = AdapterExecutionContext::new(
        milkdrift_workspace::RunId::new(&origin.run)?,
        serde_json::from_value(serde_json::Value::String(origin.revision.clone()))?,
        milkdrift_blueprint::NodeId::new(&origin.node)?,
        milkdrift_persistence::NodeExecutionId::new(&origin.execution)?,
        milkdrift_persistence::AttemptId::new(&origin.attempt)?,
    );
    assert!(
        milkdrift_capability_host::conformance::entered_serving_context(context.clone(), &claimed)
            .is_err()
    );
    enter(
        &core,
        &target,
        &peer,
        &execution,
        &worker,
        claimed.phase.claim().ok_or("claim missing")?.generation,
    )?;
    let PeerExecutionSnapshot::Hot(entered) = core
        .peer_execution(
            &milkdrift_peer_protocol::ServingCaller::peer(&target, &peer),
            &execution,
        )?
        .ok_or("entry missing")?
    else {
        return Err("entry archived".into());
    };
    let context =
        milkdrift_capability_host::conformance::entered_serving_context(context, &entered)?;
    let access = StoreInvocationDataAccess::new(
        core.clone(),
        work.path(),
        ArtifactReadAuthority::PublicOnly,
    )?;
    let limits = MaterializationLimits {
        max_files: 8,
        max_file_bytes: 1024,
        max_total_bytes: 1024,
        max_path_bytes: 256,
        max_directory_depth: 4,
        chunk_bytes: 1024,
    };
    let reference = access.publish_bytes(
        &context,
        &request.request,
        "result",
        "text/plain",
        b"abc",
        limits,
    )?;
    assert_eq!(
        access.publish_bytes(
            &context,
            &request.request,
            "result",
            "text/plain",
            b"abc",
            limits
        )?,
        reference
    );
    assert!(
        access
            .publish_bytes(
                &context,
                &request.request,
                "excess",
                "text/plain",
                b"x",
                limits
            )
            .is_err(),
        "input and output share the accepted allowance"
    );
    core.append_peer_observation(
        &milkdrift_peer_protocol::ServingCaller::peer(&target, &peer),
        &execution,
        &PeerObservation {
            execution: execution.clone(),
            sequence: 1,
            category: ObservationCategory::Artifact,
            event: InvocationEvent::new(
                request.request.invocation().clone(),
                1,
                InvocationEventKind::Output {
                    name: "result".to_owned(),
                    reference: reference.clone(),
                },
            )?,
            observed_at_unix_ms: now(),
        },
    )?;
    core.append_peer_observation(
        &milkdrift_peer_protocol::ServingCaller::peer(&target, &peer),
        &execution,
        &terminal_observation(&request, &execution, 2, TerminalStatus::Success)?,
    )?;
    let artifacts = Arc::new(CorePeerArtifactStore::new(
        core.clone(),
        1024,
        2048,
        system_peer_clock(),
    )?);
    let (host, _) = host_with_adapter(Arc::new(TerminalAdapter {
        capability: descriptor.identity().clone(),
        delay: Duration::ZERO,
        active: Arc::new(AtomicUsize::new(0)),
        maximum: Arc::new(AtomicUsize::new(0)),
        calls: Arc::new(AtomicUsize::new(0)),
        requirements: CapabilityExecutionRequirements::default(),
    }))?;
    let service = PeerService::new_with_artifacts(
        config.clone(),
        host.clone(),
        core.clone(),
        artifacts.clone(),
        system_peer_clock(),
    )?;
    let offer = service.output_artifact_offer(&peer, &execution, 1)?;
    assert_eq!(offer.artifact.digest().to_hex(), reference.digest());
    assert_eq!(offer.artifact.size_bytes(), 3);
    assert!(
        service
            .output_artifact_offer(&PeerId::new("unrelated-peer")?, &execution, 1)
            .is_err()
    );
    assert!(service.output_artifact_offer(&peer, &execution, 2).is_err());
    service.negotiate_artifact(&peer, &offer)?;
    service.revoke_peer(&peer)?;
    assert!(service.output_artifact_offer(&peer, &execution, 1).is_err());
    assert!(
        service
            .read_artifact_chunk(&peer, &offer.transfer, 0, 3)
            .is_err()
    );
    assert!(service.shutdown_workers(Duration::from_secs(1)).clean);
    config.relationships[0]
        .authority
        .actions
        .remove(&PeerAction::ArtifactDownload);
    config.relationships[0].revocation_generation = 2;
    let denied =
        PeerService::new_with_artifacts(config, host, core, artifacts, system_peer_clock())?;
    assert!(denied.output_artifact_offer(&peer, &execution, 1).is_err());
    assert!(denied.shutdown_workers(Duration::from_secs(1)).clean);
    Ok(())
}
