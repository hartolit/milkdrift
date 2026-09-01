//! Core peer artifact negotiation, resumability, and verification behavior.

use super::support::*;

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
        execution: execution.clone(),
        expires_at_unix_ms: now().saturating_add(60_000),
    };

    let core = Arc::new(RedbStore::open(root.path())?);
    let transfer = CorePeerArtifactStore::new(core.clone(), 1_048_576, 2_097_152)?;
    assert!(
        transfer
            .negotiate(&peer, &offer, u64::try_from(bytes.len())?.saturating_sub(1),)
            .is_err()
    );
    assert!(matches!(
        transfer.negotiate(&peer, &offer, 1_048_576)?,
        ArtifactTransferDecision::Transfer { next_offset: 0, .. }
    ));
    assert!(
        transfer
            .abort(&PeerId::new("peer-foreign")?, &offer.transfer)
            .is_err()
    );
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
    let transfer = CorePeerArtifactStore::new(core.clone(), 1_048_576, 2_097_152)?;
    assert!(matches!(
        transfer.negotiate(&peer, &offer, 1_048_576)?,
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
        &CausalReference::External {
            source: CausalId::new("peer:peer-a/execution:execution-artifact")?,
        }
    );
    assert_eq!(metadata.provenance().causes().len(), 1);
    assert_eq!(&metadata.provenance().causes()[0], provenance.producer());
    assert_eq!(
        transfer.negotiate(&peer, &offer, 1_048_576)?,
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
        execution,
        expires_at_unix_ms: now().saturating_add(60_000),
    };
    transfer.negotiate(&peer, &download, 1_048_576)?;
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
        execution: PeerExecutionId::new("execution-corrupt-artifact")?,
        expires_at_unix_ms: now().saturating_add(60_000),
    };
    transfer.negotiate(&peer, &corrupt_offer, 1_048_576)?;
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
