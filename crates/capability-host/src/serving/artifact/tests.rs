use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use milkdrift_capability::PeerId;
use milkdrift_peer_protocol::{
    ArtifactMetadataOffer, ArtifactTransferBinding, ArtifactTransferDecision,
    ArtifactTransferDirection, PeerExecutionId, TransferId,
};
use milkdrift_persistence::{ArtifactStore, PersistenceError};
use milkdrift_redb_store::{
    FaultInjector, FaultPoint, RedbStore, RedbStoreConfig, injected_failure,
};
use milkdrift_workspace::{
    ArtifactId, ArtifactProvenance, ArtifactReference, ArtifactRetention, ArtifactSensitivity,
    CausalId, CausalReference, ContentDigest, MediaType,
};

use super::{CorePeerArtifactStore, PeerArtifactStore};
use crate::{PeerClock, PeerClockError};

struct FixedClock;
impl PeerClock for FixedClock {
    fn now_unix_ms(&self) -> Result<u64, PeerClockError> {
        Ok(100)
    }
}

struct FailCommitAndAbort {
    commit: AtomicBool,
    abort: AtomicBool,
}
impl FaultInjector for FailCommitAndAbort {
    fn check(&self, point: FaultPoint) -> Result<(), PersistenceError> {
        let fail = match point {
            FaultPoint::BeforeArtifactMetadataCommit => self.commit.swap(false, Ordering::SeqCst),
            FaultPoint::BeforeArtifactAbortCommit => self.abort.swap(false, Ordering::SeqCst),
            _ => false,
        };
        if fail {
            Err(injected_failure(point))
        } else {
            Ok(())
        }
    }
}

#[test]
fn empty_transfer_reports_failed_abort_and_recovers_exactly()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let core = Arc::new(RedbStore::open_with_config(
        RedbStoreConfig::new(directory.path()).with_fault_injector(Arc::new(FailCommitAndAbort {
            commit: AtomicBool::new(true),
            abort: AtomicBool::new(true),
        })),
    )?);
    let peer = PeerId::new("peer-empty-artifact")?;
    let offer = ArtifactMetadataOffer {
        transfer: TransferId::new("transfer-empty-artifact")?,
        direction: ArtifactTransferDirection::Upload,
        artifact: ArtifactReference::new(
            ArtifactId::new("empty-artifact")?,
            ContentDigest::for_bytes(b""),
            MediaType::new("application/octet-stream")?,
            0,
        ),
        sensitivity: ArtifactSensitivity::Internal,
        retention: ArtifactRetention::WhileReferenced,
        provenance: ArtifactProvenance::new(
            CausalReference::External {
                source: CausalId::new("remote-empty-artifact")?,
            },
            Vec::new(),
        )?,
        source_peer: peer.clone(),
        binding: ArtifactTransferBinding::Execution {
            execution: PeerExecutionId::new("execution-empty-artifact")?,
        },
        expires_at_unix_ms: 200,
    };
    let transfers = CorePeerArtifactStore::new(core.clone(), 1024, 1024, Arc::new(FixedClock))?;
    let error = transfers
        .negotiate(&peer, &offer, 1024, None)
        .err()
        .ok_or("empty publication unexpectedly succeeded")?;
    assert!(
        error
            .to_string()
            .contains("publication cleanup also failed and the session may remain")
    );
    assert!(!core.is_committed(&offer.artifact)?);
    assert_eq!(
        transfers.negotiate(&peer, &offer, 1024, None)?,
        ArtifactTransferDecision::AlreadyPresent
    );
    assert!(core.is_committed(&offer.artifact)?);
    Ok(())
}
